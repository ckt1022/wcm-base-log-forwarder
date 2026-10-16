#!/bin/bash
# latency_wasm_80%.sh — Fluent Bit + Rust WASM filter 80% 容量點延遲測試
# 基於 latency_wasm.sh，變更對齊容量實驗（test/compare/capacity/）的固定變因：
#   - rate 19200 eps（= 容量 24,000 的 80%）× 300 秒
#   - CPU 1 core、mem 512MB、Flush 1s（由本腳本 sed 注入 conf，同 run_all_wasm.sh 機制）
#   - gen 用 batch 送流（500 行/20ms）+ 排程時間戳（-ts-mode sched，避免 coordinated omission）
#   - 統計用 calc_latency_stream.py（30~230 秒穩態窗，記憶體安全）
# 流程：時鐘檢查 → 確認 image → 啟動 HTTP server → 啟動 fluent-bit → 等就緒 + 5 秒暖機
#       → gen 300 秒 → drain 10 秒

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../../.." && pwd)"

LOG_DIR="$LAT_DIR/fluentbit_wasm_time/fluentbit_wasm_80%"
RUN_NAME="$(basename "$LOG_DIR")"
CONF="$LAT_DIR/conf/fluentbit_latency_wasm.conf"
WASM_FILE="$LAT_DIR/wasm_file/latency_filter.wasm"
SERVER_GO="$REPO_ROOT/tools/test/server.go"

# 釘版本（同 fluentd 釘 v1.16-debian-1、vector 釘 0.49.0 的做法）：
# 5.0.9 是 2026-07 查詢 Docker Hub 時的最新穩定版，之後每次實驗都固定用它
IMAGE="fluent/fluent-bit:5.0.9"
CONTAINER="fb-wasm-latency"

# 容器資源上限（可用環境變數覆寫，如 CPU_LIMIT=1 MEM_LIMIT=256m ./latency_wasm.sh）：
# --memory 配合 --memory-swap 同值 = 禁用 swap，讓 RSS 上限是硬限制
# 注意：MEM_LIMIT 需大於 Wasm_Heap_Size（conf 設 256MB），否則 heap 長滿前就會被 OOM kill
# 與容量實驗相同：1 core / 512MB / Flush 1s / 19200 eps（容量 24000 的 80%）
CPU_LIMIT="${CPU_LIMIT:-1}"
MEM_LIMIT="${MEM_LIMIT:-512m}"
FLUSH_INTERVAL="${FLUSH_INTERVAL:-1}"
RATE="${RATE:-19200}"
DURATION="${DURATION:-300}"

TCP_ADDR="127.0.0.1:5140"   # 對應 conf [INPUT] tcp
SERVER_PORT="8080"          # 對應 conf [OUTPUT] http

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

# ─── WASM filter：不存在就先建置 ─────────────────────────────────────────────
if [[ ! -f "$WASM_FILE" ]]; then
    echo "[test] $WASM_FILE 不存在，執行 build.sh ..."
    "$LAT_DIR/wasm_file/build.sh"
fi

# ─── Flush 窗注入（同 run_all_wasm.sh 的 sed 機制）───────────────────────────
sed -i -E "s|^( *Flush +).*|\1${FLUSH_INTERVAL}|" "$CONF"
echo "[test] conf Flush 已設為 ${FLUSH_INTERVAL}s"

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

    # 先關 fluent-bit（docker stop 送 SIGTERM，Grace 5 讓它排清 buffer），
    # 再關 server（SIGTERM 觸發寫出 CSV）；容器一律移除，下次實驗重新建立
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

# SERVER_OUT_DIR：CSV 直接寫到 $LOG_DIR（cleanup 送 SIGTERM 時由 server 寫出）
( cd "$LAT_DIR" && SERVER_OUT_DIR="$LOG_DIR" exec "$SERVER_BIN" "$SERVER_PORT" ) >> "$LOG_DIR/A.log" 2>&1 &
SERVER_PID=$!
echo "[test]   EP-A  port=$SERVER_PORT  pid=$SERVER_PID  log=$LOG_DIR/A.log"

# ─── 步驟 2：啟動 Fluent Bit（docker，host network）─────────────────────────
echo ""
echo "[test] ══ 步驟 2：啟動 Fluent Bit（$IMAGE, WASM filter）══"
# LAT_DIR 以相同路徑掛進容器、workdir 設在 conf/：
# [FILTER] WASM_Path 的相對路徑（../wasm_file/latency_filter.wasm）解析結果與原生執行一致
sudo docker run -d \
    --name "$CONTAINER" \
    --network host \
    --cpus "$CPU_LIMIT" \
    --memory "$MEM_LIMIT" \
    --memory-swap "$MEM_LIMIT" \
    -v "$LAT_DIR:$LAT_DIR:ro" \
    -w "$LAT_DIR/conf" \
    "$IMAGE" -c "$CONF" >/dev/null

