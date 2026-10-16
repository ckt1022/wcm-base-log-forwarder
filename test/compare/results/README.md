# 實驗結果整理規則

根目錄的 `實驗結果.csv` 是機器可讀的結果來源。每一列代表一個產品、配置與
工作量的結果；原本的 `實驗記錄.xlsx` 保留為舊版人工排版文件。

以下指令只重新整理現有資料，不會執行實驗：

```bash
python3 test/compare/results/build_results_csv.py
```

規則：

- 有原始 latency CSV 時，一律重新計算延遲，不採用人工抄錄值。
- 只有存在相符的 stats CSV 時才填入 CPU 與記憶體。
- 空白資源欄位代表舊實驗沒有資料，不代表用量為零。
- 新執行的 capacity/latency 實驗一律使用 `sample_cgroup_stats.sh`，CPU 為 DUT
  容器 cgroup 總 CPU，MEM 為 `memory.current`；不得再與舊的單程序 RSS 混合排名。
- `stats.csv` 為了相容既有分析器仍可能沿用 `rss_mb` 欄名；由 cgroup sampler
  產生時，該欄實際語意是容器 `memory.current` MiB。
- `MB/s` 使用十進位（bytes / 1,000,000）；記憶體限制與 RSS 使用 MiB。
- `capacity_80pct_latency` 的各產品 EPS 不同，不可用來做相同工作量排名。
- 舊實驗未保存 image digest 時會明確標示，不以目前 digest 冒充歷史版本。
- `data_quality` 說明該列是原始 CSV 重算、產生器文字匯入或容量搜尋結果。

整理程式不會覆寫 `實驗記錄.xlsx`。
