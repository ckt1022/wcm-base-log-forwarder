#!/usr/bin/env python3
"""stepdetect.py — 從延遲資料本身偵測 WSL2 時鐘階梯並扣除（給沒有 clock_drift.csv 的 run 用）。

改寫自 test/ours/per_record_invocation/scripts/analyze_followup.py 的 correct_steps()（已在正式 wcm-b1 驗證），
為了也能處理 1 s 時間窗（自然延遲範圍可達 ~1.2 s、啟動前 10 s 有暖機尖峰、偶發單秒尖峰）做了三點強化：
  1. 每 1 s bin 取「第 5 小」的延遲（不是最小值），避免 gen 的單筆 LOOP-trigger 等離群值；
  2. 以 5-bin 中位數濾波後的序列找階梯（中位數濾波保留階梯邊緣、濾掉 1–2 秒的尖峰），跳過第 0 秒（第一個不完整批次）；
     時鐘只會向前跳，所以「向下」的水準變化視為暖機/尖峰結束：一樣切段，但不改變 offset（offset 只累加向上的階梯）；
  3. 逐筆指派段落時，條件改為 lat − offset_j ≥ −150 ms（原版 +10 ms 在 W=0.01 時會把自然延遲 < 10 ms 的
     記錄留在舊段落，p99 殘留一整個階梯）。
前提：階梯 ≥ ~400 ms 且同一段內 bin 最小延遲穩定（吞吐固定、無積壓）。若 run 在第一筆 receive 之前就發生階梯，
本方法無法察覺（clock_drift.csv 校正則可以）——因此對有 clock_drift.csv 的 run 一律以 clock_drift 校正為準，
本方法只作交叉驗證；在 34 個有 clock_drift 的 PASS run 上，兩者 p99 差 ≤ ~10 ms（見 clock_correction_audit.csv）。
"""
import numpy as np


def load_latency(run_dir):
    x = np.loadtxt(f"{run_dir}/latency_worker_1.csv", delimiter=",", skiprows=1, usecols=(1, 2), ndmin=2)
    o = np.argsort(x[:, 0], kind="stable")
    return x[o, 0], x[o, 1]


def _medfilt(v, k=5):
    out = np.full(len(v), np.nan)
    idx = np.where(~np.isnan(v))[0]
    vals = v[idx]
    h = k // 2
    for n, i in enumerate(idx):
        out[i] = np.median(vals[max(0, n - h): n + h + 1])
    return out


def correct_steps(recv, lat, horizon=200.0, jump=400.0, floor=-150.0, warmup=1.0, kth=4):
    """回傳 dict(t, raw, corr, offsets, step_times, level0, warnings)。t = 相對第一筆 receive 的秒數。"""
    t = recv - recv[0]
    keep = t <= horizon
    t, lat = t[keep], lat[keep]
    nb = int(t.max()) + 1
    bins = t.astype(int)
    bmin = np.full(nb, np.nan)
    order = np.argsort(bins, kind="stable")
    sb = bins[order]
    starts = np.searchsorted(sb, np.arange(nb + 1))
    for i in range(nb):
        m = lat[order[starts[i]:starts[i + 1]]]
        if len(m):
            k = min(kth, len(m) - 1)
            bmin[i] = np.partition(m, k)[k]
    sm = _medfilt(bmin)
    seg_of_bin = np.zeros(nb, dtype=int)
    seg, prev, warn, step_t, kinds = 0, None, [], [], [0]
    w0 = int(warmup)
    for i in range(nb):
        if i < w0 or np.isnan(sm[i]):
            seg_of_bin[i] = seg
            continue
        if prev is not None:
            if sm[i] - prev > jump:
                seg += 1
                step_t.append(i)
                kinds.append(+1)
            elif prev - sm[i] > jump:
                seg += 1
                kinds.append(-1)
                warn.append(f"downward level change at {i}s ({prev:.0f}->{sm[i]:.0f} ms), treated as end of transient")
        seg_of_bin[i] = seg
        prev = sm[i]
    nseg = seg + 1
    levels = []
    for j in range(nseg):
        idx = [i for i in range(w0, nb) if seg_of_bin[i] == j and not np.isnan(bmin[i])]
        core = idx[1:-1] if len(idx) > 3 else idx
        levels.append(float(np.median(bmin[core])) if core else np.nan)
    offs = np.zeros(nseg)
    for j in range(1, nseg):
        if kinds[j] > 0 and levels[j] - levels[j - 1] < jump / 2:
            kinds[j] = 0          # 濾波後跨過門檻、但段水準差 < 200 ms：不是時鐘階梯（尾端記錄稀疏時會出現）
        offs[j] = offs[j - 1] + (levels[j] - levels[j - 1] if kinds[j] > 0 else 0.0)
    first_up = next((j for j in range(1, nseg) if kinds[j] > 0), nseg)
    base = levels[first_up - 1]
    nxt = np.append(seg_of_bin[1:], seg_of_bin[-1])
    maxseg = nxt[bins]
    assigned = np.zeros(len(lat), dtype=int)
    for j in range(1, nseg):
        if offs[j] != offs[j - 1]:
            assigned = np.where((lat - offs[j] >= floor) & (maxseg >= j), j, assigned)
        else:
            assigned = np.where(assigned == j - 1, np.where(maxseg >= j, j, assigned), assigned)
    corr = lat - offs[assigned]
    steps = [offs[j] - offs[j - 1] for j in range(1, nseg) if kinds[j] > 0]
    step_t = [st for st, j in zip(step_t, [j for j in range(1, nseg) if kinds[j] != -1]) if kinds[j] > 0]
    shifted = offs[assigned] != 0
    return dict(t=t, raw=lat, corr=corr, offsets=offs, step_sizes=steps, step_times=step_t,
                level0=base, warnings=warn, shifted=shifted)


def pct(v, p):
    """與 capacity/calc_latency_stream.py 相同的線性插值百分位。"""
    s = np.sort(v)
    if len(s) == 0:
        return float("nan")
    k = (len(s) - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, len(s) - 1)
    return float(s[lo] + (k - lo) * (s[hi] - s[lo]))


def window_stats(t, v, lo=30, hi=150):
    m = (t >= lo) & (t <= hi)
    w = v[m]
    return dict(n=int(m.sum()), p50=pct(w, 50), p99=pct(w, 99), p999=pct(w, 99.9),
                mean=float(w.mean()) if len(w) else float("nan"), max=float(w.max()) if len(w) else float("nan"))


def subwindows(t, v):
    out = []
    for s in range(0, 180, 30):
        w = v[(t >= s) & (t < s + 30)]
        if len(w):
            out.append((s, float(np.percentile(w, 50)), float(np.percentile(w, 99))))
    return out


def stable_flag(sub):
    """collect.py 的 latency_stable：30–60 s 與 120–150 s 子窗 p50 比較（p50 才不會被單一尖峰誤判）。"""
    sw = [s for s in sub if 30 <= s[0] < 150]
    if len(sw) < 2:
        return ""
    first, last = sw[0], sw[-1]
    return "yes" if last[1] <= 1.3 * first[1] + 20 else "NO(growing)"
