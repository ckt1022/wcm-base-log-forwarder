#!/bin/bash
# latency.sh — 延遲測試環境（對應 latency-benchmark/forwarder.yaml，input.mode=tcp）
# 流程：啟動 HTTP servers → docker build 並啟動 forwarder container → 等待 5 秒 → 透過 TCP 產生 log
# forwarder 以 docker（--network host）執行，資源上限比照其他產品實驗
# （fluentd/vector/fluentbit：--cpus 1 / --memory 512m / --memory-swap 512m）

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$SCRIPT_DIR"

LOG_DIR="$SCRIPT_DIR/Rust_time_500_512_large/Rust_time_20_500"
RUN_NAME="$(basename "$LOG_DIR")"
YAML="$SCRIPT_DIR/forwarder.yaml"
SERVER_GO="$REPO_ROOT/tools/test/server.go"

IMAGE="wcm-base-log-forwarder:latest"
CONTAINER="wcm-latency"
DOCKER="/snap/docker/current/bin/docker"
GO="/snap/go/current/bin/go"

# 容器資源上限（可用環境變數覆寫，如 CPU_LIMIT=2 MEM_LIMIT=256m ./latency.sh）：
# --memory 配合 --memory-swap 同值 = 禁用 swap，讓 RSS 上限是硬限制
# 與 fluentd/vector/fluentbit 實驗（latency.sh / latency_lua.sh）同值
CPU_LIMIT="${CPU_LIMIT:-1}"
MEM_LIMIT="${MEM_LIMIT:-512m}"

# ─── 直接在這裡選擇輸入 workload，不從命令列／環境變數讀取 ───────────────
# 可選值：
#   160b        ：約 160 B，原始五條固定 log
#   256-few     ：固定 256 B，維持 5 個 top-level JSON 欄位
#   256-fields  ：固定 256 B，以 8–9 個 JSON 欄位補足，沒有 payload
#   512-few     ：固定 512 B，維持 5 個 top-level JSON 欄位
#   512-fields  ：固定 512 B，以 17–18 個 JSON 欄位補足，沒有 payload
#   1k-payload  ：固定 1024 B，以單一 payload 欄位補足大小
#   1k-fields   ：固定 1024 B，以 35–36 個 JSON 欄位補足，沒有 payload
#   2k-payload  ：固定 2048 B，以單一 payload 欄位補足大小
LOG_INPUT_MODE="512-fields"

case "$LOG_INPUT_MODE" in
  160b)
    GEN_MODE="json-fixed5"
    LOG_SIZE_BYTES=160
    ;;
  256-fields)
    GEN_MODE="json-fields256"
    LOG_SIZE_BYTES=256
    ;;
  256-few)
    GEN_MODE="json-fixed256"
    LOG_SIZE_BYTES=256
    ;;
  512-few)
    GEN_MODE="json-fixed512"
    LOG_SIZE_BYTES=512
    ;;
  512-fields)
    GEN_MODE="json-fields512"
    LOG_SIZE_BYTES=512
    ;;
  1k-payload)
    GEN_MODE="json-fixed1k"
    LOG_SIZE_BYTES=1024
    ;;
  1k-fields)
    GEN_MODE="json-fields1k"
    LOG_SIZE_BYTES=1024
    ;;
  2k-payload)
    GEN_MODE="json-fixed2k"
    LOG_SIZE_BYTES=2048
    ;;
  *)
    echo "[test] ERROR: LOG_INPUT_MODE 只支援 160b、256-few、256-fields、512-few、512-fields、1k-payload、1k-fields、2k-payload，收到：$LOG_INPUT_MODE" >&2
    exit 2
    ;;
esac

# ─── 清理函式 ────────────────────────────────────────────────────────────────
SERVER_PIDS=()
LOGS_PID=""
_CLEANUP_DONE=0

cleanup() {
    (( _CLEANUP_DONE )) && return
    _CLEANUP_DONE=1

    echo ""
    echo "[test] 關閉所有背景程序..."

    # 先關 forwarder container（docker stop 送 SIGTERM，讓 pipeline 排清剩餘批次），
    # 再關 server（SIGTERM 觸發寫出 CSV）
    "$DOCKER" stop -t 10 "$CONTAINER" >/dev/null 2>&1 || true
    "$DOCKER" rm "$CONTAINER"         >/dev/null 2>&1 || true

    [[ -n "$LOGS_PID" ]] && kill -TERM "$LOGS_PID" 2>/dev/null || true
    for pid in "${SERVER_PIDS[@]+"${SERVER_PIDS[@]}"}"; do
        kill -TERM "$pid" 2>/dev/null || true
    done

    # 等待最多 3 秒，之後補 SIGKILL
    local deadline=$(( SECONDS + 3 ))
    while (( SECONDS < deadline )); do
        local alive=0
        [[ -n "$LOGS_PID" ]] && kill -0 "$LOGS_PID" 2>/dev/null && alive=1
        for pid in "${SERVER_PIDS[@]+"${SERVER_PIDS[@]}"}"; do
            kill -0 "$pid" 2>/dev/null && alive=1
        done
        (( alive )) || break
        sleep 0.5
    done

    [[ -n "$LOGS_PID" ]] && kill -9 "$LOGS_PID" 2>/dev/null || true
    for pid in "${SERVER_PIDS[@]+"${SERVER_PIDS[@]}"}"; do
        kill -9 "$pid" 2>/dev/null || true
    done

    wait 2>/dev/null || true
    echo "[test] 清理完成。"
}
trap cleanup EXIT INT TERM

