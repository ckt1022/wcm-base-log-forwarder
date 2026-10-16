# WCM 最大可持續容量實驗結果

## 結論

初始 8 MiB 配置的最大可持續輸入容量為 **132,000 EPS**。後續 batch-size 掃描與正式確認顯示，**1 MiB 配置可持續達到 136,000 EPS**，並且在共同 100k EPS 下有最低延遲。

- CPU：1 core
- 容器記憶體：512 MB
- Swap：關閉
- Batch 最大等待時間：1 秒
- 最終建議 Batch 大小上限：1 MiB
- Transport worker：1
- Retry：不額外啟用
- 輸入：TCP 單連線

1 MiB 配置的第一個正式失敗點為 **140,000 EPS**。136,000 EPS 已進行確認重跑並再次通過。

```text
FINAL CAPACITY dut=wcm-b1 eps=136000 first_fail=140000 confirmed=yes
```

本結果應描述為：

> 在 1 CPU、512 MB、無 swap、1 秒最大等待時間、1 MiB batch、單 transport worker 的條件下，WCM 的最大可持續輸入容量為 136,000 EPS；140,000 EPS 因負載結束時的 pipeline backlog 超過允許的三秒輸出量而判定不可持續。在共同 100k EPS 下，穩態 p50 為 82.390 ms，p99 為 144.079 ms。

## WCM 固定配置

後續容量與高流量延遲實驗使用：

```text
test/ours/latency-benchmark-Rust/forwarder_membound.yaml
```

完整容量相關配置：

```yaml
plugins:
  parse: "latency_rust_parse.wasm"
  parse_noop: ~
  filter: "latency_rust_filter.wasm"
  format: "latency_rust_format.wasm"
  transport: "transport.wasm"

stages:
  filter: true
  format: true
  transport: true

input:
  mode: tcp
  tcp:
    host: "0.0.0.0"
    port: 5140

batch:
  mem_limit_mb: 128
  buffer_size_mb: 1
  max_wait_ms: 1000
  channel_capacity: 50000
  max_format_chunk: 10000
  max_transport_bytes: 1048576
  transport_workers: 1
  pipeline_channel_capacity: 2
  stage_timeout_ms: 30000
  plugin_instance_reuse: true

endpoint:
  A: "http://127.0.0.1:8080/ingest"

config_reload_secs: 3600
```

配置檔 SHA-256：

```text
027769d386fdfccff1ffa123586a555c006b35afc23fad0fb570a37623419b41
```

## 固定測試條件

| 項目 | 設定 |
|---|---:|
| DUT CPU | 1 core |
| DUT memory | 512 MB |
| Swap | 關閉 |
| Transport concurrency | 1 |
| Input protocol | TCP |
| Input connections | 1 |
| Generator traffic | flat |
| Generator format | json-fixed5 |
| Generator timestamp | scheduled timestamp |
| Generator send unit | batch |
| Generator batch | 500 lines |
| Generator flush interval | 20 ms |
| Capacity load duration | 60 秒 |
| Drain duration | 20 秒 |
| Sink | 獨立 CPU core |
| Generator | 與 DUT 不同的 CPU core |

Generator 主要參數：

```text
-traffic flat
-mode json-fixed5
-ts-mode sched
-output tcp
-send-unit batch
-batch-lines 500
-flush-ms 20
-conns 1
```

## 容量通過條件

一個流量點必須同時滿足以下條件才視為可持續：

1. Generator 實際平均速率至少達到目標速率的 99.5%。
2. Generator `send_lag p99` 不超過 10 ms。
3. Sink 收到至少 99.9% 的預期輸出資料。
4. 負載結束時的 backlog 不超過三秒預期輸出量。
5. Drain 後資料完整。
6. DUT 沒有 OOM、crash 或非預期 connection reset。

固定測試資料經 filter 後約保留 40% 的事件，因此：

```text
expected sink records = generator records × 0.4
```

## 搜尋過程

容量搜尋先使用倍增方式找出失敗上界，再於最後通過點和第一個失敗點之間進行二分搜尋。

| 目標速率 | 結果 |
|---:|:---|
| 64,000 EPS | PASS |
| 128,000 EPS | PASS |
| 256,000 EPS | FAIL |
| 192,000 EPS | FAIL |
| 160,000 EPS | FAIL |
| 144,000 EPS | FAIL |
| 136,000 EPS | FAIL |
| 132,000 EPS | PASS |
| 132,000 EPS 確認重跑 | PASS |

最終搜尋結果：

```text
CAPACITY dut=wcm-v2 eps=132000 first_fail=136000 confirmed=yes
```

## 132,000 EPS 通過結果

確認重跑的結果：

| 指標 | 結果 |
|---|---:|
| Target input rate | 132,000 EPS |
| Actual generator rate | 131,997 EPS |
| Generator input throughput | 21.081 MB/s |
| Generator send lag p99 | 5 ms |
| Generator send lag max | 6.6 ms |
| Sink received records | 3,167,965 |
| Expected sink records | 3,167,964 |
| End-of-load backlog | 123,495 records |
| Steady output rate | 52,009 records/s |
| Output throughput | 10.520 MB/s |
| OOM | false |
| Verdict | PASS |

原始 verdict：

```text
PASS rate=132000 gen_avg=131997/s gen_mb_s=21.081 lag_p99=5ms lag_max=6.6ms sink_total=3167965 expected=3167964 backlog=123495 win_out_eps=52009 win_out_mb_s=10.520 oom=false
```

Generator 實際速率達到目標，`send_lag p99` 為 5 ms，表示 generator 沒有因明顯 backpressure 而漏送排程流量。Sink 收到的資料和預期輸出一致，且容器沒有 OOM。

## 136,000 EPS 失敗結果

| 指標 | 結果 |
|---|---:|
| Target input rate | 136,000 EPS |
| Actual generator rate | 135,997 EPS |
| Generator input throughput | 21.716 MB/s |
| Generator send lag p99 | 5 ms |
| Generator send lag max | 3.8 ms |
| Sink received records | 3,263,951 |
| Expected sink records | 3,263,950 |
| End-of-load backlog | 176,633 records |
| Allowed backlog | 163,200 records |
| Steady output rate | 52,441 records/s |
| Output throughput | 10.607 MB/s |
| OOM | false |
| Verdict | FAIL |

原始 verdict：

```text
FAIL rate=136000 gen_avg=135997/s gen_mb_s=21.716 lag_p99=5ms lag_max=3.8ms sink_total=3263951 expected=3263950 backlog=176633 win_out_eps=52441 win_out_mb_s=10.607 oom=false reasons=backlog=176633>163200
```

136,000 EPS 時，generator 仍能達成目標輸入速率，資料也沒有遺失，而且沒有發生 OOM。失敗原因是負載結束時累積了 176,633 筆 backlog，超過允許的 163,200 筆。

這表示 136,000 EPS 已超過此配置所定義的最大可持續容量；失敗原因是 pipeline 服務率不足以讓 backlog 維持在限制內，而不是 generator 效能不足、資料遺失或容器 OOM。

## 結果檔案

```text
test/compare/capacity/runs/wcm-v2/search_result.txt
test/compare/capacity/runs/wcm-v2/cap_rate_132000/verdict.txt
test/compare/capacity/runs/wcm-v2/cap_rate_132000/gen.log
test/compare/capacity/runs/wcm-v2/cap_rate_132000/sink.log
test/compare/capacity/runs/wcm-v2/cap_rate_132000/recv_per_sec.csv
test/compare/capacity/runs/wcm-v2/cap_rate_132000/oom.txt
test/compare/capacity/runs/wcm-v2/cap_rate_136000/verdict.txt
```

---

## Batch size 掃描：8、16、32、64 MiB

為確認 8 MiB 是否只是保守設定，另外建立 8、16、32、64 MiB 四種配置。除了 `buffer_size_mb` 與對應的 `max_transport_bytes` 外，其餘參數完全相同：

```yaml
batch:
  mem_limit_mb: 128
  max_wait_ms: 1000
  channel_capacity: 50000
  max_format_chunk: 10000
  transport_workers: 1
  pipeline_channel_capacity: 2
  stage_timeout_ms: 30000
  plugin_instance_reuse: true
```

| Profile | `buffer_size_mb` | `max_transport_bytes` |
|---|---:|---:|
| wcm-b8 | 8 MiB | 8,388,608 bytes |
| wcm-b16 | 16 MiB | 16,777,216 bytes |
| wcm-b32 | 32 MiB | 33,554,432 bytes |
| wcm-b64 | 64 MiB | 67,108,864 bytes |

配置檔：

