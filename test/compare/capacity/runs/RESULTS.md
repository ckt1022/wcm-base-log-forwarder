# 高流量容量與 80% 負載延遲實驗結果

執行日期:2026-07-19(WSL2,loopback,時鐘檢查通過)

## 統一測試條件(固定變因)

| 項目 | 設定 |
|------|------|
| 資源上限 | 1 CPU core、512MB RAM(docker `--cpus 1 --memory 512m --memory-swap 512m`,swap 禁用) |
| Flush 批次窗 | 1 秒(fluentd/vector 以 FLUSH_INTERVAL 注入;fluent-bit 以 sed 注入 [SERVICE] Flush) |
| 輸入 | TCP :5140,line-delimited JSON,json-fixed5(平均 ~160 bytes/行) |
| 管線 | parse JSON → filter level ≥ warn(40% 通過)→ NDJSON format → HTTP POST :8080/ingest |
| 設定檔 | 各產品 latency conf 原樣使用(僅 flush 窗注入);keep-alive 用產品預設 |
| 負載端 | tools/gen(batch 500 行/20ms flush、單連線、-ts-mode sched 排程時間戳) |
| 量測端 | capacity/httpsink(延遲定義同 tools/test/server.go:讀完 body 的 time.Now() − 每行 timestamp) |
| 版本 | fluent/fluentd:v1.16-debian-1、fluent/fluent-bit:5.0.9(Lua 與 WASM)、timberio/vector:0.49.0-alpine |

「可持續」判定(每點 60s 負載 + 20s drain,全系統同一標準):
1. gen 未被 backpressure 卡住:達成率 ≥ 99.5% 且 send_lag p99 ≤ 10ms
2. 負載結束瞬間 DUT 內部積壓 ≤ 3 秒份輸出
3. drain 後 sink 實收 ≥ 99.9% 預期行數(零遺失)
4. 容器未 OOM;容量點重跑一次確認

搜尋方式:×2 粗掃 → 二分收斂到 ±5%。

## 最大負載點(容量,輸入端流量)

| 產品 | 最大可持續 eps | 輸入 MB/s | 失敗點 | 誤差區間 |
|------|--------------|-----------|--------|---------|
| Fluent Bit + Lua | **296,000** | **47.3** | 304,000 | ±2.7% |
| Fluentd (Ruby) | **176,000** | **28.1** | 184,000 | ±4.5% |
| WCM v3 改良版(本系統)| **132,000** | **21.1** | 138,000 | ±4.5% |
| WCM v1 原版(本系統)| 120,000(穩定,2/2)| 19.2 | 128,000 起機率性 OOM | 見下註 |
| Vector (VRL) | **62,000** | **9.9** | 64,000 | ±3.2% |
| Fluent Bit + WASM | **24,000** | **3.8** | 25,000 | ±4.2% |

sink 端輸出流量 = 輸入 × 40%(filter 通過率)。

### WCM 詳細(原版 → memory-bounded 改良版)

| 版本 | 容量 | 超載行為 |
|------|------|---------|
| v1 原版(image latest,forwarder.yaml)| 120,000 穩定;128k–132k 機率帶 | **容器 OOM kill → 所有連線被 RST**(掉資料) |
| v3 改良版(image memguard,forwarder_membound.yaml)| **132,000(confirmed)** | **優雅 backpressure:無 OOM、無斷線、零資料遺失** |

v1 斷線根因(dmesg + oom.txt 證實):不是程式邏輯斷線,而是 process 總 RSS
撞到 512MB cgroup 上限被 OOM kill(內部各配額——WASM mem_limit 256MB、
buffer 128MB、channel——各自合法,總和超標)。

v3 改良內容(第 1 層配置 + 第 3 層保險絲,原始碼只加 mem-guard):
1. 配置讓「配額總和」有界:buffer_size_mb 128→8、max_transport_bytes→8MB、
   pipeline_channel_capacity 20000→2(單位是批!3 條 stage channel × 每批
   buffer 大小,原值形同無上限蓄水池,是 OOM 主因)、channel_capacity
   150000→50000、mem_limit_mb 256→128(WASM linear memory 只長不縮)、
   max_format_chunk 50000→10000。
