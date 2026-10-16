#!/bin/bash
# run_micro.sh — Part A micro benchmark 全套執行。
# 每個量測單位 =（一個 group × 一個 run，約 1–2 分鐘，與系統層一個 sweep 點相當）
# 單獨持有共用量測鎖；所有量測都以 taskset 釘在單一核心。
#
# 用法: run_micro.sh <out_dir> [groups...]
#   groups: prod-reuse prod-fresh instr-reuse csharp go   （預設全部）
# 環境變數: CPU（預設 2）、RUNS（預設 3）、ROUNDS（預設 100）、WARMUP（預設 10）
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$SCRIPT_DIR/../micro/target/release/percall_micro"
OUT="${1:?usage: run_micro.sh <out_dir> [groups...]}"; shift
GROUPS_SEL=("$@")
[[ ${#GROUPS_SEL[@]} -eq 0 ]] && GROUPS_SEL=(prod-reuse prod-fresh instr-reuse csharp go)
CPU="${CPU:-2}"
RUNS="${RUNS:-3}"
ROUNDS="${ROUNDS:-100}"
WARMUP="${WARMUP:-10}"
SIZES="1,4,16,64,256,1024"
LOCK=/home/ckt1022/.wcm-bench.lock
TP=/home/ckt1022/wcm-base-log-forwarder/test-plugins
CO=/home/ckt1022/wcm-base-log-forwarder/test/ours/copy_overhead
mkdir -p "$OUT"
echo "cpu=$CPU runs=$RUNS rounds=$ROUNDS warmup=$WARMUP sizes=$SIZES host=$(hostname) date=$(date -Is)" >> "$OUT/run_info.txt"
nproc >> "$OUT/run_info.txt"

# m <stage> <wasm> <lang> <mode> <sizes> <records> <rounds> <warmup> <run>
m() {
    local stage=$1 wasm=$2 lang=$3 mode=$4 sizes=$5 records=$6 rounds=$7 warmup=$8 run=$9
    local f="$OUT/${stage}_${lang}_${mode}_run${run}.csv"
    if [[ -s "$f" ]]; then echo "skip existing $f"; return 0; fi
    if taskset -c "$CPU" "$BIN" "$stage" "$wasm" "$lang" "$mode" "$sizes" "$records" "$rounds" "$warmup" "$run" \
        > "$f.tmp" 2>> "$OUT/micro_stderr.log"; then
        mv "$f.tmp" "$f"
    else
        echo "FAILED: $stage $lang $mode run$run (see micro_stderr.log)" | tee -a "$OUT/failures.txt"
        mv "$f.tmp" "$f.failed" || true
    fi
}

declare -A PROD=(
    [parse_rust]="$TP/rust-plugin/parse/target/wasm32-wasip2/release/parse.wasm"
    [filter_rust]="$TP/rust-plugin/filter/target/wasm32-wasip2/release/filter.wasm"
    [format_rust]="$TP/rust-plugin/format/target/wasm32-wasip2/release/format.wasm"
    [parse_c]="$SCRIPT_DIR/../micro/plugins/parser_c_json.wasm"   # repo 內 parser_c_json.wasm 是 WIT 文字檔（壞的產物）；以 parse_core.wasm + reactor adapter 重新 componentize，見 README
    [filter_c]="$TP/c-plugin/filter/latency_c_filter.wasm"
    [format_c]="$TP/c-plugin/format/format_json-flat.wasm"
)
declare -A INSTR=(
    [parse_rust-instr]="$CO/parse_benchmark/component/target/wasm32-wasip2/release/parse_copy_benchmark_component.wasm"
    [filter_rust-instr]="$CO/filter_benchmark/component/target/wasm32-wasip2/release/filter_copy_benchmark_component.wasm"
    [format_rust-instr]="$CO/format_benchmark/component/target/wasm32-wasip2/release/format_copy_benchmark_component.wasm"
    [parse_c-instr]="$CO/parse_benchmark/c_component/parser_c_benchmark.wasm"
    [filter_c-instr]="$CO/filter_benchmark/c_component/filter_c_benchmark.wasm"
    [format_c-instr]="$CO/format_benchmark/c_component/format_c_benchmark.wasm"
)

# 外層：每個 (group, run) 以 flock 重新呼叫自己（PERCALL_INNER=1 只跑該 run）
if [[ -z "${PERCALL_INNER:-}" ]]; then
    for g in "${GROUPS_SEL[@]}"; do
        nruns="$RUNS"; [[ "$g" == probe ]] && nruns=1
        for run in $(seq 1 "$nruns"); do
            PERCALL_INNER=1 ONLY_RUN="$run" flock "$LOCK" "$0" "$OUT" "$g"
        done
    done
    echo "done: $OUT"
    exit 0
fi

for g in "${GROUPS_SEL[@]}"; do
  case "$g" in
    probe)
      # 功能探測：每個 plugin 小樣本跑一次，確認可載入、輸出筆數正確
      for key in "${!PROD[@]}"; do
        m "${key%%_*}" "${PROD[$key]}" "${key#*_}" reuse "1,1024" 1024 3 1 0
      done
      for key in "${!INSTR[@]}"; do
        m "${key%%_*}" "${INSTR[$key]}" "${key#*_}" reuse "1,1024" 1024 3 1 0
      done
      m parse "$TP/csharp-plugin/parse/parser_csharp.wasm" csharp reuse "1,16" 16 2 1 0
      m filter "$TP/csharp-plugin/filter/filter_csharp.wasm" csharp reuse "1,16" 16 2 1 0
      m parse "$TP/go-plugin/parse/parser_json.wasm" go reuse "1,16" 16 2 1 0
      ;;
    prod-reuse)
      for run in $ONLY_RUN; do
        for key in parse_rust filter_rust format_rust parse_c filter_c format_c; do
          m "${key%%_*}" "${PROD[$key]}" "${key#*_}" reuse "$SIZES" 1024 "$ROUNDS" "$WARMUP" "$run"
        done
      done ;;
    prod-fresh)
      # B=1 每筆新 instance（InstancePre / 完整 link 兩種）＋ B=1024 每批新 instance（現行 parse 的模式）
      for run in $ONLY_RUN; do
        for key in parse_rust filter_rust format_rust parse_c filter_c format_c; do
          m "${key%%_*}" "${PROD[$key]}" "${key#*_}" fresh-pre "1,1024" 1024 30 3 "$run"
          m "${key%%_*}" "${PROD[$key]}" "${key#*_}" fresh-link "1,1024" 1024 30 3 "$run"
        done
      done ;;
    instr-reuse)
      for run in $ONLY_RUN; do
        for key in parse_rust-instr filter_rust-instr format_rust-instr parse_c-instr filter_c-instr format_c-instr; do
          m "${key%%_*}" "${INSTR[$key]}" "${key#*_}" reuse "$SIZES" 1024 "$ROUNDS" "$WARMUP" "$run"
        done
      done ;;
    csharp)
      # C#：每次呼叫的固定成本很高，縮小樣本（records/rounds 見下）
      for run in $ONLY_RUN; do
        m parse "$TP/csharp-plugin/parse/parser_csharp.wasm" csharp reuse "1,4,16,64,256,1024" 1024 "${CS_ROUNDS:-30}" 2 "$run"
        m filter "$TP/csharp-plugin/filter/filter_csharp.wasm" csharp reuse "1,4,16,64,256,1024" 1024 "${CS_ROUNDS:-30}" 2 "$run"
        m parse "$TP/csharp-plugin/parse/parser_csharp.wasm" csharp fresh-pre "1" "${CS_FRESH_RECORDS:-16}" 3 1 "$run"
      done ;;
    go)
      for run in $ONLY_RUN; do
        m parse "$TP/go-plugin/parse/parser_json.wasm" go reuse "$SIZES" 1024 "${GO_ROUNDS:-30}" 3 "$run"
        m parse "$TP/go-plugin/parse/parser_json.wasm" go fresh-pre "1" "${GO_FRESH_RECORDS:-64}" 3 1 "$run"
      done ;;
    *) echo "unknown group $g" >&2; exit 2 ;;
  esac
done
echo "done: $OUT"