```text
test/ours/latency-benchmark-Rust/forwarder_membound_b8.yaml
test/ours/latency-benchmark-Rust/forwarder_membound_b16.yaml
test/ours/latency-benchmark-Rust/forwarder_membound_b32.yaml
test/ours/latency-benchmark-Rust/forwarder_membound_b64.yaml
```

### 容量比較

| Batch size | 最大通過點 | 第一失敗點 | 確認狀態 | 重要現象 |
|---:|---:|---:|:---|:---|
| 8 MiB | **132,000 EPS** | 136,000 EPS | confirmed=yes | 無 OOM，136k 因 backlog 超限 |
| 16 MiB | 128,000 EPS | 132,000 EPS | confirmed=NO | 128k 有 PASS/FAIL 變異，不是穩定容量 |
| 32 MiB | 124,000 EPS | 128,000 EPS | confirmed=yes | 128k 發生 OOM |
| 64 MiB | 124,000 EPS | 128,000 EPS | confirmed=yes | 128k 發生 WASM trap 並停止正常輸出 |

16 MiB 的搜尋紀錄：

```text
trial rate=64000 PASS
trial rate=128000 PASS
trial rate=256000 FAIL
trial rate=192000 FAIL
trial rate=160000 FAIL
trial rate=144000 FAIL
trial rate=136000 FAIL
trial rate=132000 FAIL
trial rate=128000 FAIL
CAPACITY dut=wcm-b16 eps=128000 first_fail=132000 confirmed=NO(重跑失敗,需人工檢視)
```

因此 128,000 EPS 不能視為 16 MiB 的穩定容量。現有資料只足以說明其容量低於或不穩定於 8 MiB 的 132,000 EPS。

32 MiB 的搜尋結果：

```text
CAPACITY dut=wcm-b32 eps=124000 first_fail=128000 confirmed=yes
```

其 128,000 EPS 測試：

```text
oom=true
```

64 MiB 的搜尋結果：

```text
CAPACITY dut=wcm-b64 eps=124000 first_fail=128000 confirmed=yes
```

64 MiB 在 128,000 EPS 沒有被容器標記為 OOM，但 DUT 出現 WASM trap：

```text
[worker0][trap] exceeded retries, skip batch 19
```

### 共同 100,000 EPS 延遲比較

四組配置皆使用相同 100,000 EPS 工作量：

- 負載時間：90 秒
- 統計窗口：第 20–80 秒
- Timestamp：generator scheduled timestamp
- 延遲終點：HTTP sink 收完 request body
- 四組皆達成約 99,998–99,999 EPS
- `send_lag p99` 皆為 5 ms
- 四組皆無資料遺失、無 OOM

| Batch size | Mean | p50 | p90 | p95 | p99 | p99.9 | Max |
|---:|---:|---:|---:|---:|---:|---:|---:|
| **8 MiB** | **637.705 ms** | **637.707 ms** | **849.003 ms** | **875.413 ms** | **898.415 ms** | **931.708 ms** | **950.747 ms** |
| 16 MiB | 1227.763 ms | 1227.764 ms | 1627.816 ms | 1677.821 ms | 1722.521 ms | 1756.012 ms | 1769.974 ms |
| 32 MiB | 1230.724 ms | 1230.721 ms | 1630.773 ms | 1680.779 ms | 1724.499 ms | 1759.221 ms | 1784.656 ms |
| 64 MiB | 1235.889 ms | 1235.881 ms | 1635.901 ms | 1685.904 ms | 1731.274 ms | 1767.570 ms | 1789.770 ms |

對應的容量／完整性結果：

| Batch size | Actual input | `send_lag p99` | Sink/expected | Backlog | OOM |
|---:|---:|---:|---:|---:|:---|
| 8 MiB | 99,998 EPS | 5 ms | 3,599,976 / 3,599,975 | 50,180 | false |
| 16 MiB | 99,999 EPS | 5 ms | 3,599,999 / 3,599,998 | 79,509 | false |
| 32 MiB | 99,998 EPS | 5 ms | 3,599,967 / 3,599,965 | 39,414 | false |
| 64 MiB | 99,999 EPS | 5 ms | 3,599,969 / 3,599,968 | 39,469 | false |

8 MiB 延遲較低的主要原因不是 WASM 執行本身快一倍，而是其 size trigger 早於 1 秒 timer。

測試資料約 158–160 bytes/event，因此 100,000 EPS 時：

```text
輸入頻寬約 15.9 MB/s
8 MiB 填滿時間約 0.5 秒
16/32/64 MiB 在一秒內通常不會填滿
```

所以：

- 8 MiB 主要約每 0.5 秒由 size trigger flush。
- 16、32、64 MiB 主要由一秒 `max_wait_ms` 觸發。
- 這是 batch size 本身造成的實際端到端延遲差異。

### 第一階段 Batch size 掃描結論（後續已由 1 MiB 正式結果取代）

在目前的 1 CPU、512 MB、無 swap、單 transport worker、`max_wait_ms=1000` 條件下，**8 MiB 同時得到最高的已確認容量與最低的共同工作量延遲**：

```text
最高 confirmed capacity：132,000 EPS
100k EPS p50：637.707 ms
100k EPS p99：898.415 ms
```

在當時只比較 8/16/32/64 MiB 時，暫時建議使用：

```yaml
buffer_size_mb: 8
max_transport_bytes: 8388608
max_wait_ms: 1000
```

此暫時結論已由後續 1 MiB 正式確認取代；最終建議請以本文最後的「1 MiB 正式確認結果」為準。

### Batch size 掃描原始資料

```text
test/compare/capacity/runs/wcm-b16/search_result.txt
test/compare/capacity/runs/wcm-b32/search_result.txt
test/compare/capacity/runs/wcm-b64/search_result.txt

test/compare/capacity/runs/wcm-b8/lat_rate_100000/
test/compare/capacity/runs/wcm-b16/lat_rate_100000/
test/compare/capacity/runs/wcm-b32/lat_rate_100000/
test/compare/capacity/runs/wcm-b64/lat_rate_100000/
```

---

## 小 Batch 快速掃描：1、2、4 MiB

為縮短執行時間，本輪使用快速篩選，不取代前述 60 秒正式容量搜尋：

- 容量負載：30 秒
- Drain：10 秒
- 容量點：132k、136k、144k EPS
- 延遲負載：100k EPS、45 秒
- 延遲統計窗口：第 10–40 秒
- 其餘 CPU、memory、worker、timeout、generator 與前述實驗相同

### 快速容量結果

| Batch | 132k | 136k | 144k | 144k backlog | OOM |
|---:|:---:|:---:|:---:|---:|:---:|
| 1 MiB | PASS | PASS | PASS | 163,373 | false |
| 2 MiB | PASS | PASS | PASS | 168,559 | false |
| 4 MiB | PASS | PASS | **FAIL** | 205,588 | false |

144k 的 backlog 上限為 172,800 records。1、2 MiB 雖通過短測，但已接近上限：

```text
1 MiB: 163373 / 172800 = 94.5%
2 MiB: 168559 / 172800 = 97.5%
```

而三組在 144k 的穩態 sink 輸出約為 52–53k records/s。依 40% filter 保留率換算，實際 pipeline 服務率約為 131–133k input EPS。因此 144k PASS 主要是因為 30 秒負載尚未累積足夠 backlog，不能直接宣稱 144k 是長期可持續容量。

正式容量結論仍維持：

```text
8 MiB: 132,000 EPS confirmed（60 秒負載）
```

1、2 MiB 是否能在正式 60 秒測試超過 132k，仍需另外確認。

### 共同 100k EPS 快速延遲結果

三組都完整達成約 99,997 EPS，`send_lag p99=5ms`，資料完整且沒有 OOM。

| Batch | Mean | p50 | p90 | p95 | p99 | p99.9 | Max |
|---:|---:|---:|---:|---:|---:|---:|---:|
| **1 MiB** | **83.123 ms** | **82.719 ms** | **109.539 ms** | **113.594 ms** | **144.270 ms** | **155.134 ms** | **159.615 ms** |
| 2 MiB | 161.265 ms | 161.266 ms | 214.089 ms | 220.761 ms | 248.676 ms | 264.307 ms | 270.503 ms |
| 4 MiB | 319.491 ms | 319.489 ms | 425.159 ms | 438.366 ms | 452.066 ms | 487.238 ms | 499.617 ms |

延遲大致隨 batch 填滿時間增加：

```text
100k EPS 輸入約 15.9 MB/s
1 MiB 約 63 ms 填滿
2 MiB 約 126 ms 填滿
4 MiB 約 252 ms 填滿
```