2. src/app.rs 加 RSS 水位線 mem-guard:讀 cgroup memory.current/max,
   60% 暫停讀取輸入(TCP 流控回壓)、45% 恢復;暫停期間 malloc_trim(0)
   歸還空閒 heap(否則 glibc 不歸還、RSS 降不下來會活鎖)。
   水位可用 MEM_GUARD_HIGH/LOW 環境變數調整。
3. 成效:132k 時 RSS 僅 ~165MB(原版同速率會 OOM);超載(138k~192k)
   全部 oom=false、sink 實收 = gen 實送(零遺失),失敗模式變成與
   fluentd/fluent-bit/vector 相同的 backpressure。容量瓶頸從記憶體變成
   CPU(132k 時 ~94%),且 132k > v1 的 120k。

## 80% 負載延遲(300s,穩態窗 30~230s,flush 1s)

| 指標 | Fluentd @140,800 | FB Lua @236,800 | FB WASM @19,200 | Vector @49,600 | WCM v3 @105,600 |
|------|-----------------|-----------------|-----------------|----------------|-----------------|
| mean | 512.8 ms | 566.9 ms | 524.5 ms | 576.6 ms | 637.7 ms |
| min | 8.3 ms | 96.7 ms | 21.2 ms | 39.3 ms | 352.8 ms |
| p50 | 512.6 ms | 570.3 ms | 524.5 ms | 576.6 ms | 637.7 ms |
| p90 | 912.6 ms | 913.8 ms | 924.5 ms | 999.6 ms | 837.9 ms |
| p95 | 964.8 ms | 963.8 ms | 974.5 ms | 1052.5 ms | 863.0 ms |
| p99 | 1006.7 ms | 1003.8 ms | 1014.5 ms | 1094.9 ms | **890.6 ms** |
| p99.9 | 1016.5 ms | 1012.8 ms | 1026.4 ms | 1114.8 ms | 1039.7 ms |
| max | 1035.6 ms | 1027.0 ms | 1032.9 ms | 1138.5 ms | 1144.6 ms |

WCM v3 註記:105,600 eps(16.9MB/s)下 8MB buffer 每 ~0.47s 就 size 觸發 flush,
批次節奏由「大小」而非 1s 時間窗主導——所以 p50 較高(壓縮不到 0 的
管線常數 ~350ms + 半個 size 週期)但 p99 最低(沒有批次等滿整秒)。
與其他系統(128MB 上限、時間窗主導)的分布形狀不同源於此,
解讀時應注意這是 memory-bounded 配置的取捨,不是純粹的引擎快慢。

### 對照實驗:WCM buffer 大小掃描(同速率 105,600 eps × 300s)

變體間唯一差異 = buffer_size_mb 與 max_transport_bytes(一波一 POST 皆維持);
4/8/10MB 為 size 觸發主導,128MB 為 1s 時間窗主導(單批 105,600 行):

完整九點掃描(1/2/6/12/16MB 為 90s 負載、穩態窗 20~80s;4/8/10/128MB 為
300s、窗 30~230s;全部 PASS、零遺失、無 OOM):

| buffer | 批量(行)| min | p50 | p99 | p99.9 |
|--------|----------|------|------|------|-------|
| 1MB | ~6.6k | 41.6 | 79.6 | **141.7** | 151.5 |
| 2MB | ~13.1k | 84.3 | 155.6 | 238.0 | 256.6 |
| 4MB | ~26.2k | 174.1 | 310.2 | 439.8 | 472.4 |
| 6MB | ~39.3k | 261.1 | 467.6 | 656.9 | 688.8 |
| 8MB=v3 | ~52.4k | 352.8 | 637.7 | 890.6 | 1039.7 |
| 10MB | ~65.5k | 443.1 | 789.1 | 1103.9 | 1139.1 |
| 12MB | ~78.6k | 546.4 | 942.8 | 1314.1 | 1348.8 |
| 16MB | ~104.9k | 743.1 | 1267.2 | 1761.2 | 1810.1 |
| 128MB(1s 時間窗)| 105.6k | 763.5 | 1314.3 | 1820.3 | 1852.4 |