: > "$LOG_DIR/fluentbit.log"
sudo docker logs -f "$CONTAINER" >> "$LOG_DIR/fluentbit.log" 2>&1 &
LOGS_PID=$!
echo "[test] fluent-bit 已啟動  container=$CONTAINER  cpus=$CPU_LIMIT  mem=$MEM_LIMIT  log=$LOG_DIR/fluentbit.log"

# 等待 fluent-bit engine 就緒（先等就緒再進入統一暖機）
# v5 不再輸出「[engine] started」，改以 output worker 啟動訊息為準（兩種格式都接受）
printf "[test] 等待 fluent-bit 就緒"
WAIT=0
until sudo docker logs "$CONTAINER" 2>&1 | grep -Eq "\[engine\] started|\[output:http:http.0\] worker #0 started"; do
    sleep 1
    printf "."
    WAIT=$(( WAIT + 1 ))
    if (( WAIT >= 60 )); then
        echo ""
        echo "[test] ERROR: fluent-bit 60 秒內未就緒，見 $LOG_DIR/fluentbit.log"
        exit 1
    fi
done
echo "  完成（${WAIT}s）！"

# ─── 資源取樣：每秒記錄 fluent-bit 的 CPU / RSS 到 <資料夾名>_stats.csv ──────
FB_HOST_PID=$(sudo docker inspect -f '{{.State.Pid}}' "$CONTAINER")
STATS_CSV="$LOG_DIR/${RUN_NAME}_stats.csv"
"$REPO_ROOT/test/compare/sample_stats.sh" "$FB_HOST_PID" "$STATS_CSV" "$MEM_LIMIT" "$CPU_LIMIT" &
STATS_PID=$!
echo "[test] 資源取樣已啟動  target_pid=$FB_HOST_PID  csv=$STATS_CSV"

# 等待 5 秒完成初始化（同 latency.sh 的暖機時間）
echo ""
printf "[test] 等待 5 秒讓系統完成初始化"
for _ in 1 2 3 4 5; do sleep 1; printf "."; done
echo "  完成！"

# ─── 步驟 3：產生測試 log（參數與 latency.sh 完全相同）───────────────────────
echo ""
echo "[test] ══ 步驟 3：產生測試 log ══"
echo "[test]   rate=${RATE}/s  duration=${DURATION}s  traffic=flat  mode=json-fixed5  send-unit=batch  ts-mode=sched"
echo "[test]   送入：$TCP_ADDR"
echo ""
echo "[test] ─── 即時監控指令（在另一個終端執行）─────────────────────────────"
echo "[test]     tail -f $LOG_DIR/A.log"
echo "[test]     tail -f $LOG_DIR/fluentbit.log"
echo "[test] ────────────────────────────────────────────────────────────────"
echo ""

# gen 綁到與 DUT 不同的核；batch 送流 + 排程時間戳（與容量實驗相同的 gen 參數）
taskset -c 9,10 go run "$REPO_ROOT/tools/gen/main.go" \
    -rate "$RATE" \
    -duration "$DURATION" \
    -traffic flat \
    -mode json-fixed5 \
    -ts-mode sched \
    -output tcp \
    -send-unit batch -batch-lines 500 -flush-ms 20 \
    -tcp-addr "$TCP_ADDR" \
    -log-file "$LOG_DIR/gen.log"

# gen 結尾兩行含達成速率與 send_lag 直方圖（無排隊驗證證據），附到 A.log
tail -2 "$LOG_DIR/gen.log" | tee -a "$LOG_DIR/A.log"

echo ""
echo "[test] Log 產生完畢，等待 fluent-bit 處理剩餘批次（10 秒）..."
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
# 高流量筆數多（19200×40%×300 ≈ 2.3M 筆），用串流版統計避免記憶體尖峰
if [[ -f "$CSV_FILE" ]]; then
    python3 "$REPO_ROOT/test/compare/capacity/calc_latency_stream.py" "$CSV_FILE" 30 230 \
        | tee -a "$LOG_DIR/A.log" || echo "[test] WARNING: 延遲統計計算失敗"
fi

echo ""
echo "[test] ══════════ 測試完成 ══════════"
echo "[test] 結果檔案："
count=$(grep -c "" "$LOG_DIR/A.log" 2>/dev/null || echo 0)
echo "[test]   $LOG_DIR/A.log  ($count 行，含 30~230 秒穩態統計)"
echo "[test]   $CSV_FILE"
echo "[test]   $LOG_DIR/${RUN_NAME}_stats.csv（每秒 CPU/RSS 取樣）"
echo "[test]   $LOG_DIR/fluentbit.log"
echo "[test]   $LOG_DIR/gen.log"