實測還包含 parse/filter/format、queue 與 HTTP transport 成本，因此端到端 p50 分別約為 83、161、319 ms。

### 小 Batch 快速掃描結論

- 若目標是目前已正式確認的最高容量：仍使用 **8 MiB / 132k EPS**。
- 若目標是最低高流量延遲：快速測試中 **1 MiB 最佳**。
- 1 MiB 的 100k p99 為 144.270 ms，遠低於 8 MiB 的 898.415 ms。
- 1、2 MiB 的 144k 結果只是短測通過，不是正式容量證明。
- 若後續要選一份兼顧吞吐與延遲的正式配置，最值得補做的是 1 MiB 在 132k、136k 的 60 秒確認，而不需要再測 2/4/16/32/64 MiB。

快速掃描原始資料：

```text
test/compare/capacity/runs/wcm-b1/
test/compare/capacity/runs/wcm-b2/
test/compare/capacity/runs/wcm-b4/
```

---

## 1 MiB 正式確認結果

快速掃描後，以正式 60 秒容量負載與 90 秒延遲負載確認 1 MiB 配置。

使用配置：

```yaml
batch:
  mem_limit_mb: 128
  buffer_size_mb: 1
  max_wait_ms: 1000
  channel_capacity: 50000
  max_format_chunk: 10000
  max_transport_bytes: 1048576
  transport_workers: 1
  pipeline_channel_capacity: 2
  stage_timeout_ms: 30000
  plugin_instance_reuse: true
```

配置檔：

```text
test/ours/latency-benchmark-Rust/forwarder_membound_b1.yaml
```

配置 SHA-256：

```text
459e256d08b426b08131d606d2f6841d22b87faa2aec1f27f105a7347bb51813
```

### 正式容量邊界

| Target | 結果 | Actual input | `send_lag p99` | Backlog | Backlog limit | OOM |
|---:|:---:|---:|---:|---:|---:|:---:|
| 132,000 EPS | PASS | 131,997 EPS | 5 ms | 1,363 | 158,400 | false |
| 136,000 EPS | PASS | 135,997 EPS | 5 ms | 73,837 | 163,200 | false |
| 136,000 EPS 確認重跑 | PASS | 135,997 EPS | 5 ms | 80,557 | 163,200 | false |
| 140,000 EPS | FAIL | 139,997 EPS | 5 ms | 198,331 | 168,000 | false |
| 144,000 EPS | FAIL | 141,968 EPS | 1000 ms | 204,503 | 172,800 | false |

136k 確認重跑：

```text
PASS rate=136000 gen_avg=135997/s gen_mb_s=21.708 lag_p99=5ms lag_max=5.0ms sink_total=3263955 expected=3263954 backlog=80557 win_out_eps=53462 win_out_mb_s=10.854 oom=false
```

140k 第一失敗點：

```text
FAIL rate=140000 gen_avg=139997/s gen_mb_s=22.352 lag_p99=5ms lag_max=2.7ms sink_total=3359979 expected=3359977 backlog=198331 win_out_eps=53187 win_out_mb_s=10.800 oom=false reasons=backlog=198331>168000
```

因此 1 MiB 正式容量結果為：

```text
CAPACITY dut=wcm-b1 eps=136000 first_fail=140000 confirmed=yes
```

相較 8 MiB 的 132,000 EPS，1 MiB confirmed capacity 提高：

```text
(136000 - 132000) / 132000 = 3.03%
```

### 正式 100k EPS 延遲

- 負載：90 秒
- 統計窗口：第 20–80 秒
- 窗口資料：2,400,669 records
- 全部資料：3,599,959 records
- Actual input：99,999 EPS
- Generator `send_lag p99`：5 ms
- OOM：false
- Sink/expected：3,599,959 / 3,599,958

| 指標 | 延遲 |
|---|---:|
| Mean | 82.807 ms |
| Min | 42.717 ms |
| p50 | 82.390 ms |
| p90 | 109.217 ms |
| p95 | 113.135 ms |
| p99 | 144.079 ms |
| p99.9 | 155.289 ms |
| Max | 160.670 ms |

原始容量／完整性 verdict：

```text
PASS rate=100000 gen_avg=99999/s gen_mb_s=15.980 lag_p99=5ms lag_max=3.1ms sink_total=3599959 expected=3599958 backlog=21379 win_out_eps=39978 win_out_mb_s=8.127 oom=false
```

### 1 MiB 與 8 MiB 比較

| 指標 | 1 MiB | 8 MiB | 改善 |
|---|---:|---:|---:|
| Confirmed capacity | **136,000 EPS** | 132,000 EPS | +3.03% |
| 100k p50 | **82.390 ms** | 637.707 ms | -87.1% |
| 100k p99 | **144.079 ms** | 898.415 ms | -84.0% |
| 100k p99.9 | **155.289 ms** | 931.708 ms | -83.3% |
| OOM | false | false | 相同 |

### 最終建議

後續 WCM 容量與高流量延遲實驗，建議改用 1 MiB：

```yaml
buffer_size_mb: 1
max_transport_bytes: 1048576
max_wait_ms: 1000
```

在目前固定資源下，1 MiB 不只顯著降低延遲，也將 confirmed capacity 從 132k 提高至 136k EPS。

正式原始資料：

```text
test/compare/capacity/runs/wcm-b1/cap_rate_132000/
test/compare/capacity/runs/wcm-b1/cap_rate_136000/
test/compare/capacity/runs/wcm-b1/cap_rate_140000/
test/compare/capacity/runs/wcm-b1/cap_rate_144000/
test/compare/capacity/runs/wcm-b1/lat_rate_100000/
```

---

## 固定 1 KiB 大型 Log 實驗

### Workload 設計

原始 `json-fixed5` 約為 160 bytes/event。大型 log 實驗選擇 **1024 bytes/event**，代表包含較多 HTTP metadata、labels、context 或部分 stack trace 的常見結構化應用日誌。

Generator 新增：

```text
-mode json-fixed1k
```

此模式具備：

- 每筆 JSON 精確為 1024 bytes，不含 TCP newline。
- 與 `json-fixed5` 使用完全相同的五種 level/message/attribute 循環。
- warn/error 仍占 40%，因此 filter 保留比例不變。
- Payload 固定且 deterministic，不引入隨機大小變異。
- Timestamp 仍使用 `-ts-mode sched`。

實際長度驗證：

```text
1024
```

為避免覆蓋 160 B 實驗，本次原始資料使用 `_1k` 後綴。

### 固定 WCM 配置

仍使用最終選定的 1 MiB profile：

```yaml
batch:
  mem_limit_mb: 128
  buffer_size_mb: 1
  max_wait_ms: 1000
  channel_capacity: 50000
  max_format_chunk: 10000
  max_transport_bytes: 1048576
  transport_workers: 1
  pipeline_channel_capacity: 2
  stage_timeout_ms: 30000
  plugin_instance_reuse: true
```

### 1 KiB Log 最大可持續容量

| Target rate | 結果 |
|---:|:---|
| 16,000 EPS | PASS |
| 32,000 EPS | PASS |
| 64,000 EPS | PASS |
| 128,000 EPS | FAIL |
| 96,000 EPS | FAIL |
| 80,000 EPS | FAIL |
| 72,000 EPS | PASS |
| 76,000 EPS | FAIL |
| 74,000 EPS | PASS |
| 74,000 EPS 確認重跑 | PASS |

最終結果：

```text
CAPACITY dut=wcm-b1 eps=74000 first_fail=76000 confirmed=yes
```

74k 確認重跑：

```text
PASS rate=74000 gen_avg=73999/s gen_mb_s=75.848 lag_p99=5ms lag_max=7.7ms sink_total=1775998 expected=1775997 backlog=47067 win_out_eps=29360 win_out_mb_s=31.031 oom=false
```

76k 第一失敗點：

```text
FAIL rate=76000 gen_avg=72994/s gen_mb_s=74.819 lag_p99=2328ms lag_max=2327.5ms sink_total=1772712 expected=1772711 backlog=56070 win_out_eps=28511 win_out_mb_s=30.134 oom=false reasons=gen_blocked(avg=72994/76000,lag_p99=2328.0ms)
```

因此：

```text
最大可持續事件率：74,000 EPS
最大可持續輸入頻寬：75.848 MB/s
Filter 後穩態輸出率：29,360 records/s
Filter 後穩態輸出頻寬：31.031 MB/s
第一失敗點：76,000 EPS
OOM：false
```

### 1 KiB Log 延遲

延遲測試使用容量 80%：

```text
74,000 × 0.8 = 59,200 EPS
```

測試條件：