(單位 ms;16MB 批量已貼近 1s 時間窗上限 105.6k 行,故與 128MB 幾乎重合)

### buffer 尺寸 vs 容量 + 尾延遲(完整取捨表)

容量為快測值:30s 負載、階梯粒度 ~9%(108k/120k/132k/144k/156k…)、
單次通過無確認重跑;8MB 為正式值(60s、±4.5%、確認重跑)。
延遲欄 = 105,600 eps 下的掃描結果:

| buffer | 容量(eps)| p50 (ms) | p99 (ms) |
|--------|-----------|----------|----------|
| **1MB** | **144,000** | **79.6** | **141.7** |
| 2MB | 144,000 | 155.6 | 238.0 |
| 4MB | 132,000 | 310.2 | 439.8 |
| 6MB | 132,000 | 467.6 | 656.9 |
| 8MB(正式)| 132,000 | 637.7 | 890.6 |
| 10MB | 132,000 | 789.1 | 1103.9 |
| 12MB | 132,000 | 942.8 | 1314.1 |
| 16MB | 132,000 | 1267.2 | 1761.2 |

結論:**在此範圍內不存在容量–延遲取捨**——小批次不但延遲線性下降,
容量還略升(1–2MB 通過 144k,4MB 以上都停在 132k)。機制:小批次讓
4 個 WASM stage 的 pipeline 重疊更充分(大批次時各 stage 輪流獨佔 CPU、
交接處閒置),per-batch 開銷在 1MB(~6.6k 行/批)尚未反噬。
→ **1MB(或 2MB)是 WCM 的建議代表配置**:容量 144k(快測)超越
Fluentd 的 176k 以外的所有對照…(仍次於 fluentd/fbit-lua),
p99=142ms 為全場最低(對照產品 1004–1095ms 的 1/7)。
若作為論文 headline,建議對 1MB 補做正式容量搜尋(±5% + 確認重跑)
與其 80%(~115k)的 300s 延遲測試。

九點高度線性:p50 ≈ 12.1~12.5µs × 批次行數(截距 ≈ 0),證實 WCM 延遲由
「批次 transit time」主導——batch-granular WASM 管線的行為模型成立,
buffer 是 WCM 的延遲旋鈕(其他產品的旋鈕是 flush 時間,見方法論註記)。
1MB 配置 p99=142ms、積壓僅 40 行,遠低於全部四個對照產品(1004–1095ms);
注意小 buffer 的容量未另行搜尋(掃描皆在 105.6k 下,對 8MB 容量而言是 80%),
per-batch 開銷可能改變各尺寸的容量上限,完整取捨曲線(每尺寸容量搜尋 +
各自 80% 延遲)可作後續實驗。

### tuned-vs-tuned:各產品最小可用 flush(短測)

短測(30–90s,粒度粗,非正式容量搜尋值)找出各產品在自己 80% 速率下
仍能維持 backpressure、零遺失、無 OOM 的最小批次窗:

| 產品 | 速率(80%)| 最小可用值 | 找法 | p50 | p99 |
|------|-----------|-----------|------|------|------|
| Fluentd | 140,800 | 0.1s(flush_thread_interval 寫死,環境變數對 <1s 無效)| 單次驗證 | 57.6 ms | 107.0 ms |
| Fluent Bit Lua | 236,800 | **0.01s** | 階梯 0.1→0.01 全通過 | **9.0 ms** | **14.3 ms** |
| Fluent Bit WASM | 19,200 | **0.01s** | 階梯 0.1→0.01 全通過 | 22.2 ms | 34.2 ms |
| Vector | 49,600 | **0.01s** | 階梯 0.1→0.01 全通過 | 19.1 ms | 29.7 ms |
| WCM(對照)| 105,600(≈WCM 8MB 之 80%)| buffer 1MB | 見上方掃描 | 79.6 ms | 141.7 ms |