mkdir -p "$LOG_DIR"

# ─── 清空舊 log ───────────────────────────────────────────────────────────────
echo "[test] 清空舊 log..."
rm -f "$LOG_DIR"/*.log "$LOG_DIR"/*.csv
rm -rf "$LOG_DIR"/csv_*
"$DOCKER" rm -f "$CONTAINER" >/dev/null 2>&1 || true
echo "[test] 清空完成。"

# ─── 步驟 0：解析 yaml ───────────────────────────────────────────────────────
echo "[test] 解析 $YAML ..."

ENDPOINT_LIST=$(python3 -c '
import re, sys
with open(sys.argv[1]) as f:
    content = f.read()
in_ep = False
for line in content.split("\n"):
    if re.match(r"^endpoint:", line):
        in_ep = True
        continue
    if in_ep:
        if line and line[0] not in (" ", "\t"):
            break
        m = re.match(r"\s+(\S+):\s*\"?http://[\d.]+:(\d+)", line)
        if m:
            print(m.group(1), m.group(2))
' "$YAML")

declare -A ENDPOINTS
while IFS=' ' read -r key port; do
    [[ -n "$key" ]] && ENDPOINTS["$key"]="$port"
done <<< "$ENDPOINT_LIST"

if [[ ${#ENDPOINTS[@]} -eq 0 ]]; then
    echo "[test] ERROR: 無法從 yaml 解析到任何 endpoint"
    exit 1
fi

# input.mode=tcp：解析 forwarder 監聽的 TCP host/port（log generator 透過 nc 送入）
TCP_HOST=$(python3 -c '
import re, sys
with open(sys.argv[1]) as f:
    content = f.read()
m = re.search(r"tcp:\s*\n\s*host:\s*\"([^\"]+)\"", content)
print(m.group(1) if m else "127.0.0.1")
' "$YAML")
TCP_PORT=$(python3 -c '
import re, sys
with open(sys.argv[1]) as f:
    content = f.read()
m = re.search(r"tcp:\s*\n\s*host:\s*\"[^\"]+\"\s*\n\s*port:\s*(\d+)", content)
print(m.group(1) if m else "5140")
' "$YAML")
# 0.0.0.0 代表監聽所有介面，generator 送資料時改連 127.0.0.1
[[ "$TCP_HOST" == "0.0.0.0" ]] && TCP_HOST="127.0.0.1"

echo "[test] 找到 ${#ENDPOINTS[@]} 個 endpoint："
for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do
    echo "[test]   endpoint $key → port ${ENDPOINTS[$key]}  →  server_log/${key}.log"
done
echo "[test] forwarder TCP input：$TCP_HOST:$TCP_PORT"

# ─── 步驟 1：編譯並啟動 HTTP servers ────────────────────────────────────────
echo ""
echo "[test] ══ 步驟 1：編譯並啟動 HTTP servers ══"

# 預先 build server binary（go run 不轉發 SIGTERM，必須直接執行 binary）
SERVER_BIN="$LOG_DIR/server_bin"
echo "[test] 編譯 server.go → $SERVER_BIN"
"$GO" build -o "$SERVER_BIN" "$SERVER_GO"

for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do
    port="${ENDPOINTS[$key]}"
    logfile="$LOG_DIR/${key}.log"
    : > "$logfile"
    # 直接執行 binary，SIGTERM 可正確觸發 shutdown + 寫出 CSV。
    # SERVER_OUT_DIR：CSV 寫到 LOG_DIR 下的 per-endpoint 子目錄
    # （server 固定輸出 latency_worker_1.csv，分目錄避免多 endpoint 互相覆寫），
    # 測試結束後由統計步驟改名為 ${RUN_NAME}.csv 收進 LOG_DIR。
    mkdir -p "$LOG_DIR/csv_${key}"
    SERVER_OUT_DIR="$LOG_DIR/csv_${key}" "$SERVER_BIN" "$port" >> "$logfile" 2>&1 &
    SERVER_PIDS+=($!)
    echo "[test]   EP-$key  port=$port  pid=${SERVER_PIDS[-1]}  log=$logfile"
done

# ─── 步驟 2：docker build 並啟動 forwarder container ─────────────────────────
echo ""
echo "[test] ══ 步驟 2：使用固定映像（$IMAGE）══"
"$DOCKER" image inspect "$IMAGE" >/dev/null

echo ""
echo "[test] 啟動 forwarder container（cpus=$CPU_LIMIT  mem=$MEM_LIMIT，比照其他產品實驗）"
: > "$LOG_DIR/forwarder.log"
# --network host：forwarder 在容器內監聽 5140、直連 host 上的 HTTP server（127.0.0.1:8080）
# 設定檔與 plugin 以唯讀掛載進容器；plugin 路徑由 forwarder 以「設定檔目錄/plugin/」解析
"$DOCKER" run -d \
    --name "$CONTAINER" \
    --network host \
    --cpus "$CPU_LIMIT" \
    --memory "$MEM_LIMIT" \
    --memory-swap "$MEM_LIMIT" \
    -v "$YAML:/app/forwarder.yaml:ro" \
    -v "$SCRIPT_DIR/plugin:/app/plugin:ro" \
    "$IMAGE" >/dev/null

"$DOCKER" logs -f "$CONTAINER" >> "$LOG_DIR/forwarder.log" 2>&1 &
LOGS_PID=$!
echo "[test] forwarder 已啟動  container=$CONTAINER  cpus=$CPU_LIMIT  mem=$MEM_LIMIT  log=$LOG_DIR/forwarder.log"

# 等待 5 秒讓 forwarder 完成初始化（載入 WASM 插件、開啟 TCP listener）
echo ""
printf "[test] 等待 5 秒讓系統完成初始化"
for _ in 1 2 3 4 5; do sleep 1; printf "."; done
echo "  完成！"

# ─── 步驟 3：產生測試 log ─────────────────────────────────────────────────────
echo ""
echo "[test] ══ 步驟 3：產生測試 log ══"
echo "[test]   rate=500/s  duration=300s  traffic=flat  input=$LOG_INPUT_MODE  mode=$GEN_MODE  log_size=${LOG_SIZE_BYTES}B  send-unit=line"
echo "[test]   送入：$TCP_HOST:$TCP_PORT (直連 TCP，無 nc)"
echo ""
echo "[test] ─── 即時監控指令（在另一個終端執行）─────────────────────────────"
echo "[test]   # 各別 server："
for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do
    echo "[test]     tail -f $LOG_DIR/${key}.log"
done
LOG_FILES=$(for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do printf "$LOG_DIR/${key}.log "; done)
echo "[test]   # 同時監看所有 server："
echo "[test]     tail -f ${LOG_FILES% }"
echo "[test]   # Forwarder pipeline 狀態："
echo "[test]     tail -f $LOG_DIR/forwarder.log"
echo "[test] ────────────────────────────────────────────────────────────────"
echo ""

"$GO" run "$REPO_ROOT/tools/gen/main.go" \
    -rate 500 \
    -duration 300 \
    -traffic flat \
    -mode "$GEN_MODE" \
    -output tcp \
    -send-unit line \
    -tcp-addr "$TCP_HOST:$TCP_PORT" \
    -log-file "$LOG_DIR/gen.log"

echo ""
echo "[test] Log 產生完畢，等待 forwarder 處理剩餘批次（10 秒）..."
sleep 10

# ─── 收攏 CSV 並計算穩態延遲統計 ──────────────────────────────────────────────
# 明確觸發 cleanup（SIGTERM server → 寫出 latency CSV），再收 CSV 進 LOG_DIR
cleanup

echo ""
echo "[test] ══ 穩態延遲統計（calc_latency.py，取第 30~230 秒）══"
for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do
    src="$LOG_DIR/csv_${key}/latency_worker_1.csv"
    # 單一 endpoint 時命名 ${RUN_NAME}.csv（與 fluentd/vector 實驗一致）；
    # 多 endpoint 時加 _${key} 後綴避免覆寫
    if (( ${#ENDPOINTS[@]} == 1 )); then
        CSV_FILE="$LOG_DIR/${RUN_NAME}.csv"
    else
        CSV_FILE="$LOG_DIR/${RUN_NAME}_${key}.csv"
    fi
    if [[ -f "$src" ]]; then
        mv "$src" "$CSV_FILE"
        rmdir "$LOG_DIR/csv_${key}" 2>/dev/null || true
        # 穩態窗口取第 30~230 秒（避開暖機與收尾），結果同步追加到該 endpoint 的 server log
        python3 "$REPO_ROOT/test/compare/calc_latency.py" "$CSV_FILE" 30 230 \
            | tee -a "$LOG_DIR/${key}.log" || echo "[test] WARNING: 延遲統計計算失敗（EP-$key）"
    else
        echo "[test] WARNING: 找不到 $src（server 可能未正常關閉）"
    fi
done

echo ""
echo "[test] ══════════ 測試完成 ══════════"
echo "[test] 結果檔案："
for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do
    count=$(grep -c "" "$LOG_DIR/${key}.log" 2>/dev/null || echo 0)
    echo "[test]   $LOG_DIR/${key}.log  ($count 行，含 30~230 秒穩態統計)"
done
for csvf in "$LOG_DIR/${RUN_NAME}"*.csv; do
    [[ -f "$csvf" ]] && echo "[test]   $csvf"
done
fwd_count=$(grep -c "" "$LOG_DIR/forwarder.log" 2>/dev/null || echo 0)
echo "[test]   $LOG_DIR/forwarder.log  ($fwd_count 行)"
