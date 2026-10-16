# WebAssembly 隔離、Component Model 與效能相關研究整理

> 整理日期：2026-08-25  
> 用途：作為 `wcm-base-log-forwarder` 論文第二章「背景知識與相關研究」及研究問題設計的文獻起點。

## 1. 整理範圍與閱讀方式

本文獻整理聚焦於三個方向：

1. WebAssembly（Wasm）的記憶體隔離、程序內沙箱、權限與資源隔離。
2. WebAssembly Component Model（WCM）、WIT、Canonical ABI 與跨語言 binding。
3. Wasm／WCM 相對於原生程式、容器或其他執行環境的效能成本。

每篇文獻皆整理「研究焦點、方法、主要發現、與本專案的關聯」。其中「主要發現」是對原文的濃縮摘要，不應脫離該論文的工作負載、硬體、執行環境與威脅模型，直接推廣成所有 Wasm 系統都成立的結論。

目前直接以 **WebAssembly Component Model 效能**為主題的同儕審查論文仍少；因此 WCM 的介面語意與跨界機制須以官方設計文件為主要依據，再由本專案的 microbenchmark 實證 Canonical ABI 對日誌資料的成本。本文將「同儕審查論文」、「預印本／學位論文」及「官方規格」分開列出。

## 2. 建議優先閱讀順序

若時間有限，建議依下列順序閱讀：

1. *Bringing the Web up to Speed with WebAssembly*：理解 Wasm 的設計目標與安全模型。
2. *Not So Fast*：理解 Wasm 與原生程式的效能差異不能只用「接近原生」概括。
3. *Retrofitting Fine Grain Isolation in the Firefox Renderer*：理解 Wasm 如何實際用於第三方程式庫隔離。
4. *Provably-Safe Multilingual Software Sandboxing using WebAssembly*：理解沙箱正確性與高效執行的研究方法。
5. *Exploring and Exploiting the Resource Isolation Attack Surface of WebAssembly Containers*：理解記憶體安全不等於資源隔離。
6. *Finding Bugs in WebAssembly Interface Type Binding Generators*：理解 WCM/WIT binding 也可能成為正確性風險。
7. Component Model Explainer 與 Canonical ABI：建立本研究跨元件複製成本的理論基礎。

---

## 3. WebAssembly 基礎與綜述

### 3.1 Bringing the Web up to Speed with WebAssembly

