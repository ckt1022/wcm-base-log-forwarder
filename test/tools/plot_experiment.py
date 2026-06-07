"""
實驗數據 LaTeX 圖表生成工具
=================================
支援三種圖表：
  - plot_cpu(filepath)         : CPU 使用率隨時間變化
  - plot_mem(filepath)         : 記憶體使用量隨時間變化
  - plot_throughput(filepaths) : 吞吐量隨時間變化（支援多檔案、多條線）

輸出：PDF 向量圖，可直接 \includegraphics 插入 LaTeX。

依賴套件：
  pip install pandas matplotlib
"""

import re
import os
import pandas as pd
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
from pathlib import Path


# ──────────────────────────────────────────────
# LaTeX 風格設定（使用 matplotlib 內建 serif 字型，
# 不需要本機安裝 LaTeX，仍能輸出乾淨的學術風格圖表）
# ──────────────────────────────────────────────
plt.rcParams.update({
    "font.family":       "serif",
    "font.size":         11,
    "axes.labelsize":    12,
    "axes.titlesize":    13,
    "legend.fontsize":   10,
    "xtick.labelsize":   10,
    "ytick.labelsize":   10,
    "figure.figsize":    (6.5, 3.8),   # 適合雙欄論文寬度（英吋）
    "axes.grid":         True,
    "grid.linestyle":    "--",
    "grid.alpha":        0.4,
    "lines.linewidth":   1.6,
    "lines.markersize":  4,
})

OUTPUT_DIR = Path("./figures")
OUTPUT_DIR.mkdir(exist_ok=True)


# ──────────────────────────────────────────────
# 私有輔助函式
# ──────────────────────────────────────────────

def _parse_cpu_file(filepath: str) -> pd.DataFrame:
    """讀取 stats_cpu.csv 格式，解析百分比與 MiB 數值。"""
    df = pd.read_csv(filepath)
    df["timestamp"] = pd.to_datetime(df["timestamp"], utc=True)
    df["time_s"] = (df["timestamp"] - df["timestamp"].iloc[0]).dt.total_seconds()

    # cpu_pct: "0.70%" → 0.70
    df["cpu_val"] = df["cpu_pct"].str.rstrip("%").astype(float)

    # mem_usage: "4.375MiB / 7.382GiB" → 取左側 MiB 數值
    df["mem_val"] = df["mem_usage"].apply(_parse_mem_mib)

    return df


def _parse_mem_mib(mem_str: str) -> float:
    """將 '4.375MiB / 7.382GiB' 解析為 MiB 浮點數。"""
    match = re.match(r"([\d.]+)(MiB|GiB|KiB)", mem_str.strip())
    if not match:
        return float("nan")
    val, unit = float(match.group(1)), match.group(2)
    return val if unit == "MiB" else val * 1024 if unit == "GiB" else val / 1024


def _parse_throughput_file(filepath: str) -> pd.DataFrame:
    """讀取 stats_cpu_sink.csv 格式，將 Unix timestamp 轉為相對秒數。"""
    df = pd.read_csv(filepath)
    df["time_s"] = df["ts"] - df["ts"].iloc[0]
    return df


def _save(fig: plt.Figure, name: str) -> str:
    """儲存圖表為 PDF，回傳儲存路徑字串。"""
    out = OUTPUT_DIR / f"{name}.pdf"
    fig.savefig(out, format="pdf", bbox_inches="tight")
    print(f"[已儲存] {out}")
    return str(out)


# ──────────────────────────────────────────────
# 公開函式
# ──────────────────────────────────────────────

def plot_cpu(filepath: str, output_name: str = "cpu_usage") -> str:
    """
    繪製 CPU 使用率隨時間變化圖表。

    Parameters
    ----------
    filepath    : str
        stats_cpu.csv 路徑（欄位：timestamp, cpu_pct, mem_usage, mem_pct）
    output_name : str
        輸出 PDF 檔名（不含副檔名），預設 'cpu_usage'

    Returns
    -------
    str : 輸出 PDF 路徑
    """
    df = _parse_cpu_file(filepath)
    label = Path(filepath).stem

    fig, ax = plt.subplots()
    ax.plot(df["time_s"], df["cpu_val"], color="#2563eb", label=label)
    ax.set_xlabel("Time (s)")
    ax.set_ylabel("CPU Usage (%)")
    ax.set_title("CPU Usage Over Time")
    ax.yaxis.set_major_formatter(ticker.FormatStrFormatter("%.1f%%"))
    ax.legend()
    fig.tight_layout()

    return _save(fig, output_name)


def plot_mem(filepath: str, output_name: str = "mem_usage") -> str:
    """
    繪製記憶體使用量隨時間變化圖表。

    Parameters
    ----------
    filepath    : str
        stats_cpu.csv 路徑（欄位：timestamp, cpu_pct, mem_usage, mem_pct）
    output_name : str
        輸出 PDF 檔名（不含副檔名），預設 'mem_usage'

    Returns
    -------
    str : 輸出 PDF 路徑
    """
    df = _parse_cpu_file(filepath)
    label = Path(filepath).stem

    fig, ax = plt.subplots()
    ax.plot(df["time_s"], df["mem_val"], color="#16a34a", label=label)
    ax.set_xlabel("Time (s)")
    ax.set_ylabel("Memory Usage (MiB)")
    ax.set_title("Memory Usage Over Time")
    ax.legend()
    fig.tight_layout()

    return _save(fig, output_name)


def plot_throughput(
    filepaths: list[str],
    labels: list[str] | None = None,
    output_name: str = "throughput",
) -> str:
    """
    繪製吞吐量隨時間變化圖表，支援多個檔案畫成不同折線。

    Parameters
    ----------
    filepaths   : list[str]
        一或多個 stats_cpu_sink.csv 路徑（欄位：ts, throughput_lps, ...）
    labels      : list[str] | None
        每條線的圖例標籤；None 時自動使用檔名
    output_name : str
        輸出 PDF 檔名（不含副檔名），預設 'throughput'

    Returns
    -------
    str : 輸出 PDF 路徑

    範例
    ----
    plot_throughput(
        filepaths=["run1/stats_cpu_sink.csv", "run2/stats_cpu_sink.csv"],
        labels=["Baseline", "Optimized"],
    )
    """
    if labels is None:
        labels = [Path(fp).stem for fp in filepaths]

    # 顏色循環（最多支援 8 條線）
    colors = ["#2563eb", "#dc2626", "#16a34a", "#d97706",
              "#7c3aed", "#0891b2", "#be185d", "#374151"]

    fig, ax = plt.subplots()

    for fp, label, color in zip(filepaths, labels, colors):
        df = _parse_throughput_file(fp)
        ax.plot(df["time_s"], df["throughput_lps"],
                label=label, color=color, marker="o", markersize=3)

    ax.set_xlabel("Time (s)")
    ax.set_ylabel("Throughput (lines/s)")
    ax.set_title("Throughput Over Time")
    ax.legend()
    fig.tight_layout()

    return _save(fig, output_name)


# ──────────────────────────────────────────────
# 快速測試（直接執行此腳本時）
# ──────────────────────────────────────────────
if __name__ == "__main__":
    CPU_FILE  = "stats_cpu.csv"
    SINK_FILE = "stats_cpu_sink.csv"

    plot_cpu(CPU_FILE)
    plot_mem(CPU_FILE)
    plot_throughput(
        filepaths=[SINK_FILE, SINK_FILE],   # 示範：同檔案當兩條線
        labels=["Run 1", "Run 2"],
    )
    print("\n所有圖表已儲存至 ./figures/")