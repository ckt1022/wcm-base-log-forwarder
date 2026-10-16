#!/bin/bash
# latency.sh — Fluentd + Ruby filter 延遲測試
# 對照組：test/ours/latency-benchmark-C/latency.sh、
#         test/compare/fluentbit/latency/script/latency_lua.sh（同 server、同 gen、同時序）
# 只有 forwarder 本體不同：Fluentd 以 docker（--network host）啟動
# 流程：時鐘檢查 → 啟動 HTTP server → 啟動 fluentd → 等就緒 + 5 秒暖機 → gen 300 秒 → drain 10 秒
#
# 用法：./latency.sh [flush_interval]
#   flush_interval：批次窗秒數（預設 0.1），以 FLUSH_INTERVAL 環境變數注入 conf；
#   結果寫到 fluentd_time/fluentd_time_<flush_interval>_500/

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAT_DIR="$SCRIPT_DIR"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

FLUSH_INTERVAL="${1:-0.1}"
LOG_DIR="$LAT_DIR/fluentd_time/fluentd_time_${FLUSH_INTERVAL}_500"
RUN_NAME="$(basename "$LOG_DIR")"
CONF="$LAT_DIR/conf/latency.conf"
PLUGINS_DIR="$LAT_DIR/plugins"
SERVER_GO="$REPO_ROOT/tools/test/server.go"

IMAGE="fluent/fluentd:v1.16-debian-1"
CONTAINER="fd-latency"

# 容器資源上限（可用環境變數覆寫，如 CPU_LIMIT=1 MEM_LIMIT=256m ./latency.sh）：
# --memory 配合 --memory-swap 同值 = 禁用 swap，讓 RSS 上限是硬限制
# 與 fluent-bit 實驗（latency_lua.sh / latency_wasm.sh）同值
CPU_LIMIT="${CPU_LIMIT:-1}"
MEM_LIMIT="${MEM_LIMIT:-512m}"

TCP_ADDR="127.0.0.1:5140"   # 對應 conf <source> tcp
SERVER_PORT="8080"          # 對應 conf <match> http

# ─── 時鐘健康檢查（見 setting.md：時鐘被 NTP 回撥會污染延遲數據）────────────
clock_check() {
    local cs tsd ok=1
    cs=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource 2>/dev/null || echo unknown)
    tsd=$(systemctl is-active systemd-timesyncd 2>/dev/null || true)
    if [[ "$cs" != "hyperv_clocksource_tsc_page" ]]; then
        echo "[clock] WARNING: clocksource=$cs（預期 hyperv_clocksource_tsc_page，WSL 重啟後需重新設定）"
        ok=0
    fi
    if [[ "$tsd" == "active" ]]; then
        echo "[clock] WARNING: systemd-timesyncd 仍在運行，會每 32 秒回撥時鐘"
        ok=0
    fi
    if (( ! ok )); then
        echo "[clock] 修正方式："
        echo "[clock]   echo hyperv_clocksource_tsc_page | sudo tee /sys/devices/system/clocksource/clocksource0/current_clocksource"
        echo "[clock]   sudo systemctl disable --now systemd-timesyncd"
        echo "[clock] 若要強行繼續（數據可能被污染）：SKIP_CLOCK_CHECK=1 $0"
        [[ "${SKIP_CLOCK_CHECK:-0}" == "1" ]] || exit 1
    fi
}
clock_check

# ─── 確認 image：本地已有就直接用，沒有才拉取（版本已釘死，內容不會變）──────
if sudo docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "[test] image 已存在，直接使用：$IMAGE"
else
    echo "[test] image 不存在，開始拉取：$IMAGE"
    sudo docker pull "$IMAGE"
fi

# ─── 清理函式（同 latency.sh：SIGTERM → 最多等 8 秒 → SIGKILL）───────────────
SERVER_PID=""
LOGS_PID=""
STATS_PID=""
_CLEANUP_DONE=0

