#!/bin/bash
# latency_lua_80%.sh — Fluent Bit + Lua filter 80% 容量點延遲測試
#
# 容量實驗(test/compare/capacity/search.sh)結果:
#   Fluent Bit Lua 最大可持續 = 296,000 eps(1 core / 512MB / flush 1s)
#   本腳本以 80% = 236,800 eps 打 300 秒,輸出 30~230 秒穩態延遲統計。
#
# 與低流量 latency_lua.sh 的差異(高流量必要):
#   - 委派給 test/compare/capacity/capacity.sh(同一套 docker 啟動與判定;
#     conf 的 [SERVICE] Flush 由該腳本 sed 注入,同 run_all_lua.sh 機制)
#   - 量測端用 capacity/httpsink(server.go 的高流量版:延遲定義相同 =
#     讀完 HTTP body 的 time.Now() − 每行 timestamp;236,800×40%×300s ≈ 2,840 萬筆,
#     server.go 的字串記錄會吃 >2GB 導致 WSL OOM)
#   - gen 用 batch 送流(500 行/20ms)+ 排程時間戳(-ts-mode sched),
#     結尾 send_lag 直方圖 = 無 coordinated omission 的驗證證據
#   - 統計用 calc_latency_stream.py(串流,千萬筆不爆記憶體)
#
# 用法: ./latency_lua_80%.sh
#   可覆寫: RATE=... FLUSH_INTERVAL=... CPU_LIMIT=... MEM_LIMIT=...
#   docker 無權限時: 先 sudo chmod 666 /var/run/docker.sock,或 DOCKER="sudo docker" ./latency_lua_80%.sh
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../../.." && pwd)"
CAP_DIR="$REPO_ROOT/test/compare/capacity"

DUT="fbit-lua"
RATE="${RATE:-236800}"
export FLUSH_INTERVAL="${FLUSH_INTERVAL:-1}"
export CPU_LIMIT="${CPU_LIMIT:-1}"
export MEM_LIMIT="${MEM_LIMIT:-512m}"

"$CAP_DIR/capacity.sh" "$DUT" "$RATE" lat

RUN_DIR="$CAP_DIR/runs/$DUT/lat_rate_$RATE"
OUT_DIR="$LAT_DIR/fluentbit_lua_time/fluentbit_lua_80%_${RATE}"
mkdir -p "$OUT_DIR"

# 穩態延遲統計(30~230 秒)存檔,並把驗證證據一併帶回本產品的結果資料夾
python3 "$CAP_DIR/calc_latency_stream.py" "$RUN_DIR/latency_worker_1.csv" 30 230 \
    | tee "$RUN_DIR/latency_stats.txt" "$OUT_DIR/latency_stats.txt"
cp "$RUN_DIR/verdict.txt" "$OUT_DIR/verdict.txt"
tail -2 "$RUN_DIR/gen.log" > "$OUT_DIR/gen_send_lag.txt"
cp "$RUN_DIR/recv_per_sec.csv" "$OUT_DIR/recv_per_sec.csv"

echo ""
echo "[80%] 完成。統計:$OUT_DIR/latency_stats.txt"
echo "[80%] 完整原始資料(含 latency CSV):$RUN_DIR/"
