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

### [3] WIT 介面傳遞格式改為 flat buffer
**目標：** 消除 canonical ABI 對每行做一次 memory copy 的 overhead。

**現況：**
```wit
parse: func(raw-data: list<list<u8>>) -> result<list<parsed-entry>, parse-error>
```
每行是獨立的 `list<u8>`，wasmtime canonical ABI 要對每行各寫一次 ptr+len 到 WASM 記憶體。

**改法：**
```wit
parse: func(
    data: list<u8>,       // 所有行合成一個 flat buffer
    offsets: list<u32>,   // offsets[i] = 第 i 行的起始 byte index
    lengths: list<u32>,   // lengths[i] = 第 i 行長度（或用 offsets[i+1]-offsets[i]）
) -> result<list<parsed-entry>, parse-error>
```
這樣 canonical ABI 只有 3 次大塊 copy，與行數無關。

**影響：** 需要改 `log_plugin.wit` parser-plugin world + 所有語言的 parser plugin 實作。

---

## 穩定性

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
