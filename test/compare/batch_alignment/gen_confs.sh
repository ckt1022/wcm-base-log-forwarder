#!/bin/bash
# gen_confs.sh — 由正式設定檔產生 batch_alignment/conf/ 下所有設定（只改批次參數）。
# 產物已存放在 conf/（版本控管用）；本腳本可重新產生到 OUT_DIR（預設 conf/）以核對。
set -euo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
R="$(cd "$A/../../.." && pwd)"
OUT="${OUT_DIR:-$A/conf}"; mkdir -p "$OUT"
FB_LAT=$R/test/compare/fluentbit/latency
FD_LAT=$R/test/compare/fluentd/latency
WCM_LAT=$R/test/ours/latency-benchmark-Rust
# Fluent Bit：Flush = W（方法 3）或 W_eq = 1 MiB/(R×1024 B)（方法 2：0.2048/0.1024/0.0683/0.0512）
for W in 0.01 0.02 0.05 0.1 0.5 1 0.2048 0.1024 0.0683 0.0512; do
  sed -E -e "s|^( *Flush +).*|\1${W}|" -e "s|\.\./latency_filter\.lua|$FB_LAT/latency_filter.lua|" \
    -e "1i # batch_alignment copy of fluentbit/latency/conf/capacity_lua_1s.conf; only Flush changed (=${W}) and Script path made absolute" \
    $FB_LAT/conf/capacity_lua_1s.conf > $OUT/fbit_lua_flush_${W}.conf
  sed -E -e "s|^( *Flush +).*|\1${W}|" -e "s|\.\./wasm_file/latency_filter\.wasm|$FB_LAT/wasm_file/latency_filter.wasm|" \
    -e "1i # batch_alignment copy of fluentbit/latency/conf/capacity_wasm_1s.conf; only Flush changed (=${W}) and WASM_Path made absolute" \
    $FB_LAT/conf/capacity_wasm_1s.conf > $OUT/fbit_wasm_flush_${W}.conf
done
# Fluentd：以正式延遲設定 capacity_1m_1s.conf 為底，只改 <buffer>
mkfd() { # name flush_interval flush_thread_interval chunk_limit_size chunk_limit_records(可空) 說明
  local out=$OUT/fluentd_$1.conf
  { echo "# batch_alignment copy of fluentd/latency/conf/capacity_1m_1s.conf — only <buffer> batching knobs changed."
    echo "# $6"
    sed -E -e "s|^( *flush_interval ).*|\1$2|" -e "s|^( *flush_thread_interval ).*|\1$3|" -e "s|^( *chunk_limit_size ).*|\1$4|" \
      -e '/^# Fluentd capacity profile/,/^# Batch policy/d' $FD_LAT/conf/capacity_1m_1s.conf
  } > $out
  if [[ -n "$5" ]]; then sed -i -E "s|^( *)chunk_limit_size (.*)|\1chunk_limit_size \2\n\1chunk_limit_records $5|" $out; fi
}
mkfd m3_W1 1 0.1 128m "" "Method 3 W=1.0 s: flush_interval 1, flush_thread_interval 0.1 (formal), chunk 128m so size never triggers"
for W in 0.5 0.1 0.05 0.02 0.01; do
  mkfd m3_W$W $W $W 128m "" "Method 3 W=$W s: fluentd truncates flush_interval<1 to 0 and the real window = enqueue period = max(flush_interval/11, flush_thread_interval) = flush_thread_interval = $W; chunk 128m so size never triggers"
done
mkfd m2_rec410 10 0.01 128m 432 "Method 2: chunk_limit_records 432 (x chunk_full_threshold 0.95 => full at ~410 output records = 1 MiB input), long flush_interval 10, poll 0.01"
# Vector：timeout / max_bytes / max_events 由環境變數注入
{ echo "# batch_alignment copy of vector/latency/conf/latency_tuned.yaml — only sink batch knobs are parameterised:"
  echo "#   batch.timeout_secs <- FLUSH_INTERVAL, batch.max_bytes <- VECTOR_BATCH_MAX_BYTES, batch.max_events <- VECTOR_BATCH_MAX_EVENTS"
  sed -E 's|^( *)max_bytes: \$\{VECTOR_BATCH_MAX_BYTES:-134217728\}|\1max_bytes: ${VECTOR_BATCH_MAX_BYTES:-134217728}\n\1max_events: ${VECTOR_BATCH_MAX_EVENTS:-100000000}|' \
    $R/test/compare/vector/latency/conf/latency_tuned.yaml; } > $OUT/vector_align.yaml
# WCM：tw128（128 MB buffer）為底，只改 max_wait_ms；方法 2 用正式 b1 原檔
for ms in 10 20 50 100 500 1000; do
  { echo "# batch_alignment: copy of forwarder_membound_tw128.yaml (128 MB buffer => time-dominated); only max_wait_ms changed (=$ms)"
    grep -v '^#' $WCM_LAT/forwarder_membound_tw128.yaml | sed -E "s|^( *max_wait_ms: ).*|\1$ms|"; } > $OUT/wcm_tw128_wait${ms}.yaml
done
{ echo "# batch_alignment: verbatim copy of test/ours/latency-benchmark-Rust/forwarder_membound_b1.yaml (formal wcm-b1, method 2 WCM)"
  cat $WCM_LAT/forwarder_membound_b1.yaml; } > $OUT/wcm_b1.yaml
echo "configs written to $OUT"