- 負載：90 秒
- 統計窗口：第 20–80 秒
- Actual input：59,199 EPS
- Input throughput：60.679 MB/s
- Generator `send_lag p99`：5 ms
- Sink/expected：2,131,190 / 2,131,189
- OOM：false

| 指標 | 延遲 |
|---|---:|
| Mean | 29.288 ms |
| Min | 12.324 ms |
| p50 | 27.480 ms |
| p90 | 37.466 ms |
| p95 | 53.900 ms |
| p99 | 73.676 ms |
| p99.9 | 90.367 ms |
| Max | 107.148 ms |

原始 verdict：

```text
PASS rate=59200 gen_avg=59199/s gen_mb_s=60.679 lag_p99=5ms lag_max=7.1ms sink_total=2131190 expected=2131189 backlog=10274 win_out_eps=23680 win_out_mb_s=25.027 oom=false
```

### 160 B 與 1 KiB 比較

| 指標 | 約 160 B log | 固定 1 KiB log |
|---|---:|---:|
| Batch | 1 MiB | 1 MiB |
| Confirmed capacity | 136,000 EPS | 74,000 EPS |
| Input throughput at capacity | 21.708 MB/s | **75.848 MB/s** |
| First failed rate | 140,000 EPS | 76,000 EPS |
| OOM at capacity | false | false |

延遲測試的負載點不同：

- 160 B：100k EPS，約為其容量 73.5%。
- 1 KiB：59.2k EPS，為其容量 80%。

| 指標 | 約 160 B @100k | 1 KiB @59.2k |
|---|---:|---:|
| Input throughput | 15.980 MB/s | 60.679 MB/s |
| p50 | 82.390 ms | **27.480 ms** |
| p99 | 144.079 ms | **73.676 ms** |
| p99.9 | 155.289 ms | **90.367 ms** |
| Max | 160.670 ms | **107.148 ms** |

大型 log 的 EPS 容量較低，但 byte throughput 顯著提高。這表示 pipeline 存在明顯的 per-event 固定成本；當單筆 payload 變大時，每秒 event 呼叫次數下降，而連續記憶體複製能得到較高 byte throughput。

1 MiB batch 在 59.2k EPS、每筆 1024 B 時約需：

```text
1 MiB / 60.679 MB/s ≈ 17 ms
```

即可由 size trigger flush，所以端到端 batch waiting 比 160 B workload 更短。這解釋了大型 log 在 80% capacity 下仍有較低延遲。

### 大型 Log 結論

1 MiB 配置在固定 1 KiB log 下依然有效：

- 沒有 OOM。
- 可持續 74k EPS／75.85 MB/s。
- 80% capacity 下 p50 27.48 ms、p99 73.68 ms。
- 超載時透過 TCP backpressure 失敗，而不是丟資料或 OOM。

因此目前沒有證據需要因 log 增大至 1 KiB 而放大 WCM batch；**1 MiB 仍是建議配置**。

原始資料：

```text
test/compare/capacity/runs/wcm-b1/search_result_1k.txt
test/compare/capacity/runs/wcm-b1/cap_rate_74000_1k/
test/compare/capacity/runs/wcm-b1/cap_rate_76000_1k/
test/compare/capacity/runs/wcm-b1/lat_rate_59200_1k/
```

---

## Fluentd 160 B／1 KiB 最大容量實驗

### 固定條件

Fluentd使用和最終WCM容量實驗相同的外部條件：

- CPU：1 core
- Container memory：512 MiB
- Swap：關閉
- TCP單連線
- Generator send batch：500 lines
- Generator flush：20 ms
- Flat traffic
- Scheduled timestamp
- 最大batch等待：1秒
- Batch／HTTP chunk上限：1 MiB
- Output／flush worker：1
- Retry：關閉
- Timeout：30秒
- 負載：60秒
- Drain：20秒
- Filter保留率：40%

Fluentd和WCM的batch位置仍屬產品架構差異：

- WCM：parse前累積1 MiB或等待1秒。
- Fluentd：parse/filter後在output buffer累積chunk。
- Fluentd `chunk_full_threshold` 預設約0.95，因此1 MiB chunk可能約在0.95 MiB時被enqueue。

### Fluentd固定配置

配置檔：

```text
test/compare/fluentd/latency/conf/capacity_1m_1s.conf
```

SHA-256：

```text
ab4df5c8aa9e54bff9b83352ec5d2a52afb9d1ac62de82eac17ccbc3193c06a6
```

完整設定：

```conf
<source>
  @type tcp
  bind 0.0.0.0
  port 5140
  tag latency

  <parse>
    @type json
  </parse>
</source>

<filter latency>
  @type latency_test
</filter>

<match latency>
  @type http
  endpoint http://127.0.0.1:8080/ingest
  open_timeout 30
  read_timeout 30

  <format>
    @type json
  </format>
  json_array false

  <buffer>
    @type memory
    flush_mode interval
    flush_interval 1
    chunk_limit_size 1m
    total_limit_size 256m
    flush_thread_count 1
    flush_thread_interval 0.1
    flush_thread_burst_interval 0.01
    flush_at_shutdown true
    retry_max_times 0
    overflow_action block
  </buffer>
</match>
```

Fluentd image：

```text
fluent/fluentd:v1.16-debian-1
sha256:4f65d55e8f24de4c59c75d4fdd702fb73e4bbe3a25f3fdd77f3c34dc9cec1930
```

### Fluentd約160 B結果

Workload：

```text
GEN_MODE=json-fixed5
RUN_VARIANT=160b
```

原始自動搜尋曾找到176k單次PASS，但176k容量點確認重跑失敗；160k也出現一次PASS、一次FAIL。因此不能使用單次最佳值。

為取得可重複結果，152k另外連續執行兩次，兩次皆PASS。正式穩定容量取152k，第一個較高且重跑失敗的點取160k。

```text
CAPACITY dut=fluentd size=160b eps=152000 first_unstable=160000 confirmed=yes
```

152k確認結果：

```text
PASS rate=152000 gen_avg=151997/s gen_mb_s=24.273 lag_p99=5ms lag_max=4.7ms sink_total=3647994 expected=3647993 backlog=28173 win_out_eps=60832 win_out_mb_s=12.273 oom=false
```

160k失敗結果：

```text
FAIL rate=160000 gen_avg=159885/s gen_mb_s=25.550 lag_p99=50ms lag_max=64.2ms sink_total=3839277 expected=3839275 backlog=3613 win_out_eps=63780 win_out_mb_s=12.876 oom=false reasons=gen_blocked(avg=159885/160000,lag_p99=50.0ms)
```

| 指標 | 152k confirmed |
|---|---:|
| Target rate | 152,000 EPS |
| Actual rate | 151,997 EPS |
| Input throughput | 24.273 MB/s |
| `send_lag p99` | 5 ms |
| Sink total | 3,647,994 |
| Expected | 3,647,993 |
| Backlog | 28,173 |
| Filter後穩態輸出 | 60,832 records/s |
| Filter後輸出頻寬 | 12.273 MB/s |
| OOM | false |

### Fluentd固定1 KiB結果

Workload：

```text
GEN_MODE=json-fixed1k
RUN_VARIANT=1k
```

搜尋過程：

| Target rate | 結果 |
|---:|:---|
| 32,000 EPS | PASS |
| 64,000 EPS | PASS |
| 128,000 EPS | FAIL |
| 96,000 EPS | FAIL |
| 80,000 EPS | FAIL |
| 72,000 EPS | PASS |
| 76,000 EPS | FAIL |
| 74,000 EPS | PASS |
| 74,000 EPS確認重跑 | PASS |

```text
CAPACITY dut=fluentd eps=74000 first_fail=76000 confirmed=yes
```

74k確認結果：

```text
PASS rate=74000 gen_avg=73998/s gen_mb_s=75.848 lag_p99=5ms lag_max=2.4ms sink_total=1775995 expected=1775993 backlog=14477 win_out_eps=29599 win_out_mb_s=31.213 oom=false
```

76k失敗結果：

```text
FAIL rate=76000 gen_avg=75347/s gen_mb_s=77.231 lag_p99=1000ms lag_max=846.1ms sink_total=1815205 expected=1815204 backlog=-1 win_out_eps=30028 win_out_mb_s=31.665 oom=false reasons=gen_blocked(avg=75347/76000,lag_p99=1000.0ms)
```

| 指標 | 74k confirmed |
|---|---:|
| Target rate | 74,000 EPS |
| Actual rate | 73,998 EPS |
| Input throughput | 75.848 MB/s |
| `send_lag p99` | 5 ms |
| Sink total | 1,775,995 |
| Expected | 1,775,993 |
| Backlog | 14,477 |
| Filter後穩態輸出 | 29,599 records/s |
| Filter後輸出頻寬 | 31.213 MB/s |
| OOM | false |

