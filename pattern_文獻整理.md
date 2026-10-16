# Design Pattern 論文：Known Uses 與相關文獻整理

> 整理日期：2026-10-14
> 用途：替每個 pattern 找出「已知應用（Known Uses）」與理論依據。pattern 論文通常要求每個 pattern 至少有 2～3 個彼此獨立的已知應用。
> 本文件的每一筆都經過網路搜尋確認出處，但摘要是根據搜尋結果整理的，**引用前請自行閱讀原文核對**，特別是標有 ⚠️ 的項目。
> WebAssembly 隔離、Component Model 與效能的基礎文獻，已整理在 [wasm_wcm_related_research.md](wasm_wcm_related_research.md)，這裡不重複。

## 1. Pattern 方法論（寫作格式的依據）

| 文獻 | 用途 |
|---|---|
| Christopher Alexander et al., *A Pattern Language*, Oxford University Press, 1977 | pattern language 概念的起源 |
| Buschmann et al., *Pattern-Oriented Software Architecture, Vol. 1*（POSA），Wiley, 1996 | 軟體架構 pattern 的標準格式：Context、Problem、Forces、Solution、Consequences、Known Uses |
| Robert S. Hanmer, *Patterns for Fault Tolerant Software*, Wiley, 2007（ISBN 978-0-470-31979-6）—— [O'Reilly](https://oreilly.com/library/view/patterns-for-fault/9780470319796) | 63 個容錯 pattern，是本研究「逾時、重試、隔離」相關 pattern 最直接的前例。Hanmer 自己在 [PLoP 2014 論文](https://Hillside.net/plop/2014/papers/proceedings/papers/07-hanmer-1.pdf) 中也提到，雲端和虛擬化改變了書中部分假設，這正好可以作為「WASM 時代需要新 pattern」的動機 |
| Gregor Hohpe & Bobby Woolf, *Enterprise Integration Patterns*, Addison-Wesley, 2003 —— [Wikipedia](https://en.wikipedia.org/wiki/Enterprise_Integration_Patterns) | 65 個訊息整合 pattern，其中的 Dead Letter Channel、Invalid Message Channel 跟本研究「壞資料要放到哪裡」的處理直接相關 |
| Michael T. Nygard, *Release It!*（2nd ed.），Pragmatic Bookshelf, 2018 | Timeouts、Bulkheads、Circuit Breaker 等穩定性 pattern 的經典來源 |

## 2. 各 Pattern 的 Known Uses

### 2.1 Poison Record Bisection（對半拆批找出壞資料）

| Known Use | 說明 | 出處 |
|---|---|---|
| **AWS Lambda `BisectBatchOnFunctionError`** | Kinesis、DynamoDB Streams、MSK、Kafka 的事件來源設定：「函式回傳錯誤時，把批次切成兩半重試」，預設關閉。**跟本研究的作法幾乎完全相同**，是最有力的 Known Use | [AWS API 文件](https://docs.aws.amazon.com/lambda/latest/dg/API_EventSourceMappingConfiguration.html) |
| Enterprise Integration Patterns 的 Dead Letter Channel | 處理不了的訊息另外存放，不阻塞主流程；對應本研究把壞資料寫進 `error.txt` | Hohpe & Woolf（見第 1 節） |

### 2.2 Batch the Crossing（批次跨越邊界）與 Decouple Processing / Flush（把處理批次和送出批次分開）

| Known Use／文獻 | 說明 | 出處 |
|---|---|---|
| **Apache Flink network buffer timeout** | buffer 滿了或逾時（預設 100 ms）就送出。縮短逾時能降低延遲，但每個 buffer 的固定成本遠高於每筆的成本，會降低吞吐量，跟本研究「每次呼叫的固定成本 a」是同樣的取捨 | [Flink 低延遲調校（官方部落格）](https://flink.apache.org/2022/05/18/getting-into-low-latency-gears-with-apache-flink-part-one/)、[Flink 1.14 network buffer 文件](https://nightlies.apache.org/flink/flink-docs-release-1.14/docs/deployment/memory/network_mem_tuning) |
| **Drizzle**（Venkataraman et al., SOSP 2017） | 把「處理的間隔」和「協調的間隔」分開；用 group scheduling 一次排多個批次，分攤每批的排程成本。**概念上就是 Decouple Processing / Flush 的學術前例** | [論文 PDF（Berkeley）](https://www.cs.berkeley.edu/~alig/papers/drizzle.pdf) |
| Spark Streaming：Discretized Streams（Zaharia et al., SOSP 2013） | micro-batch 串流處理的代表。Drizzle 論文把它當作「micro-batch 延遲較高」的對照 | ⚠️ 請自行查證引用格式 |
| **Redpanda Data Transforms** | WASM 轉換函式**逐筆處理**（`on_record_written`），每個 shard 一個實例，並限制記憶體和 CPU 時間。可以作為「逐筆呼叫」的已知設計，與本研究的逐筆 vs 批次實驗對照 | [Redpanda 文件](https://docs.redpanda.com/25.1/develop/data-transforms/how-transforms-work) |
| Fluent Bit WASM filter | 也是逐筆呼叫 WASM，本研究的五產品比較中已經量到它的效能 | [Fluent Bit 文件](https://docs.fluentbit.io/manual/pipeline/filters/wasm) |

### 2.3 Deadline-Bounded Invocation（有時限的呼叫）

| Known Use | 說明 | 出處 |
|---|---|---|
| **Wasmtime epoch interruption 與 fuel** | 官方文件：fuel 是決定性的，但執行成本比 epoch 高；epoch 較便宜。兩者都可以觸發 trap 或非同步讓出。**官方沒有公布具體的成本數字**，所以本研究若補量 epoch 的成本，本身就是貢獻 | [Wasmtime：Interrupting Wasm Execution](https://docs.wasmtime.dev/examples-interrupting-wasm.html) |
| **Shopify Functions** | 每次執行有指令數上限（200 個購物車品項以內約 1,100 萬條指令），超過就中止並回報 `InstructionCountLimitExceededError` | [Shopify 文件](https://shopify.dev/docs/apps/build/functions/optimize-instruction-counts) |
| **Extism** | manifest 可以設定 `timeout_ms` | [Extism manifest 文件](https://extism.org/docs/concepts/manifest) |
| Nygard 的 Timeouts pattern | 穩定性 pattern 的經典定義 | *Release It!*（見第 1 節） |

### 2.4 Resource Limiter（記憶體上限）

| Known Use | 說明 | 出處 |
|---|---|---|
| **Extism** `memory.max_pages` | 限制插件可以使用的記憶體頁數 | [Extism manifest 文件](https://extism.org/docs/concepts/manifest) |
| **Fluent Bit WASM filter** `wasm_heap_size` | 設定 WASM 執行的 heap 大小 | [Fluent Bit 文件](https://docs.fluentbit.io/manual/pipeline/filters/wasm) |
| **Redpanda** `data_transforms_per_function_memory_limit` | 每個轉換函式的記憶體上限 | [Redpanda 文件](https://docs.redpanda.com/25.1/develop/data-transforms/how-transforms-work) |
| Yu et al., *Exploring and Exploiting the Resource Isolation Attack Surface of WebAssembly Containers*, USENIX Security 2025 | 學術上證明「記憶體安全 ≠ 資源隔離」 | 已在 wasm_wcm_related_research.md |

### 2.5 Capability-Scoped World（依階段限制能力）

| Known Use | 說明 | 出處 |
|---|---|---|
| **Fermyon Spin** `allowed_outbound_hosts` | 每個 component 只能連到列在 allowlist 上的主機，預設拒絕。**跟本研究 transport 只能連 endpoint 清單的設計相同** | ⚠️ 搜尋到的多半是社群文章，[2022 年官方部落格](https://fermyon.com/blog/spin-rest-apis) 用的是舊名稱 `allowed_http_hosts`，請以官方文件核對目前的設定名稱 |
| **Extism** `allowed_hosts`、`allowed_paths` | 空清單代表全部拒絕；路徑只在授權 WASI 能力時才生效 | [Extism manifest 文件](https://extism.org/docs/concepts/manifest) |
| **Zellij 插件權限** | 插件要用 `request_permission` 向使用者請求權限（讀取狀態、開檔、執行指令等），0.38.0 版加入 | [Zellij 文件](https://zellij.dev/documentation/plugin-api-permissions) |
| **Redpanda Data Transforms** | 轉換函式不能存取外部磁碟和網路 | [Redpanda 文件](https://docs.redpanda.com/25.1/develop/data-transforms/how-transforms-work) |
| **Fluent Bit WASM filter** `accessible_paths` | 指定 WASM 可以存取的路徑 | [Fluent Bit 文件](https://docs.fluentbit.io/manual/pipeline/filters/wasm) |

### 2.6 Disposable Instance／Instance Pool（用後即丟的實例／實例池）

| Known Use／文獻 | 說明 | 出處 |
|---|---|---|
| **Wasmtime pooling allocator 與 `InstancePre`** | 預先配置一批記憶體和 table，建立實例時直接從池子取用；`InstancePre` 預先完成匯入解析和型別檢查；記憶體採用 copy-on-write 初始化。**這是本研究實例池可以改進的方向** | [Wasmtime：Tuning for Fast Instantiation](https://docs.wasmtime.dev/examples-fast-instantiation.html) |
| 注意事項：CVE-2022-39393 | pooling allocator 曾經有實例之間資料洩漏的漏洞（2.0.2 修正），可以寫進 Consequences 的風險面 | [漏洞說明](https://vulert.com/vuln-db/CVE-2022-39393) |
| **Sledge**（Gadepalli et al., Middleware 2020） | 邊緣運算的 WASM serverless runtime，每次呼叫都用新的沙箱；後續研究報告建立一個新沙箱不到 30 µs | [論文 PDF](https://faculty.cs.gwu.edu/gparmer/publications/middleware20sledge.pdf) |
| **Faasm**（Shillaker & Pietzuch, USENIX ATC 2020） | Faaslet 用 WASM 隔離記憶體，從預先初始化的快照還原來縮短啟動時間；CPU 和網路隔離則用 cgroups | [USENIX 頁面](https://www.usenix.org/conference/atc20/presentation/shillaker) |
| **Redpanda** | 每個 shard 一個實例，同時只跑一個 | [Redpanda 文件](https://docs.redpanda.com/25.1/develop/data-transforms/how-transforms-work) |

### 2.7 Projection at the Boundary（只傳需要的欄位）

| Known Use | 說明 | 出處 |
|---|---|---|
| **Shopify Functions 的 input query** | 函式用 GraphQL input query 宣告需要哪些欄位。官方文件指出，input query 越大，指令數越多。**這正是「只傳需要的欄位」的已知應用** | [Shopify 文件](https://shopify.dev/docs/apps/build/functions/optimize-instruction-counts) |
| Envoy proxy-wasm 的 `proxy_get_property` | 插件需要時才向 host 要特定屬性，而不是一次收到全部資料 | [Envoy 文件](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/advanced/wasm) |

### 2.8 Verdict, Not Mutation（只回傳決策）

| Known Use／文獻 | 說明 | 出處 |
|---|---|---|
| RLBox（Narayan et al., USENIX Security 2020） | host 必須驗證沙箱回傳的資料（tainted data），跟「插件只回決策、host 驗證 id」的精神相同 | 已在 wasm_wcm_related_research.md |
| ⚠️ 仍需補找 | 目前沒找到「插件只回傳決策、由 host 套用」的產品實例。可以往 policy engine 的方向找，例如 OPA 回傳 allow/deny 決策 | — |

### 2.9 Canonical Contract（單一正規介面）

| Known Use | 說明 | 出處 |
|---|---|---|
| **Envoy Proxy-Wasm ABI** | 多語言 SDK（Rust、C++、AssemblyScript）共用同一份 ABI 規格（建議版本 0.2.1） | [Envoy 文件](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/advanced/wasm) |
| WebAssembly Component Model／WIT | 本研究採用的機制，已在 wasm_wcm_related_research.md | — |
| Redpanda | 任何能編譯成 WASM 的語言都可以寫轉換函式 | [Redpanda 文件](https://docs.redpanda.com/25.1/develop/data-transforms/how-transforms-work) |

## 3. 實驗方法論

| 文獻 | 用途 | 出處 |
|---|---|---|
| **Karimov et al., *Benchmarking Distributed Stream Data Processing Systems*, ICDE 2018** | 定義串流系統的「可持續吞吐量（sustainable throughput）」，並強調要把產生負載的程式和受測系統分開。**本研究的容量判定方式可以直接引用它作為依據**，也能回應「為何這樣定義容量」的質疑 | [arXiv 1802.08496](https://www.arxiv.org/pdf/1802.08496)、[IEEE Xplore](https://ieeexplore.ieee.org/document/8509390) |

## 4. 還需要補的部分

1. **Verdict, Not Mutation** 只有學術依據（RLBox），缺少產品實例。
2. **Versioned Plugin Slot（熱更換）** 還沒搜尋。可以找 Envoy 的 Wasm 熱更新、Zellij 插件重載等。
3. ⚠️ 標記的項目（Spin 的設定名稱、Spark Streaming 的引用格式）請以原始文件核對。
4. 每個 pattern 目前大多有 2～4 個 Known Uses。正式寫入論文前，建議每個都讀原文，確認描述準確。
