# Fluent Bit Lua 錯誤注入測試

測試 Lua filter 發生各種錯誤時 Fluent Bit 的行為，並觀察是否能繼續處理後續 log。

## 環境需求

- Docker（執行 `cr.fluentbit.io/fluent/fluent-bit:5.0.7`）
- Python 3（loggen）

## 執行測試

```bash
./script/run_crash.sh <test>
```

| `<test>` | 說明 | 持續時間 |
|---|---|---|
| `loop` | 無限迴圈，永久 block engine | 3 分鐘後自動停止 |
| `io` | blocking I/O，永久 block engine | 3 分鐘後自動停止 |
| `cpu` | 100% CPU 佔用 10 秒後返回 | 等容器自然結束 |
| `mem` | 無限分配記憶體直到 OOM kill | 約 50 秒後容器被 kill |
| `parse` | 解析 malformed JSON 時 crash | 等容器自然結束 |

範例：

```bash
./script/run_crash.sh loop
./script/run_crash.sh mem
```

## 測試流程

腳本以 Docker 啟動容器後，自動依序注入：

```
t=+3s   seq:1  正常 log（應出現在 stdout）
t=+6s   seq:2  觸發錯誤情境
t=+16s  seq:3  正常 log（loop/io/mem 測試不應出現）
        loggen 開始持續輸入 30 秒（觀察錯誤後是否能恢復）
```

`loop` / `io` 測試：觸發後持續跑 3 分鐘，到時間再自動停止容器。  
按 `Ctrl+C` 可隨時中斷並清理容器。

## 各測試預期行為

| 測試 | seq:1 | seq:2 後 | seq:3 | loggen | Fluent Bit |
|---|---|---|---|---|---|
| `loop` | 出現 | 靜默（engine 卡死）| 不出現 | 靜默 | 存活但無反應 |
| `io` | 出現 | 靜默（engine 卡死）| 不出現 | 靜默 | 存活但無反應 |
| `cpu` | 出現 | 靜默約 10 秒 | 延遲出現 | 正常 | 恢復正常 |
| `mem` | 出現 | 記憶體線性上升 | 不出現 | 靜默 | **容器被 OOM kill（exit 137）** |
| `parse` | 出現 | error log，seq:2 被 drop | 出現 | 正常 | 恢復正常（per-record 隔離）|

## 輸出：stats CSV

每次執行後，會在 `lua/` 目錄下產生：

```
stats_<test>.csv
```

例如 `./script/run_crash.sh loop` 會產生 `stats_loop.csv`。

### CSV 格式

```
timestamp,cpu_pct,mem_usage,mem_pct
2026-06-06T10:00:01+08:00,0.80%,12.3MiB / 15.4GiB,0.08%
2026-06-06T10:00:02+08:00,0.82%,12.3MiB / 15.4GiB,0.08%
2026-06-06T10:00:15+08:00,99.20%,12.4MiB / 15.4GiB,0.08%   <- loop 觸發
2026-06-06T10:00:16+08:00,99.71%,12.4MiB / 15.4GiB,0.08%
...
```

每秒一筆，容器存活期間持續記錄。

### 各測試的 CSV 特徵

| 測試 | cpu_pct 曲線 | mem_usage 曲線 |
|---|---|---|
| `loop` | 觸發後從 < 1% 跳至 ~100%，持續 3 分鐘不降 | 平穩（無分配） |
| `io` | 觸發後接近 0%（等待 I/O），持續 3 分鐘 | 平穩 |
| `cpu` | 觸發後跳至 ~100%，約 10 秒後回落 | 平穩 |
| `mem` | 平穩（分配有 0.1s sleep） | 線性上升，約 50 秒到達 512 MiB，後容器終止 |
| `parse` | 短暫小幅上升後回落 | 平穩 |

> **mem 測試說明**：容器加上 `--memory=512m` 上限，讓記憶體曲線能被完整記錄。
> 觸發後每 0.1 秒分配 1 MB，預計約 50 秒線性增長至 512 MiB，
> 接著容器被 OOM kill，exit code 為 137。

## 利用 CSV 畫圖

**不會自動產生圖表**，需手動執行以下 Python 腳本（需要 pandas、matplotlib）：

```python
import pandas as pd
import matplotlib.pyplot as plt

df = pd.read_csv("stats_loop.csv")
df["cpu"] = df["cpu_pct"].str.replace("%", "").astype(float)
df["mem_mb"] = df["mem_usage"].str.extract(r"([\d.]+)MiB").astype(float)

fig, ax1 = plt.subplots(figsize=(10, 4))
ax1.plot(df["cpu"], label="CPU %", color="steelblue")
ax1.set_ylabel("CPU %")
ax1.set_xlabel("時間（秒）")
ax2 = ax1.twinx()
ax2.plot(df["mem_mb"], color="orange", label="MEM MiB")
ax2.set_ylabel("Memory (MiB)")
plt.title("Fluent Bit — loop test")
fig.legend(loc="upper left", bbox_to_anchor=(0.1, 0.9))
plt.tight_layout()
plt.savefig("loop_stats.png")
```

將 `stats_loop.csv` 換成其他測試的檔名即可。

## protected_mode 說明

腳本會依測試類型自動設定 `protected_mode`：

| 測試 | protected_mode | 說明 |
|---|---|---|
| loop / io / cpu / parse | `true` | Lua error 被 catch，pipeline 繼續運作 |
| mem | `false` | OOM 不被攔截，process 真正被 kill，才能觀察到記憶體曲線 |

## 檔案結構

```
lua/
├── conf/
│   └── fluentbit_error_tests.conf   # Fluent Bit 設定檔（腳本自動修改 call / protected_mode）
├── script/
│   └── run_crash.sh                 # 測試執行腳本
├── test_errors.lua                  # 5 種錯誤注入函式
├── stats_<test>.csv                 # 執行後產生，每秒一筆 docker stats
└── README.md
```

## 相關工具

| 工具 | 路徑 | 說明 |
|---|---|---|
| loggen | `../../tools/loggen.py` | 產生測試 log，支援 `--error-rate` |
| sink server | `../../tools/sink_server.py` | HTTP sink，監聽 port 8080 |
