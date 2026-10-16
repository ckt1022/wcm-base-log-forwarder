#!/bin/bash
# loadcheck.sh — P5 載入期能力檢查的開銷：對 fault_isolation/plugin_load_check.csv 的 26 個 plugin，
# 分別以「預設 deny」與「inert-fs inert-sockets」執行 `--check-plugin`，記錄整個指令的 wall time
# 與 binary 自己輸出的分段計時（TIMING compile_ms／check_ms／link_ms／instantiate_pre_ms）。
# check_ms 就是「有檢查 vs 沒檢查」的差（沒檢查的載入 = compile + link + instantiate_pre）。
# 用法：loadcheck.sh <out_csv>（整個檔案在 flock 內完成；最後一行寫 #DONE）
set -uo pipefail
PS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
if [[ -z "${_PS_LOCKED:-}" ]]; then
    exec flock "$LOCK" env _PS_LOCKED=1 bash "${BASH_SOURCE[0]}" "$@"
fi
REPO_ROOT="$(cd "$PS_DIR/../../.." && pwd)"
BIN="${BIN:-$REPO_ROOT/target/release/wcm-base-log-forwarder}"
LIST="${LIST:-$REPO_ROOT/test/ours/fault_isolation/plugin_load_check.csv}"
OUT="${1:?usage: loadcheck.sh <out_csv>}"
CORE="${DUT_CORE:-4}"
mkdir -p "$(dirname "$OUT")"
echo "stage,plugin,grant,result,wall_ms,compile_ms,check_ms,link_ms,instantiate_pre_ms,plugin_sha256" > "$OUT.tmp"
tail -n +2 "$LIST" | while IFS=, read -r stage plugin _rest; do
    [[ -z "$stage" ]] && continue
    path="$REPO_ROOT/$plugin"
    sha=$(sha256sum "$path" 2>/dev/null | cut -d' ' -f1)
    for grant in "default" "inert-fs inert-sockets"; do
        args=(); [[ "$grant" != default ]] && read -r -a args <<< "$grant"
        t0=$(date +%s%N)
        res=$(taskset -c "$CORE" "$BIN" --check-plugin "$stage" "$path" "${args[@]}" 2>&1)
        t1=$(date +%s%N)
        result=$(grep -oE '^LOAD_(OK|REFUSED)' <<< "$res" | head -1)
        tim=$(grep -m1 '^TIMING' <<< "$res")
        val() { grep -oE "$1=[0-9.]+" <<< "$tim" | cut -d= -f2; }
        echo "$stage,$plugin,$grant,${result:-ERROR},$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.3f",(b-a)/1e6}'),$(val compile_ms),$(val check_ms),$(val link_ms),$(val instantiate_pre_ms),$sha" >> "$OUT.tmp"
    done
done
echo "#DONE $(date -Is) binary_sha256=$(sha256sum "$BIN" | cut -d' ' -f1)" >> "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
echo "[loadcheck] $(grep -c ',LOAD_' "$OUT") rows → $OUT"