### Fluentd大小效應

| Log大小 | Confirmed capacity | Input throughput | First failed／unstable |
|---|---:|---:|---:|
| 約160 B | 152,000 EPS | 24.273 MB/s | 160,000 EPS |
| 固定1 KiB | 74,000 EPS | 75.848 MB/s | 76,000 EPS |

和WCM相同，Fluentd在大型log下的EPS下降，但byte throughput顯著提高。這同樣支持存在明顯的per-event固定成本。

### Fluentd與WCM容量比較

兩者都使用1 MiB batch/chunk、1秒最大等待、1 CPU、512 MiB、無swap、單output worker與相同workload。

| Product | 約160 B capacity | 約160 B input MB/s | 1 KiB capacity | 1 KiB input MB/s |
|---|---:|---:|---:|---:|
| WCM | 136,000 EPS | 21.708 | 74,000 EPS | 75.848 |
| Fluentd | **152,000 EPS** | **24.273** | 74,000 EPS | 75.848 |

觀察：

- 約160 B時，Fluentd confirmed EPS比WCM高約11.8%。
- 固定1 KiB時，兩者confirmed capacity相同，都是74k EPS。
- 1 KiB時兩者都在76k出現generator backpressure，而非OOM。
- 兩個Fluentd容量點都沒有資料遺失或OOM。

### Fluentd原始資料

```text
test/compare/capacity/runs/fluentd/cap_rate_152000_160b/
test/compare/capacity/runs/fluentd/cap_rate_160000_160b/
test/compare/capacity/runs/fluentd/search_result_160b.txt
test/compare/capacity/runs/fluentd/search_result_160b_final.txt

test/compare/capacity/runs/fluentd/cap_rate_74000_1k/
test/compare/capacity/runs/fluentd/cap_rate_76000_1k/
test/compare/capacity/runs/fluentd/search_result_1k.txt
```

---

## Vector 160 B／1 KiB 最大容量實驗

### 固定條件

Vector 使用和 WCM、Fluentd 正式容量實驗相同的外部條件：

- CPU：1 core
- Container memory：512 MiB
- Swap：關閉
- TCP 單連線
- Generator send batch：500 lines
- Generator flush：20 ms
- Flat traffic
- Scheduled timestamp
- 最大 batch 等待：1 秒
- Batch／HTTP request 上限：1 MiB
- HTTP request concurrency：1
- Retry：關閉
- Timeout：30 秒
- 負載：60 秒
- Drain：20 秒
- Filter 保留率：40%

Vector 與另外兩個產品的 batch 位置仍是產品架構差異：

- WCM：parse 前累積 1 MiB 或等待 1 秒。
- Fluentd：parse/filter 後在 output buffer 累積 chunk。
- Vector：socket decoding 與 VRL parse/filter/format 完成後，在 HTTP sink 累積 batch。

### Vector 固定配置

配置檔：

```text
test/compare/vector/latency/conf/latency_tuned.yaml
```

SHA-256：

```text
9f778e3a02a285cf3876685318988e975001feebad593ac866cf42397ccbc399
```

容量實驗注入的環境參數：

```text
FLUSH_INTERVAL=1
VECTOR_BATCH_MAX_BYTES=1048576
```

核心配置如下；完整 VRL transform 保留原實驗的 level mapping、低於 warn
丟棄、欄位重新命名及 root／`att` tags 攤平邏輯。

```yaml
sources:
  tcp_in:
    type: socket
    mode: tcp
    address: "0.0.0.0:5140"
    decoding:
      codec: json

transforms:
  latency_filter:
    type: remap
    inputs: ["tcp_in"]
    drop_on_abort: true
    drop_on_error: false
    # 完整 source 見上述配置檔

sinks:
  out_http:
    type: http
    inputs: ["latency_filter"]
    uri: "http://127.0.0.1:8080/ingest"
    method: post
    encoding:
      codec: json
    framing:
      method: newline_delimited
    compression: none
    batch:
      timeout_secs: ${FLUSH_INTERVAL:-0.1}
      max_bytes: ${VECTOR_BATCH_MAX_BYTES:-134217728}
    request:
      concurrency: 1
      timeout_secs: 30
      retry_attempts: 0
    buffer:
      type: memory
      max_events: 150000
    healthcheck:
      enabled: false
```

配置已用 Vector 自身的 `validate` 命令驗證通過。

Vector image：

```text
timberio/vector:0.49.0-alpine
sha256:30141ab1ce5a7f84ef63580863e4ec561291e7c43dd0d7252985c9f2bb87aa5e
```

### Vector 約 160 B 結果

Workload：

```text
GEN_MODE=json-fixed5
RUN_VARIANT=160b
```

搜尋過程：

| Target rate | 結果 |
|---:|:---|
| 32,000 EPS | PASS |
| 64,000 EPS | PASS |
| 128,000 EPS | FAIL |
| 96,000 EPS | FAIL |
| 80,000 EPS | FAIL |
| 72,000 EPS | FAIL |
| 68,000 EPS | FAIL |
| 66,000 EPS | PASS |
| 66,000 EPS 確認重跑 | PASS |

```text
CAPACITY dut=vector eps=66000 first_fail=68000 confirmed=yes
```

66k 確認結果：

```text
PASS rate=66000 gen_avg=65999/s gen_mb_s=10.537 lag_p99=5ms lag_max=5.4ms sink_total=1583977 expected=1583976 backlog=15366 win_out_eps=26396 win_out_mb_s=5.143 oom=false
```

68k 失敗結果：

```text
FAIL rate=68000 gen_avg=67496/s gen_mb_s=10.782 lag_p99=500ms lag_max=493.0ms sink_total=1625181 expected=1625180 backlog=14191 win_out_eps=26706 win_out_mb_s=5.206 oom=false reasons=gen_blocked(avg=67496/68000,lag_p99=500.0ms)
```

| 指標 | 66k confirmed |
|---|---:|
| Target rate | 66,000 EPS |
| Actual rate | 65,999 EPS |
| Input throughput | 10.537 MB/s |
| `send_lag p99` | 5 ms |
| Sink total | 1,583,977 |
| Expected | 1,583,976 |
| Backlog | 15,366 |
| Filter 後穩態輸出 | 26,396 records/s |
| Filter 後輸出頻寬 | 5.143 MB/s |
| OOM | false |

### Vector 固定 1 KiB 結果

Workload：

```text
GEN_MODE=json-fixed1k
RUN_VARIANT=1k
```

搜尋過程：

| Target rate | 結果 |
|---:|:---|
| 16,000 EPS | PASS |
| 32,000 EPS | PASS |
| 64,000 EPS | FAIL |
| 48,000 EPS | PASS |
| 56,000 EPS | PASS |
| 60,000 EPS | PASS |
| 62,000 EPS | FAIL |
| 60,000 EPS 確認重跑 | PASS |

```text
CAPACITY dut=vector eps=60000 first_fail=62000 confirmed=yes
```

60k 確認結果：

```text
PASS rate=60000 gen_avg=59999/s gen_mb_s=61.499 lag_p99=5ms lag_max=2.4ms sink_total=1439979 expected=1439978 backlog=539 win_out_eps=24004 win_out_mb_s=25.156 oom=false
```

62k 失敗結果：

```text
FAIL rate=62000 gen_avg=60359/s gen_mb_s=61.868 lag_p99=2128ms lag_max=2128.4ms sink_total=1450515 expected=1450513 backlog=63 win_out_eps=24004 win_out_mb_s=25.156 oom=false reasons=gen_blocked(avg=60359/62000,lag_p99=2128.0ms)
```

| 指標 | 60k confirmed |
|---|---:|
| Target rate | 60,000 EPS |
| Actual rate | 59,999 EPS |
| Input throughput | 61.499 MB/s |
| `send_lag p99` | 5 ms |
| Sink total | 1,439,979 |
| Expected | 1,439,978 |
| Backlog | 539 |
| Filter 後穩態輸出 | 24,004 records/s |
| Filter 後輸出頻寬 | 25.156 MB/s |
| OOM | false |

### Vector 大小效應

| Log 大小 | Confirmed capacity | Input throughput | First failed |
|---|---:|---:|---:|
| 約 160 B | 66,000 EPS | 10.537 MB/s | 68,000 EPS |
| 固定 1 KiB | 60,000 EPS | 61.499 MB/s | 62,000 EPS |

