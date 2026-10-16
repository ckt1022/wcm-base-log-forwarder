#!/bin/bash
# reproduce.sh — 從 repo 重現 per-record invocation 研究的全部步驟（依序）。
#
# 前提：
#   - 在含本研究 host 修改的 checkout 執行（worktree branch worktree-agent-a5fbec9d83acd8f7d，
#     base = wit_test@1c4b9d4；或在 wit_test@1c4b9d4 上 `git apply test/ours/per_record_invocation/host_change.patch`）
#   - /home/ckt1022/wcm-base-log-forwarder 主樹提供唯讀的 plugin 與正式 harness
#     （test-plugins/、test/ours/latency-benchmark-Rust/plugin、test/compare/capacity、test/compare/correctness、
#      test/ours/copy_overhead 的 instrumented component）。若路徑不同請修改 scripts/ 內的 MAIN_REPO / TP / CO 常數。
#   - 所有量測與 build 都在 flock /home/ckt1022/.wcm-bench.lock 內執行（scripts 已內建）。
#
# 用法: reproduce.sh [step...]
#   steps: env build-host test-host build-image build-micro micro system-c3 system-search system-lat5000
#          system-lathigh analyze
#   不帶參數 = 依序執行全部。HIGH_RATE（預設 15000）= 四個變體都能持續的較高延遲量測速率。
#   MICRO_OUT（預設 micro/results/formal）= micro 結果目錄。
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
S="$HERE/scripts"
LOCK=/home/ckt1022/.wcm-bench.lock
HIGH_RATE="${HIGH_RATE:-15000}"
MICRO_OUT="${MICRO_OUT:-$HERE/micro/results/formal}"
STEPS=("$@")
[[ ${#STEPS[@]} -eq 0 ]] && STEPS=(env build-host test-host build-image build-micro micro system-c3 system-search system-lat5000 system-lathigh analyze)

for step in "${STEPS[@]}"; do
  echo "===== $step ====="
  case "$step" in
    env)          "$S/collect_env.sh" > "$HERE/environment.txt" ;;
    build-host)   "$S/locked_cargo.sh" build ;;                      # = flock LOCK cargo build --release -j 4
    test-host)    "$S/locked_cargo.sh" test ;;                       # = flock LOCK cargo test --release -j 4（12 個單元測試）
    build-image)  (cd "$ROOT" && flock "$LOCK" docker build \
                      -f test/ours/per_record_invocation/scripts/Dockerfile.percall \
                      -t wcm-base-log-forwarder:percall .)
                  docker image inspect --format '{{.Id}}' wcm-base-log-forwarder:percall ;;
    build-micro)  "$S/build_micro.sh" ;;                             # = flock LOCK cargo build --release -j 4（micro/）
    micro)        # CPU=2 釘核；RUNS=3 ROUNDS=100 WARMUP=10（C# 30 rounds、Go 30 rounds，見 run_micro.sh）
                  CPU=2 RUNS=3 ROUNDS=100 WARMUP=10 "$S/run_micro.sh" "$MICRO_OUT" \
                      prod-reuse instr-reuse prod-fresh go csharp ;;
    system-c3)    "$S/system_all.sh" c3 ;;       # C3：k=1 三輪、k=unlimited 一輪
    system-search) "$S/system_all.sh" search ;;  # 容量搜尋：kinf(12500 起) k1(3125 起) k16 k256(12500 起)
    system-lat5000) "$S/system_all.sh" lat5000 ;;
    system-lathigh)
                  for d in wcm-percall-kinf wcm-percall-k1 wcm-percall-k16 wcm-percall-k256; do
                      "$S/latency_percall.sh" "$d" "$HIGH_RATE"
                  done ;;
    analyze)      python3 "$S/analyze_micro.py" "$MICRO_OUT" "$HERE/micro/analysis"
                  python3 "$S/analyze_system.py" "$HERE"
                  python3 "$S/analyze_followup.py" "$HERE" ;;   # 時鐘階梯校正、B≤256 擬合、Go 部分資料、C3 對照、圖
    *) echo "unknown step $step" >&2; exit 2 ;;
  esac
done