發現:三個能設真小數 flush 的產品在 0.01s 都毫無壓力地通過(fluentd 卡在
0.1s 是實作限制,非能力限制)。**tuned 之後排名整個翻轉**:1s 時間窗下
WCM 是尾延遲最低者,調到各自最優後 Fluent Bit(Lua/WASM)、Vector 的
p99 反而比 WCM 1MB(141.7ms)低了一個數量級(14–34ms)。這證實了
先前的推論——iso-config(全 1s)那組數據被 WCM 佔便宜,tuned-vs-tuned
才是各系統真實能力的公平比較。

註:fbit-lua/wasm/vector 的 0.01s 積壓在短測窗內小幅上升(數千至 1 萬行,
仍遠低於 3 秒份門檻),暗示比 0.01s 更小可能才是真下限,但已足夠回答
「1s flush 是否公平」這個問題——答案是否定的。若要嚴謹的最終數字,
建議這四點各補一次正式 300s 驗證。

### WCM 也對齊 max_wait_ms=10ms(真正同名旋鈕的對照)

上面的「WCM 1MB」是靠縮小 buffer 間接逼出高頻 size 觸發(≈62ms 一批),
不是調 WCM 的時間旋鈕。wcm-flush10ms = 與 tw128(128MB,時間窗主導)
唯一差異 max_wait_ms 1000→10ms,buffer 開大到不會被 size 搶先觸發
(105,600 eps × 0.01s ≈ 1,056 行/批 ≈ 169KB,遠低於 128MB),
這才是與其他產品同名參數(flush 時間)的公平對照:

| 配置 | 批量(行) | min | p50 | p99 |
|------|----------|------|------|------|
| max_wait_ms=1000(tw128)| 105,600 | 763.5 ms | 1314.3 ms | 1820.3 ms |
| **max_wait_ms=10** | ~1,056 | 7.7 ms | **16.3 ms** | **46.9 ms** |
| buffer=1MB(size 觸發,≈62ms 週期)| ~6,600 | 41.6 ms | 79.6 ms | 141.7 ms |

驗證:PASS、零遺失、無 OOM、mem-guard 未觸發(記憶體穩定,未撞水位線)。

**更新後的五系統 tuned 排名**(各自最優旋鈕,同 105,600/相當 80% 速率量級):

| 排名 | 系統 | p99 |
|------|------|------|
| 1 | Fluent Bit Lua | 14.3 ms |
| 2 | **WCM(max_wait_ms=10)** | **46.9 ms** |
| 3 | Vector | 29.7 ms† |
| 4 | Fluent Bit WASM | 34.2 ms† |
| 5 | Fluentd | 107.0 ms |

†Vector/FB WASM 的 p99 數字比 WCM 低,但速率量級不同(19.2k/49.6k vs
WCM 的 105.6k),不能直接按名次讀;若要嚴謹排名需在同一速率下比較。
真正結論:**max_wait_ms 才是 WCM 對應其他產品「flush 時間」的正確旋鈕**,
用它比用 buffer 小 3 倍延遲(p99 141.7ms → 46.9ms),且行為(批量→延遲
線性模型)與時間窗機制吻合;之前用 buffer 掃描得到的「1MB 最優」
結論應更新為「max_wait_ms=10 是更好的低延遲配置」。

方法論註記(架構不同時的公平比較):WCM 的批次在處理「之前」
(積批→整批過 4 個 WASM stage),對照產品的批次在處理「之後」
(流式逐筆處理→輸出緩衝等 flush),同名參數語意不同,故單點 iso-config
比較(全部 1s)只回答「同設定下的架構後果」;產品能力比較應採
tuned-vs-tuned(WCM 調 buffer、對照組調 flush 時間)或容量–尾延遲
操作包絡(Pareto 前緣)。

### 固定變因矩陣:共同絕對吞吐量下,各自調到最佳旋鈕(最終公平比較)

