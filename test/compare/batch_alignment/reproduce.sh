#!/bin/bash
# reproduce.sh — 從 repo 重現 batch-alignment（方法 2、方法 3）全部量測、分析與圖表（方法 1 的量測見
#   test/compare/capacity/run_wcm_tw128_latency_cgroup.sh；其分析也在 analyze 步驟內）。
#
# 用法（在 repo 任意位置）：
#   bash test/compare/batch_alignment/reproduce.sh [all|env|confs|m3lat|m2|cap|last|extra|analyze]
#   預設 all。可用 RUNS_DIR=<新目錄>/runs 把原始資料寫到別處（不覆蓋已存的 runs/）；
#   RESULTS_DIR=<目錄> 讓 CSV / charts 也寫到別處。已有 verdict.txt 的點會被跳過（可中斷續跑）。
#   LOCK=<flock 檔>（預設 /home/ckt1022/.wcm-bench.lock）— 每個點在鎖內獨占執行。
#
# 需求：Docker（可用 --cpus/--memory）、python3 + numpy、go（只在 gen_bin/httpsink_bin 不存在時編譯）、
#       taskset、flock；映像見 README「重現方式」（tag + digest）。主機需至少 11 個邏輯核心
#       （sink 綁 core 8、gen 綁 core 9,10，與正式實驗相同）。
# 全部約 7–9 小時（每個延遲點 ~4 分鐘、每個容量 trial ~1.7 分鐘）。
set -euo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
R="$(cd "$A/../../.." && pwd)"
CAP="$R/test/compare/capacity"
export RUNS_DIR="${RUNS_DIR:-$A/runs}"
STEP="${1:-all}"

env_check() {
    echo "== 環境 =="
    uname -a; grep PRETTY /etc/os-release || true
    docker version --format 'docker client {{.Client.Version}} server {{.Server.Version}}'
    for i in wcm-base-log-forwarder:memguard fluent/fluent-bit:5.0.9 fluent/fluentd:v1.16-debian-1 timberio/vector:0.49.0-alpine; do
        echo "$i $(docker image inspect --format '{{.Id}}' "$i")"
    done
    python3 --version; python3 -c 'import numpy; print("numpy", numpy.__version__)'
    echo "clocksource=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource)"
    # 量測用二進位（與正式實驗共用；不存在才編譯）
    [[ -x "$CAP/gen_bin" ]] || ( cd "$R/tools/gen" && go build -o "$CAP/gen_bin" main.go )
    [[ -x "$CAP/httpsink_bin" ]] || ( cd "$CAP/httpsink" && go build -o "$CAP/httpsink_bin" main.go )
}

# 1. 設定檔（conf/ 已存放；此步驟重新產生並與存放版本比對）
confs() {
    local tmp; tmp=$(mktemp -d)
    OUT_DIR="$tmp" bash "$A/gen_confs.sh"
    if diff -r "$tmp" "$A/conf" >/dev/null; then echo "conf/ 與 gen_confs.sh 產物一致"; else
        echo "注意：conf/ 與重新產生的不同（repo 路徑不同時 FB conf 的絕對路徑會不同）；改用新產物"; cp "$tmp"/* "$A/conf/"; fi
}

# 2. 方法 3：5,000 EPS × W 延遲（queue 檔每行 = dut rate mode tag conf [env…]）
m3lat() { bash "$A/run_queue.sh" "$A/queue_m3_lat.txt"; }
# 3. 方法 2：1 MiB 輸入批量
m2() { bash "$A/run_queue.sh" "$A/queue_m2.txt"; }
# 4. 方法 3：容量粗搜尋（與 run_rest.sh 相同的清單與起點）
cap() {
    local S="$A/search_align.sh"
    while read -r dut tag conf start extra; do
        [[ -z "$dut" || "$dut" == \#* ]] && continue
        grep -q "^CAPACITY" "$RUNS_DIR/$dut/search_${tag}.txt" 2>/dev/null && { echo "skip $dut $tag"; continue; }
        bash "$S" "$dut" "$tag" "$conf" "$start" $extra || true
    done < "$A/searches.txt"
}
# 5. WCM W=1.0（tw128）延遲，排最後
last() { bash "$A/run_queue.sh" "$A/queue_last.txt"; }
# 6. 補充：各設定自身容量 80% 的延遲（若 queue_80pct.txt 存在）
extra() { [[ -f "$A/queue_80pct.txt" ]] && bash "$A/run_queue.sh" "$A/queue_80pct.txt" || true; }
# 7. 分析與圖（只讀原始資料；finalize.py 會覆寫 collect.py 的 CSV，加上時鐘校正稽核、穩定性與方法 1）
analyze() {
    python3 "$A/collect.py"
    python3 "$A/finalize.py"
    python3 "$A/plot.py"
}

case "$STEP" in
    all) env_check; confs; m3lat; m2; cap; last; extra; analyze ;;
    env) env_check ;; confs) confs ;; m3lat) m3lat ;; m2) m2 ;; cap) cap ;; last) last ;; extra) extra ;; analyze) analyze ;;
    *) echo "unknown step $STEP"; exit 2 ;;
esac
