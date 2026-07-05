#!/bin/bash
# latency.sh — 延遲測試環境（對應 latency-benchmark/forwarder.yaml，input.mode=tcp）
# 流程：啟動 HTTP servers → release 編譯並啟動 forwarder → 等待 5 秒 → 透過 TCP 產生 log

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$SCRIPT_DIR"

LOG_DIR="$SCRIPT_DIR/server_log"
YAML="$SCRIPT_DIR/forwarder.yaml"
SERVER_GO="$REPO_ROOT/tools/test/server.go"
FORWARDER_BIN="$REPO_ROOT/target/release/wcm-base-log-forwarder"

# ─── 清理函式 ────────────────────────────────────────────────────────────────
SERVER_PIDS=()
FORWARDER_PID=""
_TEE_PID=""
_CLEANUP_DONE=0

cleanup() {
    (( _CLEANUP_DONE )) && return
    _CLEANUP_DONE=1

    echo ""
    echo "[test] 關閉所有背景程序..."

    # 先送 SIGTERM，給程序機會優雅結束
    [[ -n "$FORWARDER_PID" ]] && kill -TERM "$FORWARDER_PID" 2>/dev/null || true
    [[ -n "$_TEE_PID" ]]      && kill -TERM "$_TEE_PID"      2>/dev/null || true
    for pid in "${SERVER_PIDS[@]+"${SERVER_PIDS[@]}"}"; do
        kill -TERM "$pid" 2>/dev/null || true
    done

    # 等待最多 3 秒，之後補 SIGKILL
    local deadline=$(( SECONDS + 3 ))
    while (( SECONDS < deadline )); do
        local alive=0
        [[ -n "$FORWARDER_PID" ]] && kill -0 "$FORWARDER_PID" 2>/dev/null && alive=1
        [[ -n "$_TEE_PID" ]]      && kill -0 "$_TEE_PID"      2>/dev/null && alive=1
        for pid in "${SERVER_PIDS[@]+"${SERVER_PIDS[@]}"}"; do
            kill -0 "$pid" 2>/dev/null && alive=1
        done
        (( alive )) || break
        sleep 0.5
    done

    [[ -n "$FORWARDER_PID" ]] && kill -9 "$FORWARDER_PID" 2>/dev/null || true
    [[ -n "$_TEE_PID" ]]      && kill -9 "$_TEE_PID"      2>/dev/null || true
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
rm -f "$LOG_DIR"/*.log
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
go build -o "$SERVER_BIN" "$SERVER_GO"

for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do
    port="${ENDPOINTS[$key]}"
    logfile="$LOG_DIR/${key}.log"
    : > "$logfile"
    # 直接執行 binary，SIGTERM 可正確觸發 shutdown + 寫出 CSV
    "$SERVER_BIN" "$port" >> "$logfile" 2>&1 &
    SERVER_PIDS+=($!)
    echo "[test]   EP-$key  port=$port  pid=${SERVER_PIDS[-1]}  log=$logfile"
done

# ─── 步驟 2：release 編譯並啟動 forwarder ────────────────────────────────────
echo ""
echo "[test] ══ 步驟 2：cargo build --release ══"
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"

echo ""
echo "[test] 啟動 forwarder  (stdout → 終端 + $LOG_DIR/forwarder.log)"
: > "$LOG_DIR/forwarder.log"
# 以具名 fd 開啟 tee 並取得其 PID。
# 必須用 exec {fd}> >(tee ...) 而非直接 > >(tee ...) &：
# 後者在某些 bash 版本下 parent 不會關閉 pipe 的寫端副本，
# 導致 forwarder 被 kill 後 tee 收不到 EOF 而永久阻塞，wait 永不返回。
exec {_TEE_IN}> >(tee -a "$LOG_DIR/forwarder.log")
_TEE_PID=$!
"$FORWARDER_BIN" >&$_TEE_IN 2>&1 &
FORWARDER_PID=$!
# 關閉 parent 的寫端副本，確保 forwarder 結束時 tee 收到 EOF 並自動退出
exec {_TEE_IN}>&-
echo "[test] forwarder 已啟動  pid=$FORWARDER_PID  tee_pid=$_TEE_PID"

# 等待 5 秒讓 forwarder 完成初始化（載入 WASM 插件、開啟 TCP listener）
echo ""
printf "[test] 等待 5 秒讓系統完成初始化"
for _ in 1 2 3 4 5; do sleep 1; printf "."; done
echo "  完成！"

# ─── 步驟 3：產生測試 log ─────────────────────────────────────────────────────
echo ""
echo "[test] ══ 步驟 3：產生測試 log ══"
echo "[test]   rate=500/s  duration=180s  traffic=flat  mode=json-fixed5  send-unit=line"
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

go run "$REPO_ROOT/tools/gen/main.go" \
    -rate 500 \
    -duration 300 \
    -traffic flat \
    -mode json-fixed5 \
    -output tcp \
    -send-unit line \
    -tcp-addr "$TCP_HOST:$TCP_PORT" \
    -log-file "$LOG_DIR/gen.log"

echo ""
echo "[test] Log 產生完畢，等待 forwarder 處理剩餘批次（10 秒）..."
sleep 10

echo ""
echo "[test] ══════════ 測試完成 ══════════"
echo "[test] 結果檔案："
for key in $(printf '%s\n' "${!ENDPOINTS[@]}" | sort); do
    count=$(grep -c "" "$LOG_DIR/${key}.log" 2>/dev/null || echo 0)
    echo "[test]   $LOG_DIR/${key}.log  ($count 行)"
done
fwd_count=$(grep -c "" "$LOG_DIR/forwarder.log" 2>/dev/null || echo 0)
echo "[test]   $LOG_DIR/forwarder.log  ($fwd_count 行)"
