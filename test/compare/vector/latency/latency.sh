#!/bin/bash
# latency.sh — Vector + VRL remap 延遲測試
# 對照組：test/ours/latency-benchmark-Rust/latency.sh、
#         test/compare/fluentd/latency/latency.sh、
#         test/compare/fluentbit/latency/script/latency_lua.sh（同 server、同 gen、同時序）
# 只有 forwarder 本體不同：Vector 以 docker（--network host）啟動
# 流程：時鐘檢查 → 啟動 HTTP server → validate 設定 → 啟動 vector → 等就緒 + 5 秒暖機
#       → gen 300 秒 → drain 10 秒
#
# 用法：./latency.sh [flush_interval]
#   flush_interval：批次窗秒數（預設 0.1），以 FLUSH_INTERVAL 環境變數注入
#   conf/latency.yaml 的 batch.timeout_secs；結果寫到 vector_time/vector_time_<interval>_500/

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAT_DIR="$SCRIPT_DIR"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

FLUSH_INTERVAL="${1:-0.1}"
LOG_DIR="$LAT_DIR/vector_time/vector_time_${FLUSH_INTERVAL}_500"
CONF="$LAT_DIR/conf/latency.yaml"
SERVER_GO="$REPO_ROOT/tools/test/server.go"

# 釘版本（同 fluentd 實驗釘 v1.16-debian-1 的做法）：0.49.0 是本設定
# 驗證過的版本；要換版時以 IMAGE=timberio/vector:<tag> ./latency.sh 覆寫
IMAGE="${IMAGE:-timberio/vector:0.49.0-alpine}"
CONTAINER="vt-latency"

TCP_ADDR="127.0.0.1:5140"   # 對應 conf sources.tcp_in
SERVER_PORT="8080"          # 對應 conf sinks.out_http

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

# ─── 清理函式（同 latency.sh：SIGTERM → 最多等 8 秒 → SIGKILL）───────────────
SERVER_PID=""
LOGS_PID=""
_CLEANUP_DONE=0

cleanup() {
    (( _CLEANUP_DONE )) && return
    _CLEANUP_DONE=1

    echo ""
    echo "[test] 關閉所有背景程序..."

    # 先關 vector（docker stop 送 SIGTERM，graceful shutdown 排清 buffer），
    # 再關 server（SIGTERM 觸發寫出 CSV）
    sudo docker stop -t 10 "$CONTAINER" >/dev/null 2>&1 || true
    sudo docker rm "$CONTAINER"         >/dev/null 2>&1 || true

    [[ -n "$LOGS_PID" ]]   && kill -TERM "$LOGS_PID"   2>/dev/null || true
    [[ -n "$SERVER_PID" ]] && kill -TERM "$SERVER_PID" 2>/dev/null || true

    local deadline=$(( SECONDS + 8 ))
    while (( SECONDS < deadline )); do
        local alive=0
        [[ -n "$LOGS_PID" ]]   && kill -0 "$LOGS_PID"   2>/dev/null && alive=1
        [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null && alive=1
        (( alive )) || break
        sleep 0.5
    done

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

# ─── 步驟 2：validate 設定並啟動 Vector（docker，host network）──────────────
echo ""
echo "[test] ══ 步驟 2：啟動 Vector（$IMAGE）══"

# 先 validate：VRL/設定錯誤在這裡就攔下來，不要浪費一輪 300 秒實驗
if ! sudo docker run --rm \
    -e FLUSH_INTERVAL="$FLUSH_INTERVAL" \
    -v "$CONF:/etc/vector/vector.yaml:ro" \
    "$IMAGE" validate --no-environment /etc/vector/vector.yaml; then
    echo "[test] ERROR: vector validate 失敗，請檢查 $CONF"
    exit 1
fi

sudo docker run -d \
    --name "$CONTAINER" \
    --network host \
    -e FLUSH_INTERVAL="$FLUSH_INTERVAL" \
    -v "$CONF:/etc/vector/vector.yaml:ro" \
    "$IMAGE" --config /etc/vector/vector.yaml >/dev/null

: > "$LOG_DIR/vector.log"
sudo docker logs -f "$CONTAINER" >> "$LOG_DIR/vector.log" 2>&1 &
LOGS_PID=$!
echo "[test] vector 已啟動  container=$CONTAINER  flush_interval=${FLUSH_INTERVAL}s  log=$LOG_DIR/vector.log"

# 等待 vector 就緒（先等就緒再進入統一暖機）
printf "[test] 等待 vector 就緒"
WAIT=0
until sudo docker logs "$CONTAINER" 2>&1 | grep -q "Vector has started"; do
    sleep 1
    printf "."
    WAIT=$(( WAIT + 1 ))
    if (( WAIT >= 60 )); then
        echo ""
        echo "[test] ERROR: vector 60 秒內未就緒，見 $LOG_DIR/vector.log"
        exit 1
    fi
done
echo "  完成（${WAIT}s）！"

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
echo "[test]     tail -f $LOG_DIR/vector.log"
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
echo "[test] Log 產生完畢，等待 vector 處理剩餘批次（10 秒）..."
sleep 10

echo ""
echo "[test] ══════════ 測試完成 ══════════"
echo "[test] 結果檔案："
count=$(grep -c "" "$LOG_DIR/A.log" 2>/dev/null || echo 0)
echo "[test]   $LOG_DIR/A.log  ($count 行)"
echo "[test]   $LOG_DIR/latency_worker_1.csv（cleanup 時由 server 寫出）"
echo "[test]   $LOG_DIR/vector.log"
echo "[test]   $LOG_DIR/gen.log"
