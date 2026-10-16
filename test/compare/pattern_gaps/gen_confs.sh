#!/bin/bash
# gen_confs.sh — 產生 pattern_gaps 的設定檔（只從 pattern_suite 的既有設定複製後改列出的欄位）。
#   conf/wcm_stream_k<K>_W0.1.yaml（K ∈ 4 8 512 1024）    ← pattern_suite/conf/wcm_stream_k256_W0.1.yaml，只改 micro_batch_records
#   conf/wcm_stream_k256_W0.1_wait<M>.yaml（M ∈ 10 20）   ← 同上，只改 micro_batch_max_wait_ms
#   fault/cap_<stage>_<grant>.yaml                        ← pattern_suite/fault/fault_stream_k256_t2000.yaml：
#        parse 改回正式 latency_rust_parse.wasm；<stage> 換成 <stage>_chaos.wasm；capabilities 只授權該 stage：
#        deny（不寫 capabilities）／inert（filesystem: inert + sockets: inert）／endpoint（僅 transport：filesystem: inert + sockets: endpoint）
set -euo pipefail
G="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PS="$G/../pattern_suite"
mkdir -p "$G/conf" "$G/fault"
SRC="$PS/conf/wcm_stream_k256_W0.1.yaml"
for K in 4 8 512 1024; do
    out="$G/conf/wcm_stream_k${K}_W0.1.yaml"
    { echo "# pattern_gaps: copy of pattern_suite/conf/wcm_stream_k256_W0.1.yaml; only micro_batch_records=$K"
      sed "s/^  micro_batch_records: 256$/  micro_batch_records: $K/" "$SRC"; } > "$out"
    grep -q "^  micro_batch_records: $K$" "$out"
done
for M in 10 20; do
    out="$G/conf/wcm_stream_k256_W0.1_wait${M}.yaml"
    { echo "# pattern_gaps: copy of pattern_suite/conf/wcm_stream_k256_W0.1.yaml; only micro_batch_max_wait_ms=$M"
      sed "s/^  micro_batch_max_wait_ms: 5$/  micro_batch_max_wait_ms: $M/" "$SRC"; } > "$out"
    grep -q "^  micro_batch_max_wait_ms: $M$" "$out"
done

FSRC="$PS/fault/fault_stream_k256_t2000.yaml"
for stage in filter format transport; do
    for grant in deny inert endpoint; do
        [[ $grant == endpoint && $stage != transport ]] && continue
        out="$G/fault/cap_${stage}_${grant}.yaml"
        {
            echo "# pattern_gaps G6: copy of pattern_suite/fault/fault_stream_k256_t2000.yaml; parse=latency_rust_parse.wasm, $stage=${stage}_chaos.wasm, capabilities: $stage=$grant only"
            # 刪掉原本的 capabilities 區段（parse 的 inert 授權；正式 parser 不需要）
            sed -e '/^capabilities:/,/^$/d' \
                -e 's/^  parse: "parse_chaos.wasm"/  parse: "latency_rust_parse.wasm"/' \
                "$FSRC" | case $stage in
                    filter) sed 's/^  filter: "latency_rust_filter.wasm"/  filter: "filter_chaos.wasm"/';;
                    format) sed 's/^  format: "latency_rust_format.wasm"/  format: "format_chaos.wasm"/';;
                    transport) sed 's/^  transport: "transport.wasm"/  transport: "transport_chaos.wasm"/';;
                esac
            case $grant in
                inert) printf 'capabilities:\n  %s:\n    filesystem: inert\n    sockets: inert\n\n' "$stage";;
                endpoint) printf 'capabilities:\n  %s:\n    filesystem: inert\n    sockets: endpoint\n\n' "$stage";;
            esac
        } > "$out"
        grep -q "${stage}_chaos.wasm" "$out"
        grep -q 'latency_rust_parse.wasm' "$out"
    done
done
echo "[gen_confs] $(ls "$G/conf" | wc -l) conf, $(ls "$G"/fault/*.yaml | wc -l) fault configs"
