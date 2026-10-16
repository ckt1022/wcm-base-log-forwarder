#!/bin/bash
# gen_confs.sh — 產生 pattern_suite 的所有 WCM 設定檔（conf/ 與 fault/）。
# 只從既有設定複製後改指定欄位（sed），可重複執行（輸出相同）。
#
#   conf/wcm_batch_W<W>.yaml          ← batch_alignment/conf/wcm_tw128_wait<ms>.yaml，max_transport_bytes=128 MiB
#   conf/wcm_stream_k<K>_W<W>.yaml    ← streaming_validation/conf/wcm_stream_W<W>.yaml，micro_batch_records=K、max_transport_bytes=128 MiB
#   conf/wcm_stream_k<K>_W0.1_noepoch.yaml   同上 + epoch_interruption: false（K=64,256）
#   conf/wcm_stream_k<K>_W0.1_noreuse.yaml   同上 + plugin_instance_reuse: false（K=64,256）
#   fault/fault_stream_k<K>_t<T>.yaml ← fault_isolation/forwarder_fault.yaml + streaming（K、W=0.1、128 MiB）、stage_timeout_ms=T
#   fault/fault_stream_k<K>_noepoch.yaml / _noreuse.yaml / _capdeny.yaml / _sockdeny.yaml
#   fault/plugin → ../../../ours/fault_isolation/plugin（parse_chaos + 正式 Rust filter/format/transport）
#
# 為什麼 max_transport_bytes=128 MiB：5,000 EPS × 1 KB 時 filter 後輸出約 2 MB/s，1 MiB 的 size trigger
# 約 0.5 s 就觸發，W=0.5／1 的 streaming 點因此變成 size-triggered（p99 都 ≈510 ms）。時間對齊的點必須
# 只由 W 觸發，所以所有新設定（含 batch 模式 tw128 對照）都改成 128 MiB（與 Vector max_bytes、Fluentd chunk 相同）。
set -euo pipefail
PS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALIGN_DIR="$(cd "$PS_DIR/../batch_alignment" && pwd)"
SV_DIR="$(cd "$PS_DIR/../streaming_validation" && pwd)"
FI_DIR="$(cd "$PS_DIR/../../ours/fault_isolation" && pwd)"
BIG=134217728
mkdir -p "$PS_DIR/conf" "$PS_DIR/fault"

ms_of() { case "$1" in 0.01) echo 10;; 0.1) echo 100;; 0.5) echo 500;; 1) echo 1000;; *) echo "bad W $1" >&2; exit 1;; esac; }

for W in 0.01 0.1 0.5 1; do
    src="$ALIGN_DIR/conf/wcm_tw128_wait$(ms_of $W).yaml"
    { echo "# pattern_suite: copy of batch_alignment/conf/$(basename "$src"); only max_transport_bytes 1048576 -> $BIG (time-triggered only)"
      sed -e "s/^  max_transport_bytes: .*/  max_transport_bytes: $BIG/" "$src"; } > "$PS_DIR/conf/wcm_batch_W$W.yaml"
    for K in 1 16 64 256; do
        src="$SV_DIR/conf/wcm_stream_W$W.yaml"
        { echo "# pattern_suite: copy of streaming_validation/conf/$(basename "$src"); micro_batch_records=$K, max_transport_bytes=$BIG (time-triggered only)"
          sed -e "s/^  max_transport_bytes: .*/  max_transport_bytes: $BIG/" \
              -e "s/^  micro_batch_records: .*/  micro_batch_records: $K/" "$src"; } > "$PS_DIR/conf/wcm_stream_k${K}_W$W.yaml"
    done
done
for K in 64 256; do
    base="$PS_DIR/conf/wcm_stream_k${K}_W0.1.yaml"
    { echo "# pattern_suite: wcm_stream_k${K}_W0.1.yaml + epoch_interruption: false (no epoch checks / ticker / deadlines; stage_timeout_ms ignored)"
      sed -e '/^# pattern_suite/d' -e "s/^  plugin_instance_reuse: .*/&\n  epoch_interruption: false/" "$base"; } > "$PS_DIR/conf/wcm_stream_k${K}_W0.1_noepoch.yaml"
    { echo "# pattern_suite: wcm_stream_k${K}_W0.1.yaml + plugin_instance_reuse: false (format/transport instance rebuilt per batch)"
      sed -e '/^# pattern_suite/d' -e "s/^  plugin_instance_reuse: .*/  plugin_instance_reuse: false/" "$base"; } > "$PS_DIR/conf/wcm_stream_k${K}_W0.1_noreuse.yaml"
done

# ── 原生 fault 設定（plugin 由「設定檔目錄/plugin/」解析）──────────────────────
ln -sfn ../../../ours/fault_isolation/plugin "$PS_DIR/fault/plugin"
FSRC="$FI_DIR/forwarder_fault.yaml"
stream_fault() {  # <K> <T>
    sed -e "s/^  max_wait_ms: .*/  max_wait_ms: 100/" \
        -e "s/^  max_transport_bytes: .*/  max_transport_bytes: $BIG/" \
        -e "s/^  stage_timeout_ms: .*/  stage_timeout_ms: $2/" \
        -e "s/^  parse_isolation: true/&\n  processing_mode: streaming\n  micro_batch_records: $1\n  micro_batch_max_wait_ms: 5/" \
        "$FSRC"
}
for K in 64 256; do
    for T in 1000 2000 5000 30000; do
        { echo "# pattern_suite: fault_isolation/forwarder_fault.yaml + streaming (k=$K, W=0.1 s, max_transport_bytes=$BIG), stage_timeout_ms=$T"
          stream_fault $K $T; } > "$PS_DIR/fault/fault_stream_k${K}_t$T.yaml"
    done
    { echo "# pattern_suite: fault_stream_k${K}_t2000.yaml + epoch_interruption: false (watchdog disabled ablation)"
      stream_fault $K 2000 | sed -e "s/^  parse_isolation: true/&\n  epoch_interruption: false/"; } > "$PS_DIR/fault/fault_stream_k${K}_noepoch.yaml"
    { echo "# pattern_suite: fault_stream_k${K}_t2000.yaml + plugin_instance_reuse: false"
      stream_fault $K 2000 | sed -e "s/^  plugin_instance_reuse: .*/  plugin_instance_reuse: false/"; } > "$PS_DIR/fault/fault_stream_k${K}_noreuse.yaml"
    # 能力政策變體：capdeny = 刪除 capabilities 區段（全部預設 deny）；sockdeny = 只授權 filesystem: inert
    { echo "# pattern_suite: fault_stream_k${K}_t2000.yaml with the capabilities section removed (default deny-all policy)"
      stream_fault $K 2000 | sed -e '/^capabilities:/,/^    sockets: inert/d'; } > "$PS_DIR/fault/fault_stream_k${K}_capdeny.yaml"
    { echo "# pattern_suite: fault_stream_k${K}_t2000.yaml with capabilities.parse = {filesystem: inert} only (sockets denied)"
      stream_fault $K 2000 | sed -e '/^    sockets: inert/d'; } > "$PS_DIR/fault/fault_stream_k${K}_sockdeny.yaml"
done
echo "[gen_confs] $(ls "$PS_DIR/conf" | wc -l) conf files, $(ls "$PS_DIR/fault"/*.yaml | wc -l) fault files"
