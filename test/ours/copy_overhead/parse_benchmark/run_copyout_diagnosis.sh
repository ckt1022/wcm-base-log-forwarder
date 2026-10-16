#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
rounds=${1:-20}
warmups=${2:-2}
sizes=${3:-384,448,512,640,768,896,1024,1280}
stamp=$(date +%Y%m%d_%H%M%S)

run_variant() {
  local label=$1
  local feature=$2
  local feature_args=()
  if [[ -n "$feature" ]]; then
    feature_args=(--features "$feature")
  fi

  cargo build \
    --manifest-path "$benchmark_dir/component/Cargo.toml" \
    --target wasm32-wasip2 \
    --release \
    --locked \
    --no-default-features \
    "${feature_args[@]}"

  cargo run \
    --manifest-path "$benchmark_dir/host/Cargo.toml" \
    --release \
    --locked \
    -- 384 1280 "$rounds" "$warmups" "$sizes" fresh "$label" \
    > "$benchmark_dir/results/copyout_diag_${label}_${stamp}.csv"
}

run_variant full ""
run_variant empty-list experiment-empty-list
run_variant compact-record experiment-compact-record
run_variant message-only experiment-message-only
run_variant message-256 experiment-message-256
run_variant one-message experiment-one-message

printf 'diagnostic results: %s/results/copyout_diag_*_%s.csv\n' "$benchmark_dir" "$stamp"