從約 160 B 增加至固定 1 KiB 後，Vector 的 EPS 僅下降約 9.1%，但 input
byte throughput 提高約 5.84 倍。這表示本 Vector pipeline 也有明顯的
per-event 固定成本；大型 log 增加的 payload 搬運與序列化成本存在，但不是
160 B workload 的主要容量限制。

兩個 first-fail 點都屬 generator 受到 TCP backpressure，最終收件數仍完整，
也都沒有 OOM。

### 三產品容量比較

三者都使用 1 MiB batch/chunk、1 秒最大等待、1 CPU、512 MiB、無 swap、
單一 output/request concurrency、無 retry 與相同 workload。

| Product | 約 160 B capacity | 約 160 B input MB/s | 1 KiB capacity | 1 KiB input MB/s |
|---|---:|---:|---:|---:|
| WCM | 136,000 EPS | 21.708 | 74,000 EPS | 75.848 |
| Fluentd | **152,000 EPS** | **24.273** | **74,000 EPS** | **75.848** |
| Vector | 66,000 EPS | 10.537 | 60,000 EPS | 61.499 |

在這份含完整欄位攤平與格式化的配置下：

- 約 160 B：Vector 容量為 WCM 的 48.5%，為 Fluentd 的 43.4%。
- 固定 1 KiB：Vector 容量為 WCM／Fluentd 的 81.1%。
- Vector 從小 log 換成 1 KiB 時 EPS 降幅較小，進一步顯示小 log 測試主要受
  per-event 的 JSON decode、VRL transform、event allocation 與 encode 成本影響。

這是目前固定公平參數下的「完整 pipeline 容量」，不是 Vector HTTP sink 的
純輸出極限；不同產品的 parser/filter/formatter 實作效率本來就是 DUT 的一部分。

### Vector 原始資料

```text
test/compare/capacity/runs/vector/cap_rate_66000_160b/
test/compare/capacity/runs/vector/cap_rate_68000_160b/
test/compare/capacity/runs/vector/search_result_160b.txt

test/compare/capacity/runs/vector/cap_rate_60000_1k/
test/compare/capacity/runs/vector/cap_rate_62000_1k/
test/compare/capacity/runs/vector/search_result_1k.txt
```

---

## Fluent Bit Lua／WASM 160 B／1 KiB 最大容量實驗

### 共同固定條件

兩個 Fluent Bit 版本使用與前述正式容量實驗相同的外部條件：

- Image：`fluent/fluent-bit:5.0.9`
- CPU：1 core
- Container memory：512 MiB
- Swap：關閉
- TCP 單連線
- Generator send batch：500 lines
- Generator flush：20 ms
- Flat traffic
- Scheduled timestamp
- `[SERVICE] Flush`：1 秒
- HTTP output worker：1
- Retry：關閉
- 負載：60 秒
- Drain：20 秒
- Filter 保留率：40%
- 相同 TCP input、JSON parse、欄位轉換、level filter、NDJSON HTTP output

Fluent Bit 的 engine chunk 是約 2 MiB soft limit，沒有可將 HTTP output
batch 精確限制為 1 MiB 的設定。因此這裡固定 Fluent Bit 的原生 chunk 策略，
只精確對齊 1 秒 flush、單 output worker 與其他外部條件。這是產品架構差異，
不能將結果解讀為「完全相同內部 batch 實作」。

Fluent Bit image：

```text
fluent/fluent-bit:5.0.9
sha256:bbce9a7b36b0ddd4fcb01a5320815808d1b8b5e1e100725ac95c2b6714d64964
```

兩份配置均已使用 Fluent Bit `--dry-run` 驗證通過。

### Fluent Bit Lua 固定配置

配置檔：

```text
test/compare/fluentbit/latency/conf/capacity_lua_1s.conf
```

配置 SHA-256：

```text
0a1ebdaa87c0a75f00ff3be5bfbe3cdc738aa8a8a34b70e22002b1622c5c9425
```

Lua filter：

```text
test/compare/fluentbit/latency/latency_filter.lua
SHA-256 e9c82e8849572dcfd3d5a62ddc81d2139e8af7726865809b7daf310a79d4a688
```

完整 Fluent Bit 配置：

```conf
[SERVICE]
    Flush         1
    Grace         5
    Daemon        off
    Log_Level     info
    HTTP_Server   off

[INPUT]
    Name          tcp
    Listen        0.0.0.0
    Port          5140
    Format        json
    Tag           latency

[FILTER]
    Name            lua
    Match           latency
    Script          ../latency_filter.lua
    Call            latency_filter
    protected_mode  false

[OUTPUT]
    Name           http
    Match          latency
    Host           127.0.0.1
    Port           8080
    URI            /ingest
    Format         json_lines
    Json_date_key  false
    Workers        1
    Retry_Limit    False
```

#### Lua 約 160 B 結果

Workload：

```text
GEN_MODE=json-fixed5
RUN_VARIANT=160b
```

搜尋結果：

```text
128000 PASS
256000 PASS
512000 FAIL
384000 FAIL
320000 FAIL
288000 PASS
304000 FAIL
296000 FAIL
288000 confirmed PASS
CAPACITY dut=fbit-lua eps=288000 first_fail=296000 confirmed=yes
```

288k 確認結果：

```text
PASS rate=288000 gen_avg=287994/s gen_mb_s=45.989 lag_p99=5ms lag_max=4.1ms sink_total=6911949 expected=6911948 backlog=151588 win_out_eps=115200 win_out_mb_s=23.142 oom=false
```

296k 失敗結果：

```text
FAIL rate=296000 gen_avg=291340/s gen_mb_s=46.517 lag_p99=1000ms lag_max=857.0ms sink_total=6997543 expected=6997542 backlog=140055 win_out_eps=116450 win_out_mb_s=23.391 oom=false reasons=gen_blocked(avg=291340/296000,lag_p99=1000.0ms)
```

| 指標 | 288k confirmed |
|---|---:|
| Actual rate | 287,994 EPS |
| Input throughput | 45.989 MB/s |
| `send_lag p99` | 5 ms |
| Sink total／expected | 6,911,949／6,911,948 |
| Backlog | 151,588 |
| Filter 後穩態輸出 | 115,200 records/s |
| Filter 後輸出頻寬 | 23.142 MB/s |
| OOM | false |

#### Lua 固定 1 KiB 結果

Workload：

```text
GEN_MODE=json-fixed1k
RUN_VARIANT=1k
```

搜尋結果：

```text
32000 PASS
64000 PASS
128000 PASS
256000 FAIL
192000 FAIL
160000 FAIL
144000 FAIL
136000 PASS
140000 PASS
140000 confirmed PASS
CAPACITY dut=fbit-lua eps=140000 first_fail=144000 confirmed=yes
```

140k 確認結果：

```text
PASS rate=140000 gen_avg=139997/s gen_mb_s=143.497 lag_p99=10ms lag_max=47.8ms sink_total=3359937 expected=3359936 backlog=76386 win_out_eps=56001 win_out_mb_s=59.014 oom=false
```

144k 失敗結果：

```text
FAIL rate=144000 gen_avg=143996/s gen_mb_s=147.596 lag_p99=50ms lag_max=61.8ms sink_total=3455979 expected=3455978 backlog=74811 win_out_eps=57600 win_out_mb_s=60.700 oom=false reasons=gen_blocked(avg=143996/144000,lag_p99=50.0ms)
```

| 指標 | 140k confirmed |
|---|---:|
| Actual rate | 139,997 EPS |
| Input throughput | 143.497 MB/s |
| `send_lag p99` | 10 ms |
| Sink total／expected | 3,359,937／3,359,936 |
| Backlog | 76,386 |
| Filter 後穩態輸出 | 56,001 records/s |
| Filter 後輸出頻寬 | 59.014 MB/s |
| OOM | false |

Lua 的 1 KiB confirmed 點剛好位於 `send_lag p99 <= 10 ms` 的接受邊界；
144k 的 p99 已升至 50 ms，因此正式容量取 140k。

### Fluent Bit WASM 固定配置

配置檔：

```text
test/compare/fluentbit/latency/conf/capacity_wasm_1s.conf
```

配置 SHA-256：

```text
e7375ca61b9db818d840c351c2470ef609fa1d7d395e0dec9b91feb91c64afc4
```

WASM filter：

```text
test/compare/fluentbit/latency/wasm_file/latency_filter.wasm
SHA-256 f203ced7c028827660ebf432072b22fefb03197e4f109428e760c432ce6eafaa
```

完整 Fluent Bit 配置：