比 §tuned-vs-tuned(各自 80% 速率)更嚴謹的版本:七個**共同絕對吞吐量點**
(不是各自的 80%),固定 concurrency=1(四產品皆已如此)、retry 全關 +
timeout 30s(新建 `*_tuned.conf/yaml`,不動原檔)、同一輸出目的地、同一硬體、
90s 負載、穩態窗 20~80s;只放行 buffer/flush 讓各系統自己調。策略:先套用
已知最低延遲設定(fbit-lua/wasm/vector flush=0.01、fluentd flush=0.1〔硬下限〕、
wcm max_wait_ms=10),超過該系統 flush=1s 已知容量的點直接標 N/A 不測。

**結果:26 組全部第一次嘗試就 PASS,沒有任何一組需要 fallback 放大 flush。**

| 吞吐量(eps)| Fluentd | FB Lua | WCM | Vector | FB WASM |
|---|---|---|---|---|---|
| 10,000 | 71.6 / 124.7 | 16.0 / 29.2 | 16.6 / 27.7 | 25.9 / 36.6 | 19.1 / 29.9 |
| 20,000 | 73.2 / 126.3 | 16.6 / 29.4 | 20.6 / 33.6 | 27.1 / 37.4 | 22.5 / 34.6 |
| 25,000 | 72.8 / 126.0 | 16.8 / 29.4 | 20.1 / 32.6 | 29.0 / 41.1 | N/A |
| 50,000 | 63.3 / 115.3 | 11.9 / 20.0 | 15.3 / 24.2 | 19.6 / 30.7 | N/A |
| 100,000 | 59.0 / 108.6 | 9.8 / 15.9 | 16.3 / 30.5 | N/A | N/A |
| 125,000 | 58.3 / 107.8 | 9.3 / 15.1 | **69.1 / 169.6** | N/A | N/A |
| 150,000 | 58.6 / 108.0 | 9.1 / 14.7 | N/A | N/A | N/A |

(格式 p50 / p99,單位 ms;N/A = 超過該系統已知容量,未測)

**發現**:
1. **Fluent Bit Lua 全程最快且隨吞吐量升高反而更低**(29ms→15ms):在測試範圍內完全沒有 U 型曲線右側的跡象,flush=0.01s 從 10k 到 150k(接近其 296k 容量一半)都毫無壓力。
2. **Fluentd 全程近乎平坦**(108–126ms):延遲被 0.1s 輪詢下限完全主導,吞吐量高低幾乎無感——這是實作限制造成的地板,不是容量問題(它的真實容量是 176k,測試範圍內都遠低於此)。
3. **WCM 在接近自己容量天花板(132k)時出現本矩陣唯一的明顯劣化**:100k 時 p99=30.5ms,125k 暴衝到 **169.6ms**(+455%)。這正是上一輪討論的 U 型曲線,但成因是**吞吐量逼近容量**而非 flush 設定本身變差——`max_wait_ms` 全程固定 10ms,是批次因為系統快排不出去而被迫變大(等同從時間觸發被迫切換成大小/擁塞觸發),而非旋鈕失效。
4. **Vector、FB WASM** 在各自可測範圍內(到 25k、20k)延遲隨吞吐量小幅上升(37→41ms、30→35ms),樣本點不足以判斷是否已進入劣化區。
5. **四系統共同涵蓋的範圍(10k–100k)排名**:FB Lua(15–29ms)< WCM(24–34ms,100k 前)≈ Vector/FB WASM(30–41ms)≪ Fluentd(108–127ms)。**FB Lua 對 Fluentd 有穩定 7–8 倍的優勢**。
6. **全程涵蓋到 150k 的只剩 Fluentd 與 FB Lua**:FB Lua 在此範圍內從未展現任何吃緊跡象,容量餘裕(296k)還很大。