cleanup() {
    (( _CLEANUP_DONE )) && return
    _CLEANUP_DONE=1

    echo ""
    echo "[test] 關閉所有背景程序..."

    # 先關 fluentd（docker stop 送 SIGTERM，flush_at_shutdown 排清 buffer），
    # 再關 server（SIGTERM 觸發寫出 CSV）
    sudo docker stop -t 10 "$CONTAINER" >/dev/null 2>&1 || true
    sudo docker rm "$CONTAINER"         >/dev/null 2>&1 || true

    [[ -n "$STATS_PID" ]]  && kill -TERM "$STATS_PID"  2>/dev/null || true
    [[ -n "$LOGS_PID" ]]   && kill -TERM "$LOGS_PID"   2>/dev/null || true
    [[ -n "$SERVER_PID" ]] && kill -TERM "$SERVER_PID" 2>/dev/null || true

    local deadline=$(( SECONDS + 8 ))
    while (( SECONDS < deadline )); do
        local alive=0
        [[ -n "$STATS_PID" ]]  && kill -0 "$STATS_PID"  2>/dev/null && alive=1
        [[ -n "$LOGS_PID" ]]   && kill -0 "$LOGS_PID"   2>/dev/null && alive=1
        [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null && alive=1
        (( alive )) || break
        sleep 0.5
    done

    [[ -n "$STATS_PID" ]]  && kill -9 "$STATS_PID"  2>/dev/null || true
    [[ -n "$LOGS_PID" ]]   && kill -9 "$LOGS_PID"   2>/dev/null || true
    [[ -n "$SERVER_PID" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true

    wait 2>/dev/null || true
    echo "[test] 清理完成。"
}
trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR"

echo "[test] 清空舊 log..."
rm -f "$LOG_DIR"/*.log
sudo docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
echo "[test] 清空完成。"

# ─── 步驟 1：編譯並啟動 HTTP server ──────────────────────────────────────────
echo ""
echo "[test] ══ 步驟 1：編譯並啟動 HTTP server ══"
SERVER_BIN="$LOG_DIR/server_bin"
echo "[test] 編譯 server.go → $SERVER_BIN"
go build -o "$SERVER_BIN" "$SERVER_GO"

# SERVER_OUT_DIR：CSV 寫到 $LOG_DIR/latency_worker_1.csv
( cd "$LAT_DIR" && SERVER_OUT_DIR="$LOG_DIR" exec "$SERVER_BIN" "$SERVER_PORT" ) >> "$LOG_DIR/A.log" 2>&1 &
SERVER_PID=$!
echo "[test]   EP-A  port=$SERVER_PORT  pid=$SERVER_PID  log=$LOG_DIR/A.log"

# ─── 步驟 2：啟動 Fluentd（docker，host network）─────────────────────────────
echo ""
echo "[test] ══ 步驟 2：啟動 Fluentd（$IMAGE）══"
sudo docker run -d \
    --name "$CONTAINER" \
    --network host \
    --cpus "$CPU_LIMIT" \
    --memory "$MEM_LIMIT" \
    --memory-swap "$MEM_LIMIT" \
    -e FLUSH_INTERVAL="$FLUSH_INTERVAL" \
    -v "$CONF:/fluentd/etc/fluent.conf:ro" \
    -v "$PLUGINS_DIR:/fluentd/plugins:ro" \
    "$IMAGE" >/dev/null

: > "$LOG_DIR/fluentd.log"
sudo docker logs -f "$CONTAINER" >> "$LOG_DIR/fluentd.log" 2>&1 &
LOGS_PID=$!
echo "[test] fluentd 已啟動  container=$CONTAINER  flush_interval=${FLUSH_INTERVAL}s  cpus=$CPU_LIMIT  mem=$MEM_LIMIT  log=$LOG_DIR/fluentd.log"

# 等待 fluentd worker 就緒（Ruby 啟動比原生 binary 慢，先等就緒再進入統一暖機）
printf "[test] 等待 fluentd worker 就緒"
WAIT=0
until sudo docker logs "$CONTAINER" 2>&1 | grep -q "fluentd worker is now running"; do
    sleep 1
    printf "."
    WAIT=$(( WAIT + 1 ))
    if (( WAIT >= 60 )); then
        echo ""
        echo "[test] ERROR: fluentd 60 秒內未就緒，見 $LOG_DIR/fluentd.log"
        exit 1
    fi
done
echo "  完成（${WAIT}s）！"

# ─── 資源取樣：每秒記錄 fluentd worker 的 CPU / RSS 到 <資料夾名>_stats.csv ──
# fluentd 容器內是 tini → supervisor → worker 三層，實際處理 record 的是
# worker（args 帶 --under-supervisor）；找不到時退回容器主程序
FD_HOST_PID=$(sudo docker top "$CONTAINER" -eo pid,args 2>/dev/null \
    | awk '/under-supervisor/{print $1; exit}')
[[ -n "$FD_HOST_PID" ]] || FD_HOST_PID=$(sudo docker inspect -f '{{.State.Pid}}' "$CONTAINER")
STATS_CSV="$LOG_DIR/${RUN_NAME}_stats.csv"
"$REPO_ROOT/test/compare/sample_stats.sh" "$FD_HOST_PID" "$STATS_CSV" "$MEM_LIMIT" "$CPU_LIMIT" &
STATS_PID=$!
echo "[test] 資源取樣已啟動  target_pid=$FD_HOST_PID  csv=$STATS_CSV"

# 等待 5 秒完成初始化（同 latency.sh 的暖機時間）
echo ""
printf "[test] 等待 5 秒讓系統完成初始化"
for _ in 1 2 3 4 5; do sleep 1; printf "."; done
echo "  完成！"

# ─── 步驟 3：產生測試 log（參數與 latency.sh 完全相同）───────────────────────
echo ""
echo "[test] ══ 步驟 3：產生測試 log ══"
echo "[test]   rate=500/s  duration=300s  traffic=flat  mode=json-fixed5  send-unit=line"
echo "[test]   送入：$TCP_ADDR"
echo ""
echo "[test] ─── 即時監控指令（在另一個終端執行）─────────────────────────────"
echo "[test]     tail -f $LOG_DIR/A.log"
echo "[test]     tail -f $LOG_DIR/fluentd.log"
echo "[test] ────────────────────────────────────────────────────────────────"
echo ""

go run "$REPO_ROOT/tools/gen/main.go" \
    -rate 500 \
    -duration 300 \
    -traffic flat \
    -mode json-fixed5 \
    -output tcp \
    -send-unit line \
    -tcp-addr "$TCP_ADDR" \
    -log-file "$LOG_DIR/gen.log"

echo ""
echo "[test] Log 產生完畢，等待 fluentd 處理剩餘批次（10 秒）..."
sleep 10

# 明確觸發 cleanup（SIGTERM server → 寫出 CSV），CSV 以資料夾名稱命名
cleanup
CSV_FILE="$LOG_DIR/${RUN_NAME}.csv"
if [[ -f "$LOG_DIR/latency_worker_1.csv" ]]; then
    mv "$LOG_DIR/latency_worker_1.csv" "$CSV_FILE"
else
    echo "[test] WARNING: 找不到 $LOG_DIR/latency_worker_1.csv"
fi

# 穩態延遲統計：取第 30~230 秒（避開暖機與收尾），結果追加到 server 的 A.log
if [[ -f "$CSV_FILE" ]]; then
    python3 "$REPO_ROOT/test/compare/calc_latency.py" "$CSV_FILE" 30 230 \
        | tee -a "$LOG_DIR/A.log" || echo "[test] WARNING: 延遲統計計算失敗"
fi

echo ""
echo "[test] ══════════ 測試完成 ══════════"
echo "[test] 結果檔案："
count=$(grep -c "" "$LOG_DIR/A.log" 2>/dev/null || echo 0)
echo "[test]   $LOG_DIR/A.log  ($count 行，含 30~230 秒穩態統計)"
echo "[test]   $CSV_FILE"
echo "[test]   $LOG_DIR/${RUN_NAME}_stats.csv（每秒 CPU/RSS 取樣）"
echo "[test]   $LOG_DIR/fluentd.log"
echo "[test]   $LOG_DIR/gen.log"
