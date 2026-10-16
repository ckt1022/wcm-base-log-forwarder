#!/usr/bin/env python3
"""clockfix.py — 共用：由 clock_drift.csv 建立 wall→monotonic 的換算。
D(t) = wall − monotonic；WSL2 clocksource=tsc 時 D 每 ~32 s 向前跳 ~1.6 s。
mono(t_wall) = t_wall − D(t_wall)，D 取「該 wall 時刻之前最後一筆」紀錄。"""
import bisect
import csv


class Clock:
    def __init__(self, run_dir):
        self.walls, self.drifts = [], []
        with open(f"{run_dir}/clock_drift.csv") as fh:
            for row in csv.DictReader(fh):
                self.walls.append(float(row["wall"]))
                self.drifts.append(float(row["drift"]))

    def D(self, wall):
        i = bisect.bisect_right(self.walls, wall) - 1
        return self.drifts[max(i, 0)]

    def mono(self, wall):
        return wall - self.D(wall)

    def steps(self):
        return [(w, d - p) for w, d, p in zip(self.walls[1:], self.drifts[1:], self.drifts[:-1])]
