#!/bin/bash
# sample_stats.sh — 每秒取樣單一程序的 CPU 與 RSS 使用量，寫成 CSV
# 用法：sample_stats.sh <pid> <out_csv> [mem_limit(如 512m)] [cpu_limit(核數)] [single|tree]
# 目標程序消失（實驗結束）時自動停止。
#
# 欄位：
#   elapsed_s  取樣時刻（自取樣開始起算，秒）
#   cpu_pct    取樣區間的 CPU 使用率，100 = 佔滿一顆核心（同 top；可超過 100）
#   cpu_lim_pct CPU 使用率相對 cpu_limit 的比例（未給 cpu_limit 時 = cpu_pct）
#   cpu_time_s 累計 CPU 時間（user+sys，秒）
#   rss_mb     VmRSS（MB）
#   mem_pct    RSS / mem_limit（未給 mem_limit 時以系統總記憶體為分母）
set -uo pipefail

PID="$1"; OUT="$2"; MEM_LIMIT_RAW="${3:-}"; CPU_LIMIT="${4:-1}"; SCOPE="${5:-single}"

to_bytes() {
    local v="${1,,}"
    case "$v" in
        *k) echo $(( ${v%k} * 1024 ));;
        *m) echo $(( ${v%m} * 1024 * 1024 ));;
        *g) echo $(( ${v%g} * 1024 * 1024 * 1024 ));;
        *)  echo "$v";;
    esac
}

if [[ -n "$MEM_LIMIT_RAW" ]]; then
    MEM_LIMIT_B=$(to_bytes "$MEM_LIMIT_RAW")
else
    MEM_LIMIT_B=$(awk '/MemTotal/{print $2*1024}' /proc/meminfo)
fi

CLK_TCK=$(getconf CLK_TCK)

# /proc/<pid>/stat 的 comm 欄可能含空白，剝掉「pid (comm) 」後
# 剩餘欄位自 state 起算：utime=第 12、stime=第 13
list_scope_pids() {
    if [[ "$SCOPE" != "tree" ]]; then
        echo "$PID"
        return
    fi
    ps -eo pid=,ppid= | awk -v root="$PID" '
        { pid[n]=$1; ppid[n]=$2; n++ }
        END {
            keep[root]=1
            do {
                changed=0
                for (i=0; i<n; i++) {
                    # 用 (x in keep) 判斷：直接讀 keep[x] 會在 awk 中建立空鍵，
                    # 使 END 的 for (p in keep) 把系統上所有父程序都算進來（舊版 bug）
                    if ((ppid[i] in keep) && !(pid[i] in keep)) {
                        keep[pid[i]]=1; changed=1
                    }
                }
            } while (changed)
            for (p in keep) print p
        }
    '
}

read_cpu_ticks() {
    local scope_pid stat rest total=0 found=0
    for scope_pid in $(list_scope_pids); do
        stat=$(cat "/proc/$scope_pid/stat" 2>/dev/null) || continue
        rest="${stat##*) }"
        # shellcheck disable=SC2086
        set -- $rest
        total=$(( total + ${12} + ${13} ))
        found=1
    done
    (( found )) || return 1
    echo "$total"
}

read_rss_kb() {
    local scope_pid value total=0 found=0
    for scope_pid in $(list_scope_pids); do
        value=$(awk '/^VmRSS/{print $2; exit}' "/proc/$scope_pid/status" 2>/dev/null) || continue
        [[ -n "$value" ]] || continue
        total=$(( total + value ))
        found=1
    done
    (( found )) || return 1
    echo "$total"
}

echo "elapsed_s,cpu_pct,cpu_lim_pct,cpu_time_s,rss_mb,mem_pct" > "$OUT"

T_START=$EPOCHREALTIME
PREV_TICKS=$(read_cpu_ticks) || exit 0
PREV_T=$EPOCHREALTIME

while sleep 1; do
    TICKS=$(read_cpu_ticks)  || break
    RSS_KB=$(read_rss_kb)    || break
    NOW=$EPOCHREALTIME
    awk -v t0="$T_START" -v now="$NOW" -v pt="$PREV_T" \
        -v ticks="$TICKS" -v pticks="$PREV_TICKS" -v hz="$CLK_TCK" \
        -v rss_kb="$RSS_KB" -v mem_b="$MEM_LIMIT_B" -v ncpu="$CPU_LIMIT" \
        'BEGIN {
            dt = now - pt
            cpu_pct = dt > 0 ? (ticks - pticks) / hz / dt * 100 : 0
            printf "%.1f,%.1f,%.1f,%.2f,%.2f,%.2f\n",
                now - t0, cpu_pct, cpu_pct / ncpu, ticks / hz,
                rss_kb / 1024, rss_kb * 1024 / mem_b * 100
        }' >> "$OUT"
    PREV_TICKS=$TICKS
    PREV_T=$NOW
done
