#!/bin/bash
# run_remaining.sh — 2026-10-13 背景 agent 因用量上限中斷後，用腳本接續剩下的實驗（不需要 agent 在場）。
# 所有量測與建置都在共用鎖 /home/ckt1022/.wcm-bench.lock 內，一次只跑一項。
#
# 步驟：
#   0. 等逐筆呼叫實驗（per_record_invocation 的 system_all.sh）跑完
#   1. 建置目前的 host（release）並執行 cargo test，結果寫入 log
#   2. 無 WASM 對照組：原生 vs WCM 的 5,000 EPS 延遲與容量階梯（原生、taskset 釘核）
#   3. 用目前程式碼建立 image `wcm-base-log-forwarder:isolation`，在容器內重跑正確性 C1–C4 各三輪
#   4. 批次對齊方法 3 的容量搜尋（batch_alignment/run_rest.sh，會跳過已完成的點）
#
# 用法：nohup bash test/tools/run_remaining.sh > test/tools/run_remaining.log 2>&1 &
# 在步驟之間停下：touch test/tools/STOP_REMAINING
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOCK=/home/ckt1022/.wcm-bench.lock
STOPF="$REPO/test/tools/STOP_REMAINING"
log() { echo "[remaining] $(date -Is) $*"; }
stop() { [[ -f "$STOPF" ]] && { log "STOP 檔存在，結束"; exit 0; }; }

# 0. 等逐筆呼叫實驗結束
while pgrep -f 'per_record_invocation/scripts/system_all.sh' >/dev/null; do sleep 60; done
log "per_record_invocation system_all 已結束"; stop

# 1. 建置與測試
cd "$REPO"
log "cargo build --release"
flock "$LOCK" cargo build --release -j 4 > test/tools/run_remaining_build.log 2>&1 || log "build 失敗，見 run_remaining_build.log"
log "cargo test"
flock "$LOCK" cargo test --release -j 4 > test/tools/run_remaining_test.log 2>&1 \
    && log "cargo test 通過" || log "cargo test 失敗，見 run_remaining_test.log"
stop

# 2. 無 WASM 對照組（系統層，原生執行）
SYS_ID="system_$(date +%Y%m%d_%H%M%S)"
log "baseline lat 5000 (RUN_ID=$SYS_ID)"
RUN_ID="$SYS_ID" MODE=lat RATES=5000 bash test/ours/baselines/scripts/run_system_capacity.sh
stop
log "baseline cap ladder"
RUN_ID="$SYS_ID" MODE=cap RATES="10000 15000 20000 25000 30000 40000 50000 70000 100000" \
    bash test/ours/baselines/scripts/run_system_capacity.sh
stop

# 3. 容器內正確性重跑（新程式碼）
log "docker build wcm-base-log-forwarder:isolation"
flock "$LOCK" docker build -t wcm-base-log-forwarder:isolation . > test/tools/run_remaining_docker_build.log 2>&1 \
    || { log "docker build 失敗，略過步驟 3"; SKIP3=1; }
if [[ -z "${SKIP3:-}" ]]; then
    docker image inspect --format '{{.Id}}' wcm-base-log-forwarder:isolation > test/tools/isolation_image_id.txt
    COR_ID="isolation_docker_$(date +%Y%m%d_%H%M%S)"
    for sc in C3 C1 C2 C4; do
        log "correctness $sc x3 (RUN_ID=$COR_ID)"
        flock "$LOCK" env IMAGE=wcm-base-log-forwarder:isolation RUN_ID="$COR_ID" REPEATS=3 SCENARIOS="$sc" \
            bash test/compare/correctness/run_wcm_correctness_image.sh || log "correctness $sc 回傳非 0"
        stop
    done
fi

# 4. 批次對齊：容量搜尋（最久）
log "batch_alignment run_rest.sh"
bash test/compare/batch_alignment/run_rest.sh
log "全部完成"
