#!/bin/bash
# checkplugin.sh — G6：每個 stage 的正式 plugin 與 chaos 變體，在 deny（預設）／inert／（transport）endpoint 授權下的
# `--check-plugin` 結果與分段計時。改寫自 pattern_suite/loadcheck.sh（同欄位），差異只有 plugin 清單與多一種授權。
# 用法：checkplugin.sh <out_csv>（整個檔案在 flock 內完成；最後一行寫 #DONE）
set -uo pipefail
G="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
if [[ -z "${_PG_LOCKED:-}" ]]; then
    exec flock "$LOCK" env _PG_LOCKED=1 bash "${BASH_SOURCE[0]}" "$@"
fi
REPO_ROOT="$(cd "$G/../../.." && pwd)"
BIN="${BIN:-$REPO_ROOT/target/release/wcm-base-log-forwarder}"
OUT="${1:?usage: checkplugin.sh <out_csv>}"
CORE="${DUT_CORE:-4}"
P="$G/fault/plugin"
LIST="parse:latency_rust_parse parse:parse_chaos filter:latency_rust_filter filter:filter_chaos format:latency_rust_format format:format_chaos transport:transport transport:transport_chaos"
mkdir -p "$(dirname "$OUT")"
echo "stage,plugin,grant,result,n_violations,wall_ms,compile_ms,check_ms,link_ms,instantiate_pre_ms,plugin_sha256" > "$OUT.tmp"
for item in $LIST; do
    stage=${item%%:*}; name=${item#*:}
    path="$P/$name.wasm"
    sha=$(sha256sum "$(readlink -f "$path")" 2>/dev/null | cut -d' ' -f1)
    grants=("default" "inert-fs inert-sockets")
    [[ $stage == transport ]] && grants+=("inert-fs endpoint-sockets")
    for grant in "${grants[@]}"; do
        args=(); [[ "$grant" != default ]] && read -r -a args <<< "$grant"
        t0=$(date +%s%N)
        res=$(taskset -c "$CORE" "$BIN" --check-plugin "$stage" "$path" "${args[@]}" 2>&1)
        t1=$(date +%s%N)
        result=$(grep -oE '^LOAD_(OK|REFUSED)' <<< "$res" | head -1)
        nv=$(grep -oE '：[0-9]+ 個未授權 import' <<< "$res" | head -1 | grep -oE '[0-9]+')
        tim=$(grep -m1 '^TIMING' <<< "$res")
        val() { grep -oE "$1=[0-9.]+" <<< "$tim" | cut -d= -f2; }
        echo "$stage,$name,$grant,${result:-ERROR},${nv:-0},$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.3f",(b-a)/1e6}'),$(val compile_ms),$(val check_ms),$(val link_ms),$(val instantiate_pre_ms),$sha" >> "$OUT.tmp"
    done
done
echo "#DONE $(date -Is) binary_sha256=$(sha256sum "$BIN" | cut -d' ' -f1)" >> "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
echo "[checkplugin] $(grep -c ',LOAD_' "$OUT") rows → $OUT"
