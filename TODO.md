# Architecture TODO

## 速度

### [1] Parse 並行化（N parse workers）
**目標：** 突破單執行緒 parse 上限（~12,500 eps），線性擴展吞吐量。

**設計：**
- `run_pipeline` 中開 N 個 parse worker，各持有獨立 `Store`，共用同一組 `Engine` / `Component`
- N 個 worker 從同一個 `rx_raw` 搶讀 `LineItem`（`sync_channel` 天然分流）
- 每個 worker 產出帶 `seq` 的 `ParsedBatch` 丟入同一個 channel

**順序問題：**
- 若下游不要求順序（論文吞吐量實驗）：直接讓 format 接收亂序 batch
- 若需要順序（正確性驗證）：在 format 前加 reorder buffer，等待 seq 連續才往下送

**WIT 無需修改，只改 runtime.rs。**

---

### [2] Instance Pool + `gc-reset` WIT 導出（解決 OOM + 消除每 batch 啟動成本）
**目標：** 把目前「每 batch 重建 Store + 重新 instantiate」改為少量長駐實例，同時避免 TinyGo heap 在 reuse 時累積導致 OOM。

**OOM 的真正原因：**
```
batch N 處理完 → Rust 已拿走結果
              → TinyGo GC 尚未回收 → 舊資料仍佔 WASM heap
batch N+1 立刻開始 → TinyGo heap = (舊 4MB) + (新 4MB) → OOM
```
問題不是 WASM 記憶體不能縮，而是 **host 端在 GC 還沒跑完就餵下一批**，時機不受控。

**解法：WIT 加入 `gc-reset` 導出，讓 GC 時機變成雙方協議**

```wit
// wit/log_plugin.wit — parser-plugin world 新增
export gc-reset: func() -> u64;
// plugin 實作：runtime.GC() + 回傳清理後的 HeapInuse bytes
// host 用此決定是否繼續重用這個實例
```

**Host 端 Instance Pool 設計（runtime.rs）：**
```
struct WarmInstance { store: Store<MyState>, plugin: ParserPlugin }
struct InstancePool { ready: VecDeque<WarmInstance>, capacity: usize }

parse_loop 流程：
  1. pool.try_take()  → 有 → 直接用（零啟動成本）
                      → 沒有 → 新建（blocking，稀有）
  2. 執行 batch
  3. call gc-reset() → heap < 70% limit → pool.return(instance)
                     → heap 超標     → drop，pool.request_new()
  4. 背景 pre-warmer 確保 pool 常備 ≥1 個暖實例
```

**影響範圍：**
- `wit/log_plugin.wit`：parser / format / enricher plugin worlds 各加一行 `export gc-reset`
- `src/runtime.rs`：`do_parse_batch` 改用 pool；新增 `InstancePool` struct
- Plugin 實作需加 `GcReset()` 函式（行為固定，plugin 作者只需複製範例）

**論文角度：** 這是 WCM 特有的設計模式 — *Explicit Memory Contract*：host 與 plugin 通過 WIT 協商 GC 時機，是原生 WASM（無法跨邊界控制 GC）做不到的。值得作為獨立 contribution 描述。

---

### [3] WIT 介面傳遞格式改為 flat buffer
**現況：** 目前已改為 `list<string>`（從原本的 `list<list<u8>>`）
**目標：** 消除 canonical ABI 對每行做一次 memory copy 的 overhead。

**問題：** `list<string>` 的 Component Model ABI，wasmtime 對每條 line 各呼叫一次 `cabi_realloc` + 一次 memcpy，N 條 → N 次 WASM 函式呼叫。

**改法 A（簡單，推薦先做）：**
```wit
// 新增 parse-flat，舊 parse 保留，host 視情況選擇呼叫
export parse-flat: func(
    raw-data: list<u8>,    // concat(lines, '\n')
    line-count: u32,
) -> result<list<parsed-entry>, parse-error>;
```
Host 端：`batch.lines.join("\n").into_bytes()` → 一次 memcpy 傳入 WASM，`cabi_realloc` 呼叫從 N 次降為 1 次。

**改法 B（彈性，支援行內有換行的二進位資料）：**
```wit
export parse-flat: func(
    data: list<u8>,        // flat byte buffer
    offsets: list<u32>,    // offsets[i] = 第 i 行起始
    lengths: list<u32>,    // lengths[i] = 第 i 行長度
) -> result<list<parsed-entry>, parse-error>;
```
ABI 只有 3 次大塊 copy，與行數完全無關。

**影響：** `log_plugin.wit` parser-plugin world + 所有語言 parser plugin 加一個函式（舊 `parse` 保留相容）。

---

### [4] Output ABI 優化：tags 改為 parallel flat list
**目標：** 消除 `list<tuple<string,string>>` 的 nested list ABI 間接層，降低每個 entry 的 Rust heap allocation 次數。