```conf
[SERVICE]
    Flush         1
    Grace         5
    Daemon        off
    Log_Level     info
    HTTP_Server   off

[INPUT]
    Name          tcp
    Listen        0.0.0.0
    Port          5140
    Format        json
    Tag           latency

[FILTER]
    Name             wasm
    Match            latency
    WASM_Path        ../wasm_file/latency_filter.wasm
    Function_Name    filter_func
    Wasm_Heap_Size   268435456
    Wasm_Stack_Size  8388608

[OUTPUT]
    Name           http
    Match          latency
    Host           127.0.0.1
    Port           8080
    URI            /ingest
    Format         json_lines
    Json_date_key  false
    Workers        1
    Retry_Limit    False
```

#### WASM 約 160 B 結果

Workload：

```text
GEN_MODE=json-fixed5
RUN_VARIANT=160b
```

搜尋結果：

```text
16000 PASS
32000 FAIL
24000 PASS
28000 PASS
30000 FAIL
29000 FAIL
28000 confirmed PASS
CAPACITY dut=fbit-wasm eps=28000 first_fail=29000 confirmed=yes
```

28k 確認結果：

```text
PASS rate=28000 gen_avg=27999/s gen_mb_s=4.474 lag_p99=5ms lag_max=2.5ms sink_total=671993 expected=671992 backlog=9389 win_out_eps=11199 win_out_mb_s=2.251 oom=false
```

29k 失敗結果：

```text
FAIL rate=29000 gen_avg=28537/s gen_mb_s=4.557 lag_p99=1000ms lag_max=970.9ms sink_total=689893 expected=689892 backlog=15473 win_out_eps=11191 win_out_mb_s=2.248 oom=false reasons=gen_blocked(avg=28537/29000,lag_p99=1000.0ms)
```

| 指標 | 28k confirmed |
|---|---:|
| Actual rate | 27,999 EPS |
| Input throughput | 4.474 MB/s |
| `send_lag p99` | 5 ms |
| Sink total／expected | 671,993／671,992 |
| Backlog | 9,389 |
| Filter 後穩態輸出 | 11,199 records/s |
| Filter 後輸出頻寬 | 2.251 MB/s |
| OOM | false |

#### WASM 固定 1 KiB 結果

Workload：

```text
GEN_MODE=json-fixed1k
RUN_VARIANT=1k
```

搜尋結果：

```text
16000 PASS
32000 FAIL
24000 FAIL
20000 FAIL
18000 PASS
19000 PASS
19500 FAIL
19000 confirmed PASS
CAPACITY dut=fbit-wasm eps=19000 first_fail=19500 confirmed=yes
```

19k 確認結果：

```text
PASS rate=19000 gen_avg=19000/s gen_mb_s=19.475 lag_p99=5ms lag_max=4.3ms sink_total=455997 expected=455995 backlog=5968 win_out_eps=7600 win_out_mb_s=8.010 oom=false
```

19.5k 失敗結果：

```text
FAIL rate=19500 gen_avg=19447/s gen_mb_s=19.933 lag_p99=500ms lag_max=189.6ms sink_total=467122 expected=467121 backlog=12640 win_out_eps=7737 win_out_mb_s=8.154 oom=false reasons=gen_blocked(avg=19447/19500,lag_p99=500.0ms)
```

| 指標 | 19k confirmed |
|---|---:|
| Actual rate | 19,000 EPS |
| Input throughput | 19.475 MB/s |
| `send_lag p99` | 5 ms |
| Sink total／expected | 455,997／455,995 |
| Backlog | 5,968 |
| Filter 後穩態輸出 | 7,600 records/s |
| Filter 後輸出頻寬 | 8.010 MB/s |
| OOM | false |

### Fluent Bit log 大小效應

| Filter | Log 大小 | Confirmed capacity | Input throughput | First failed |
|---|---|---:|---:|---:|
| Lua | 約 160 B | 288,000 EPS | 45.989 MB/s | 296,000 EPS |
| Lua | 固定 1 KiB | 140,000 EPS | 143.497 MB/s | 144,000 EPS |
| WASM | 約 160 B | 28,000 EPS | 4.474 MB/s | 29,000 EPS |
| WASM | 固定 1 KiB | 19,000 EPS | 19.475 MB/s | 19,500 EPS |

觀察：

- Lua 從約 160 B 換成 1 KiB，EPS 下降約 51.4%，input byte throughput
  提高約 3.12 倍。
- WASM 從約 160 B 換成 1 KiB，EPS 下降約 32.1%，input byte throughput
  提高約 4.35 倍。
- 四個首個失敗點皆因 generator 受到 TCP backpressure；最終資料完整，
  且都沒有 OOM。
- Lua 與 WASM 共用相同 Fluent Bit input、engine、HTTP output 和外部資源；
  巨大差距主要反映這兩份 filter runtime／實作的成本，不能推廣成所有
  Lua runtime 必然比所有 WASM runtime 快。

### 五種受測 pipeline 容量比較

| Product／filter | 約 160 B capacity | 約 160 B input MB/s | 1 KiB capacity | 1 KiB input MB/s |
|---|---:|---:|---:|---:|
| WCM Rust WASM | 136,000 EPS | 21.708 | 74,000 EPS | 75.848 |
| Fluentd Ruby | 152,000 EPS | 24.273 | 74,000 EPS | 75.848 |
| Vector VRL | 66,000 EPS | 10.537 | 60,000 EPS | 61.499 |
| Fluent Bit Lua | **288,000 EPS** | **45.989** | **140,000 EPS** | **143.497** |
| Fluent Bit WASM | 28,000 EPS | 4.474 | 19,000 EPS | 19.475 |

在本次功能等價配置下，Fluent Bit Lua 是兩種 log 大小的最高容量；
Fluent Bit WASM 則是最低。這是完整 pipeline 和指定 filter 實作的結果，
不是單獨 Fluent Bit engine、Lua 語言或 WebAssembly 技術的一般性排名。

### Fluent Bit 原始資料

```text
test/compare/capacity/runs/fbit-lua/search_result_160b.txt
test/compare/capacity/runs/fbit-lua/cap_rate_288000_160b/
test/compare/capacity/runs/fbit-lua/cap_rate_296000_160b/

test/compare/capacity/runs/fbit-lua/search_result_1k.txt
test/compare/capacity/runs/fbit-lua/cap_rate_140000_1k/
test/compare/capacity/runs/fbit-lua/cap_rate_144000_1k/

test/compare/capacity/runs/fbit-wasm/search_result_160b.txt
test/compare/capacity/runs/fbit-wasm/cap_rate_28000_160b/
test/compare/capacity/runs/fbit-wasm/cap_rate_29000_160b/

test/compare/capacity/runs/fbit-wasm/search_result_1k.txt
test/compare/capacity/runs/fbit-wasm/cap_rate_19000_1k/
test/compare/capacity/runs/fbit-wasm/cap_rate_19500_1k/
```

---

## 固定 1 KiB 多 JSON 欄位五產品容量實驗

### 實驗目的

前一組固定 1 KiB workload 主要以單一 `payload` 字串補足大小；本組保持每筆
JSON 精確 1024 bytes，但完全移除 `payload`，改成多個獨立欄位，以量測
per-field parse、欄位配置、flatten、copy 與 encode 成本。

### Workload

Generator mode：

```text
GEN_MODE=json-fields1k
RUN_VARIANT=fields1k
```

實作：

```text
tools/gen/main.go
SHA-256 f41236fdeb9cc2289bea948e332d2a05987d6929d02efbac50d470166d6c148b
```

已編譯 generator：

```text
test/compare/capacity/gen_bin
SHA-256 8cf3599daeeb67b7625492aa8369fdc98b3f8d236d35ca5fca492fc1c32c08b1
```

Workload 特性：

- 每筆 JSON 不含 TCP 換行時精確 1024 bytes。
- 五條固定內容循環。
- Level 分布：`info / warn / error / debug / info`。
- Filter 後保留率：40%。
- 每筆有 35–36 個 top-level 欄位，另有 `att` 物件內兩個欄位。
- 沒有 `payload` 欄位。
- 26 個主要新增欄位包含 region、zone、environment、hostname、namespace、
  pod、container、cluster、version、trace/span/request ID、HTTP metadata、
  tenant/account、operation、component、datacenter、release、team。
- 剩餘 bytes 平均分配至 4–5 個 `detail_NN` 欄位，每個 value 不超過
  32 bytes，避免退化成另一個大型 payload。
- 五條樣本均已驗證為有效 JSON、精確 1024 bytes 且沒有 `payload`。

其餘外部條件與前一輪固定 1 KiB 實驗相同：

- 1 CPU、512 MiB、無 swap
- TCP 單連線
- Generator 500 lines/send batch、20 ms flush
- Flat traffic、scheduled timestamp
- 1 秒最大 flush interval
- 單一 HTTP output worker／request concurrency
- Retry 關閉
- 60 秒負載、20 秒 drain
- 相同 parse/filter/format/output 語意與相同 PASS 門檻

