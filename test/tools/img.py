from plot_experiment import plot_cpu, plot_mem, plot_throughput

# CPU 圖（單一檔案）
plot_cpu("stats_cpu.csv")

# MEM 圖（單一檔案）
plot_mem("stats_cpu.csv")

# 吞吐量圖（多個檔案 → 多條線）
plot_throughput(
    filepaths=["run1/stats_cpu_sink.csv", "run2/stats_cpu_sink.csv"],
    labels=["Baseline", "Optimized"],
    output_name="throughput_comparison",
)