- **作者／出處**：Andreas Haas et al., ACM PLDI 2017。
- **類型**：同儕審查會議論文；WebAssembly 奠基性論文。
- **論文連結**：[PLDI 出版頁](https://pldi17.sigplan.org/details/pldi-2017-papers/48/Bringing-the-Web-up-to-Speed-with-WebAssembly)；[公開 PDF](https://github.com/WebAssembly/spec/blob/main/papers/pldi2017.pdf)
- **大綱**：
  - 說明 JavaScript 作為低階編譯目標時的限制，以及設計新型可攜式位元碼的動機。
  - 定義 Wasm 的型別系統、結構化控制流程、線性記憶體、模組、驗證與執行語意。
  - 討論快速解碼、驗證、編譯、跨平台及與 Web 平台互通等設計目標。
  - 透過早期實作與 benchmark 說明 Wasm 具備接近原生執行的潛力。
- **與本研究的關聯**：
  - 可作為第二章 Wasm 設計、線性記憶體、型別安全與控制流程隔離的主要引用來源。
  - 應注意此文研究 Core Wasm，並未涵蓋後來的 Component Model、WIT 或 Canonical ABI 高階型別轉換成本。

### 3.2 Research on WebAssembly Runtimes: A Survey

- **作者／出處**：Yixuan Zhang, Mugeng Liu, Haoyu Wang, Yun Ma, Gang Huang, Xuanzhe Liu；2024 預印本，後續出版資訊應在引用前再次核對。
- **類型**：綜述研究。
- **論文連結**：[arXiv](https://arxiv.org/abs/2404.12621)
- **大綱**：
  - 整理 98 篇與 Wasm runtime 有關的研究。
  - 將研究分為 runtime 內部議題（設計、測試、分析）及外部應用領域。
  - 比較不同 runtime 架構、編譯策略、測試方法及應用情境。
  - 歸納既有研究不足與未來研究方向。
- **與本研究的關聯**：
  - 適合用來建立相關研究分類，並追查 Wasm runtime、serverless、edge、IoT 及安全研究的原始文獻。
  - 它是文獻地圖，不應取代本文對關鍵原始論文的直接引用。

### 3.3 WebAssembly and Security: A Review

- **作者／出處**：Gaetano Perrone, Simon Pietro Romano, 2024。
- **類型**：安全研究綜述／預印本。
- **論文連結**：[arXiv](https://arxiv.org/abs/2407.12297)
- **大綱**：
  - 整理 121 篇 WebAssembly 安全相關研究。
  - 涵蓋 Wasm 作為防禦機制、Wasm 程式自身漏洞、惡意 Wasm、密碼挖礦、分析與偵測技術等主題。
  - 區分來源語言、編譯工具鏈、Wasm binary、runtime 與宿主介面等不同攻擊面。
  - 討論現有防禦方法與仍待處理的研究問題。
- **與本研究的關聯**：
  - 可支持「Wasm 沙箱不是完整安全保證」的論述。
  - 有助於在論文中清楚區分記憶體隔離、能力限制、runtime 漏洞、資源耗盡及應用層資料正確性。

---

## 4. WebAssembly 隔離與安全研究

### 4.1 Retrofitting Fine Grain Isolation in the Firefox Renderer

- **作者／出處**：Shravan Narayan et al., USENIX Security 2020。
- **類型**：同儕審查會議論文。
- **論文連結**：[USENIX 出版頁與 PDF](https://www.usenix.org/conference/usenixsecurity20/presentation/narayan)
- **大綱**：
  - 提出 RLBox，協助既有 C/C++ 應用程式隔離不受信任的第三方程式庫。
  - 透過型別包裝與資料驗證，降低宿主錯誤使用 sandbox 資料所造成的風險。
  - 支援軟體錯誤隔離與程序隔離等不同後端，其中包含 Wasm sandbox。
  - 將方法套用至 Firefox 及影像、影音、字型等真實程式庫，評估移植成本與效能影響。
- **與本研究的關聯**：
  - 證明 Wasm 可作為真實產品中的程序內插件／函式庫隔離邊界。
  - RLBox 強調宿主與不可信程式庫之間的資料驗證；可對應本專案以 host-assigned ID 驗證 filter 回傳結果的設計。
  - 也提醒研究者：沙箱內存安全之外，host API 與跨邊界資料仍必須驗證。

### 4.2 Everything Old is New Again: Binary Security of WebAssembly

- **作者／出處**：Daniel Lehmann, Johannes Kinder, Michael Pradel, USENIX Security 2020。
- **類型**：同儕審查會議論文。
- **論文連結**：[USENIX PDF](https://www.usenix.org/system/files/sec20-lehmann.pdf)
- **大綱**：
  - 區分 host security 與 Wasm binary security。
  - 研究 C/C++ 等記憶體不安全來源語言的漏洞編譯成 Wasm 後，是否仍可被利用。
  - 建立攻擊 primitives 與端到端概念驗證，分析 Wasm 線性記憶體內的資料破壞與控制影響。
  - 以真實 Wasm binary 與 benchmark 評估攻擊可行性，並提出緩解方向。
- **與本研究的關聯**：
  - 可用來避免「Wasm 能消除插件內所有漏洞」的過度宣稱。
  - Wasm 可保護 host 不被 guest 任意讀寫，但 guest 自身線性記憶體中的資料仍可能受來源語言漏洞破壞。
  - 因此論文應將本系統目標寫成「限制故障與權限影響範圍」，而不是宣稱插件程式本身必然安全。

### 4.3 Provably-Safe Multilingual Software Sandboxing using WebAssembly

- **作者／出處**：Jay Bosamiya, Wen Shih Lim, Bryan Parno, USENIX Security 2022。
- **類型**：同儕審查會議論文。
- **論文連結**：[USENIX 出版頁與 PDF](https://www.usenix.org/conference/usenixsecurity22/presentation/bosamiya)
- **大綱**：
  - 研究如何在同一程序內安全執行由多種語言編譯而來的不可信程式碼。
  - 提出 vWasm：以形式驗證的編譯流程建立具機器檢查證明的 sandbox。
  - 提出 rWasm：將 Wasm 語意嵌入安全 Rust，以兼顧執行效能及隔離。
  - 評估安全性、可信運算基礎、可攜性與執行效能之間的取捨。
- **與本研究的關聯**：
  - 支持「多語言插件可透過共同 Wasm 邊界隔離」的研究動機。
  - 可用來說明沙箱安全不只取決於 Wasm 規格，也取決於 runtime／compiler 實作的可信程度。
  - 本專案使用 Wasmtime 而非 vWasm/rWasm，因此不能直接繼承該論文的形式驗證結論。

### 4.4 WaVe: A Verifiably Secure WebAssembly Sandboxing Runtime

- **作者／出處**：Evan Johnson, Evan Laufer, Zijie Zhao, Shravan Narayan, Stefan Savage, Deian Stefan, Fraser Brown, IEEE Symposium on Security and Privacy 2023。
- **類型**：同儕審查會議論文。
- **論文連結**：[作者公開 PDF](https://cseweb.ucsd.edu/~dstefan/pubs/johnson%3A2023%3Awave.pdf)；[DOI](https://doi.org/10.1109/SP46215.2023.10179357)
- **大綱**：
  - 指出高效 Wasm runtime 的實作本身可能包含破壞沙箱的漏洞。
  - 提出經驗證的 Wasm sandbox runtime，並納入 WASI 系統介面。
  - 驗證 guest 線性記憶體與宿主資源存取的隔離性質。
  - 評估經驗證 runtime 與既有工業 runtime 的效能差距。
- **與本研究的關聯**：
  - 可支持「WASI 能力介面也是安全邊界的一部分」。
  - 本專案應把 Wasmtime/WASI runtime 視為可信運算基礎，而不是假定底層 runtime 絕對無漏洞。

### 4.5 Exploring and Exploiting the Resource Isolation Attack Surface of WebAssembly Containers

- **作者／出處**：Zhaofeng Yu, Dongyang Zhan, Lin Ye, Haining Yu, Hongli Zhang, Zhihong Tian, USENIX Security 2025。
- **類型**：同儕審查會議論文。
- **論文連結**：[USENIX 出版頁與 PDF](https://www.usenix.org/conference/usenixsecurity25/presentation/yu-zhaofeng)
- **大綱**：
  - 系統化分析多個 Wasm runtime 的資源隔離攻擊面。
  - 研究惡意 guest 如何透過 WASI/WASIX 或 runtime 行為消耗 CPU、記憶體及宿主資源。
  - 建立靜態分析與攻擊策略，觀察單一 Wasm instance 對同宿主其他工作負載的影響。
  - 討論 runtime 及部署端可採取的緩解方式。
- **與本研究的關聯**：
  - 直接支持本專案為插件加入 memory limiter、stage timeout、queue/backpressure 與容器 cgroup 的必要性。
  - 可用來明確說明：Wasm 的記憶體安全與 capability control 不會自動提供 CPU、記憶體或整體服務品質隔離。

### 4.6 Faasm: Lightweight Isolation for Efficient Stateful Serverless Computing

- **作者／出處**：Simon Shillaker, Peter Pietzuch, USENIX ATC 2020。
- **類型**：同儕審查會議論文；同時涉及隔離與效能。
- **論文連結**：[USENIX 出版頁與 PDF](https://www.usenix.org/conference/atc20/presentation/shillaker)
- **大綱**：
  - 指出 container-based serverless 在啟動、記憶體與函式間資料傳遞方面的成本。
  - 提出 Faaslet，以 Wasm software-fault isolation 在同一程序中隔離函式。
  - 允許同機函式共享特定記憶體區域，減少重複序列化及資料複製。
  - 結合 Linux cgroups 控制 CPU、網路等非線性記憶體資源，並以 snapshot 降低初始化成本。
- **與本研究的關聯**：
  - 顯示實際系統通常需要「Wasm memory isolation + OS resource control」，與本專案的 Wasmtime limiter 加 cgroup 實驗設計一致。
  - 其避免資料移動的設計可作為本研究分析 Canonical ABI 複製成本時的對照思路。

---

## 5. WebAssembly Component Model 相關研究

### 5.1 Finding Bugs in WebAssembly Interface Type Binding Generators

- **作者／出處**：Ethan Stanley, Eric Eide, IEEE/ACM ASE 2025, pp. 204–216。
- **類型**：同儕審查會議論文；目前與 WCM/WIT 最直接相關的研究之一。
- **論文連結**：[ASE 論文頁](https://conf.researchr.org/details/ase-2025/ase-2025-papers/124/Finding-Bugs-in-WebAssembly-Interface-Type-Binding-Generators)；[DOI](https://doi.org/10.1109/ASE63991.2025.00025)
- **大綱**：
  - 指出 Component Model 依賴 binding generator，將各來源語言的資料表示轉換為 WIT／Canonical ABI 所要求的形式。
  - 建立隨機差分測試系統，讓不同語言產生語意相同的 component。
  - 比較 `wit-bindgen` 與 `wit-bindgen-go` 所生成元件的執行結果。
  - 發現七個先前未知的程式碼生成缺陷，包含 crash 與 silent data corruption 風險。
- **與本研究的關聯**：
  - 本專案使用 WIT binding 連接 Rust host 與 Rust／C／Go／C# 插件；該論文說明「介面標準化」不代表所有工具鏈實作必然正確。
  - 可支持本專案針對 Unicode、null、數值、引號、反斜線等 edge values 進行端到端正確性測試。
  - 多語言插件比較時應固定 WIT 版本、binding generator 版本及 component adapter，並保存可重現的工具鏈資訊。

### 5.2 Random Testing of the WebAssembly System Interface

- **作者／出處**：Ethan Stanley, University of Utah Senior Honors Thesis, 2024。
- **類型**：學位論文／技術報告，非正式會議主論文。
- **論文連結**：[University of Utah PDF](https://www.cs.utah.edu/docs/techreports/2024/UUCS-24-003.pdf)
- **大綱**：
  - 研究 WASI 實作及語言 binding 的隨機測試方法。
  - 產生具型別的測試輸入，跨不同實作比較結果。
  - 分析介面層、生成程式碼與 runtime 行為不一致所造成的錯誤。
  - 為後續 WIT binding generator 差分測試研究建立方法基礎。
- **與本研究的關聯**：
  - 適合用來補充 WIT/WASI 正確性驗證方法，但正式論文引用應優先使用前述 ASE 2025 論文。

### 5.3 WCM 研究現況與缺口

截至本整理日期，直接量測下列問題的同儕審查研究仍相對少見：

- WIT `string`、`list`、巢狀 `record` 與大量 tags 的單次 lift/lower 成本。
- WCM 與 raw Core Wasm C ABI 在相同資料語意及相同 runtime 下的差異。
- 多個 Component Model 階段串聯後的累積複製、配置、GC 與 linear-memory grow 成本。
- 不同來源語言 binding 在相同 WIT 介面下的 copy-in、guest execution 與 copy-out 差異。
- WCM 用於長時間串流日誌管線時的吞吐量、尾延遲、背壓、錯誤隔離及恢復能力。

這些缺口正好對應本專案 `test/ours/copy_overhead/` 與 `test/compare/` 的研究價值。論文可以把相關工作定位為：既有研究分別說明 Wasm 的執行安全、runtime 效能及 WCM binding 正確性，本研究則在日誌處理場景中量化 WCM 高階型別邊界與多階段管線的整體成本。

---

## 6. WebAssembly／WCM 效能研究

### 6.1 Not So Fast: Analyzing the Performance of WebAssembly vs. Native Code

- **作者／出處**：Abhinav Jangda, Bobby Powers, Emery D. Berger, Arjun Guha, USENIX ATC 2019。
- **類型**：同儕審查會議論文。
- **論文連結**：[USENIX 技術議程](https://www.usenix.org/conference/atc19/technical-sessions)；[公開 PDF](https://www.usenix.org/sites/default/files/atc19_full_proceedings.pdf#page=127)
- **大綱**：
  - 質疑僅以小型科學運算 kernel 得出「Wasm 接近原生」結論的代表性。
  - 建立 Browsix-Wasm，使較完整的 Unix/SPEC 工作負載能在瀏覽器中執行。
  - 比較 Wasm 與原生程式的執行效能，並以硬體效能計數器與編譯結果分析差距來源。
  - 將成本區分為編譯器最佳化不足與 Wasm 平台／執行模型本身的限制。
- **與本研究的關聯**：
  - 支持本研究不能只用單一 microbenchmark 宣稱 WCM 的整體效能。
  - 本專案應同時保留 isolated boundary benchmark 與完整 log pipeline benchmark，分別回答機制成本與實務表現。

### 6.2 Sledge: A Serverless-first, Light-weight Wasm Runtime for the Edge

- **作者／出處**：Phani Kishore Gadepalli, Sean McBride, Gregor Peach, Ludmila Cherkasova, Gabriel Parmer, ACM/IFIP Middleware 2020。
- **類型**：同儕審查會議論文。
- **論文連結**：[DOI](https://doi.org/10.1145/3423211.3425680)；[作者公開 PDF](https://www2.seas.gwu.edu/~gparmer/publications/middleware20sledge.pdf)
- **大綱**：
  - 針對 edge serverless 的低延遲及頻繁建立／銷毀函式需求設計 Wasm runtime。
  - 實作 aWsm 編譯器與 Sledge runtime，將輕量沙箱、排程及資源管理整合。
  - 比較原生執行、Wasm 執行及既有 serverless framework 的延遲與吞吐量。
  - 分析 AOT 編譯、sandbox 建立及呼叫路徑的成本。
- **與本研究的關聯**：
  - 可作為「Wasm 執行成本高度依賴 runtime、編譯策略及 host integration」的證據。
  - 本專案主要瓶頸可能在結構化資料跨界與 pipeline，而非單純 guest 算術指令速度，因此需另外量測 copy-in/out。

### 6.3 Pushing Serverless to the Edge with WebAssembly Runtimes

- **作者／出處**：Philipp Gackstatter, Pantelis A. Frangoudis, Schahram Dustdar, IEEE/ACM CCGrid 2022。
- **類型**：同儕審查會議論文。
- **論文連結**：[機構典藏與摘要](https://repositum.tuwien.at/handle/20.500.12708/80548)；[DOI](https://doi.org/10.1109/CCGrid54584.2022.00023)
- **大綱**：
  - 研究資源有限的 edge device 上，Wasm runtime 是否適合取代 container 執行 serverless function。
  - 提出整合 Apache OpenWhisk 的 WOW prototype。
  - 比較 Docker 與多種 Wasm runtime 的 cold start、記憶體、執行時間與吞吐量。
  - 顯示 Wasm 在冷啟動與資源占用方面具有優勢，但結果取決於 runtime 及工作負載。
- **與本研究的關聯**：
  - 支持在受限 CPU／記憶體條件下評估 Wasm，而非只報告無限制環境的峰值。
  - 其核心是 serverless cold start；本專案重點是長存活 log pipeline，引用時不可把冷啟動結果直接套用到穩態吞吐量。

### 6.4 Roadrunner: Accelerating Data Delivery to WebAssembly-Based Serverless Functions

- **作者／出處**：Cynthia Marcelino, Thomas Pusztai, Stefan Nastic, ACM Middleware 2025。
- **類型**：同儕審查會議論文。
- **論文連結**：[機構典藏](https://repositum.tuwien.at/handle/20.500.12708/227835)；[arXiv](https://arxiv.org/abs/2511.01888)
- **大綱**：
  - 將 Wasm serverless 函式的資料傳遞拆解為網路、serialization、memory copy 及 context switch 等成本。
  - 提出 sidecar/runtime shim，利用 memory mapping 與專用資料傳輸路徑減少資料移動。
  - 比較既有遠端資料交換方式與接近 zero-copy 的設計。
  - 評估不同資料量下的通訊延遲、serialization 成本與吞吐量。
- **與本研究的關聯**：
  - 直接支持「資料搬移可能比 Wasm 內部計算更重要」的研究假設。
  - 可作為本專案 Core Wasm direct-memory ABI 與 WCM Canonical ABI 比較的概念參考。
  - 該文研究 serverless inter-function communication，不等於 WCM Canonical ABI；只能比較設計問題，不能直接比較數值。

### 6.5 Hierarchical Integration of WebAssembly in Serverless for Efficiency and Interoperability

- **作者／出處**：Mohammadamin Baqershahi, Changyuan Lin, Visal Saosuo, Paul Chen, Mohammad Shahrad, USENIX NSDI 2026。
- **類型**：同儕審查會議論文。
- **論文連結**：[USENIX 出版頁與 PDF](https://www.usenix.org/conference/nsdi26/presentation/baqershahi)
- **大綱**：
  - 指出 serverless 系統因隔離、保守擴縮及 keep-alive 而產生資源浪費。
  - 提出階層式隔離架構，在 container sandbox 內共同放置多個 Wasm application。
  - 結合資源感知擴縮、排隊與 overbooking，提高部署密度。
  - 比較資源效率、隔離、效能與既有 serverless 架構的相容性。
- **與本研究的關聯**：
  - 說明 Wasm 並不一定取代 container；兩者可以分別負責細粒度程序內隔離與 OS 層資源邊界。
  - 對應本專案使用 Wasmtime 隔離 plugin、同時使用 container cgroup 固定 1 CPU／512 MiB 的實驗設計。

---

## 7. WCM 必要官方規格與技術文件（非學術論文）

以下資料不是同儕審查論文，但描述 WCM 實際標準行為，撰寫 WIT、Canonical ABI、WASI 與能力模型時應優先引用。若學校規定「相關研究只能引用論文」，可將它們列為規格或網路參考資料，而不要標成學術研究。

### 7.1 Component Model Design and Specification

- **文件連結**：[WebAssembly Component Model 官方設計儲存庫](https://github.com/WebAssembly/component-model)
- **大綱**：
  - Component Model 的目標、use cases、元件組合方式與標準化狀態。
  - WIT、binary/text format、Canonical ABI、linking、resources 與 concurrency 等規格入口。
- **與本研究的關聯**：作為 WCM 架構、名詞與標準狀態的主要來源。

### 7.2 Component Model Explainer

- **文件連結**：[MVP Explainer](https://github.com/WebAssembly/component-model/blob/main/design/mvp/Explainer.md)
- **大綱**：
  - 說明 Component、Core Module、Instance、Import、Export、Linking 的關係。
  - 說明 component function 如何透過 `canon lift` 與 `canon lower` 連接 Core Wasm function。
  - 說明 shared-nothing boundary 及高階型別的跨界機制。
- **與本研究的關聯**：可用來推導每經過一次 WCM 階段都可能發生型別轉換、配置與資料複製。

### 7.3 WebAssembly Interface Types（WIT）

- **文件連結**：[WIT 規格](https://github.com/WebAssembly/component-model/blob/main/design/mvp/WIT.md)
- **大綱**：
  - 定義 package、interface、world、record、variant、result、resource 等介面型別。
  - 說明名稱解析、版本與 WIT 定義如何轉換成 Component Model type。
- **與本研究的關聯**：作為 `parsed-entry`、`log-entry`、`filter-result`、`transport-config` 等介面設計的規格依據。

### 7.4 Canonical ABI

- **文件連結**：[Canonical ABI 正式 explainer](https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md)；[較易閱讀的 Component Model 文件](https://component-model.bytecodealliance.org/advanced/canonical-abi.html)
- **大綱**：
  - 定義 WIT 高階型別如何 lower 成 Core Wasm 數值、pointer 與 linear-memory layout。
  - 定義回傳值如何 lift 回 component-level value。
  - 說明字串編碼、`realloc`、`post-return`、resource handle 與複合型別配置。
- **與本研究的關聯**：是 copy-in、guest execution、copy-out 與不同 batch size 成本分析的理論核心。

### 7.5 WASI Capability Model

- **文件連結**：[WASI 官方網站](https://wasi.dev/)
- **大綱**：
  - Wasm module/component 預設沒有 ambient authority。
  - Host 透過明確 import 授予 filesystem、socket、clock、random、HTTP 等能力。
  - 權限取決於 host 實際提供的介面，而不是插件自行宣告後就能使用。
- **與本研究的關聯**：支持 parse/filter/format 不連結 socket 與 filesystem，只有 transport 取得網路能力的最小權限設計。

---

## 8. 可直接用於論文的相關研究組織方式

### 8.1 隔離研究的論述順序

1. 由 *Bringing the Web up to Speed with WebAssembly* 說明 Wasm 的線性記憶體、型別與控制流程安全基礎。
2. 由 RLBox 說明 Wasm 已實際用於程序內第三方程式庫隔離。
3. 由 *Everything Old is New Again* 說明 sandbox 不等於 guest 內部漏洞消失。
4. 由 vWasm/rWasm 與 WaVe 說明 runtime/compiler 本身的正確性仍是可信基礎。
5. 由 2025 resource-isolation 論文說明 CPU、記憶體與 WASI 資源耗盡需要額外限制。
6. 最後導入本研究的 stage capability、memory limiter、timeout、backpressure 與 cgroup 設計。

### 8.2 WCM 研究的論述順序

1. Core Wasm 只提供低階值與線性記憶體，跨語言複合型別需要共同 ABI。
2. WCM 以 WIT 描述高階介面，以 Canonical ABI 定義 lift/lower。
3. 標準介面提升跨語言互通性，但 string/list/record 可能需要配置與複製。
4. ASE 2025 顯示 binding generator 也可能發生 crash 或 silent data corruption。
5. 既有研究對 WCM 多階段串流管線的成本仍缺乏完整量化。
6. 本研究因此比較 Native、Core Wasm direct-memory ABI、Core Wasm JSON 與 WCM，並加入完整日誌轉送器的實務比較。

### 8.3 效能研究的論述順序

1. Wasm 原始設計以接近原生、快速驗證及可攜性為目標。
2. *Not So Fast* 顯示完整應用的差距可能大於小型 kernel benchmark，必須分析編譯器及平台因素。
3. Sledge、WOW、Faasm 等系統顯示 Wasm 可降低隔離與冷啟動成本，但結果依 runtime、AOT/JIT、工作負載及資料交換方式而異。
4. Faasm 與 Roadrunner 顯示 serialization、資料複製及跨執行邊界可能成為主要成本。
5. 本研究進一步把日誌管線時間拆為 copy-in、guest execution、copy-out，並觀察成本如何隨 batch、log bytes、欄位數與處理階段增加。

## 9. 本研究可主張的文獻缺口

可在相關研究章末使用下列方向，但正式定稿前仍應以學校可存取的 IEEE、ACM、USENIX、Scopus 或 Web of Science 再執行一次系統化檢索：

> 既有研究已分別探討 WebAssembly 的執行語意、程序內隔離、runtime 安全、資源隔離與 serverless 效能；WebAssembly Component Model 的既有研究則開始關注 WIT binding generator 的正確性。然而，針對 WCM Canonical ABI 在多階段、資料密集且長時間運作的日誌處理管線中所產生之配置、複製、吞吐量及尾延遲成本，仍缺乏完整的實證比較。特別是，在相同資料語意下，同時比較原生函式、Core Wasm 低階 ABI、WCM 高階介面與實際日誌產品，並納入正確性、突發恢復及長時間穩定性之研究仍相對有限。本研究以此作為主要研究缺口。

使用「仍相對有限」比「完全沒有研究」安全，因為 Component Model 與 WASI 持續快速演進，且可能存在尚未被主要索引收錄的新論文。

## 10. 建議檢索關鍵字

- `WebAssembly sandbox isolation`
- `WebAssembly in-process isolation`
- `WebAssembly resource isolation`
- `WebAssembly untrusted plugins`
- `WebAssembly host guest call overhead`
- `WebAssembly serialization overhead`
- `WebAssembly runtime performance native`
- `WebAssembly serverless cold start`
- `WebAssembly Component Model performance`
- `WIT binding generator`
- `Canonical ABI overhead`
- `Component Model lift lower copy`
- `WASI capability security`
- `WebAssembly logging pipeline`

建議資料庫：ACM Digital Library、IEEE Xplore、USENIX、DBLP、Google Scholar、Scopus 與 Web of Science。引用時優先連到 DOI 或正式出版頁；arXiv 僅在找不到正式版本或該成果仍為預印本時使用。