WCM、Fluentd、Vector 使用 1 MiB batch/chunk 設定；Fluent Bit 保留產品原生
約 2 MiB soft chunk，與前一輪相同。

### 最大確認容量

| Product／filter | Confirmed capacity | Input throughput | First failed | `send_lag p99` at capacity | OOM |
|---|---:|---:|---:|---:|:---:|
| WCM Rust WASM | 23,000 EPS | 23.574 MB/s | 24,000 EPS | 5 ms | false |
| Fluentd Ruby | 50,000 EPS | 51.249 MB/s | 52,000 EPS | 5 ms | false |
| Vector VRL | 15,500 EPS | 15.887 MB/s | 16,000 EPS | 5 ms | false |
| Fluent Bit Lua | **66,000 EPS** | **67.648 MB/s** | 68,000 EPS | 5 ms | false |
| Fluent Bit WASM | 5,500 EPS | 5.637 MB/s | 5,700 EPS | 5 ms | false |

### WCM 結果

搜尋：

```text
16000 PASS
32000 FAIL
24000 FAIL
20000 PASS
22000 PASS
23000 PASS
23000 confirmed PASS
CAPACITY dut=wcm-b1 eps=23000 first_fail=24000 confirmed=yes
```

確認容量：

```text
PASS rate=23000 gen_avg=22999/s gen_mb_s=23.574 lag_p99=5ms lag_max=3.7ms sink_total=551991 expected=551990 backlog=3939 win_out_eps=9200 win_out_mb_s=9.718 oom=false
```

首個失敗點：

```text
FAIL rate=24000 gen_avg=23030/s gen_mb_s=23.606 lag_p99=2282ms lag_max=2282.1ms sink_total=572040 expected=572039 backlog=46925 win_out_eps=8444 win_out_mb_s=8.919 oom=false reasons=gen_blocked(avg=23030/24000,lag_p99=2282.0ms);backlog=46925>28800
```

### Fluentd 結果

搜尋：

```text
8000 PASS
16000 PASS
32000 PASS
64000 FAIL
48000 PASS
56000 FAIL
52000 FAIL
50000 PASS
50000 confirmed PASS
CAPACITY dut=fluentd eps=50000 first_fail=52000 confirmed=yes
```

確認容量：

```text
PASS rate=50000 gen_avg=49999/s gen_mb_s=51.249 lag_p99=5ms lag_max=4.0ms sink_total=1199977 expected=1199976 backlog=8241 win_out_eps=19984 win_out_mb_s=21.067 oom=false
```

首個失敗點：

```text
FAIL rate=52000 gen_avg=49394/s gen_mb_s=50.629 lag_p99=3025ms lag_max=3024.7ms sink_total=1202021 expected=1202019 backlog=6392 win_out_eps=19722 win_out_mb_s=20.790 oom=false reasons=gen_blocked(avg=49394/52000,lag_p99=3025.0ms)
```

### Vector 結果

搜尋：

```text
8000 PASS
16000 FAIL
12000 PASS
14000 PASS
15000 PASS
15500 PASS
15500 confirmed PASS
CAPACITY dut=vector eps=15500 first_fail=16000 confirmed=yes
```

確認容量：

```text
PASS rate=15500 gen_avg=15500/s gen_mb_s=15.887 lag_p99=5ms lag_max=2.6ms sink_total=371999 expected=371997 backlog=2627 win_out_eps=6206 win_out_mb_s=6.504 oom=false
```

首個失敗點：

```text
FAIL rate=16000 gen_avg=15951/s gen_mb_s=16.350 lag_p99=500ms lag_max=228.0ms sink_total=383182 expected=383181 backlog=6804 win_out_eps=6306 win_out_mb_s=6.609 oom=false reasons=gen_blocked(avg=15951/16000,lag_p99=500.0ms)
```

### Fluent Bit Lua 結果

搜尋：

```text
32000 PASS
64000 PASS
128000 FAIL
96000 FAIL
80000 FAIL
72000 FAIL
68000 FAIL
66000 PASS
66000 confirmed PASS
CAPACITY dut=fbit-lua eps=66000 first_fail=68000 confirmed=yes
```

確認容量：

```text
PASS rate=66000 gen_avg=65998/s gen_mb_s=67.648 lag_p99=5ms lag_max=17.4ms sink_total=1583982 expected=1583981 backlog=21298 win_out_eps=26400 win_out_mb_s=27.811 oom=false
```

首個失敗點：

```text
FAIL rate=68000 gen_avg=67998/s gen_mb_s=69.698 lag_p99=50ms lag_max=47.2ms sink_total=1631975 expected=1631974 backlog=21568 win_out_eps=27199 win_out_mb_s=28.653 oom=false reasons=gen_blocked(avg=67998/68000,lag_p99=50.0ms)
```

### Fluent Bit WASM 結果

搜尋：

```text
4000 PASS
8000 FAIL
6000 FAIL
5000 PASS
5500 PASS
5700 FAIL
5500 confirmed PASS
CAPACITY dut=fbit-wasm eps=5500 first_fail=5700 confirmed=yes
```

確認容量：

```text
PASS rate=5500 gen_avg=5500/s gen_mb_s=5.637 lag_p99=5ms lag_max=2.5ms sink_total=131999 expected=131998 backlog=2968 win_out_eps=2200 win_out_mb_s=2.317 oom=false
```

首個失敗點：

```text
FAIL rate=5700 gen_avg=5454/s gen_mb_s=5.590 lag_p99=2677ms lag_max=2677.0ms sink_total=136605 expected=136604 backlog=2015 win_out_eps=2166 win_out_mb_s=2.282 oom=false reasons=gen_blocked(avg=5454/5700,lag_p99=2677.0ms)
```

### 與單一 payload 1 KiB 的比較

兩組輸入每筆都是精確 1024 bytes；唯一主要差異是 byte 分布在單一大型
`payload`，或分布在 35–36 個獨立 JSON 欄位。

| Product／filter | 單一 payload 1 KiB | 多欄位 1 KiB | EPS 下降 |
|---|---:|---:|---:|
| WCM Rust WASM | 74,000 | 23,000 | 68.9% |
| Fluentd Ruby | 74,000 | 50,000 | 32.4% |
| Vector VRL | 60,000 | 15,500 | 74.2% |
| Fluent Bit Lua | 140,000 | 66,000 | 52.9% |
| Fluent Bit WASM | 19,000 | 5,500 | 71.1% |

所有產品都在相同 byte size 下因欄位數增加而降低 EPS，證明容量不只由 log
bytes 決定，JSON token 數、object entries、key/value 配置、欄位遍歷、
flatten 與重新編碼都有顯著成本。

本組中：

- Fluent Bit Lua 仍是最高容量。
- Fluentd 對欄位增加的相對退化最小。
- Vector 相對退化最大，符合其 VRL 對每個 root 欄位執行動態
  `for_each`、型別判斷及 `set!` 的熱路徑。
- WCM 的 Rust parser 需為每個欄位建立 tag key/value，經 WIT component
  boundary 傳遞，再由 formatter 逐欄位編碼；這解釋為何同樣 1 KiB，
  多欄位容量遠低於單一 payload。
- Fluent Bit WASM 的絕對容量最低，顯示這份 filter 的逐欄位 WASM
  runtime／序列化成本很高。

### 原始資料

```text
test/compare/capacity/runs/wcm-b1/search_result_fields1k.txt
test/compare/capacity/runs/wcm-b1/cap_rate_23000_fields1k/
test/compare/capacity/runs/wcm-b1/cap_rate_24000_fields1k/

test/compare/capacity/runs/fluentd/search_result_fields1k.txt
test/compare/capacity/runs/fluentd/cap_rate_50000_fields1k/
test/compare/capacity/runs/fluentd/cap_rate_52000_fields1k/

test/compare/capacity/runs/vector/search_result_fields1k.txt
test/compare/capacity/runs/vector/cap_rate_15500_fields1k/
test/compare/capacity/runs/vector/cap_rate_16000_fields1k/

test/compare/capacity/runs/fbit-lua/search_result_fields1k.txt
test/compare/capacity/runs/fbit-lua/cap_rate_66000_fields1k/
test/compare/capacity/runs/fbit-lua/cap_rate_68000_fields1k/

test/compare/capacity/runs/fbit-wasm/search_result_fields1k.txt
test/compare/capacity/runs/fbit-wasm/cap_rate_5500_fields1k/
test/compare/capacity/runs/fbit-wasm/cap_rate_5700_fields1k/
```