**問題：**
```
list<parsed-entry> 反序列化時（wasmtime → Rust）：
  每個 entry 的 tags: list<tuple<string,string>>
    → wasmtime 需先讀 outer list ptr/len
    → 對每個 tuple 再讀 2 個 string ptr/len
    → 2 次獨立 heap alloc + memcpy per tag pair
  syslog 每條 entry ≈ 4–6 個 tags → 8–12 次 String alloc
```

**改法：** 把一個 nested list 拆成兩個 parallel flat list
```wit
record parsed-entry {
    timestamp: string,
    level: log-level,
    message: string,
    // 原本：tags: list<tuple<string, string>>
    tag-keys:   list<string>,   // ["host", "app", ...]
    tag-values: list<string>,   // ["web1", "nginx", ...]
}
```
wasmtime 可對 `list<string>` 做連續記憶體讀取（一次 bounds check + batch memcpy），取代原本 per-tuple 的間接跳讀。

**Host 端重組：**
```rust
// 反序列化後：
let tags: Vec<(String, String)> = entry.tag_keys.into_iter()
    .zip(entry.tag_values.into_iter())
    .collect();
```

**影響：** `log_plugin.wit` 修改 `parsed-entry` record + `log-entry` record（同步改）+ 所有語言 plugin 的 tags 組裝方式（低改動量）。

---

~~## 穩定性~~
完成parse
### [5] Parse 錯誤不應 crash 整個 pipeline
**目標：** 一個 bad batch 只丟失該批次，不終止整條 pipeline。

**現況（runtime.rs do_parse_batch）：**
```rust
Err(e) => {
    eprintln!("[parse-oom] batch={}", seq);
    batch.clear();
    return Err(e);  // ← 這會讓 parse thread 終止，整個 pipeline 停掉
}
```

**改法：**
```rust
Err(e) => {
    eprintln!("[parse-error] batch={} skipped: {}", seq, e);
    stats.total_error_batches += 1;
    batch.clear();
    return Ok(None);  // 跳過這批，繼續下一批
}
```

**ParseStats 需新增：** `total_error_batches: u64`

---

### [6] Poison message 隔離（WIT 層設計）
**目標：** 讓 plugin 能逐行回報解析結果，而非整批成功或整批失敗。

**現況：**
```wit
// 整批成功或整批失敗，host 無法知道哪幾行出問題
parse: func(raw-data: list<list<u8>>) -> result<list<parsed-entry>, parse-error>
```

**改法：**
```wit
variant line-result {
    ok(parsed-entry),
    err(parse-error),   // 包含 line-index: u32 + reason
}

parse: func(raw-data: list<list<u8>>) -> list<line-result>
```
Host 拿到後：
- `ok` → 送入下游
- `err` → 寫入 dead-letter queue（未來 Phase 2 實作）或 stderr log

**影響：** 需要改 WIT + 所有語言 parser plugin。

---

### [7] ChannelStats 串入 pipeline（背壓可觀測性）
**目標：** 讓 stdin reader 和 parse thread 能觀測 channel 積壓狀況，為論文背壓實驗提供數據。

**現況：** `ChannelStats` 結構已在 config.rs 建好，但未連接進 pipeline。

**改法：**

1. `spawn_stdin_reader` 接收 `ChannelStats` 並在每次 `tx.send()` 後呼叫 `on_send`
2. `parse_loop` 在每次 `rx.recv()` 後呼叫 `on_recv`
3. Parse loop 定期取 `snapshot()` 記錄高水位，加入 `ParseStats`
4. `print_pipeline_summary` 印出 channel 平均積壓 / 峰值積壓

**論文用途：** 直接量化「parse 跟不上時 channel 積壓曲線」，是背壓機制的核心實驗數據。

**只改 app.rs + runtime.rs + output.rs，不改 WIT。**

### [8] Q1：設計上需要順序嗎？
架構角度：不需要強制順序，但要能追溯。

Log forwarder 的傳送目標（Elasticsearch、Loki、S3）本質上是 append-only 的，它們依賴 timestamp 欄位排序，而非接收順序。亂序的 batch 送到後，目標系統自己會用 timestamp 排好。

論文角度：這個問題本身就是一個研究點。

情境	需要順序？	理由
吞吐量 / 延遲基準測試	不需要	評估的是處理能力，不是保序
正確性驗證	需要	要確認每條 log 都送到且沒重複
安全隔離實驗（plugin crash）	不需要	看的是「crash 後系統是否繼續運作」
與 Fluent Bit 比較	要記錄差異	Fluent Bit 單執行緒保序，本系統 N workers 不保序，是一個 trade-off 值得討論
建議設計： 在 ParsedBatch 上保留 seq 欄位（已有），讓系統「有能力重排但預設不重排」。論文可以把這個作為 configurable trade-off 討論，不需要在系統裡強制實作 reorder buffer。