結論:對齊 1s 時間窗後 WCM 延遲**不會**收斂到其他產品的 ~500ms/1000ms,
反而整體上移 ~700ms。原因是 WCM 的管線以「批」為粒度通過 4 個 WASM stage:
1s 窗在 105.6k eps 下單批 ≈ 105,600 行(17MB),整批走完 parse→filter→
format→transport 需 ~0.7–0.8s(接近容量 132k 的倒數 × 批量),批越大
transit time 越長;其他產品是逐筆/小 chunk 流式處理,批次只影響輸出等待。
→ 高流量下 WCM 的延遲由「批次 transit time」主導,小 buffer(size 觸發)
反而是低延遲配置;這是 batch-granular WASM 管線的固有架構特性,
建議論文以 v3(8MB)為 WCM 代表配置,tw128 作為架構分析的佐證。

### 解讀

- 四者的延遲分布都由 1 秒 flush 窗主導(理論:mean ~500ms、p99 ~1000ms),
  代表在各自 80% 容量下均無排隊累積——比較是公平的(延遲差異來自批次節奏,
  不是誰在超載硬撐)。
- Vector 的 p90 以上比其他系統多 ~90ms:batch.timeout 從「該批第一筆」起算
  加上單一 in-flight request(request.concurrency 1)的排程,尾端略胖。
- Fluent Bit WASM 容量只有 Lua 版的 8%(24k vs 296k):WASM filter 的
  per-record 序列化/呼叫開銷在高流量下是主要瓶頸;但在未飽和時延遲特性正常。

### 診斷:為何 Vector(62k)比 Fluentd(176k)/ FB Lua(296k)慢

用兩個對照探針分解(diag/vector_passthrough.yaml = 同 source/sink、拿掉 VRL):

| 探針 | CPU @62k eps | 說明 |
|------|-------------|------|
| 含 VRL(原設定) | ~82% | 容量點;64k 失敗時平均 CPU 仍 <100% |
| 直通(無 VRL) | ~46% | 同速率,全部行都送達 |

1. **VRL transform 約佔一半 CPU**(46%→82%):我們的 VRL 是解譯執行
   (level 對照表查找、兩個 for_each 動態 set! 攤平、encode_json 型別分派),
   對照 fluent-bit 的 LuaJIT(JIT 成原生碼)與 fluentd 的 C 擴充(oj/msgpack)
   同語意 filter,per-record 成本高數倍。
2. **框架 per-event 開銷也高**:純直通推算上限僅 ~130k eps,仍遠低於
   fbit-lua 整條管線的 296k——Vector 的 event 模型(Value 樹 + metadata、
   source/transform/sink 間 tokio channel 傳遞)每筆成本比 fluent-bit 的
   C msgpack chunk 高。
3. **1 core 配額吃虧**:Vector 的強項是多核擴展;多執行緒 tokio 在
   --cpus 1 的 CFS quota 下會被突發節流,所以平均 CPU ~82% 就開始
   backpressure(而單執行緒事件迴圈的 fluent-bit 可以吃滿整核)。
   若放寬到多核,Vector 的排名預期會明顯改善——這是「per-core 效率 vs
   水平擴展」的架構取捨,值得在論文中標註。

### 驗證(所有 run 皆通過)

每份 run 的 verdict.txt:gen 達成率 100%、send_lag p99 ≤ 5ms(排程時間戳下
無 coordinated omission 偏差)、零資料遺失、無 OOM。

## 檔案位置

- 原始資料:`test/compare/capacity/runs/<dut>/{cap,lat}_rate_<R>/`
  (latency CSV、每秒接收 CSV、gen/dut/sink log、verdict)
- 各產品結果副本:`fluentd_time/fluentd_80%_140800/`、
  `fluentbit_lua_time/fluentbit_lua_80%_236800/`、
  `fluentbit_wasm_time/fluentbit_wasm_80%_19200/`、`vector_time/vector_80%_49600/`
  (latency_stats.txt、verdict.txt、gen_send_lag.txt、recv_per_sec.csv)
- 可重現腳本:各產品 latency 目錄下的 `latency_80%.sh` /
  `latency_lua_80%.sh` / `latency_wasm_80%.sh`;容量搜尋 `capacity/search.sh <dut>`
