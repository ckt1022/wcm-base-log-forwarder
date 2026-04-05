# wcm-base-log-forwarder

A simple log forwarder tool built with Rust, designed to process and forward streamed logs efficiently.

---

## 🚀 Getting Started

### 1. Build the Project

```bash
cargo build
```

---

### 2. Run with Log Generator

Use the following command to pipe logs into the forwarder:

```bash
go run tools/gen/main.go -rate 1000 -duration 10 | ./target/debug/wcm-base-log-forwarder
```

---

## ⚙️ Parameters

### Log Generator (`abc.go`)

* `-rate` : Number of logs generated per second
* `-duration` : Duration of log generation (in seconds)

Example:

```bash
go run tools/gen/main.go -rate 1000 -duration 10 | ./target/debug/wcm-base-log-forwarder
```

---

## 📌 Requirements

* Rust (https://www.rust-lang.org/)
* Go (https://golang.org/)

---

## 📂 Project Structure

```
.
├── src/
├── target/
├── README.md
```

---

## 🧪 Use Cases

This tool is suitable for:

* High-throughput log testing
* Pipeline validation
* Streaming performance benchmarking

---

## ⚠️ Notes

* The compiled binary will be located at:

  ```
  target/debug/wasm-base-log-forwarder
  ```
* Adjust `rate` and `duration` based on your testing requirements.
* Make sure the `abc.go` file exists in the specified relative path.

---

## 注意編譯細節與錯誤

### WIT 介面版本管理

- `wit/log_plugin.wit` 是唯一的正規來源（canonical source），所有 plugin 目錄下的 `wit/log_plugin.wit` 都必須與此完全一致，修改後需手動同步複製（`cp wit/log_plugin.wit test-plugins/.../wit/log_plugin.wit`）
- WIT package 宣告加入版本號（例如 `package local:log-process@0.1.0`）後，`wkg wit build` 輸出的檔名會帶版本（`local:log-process@0.1.0.wasm`），對應的 tinygo `--wit-package` 參數與 README 範例也必須同步更新，否則找不到套件
- `wit-bindgen-go generate` 的 `--package-root` 必須設為 `example.com/internal`（而非 `example.com`），否則生成的 import 路徑缺少 `internal/` 前綴，導致編譯錯誤

### TinyGo 版本與 Go 工具鏈相容性

- TinyGo 0.40.x 僅支援 Go **1.19–1.25**，若系統預設 Go 版本 ≥ 1.26，編譯時會報錯 `requires go version 1.19 through 1.25, got go1.26`
- 設定 `GOROOT` 環境變數單獨使用無效，TinyGo 檢查版本時走的是 `PATH` 裡的 `go` 二進位，必須**同時設定 `GOROOT` 和 `PATH`** 才能生效：
  ```bash
  GO125="$(GOTOOLCHAIN=go1.25.0 go env GOROOT)"
  GOROOT="$GO125" PATH="$GO125/bin:$PATH" tinygo build ...
  ```
- 若系統沒有 go1.25，可用 Go 工具鏈管理自動下載：`GOTOOLCHAIN=go1.25.0 go version`（首次執行會自動下載至 `$GOPATH/pkg/mod/golang.org/toolchain@v0.0.1-go1.25.0.linux-amd64`）
- `go.mod` 的 `go` 指令版本必須 ≤ 1.25（例如 `go 1.25.0`），否則 TinyGo 直接拒絕編譯，與 GOROOT 設定無關

### Go 綁定重新生成（wit-bindgen-go）

- 修改 WIT 後必須先 `wkg wit build` 重建 `.wasm` 套件，再執行 `wit-bindgen-go generate`，否則生成的型別與新 WIT 不一致
- 刪除舊的 `internal/local/` 目錄再重新生成，避免殘留舊介面名稱（例如舊的 `parse-process/` 目錄在改名為 `pipeline-process/` 後若不清除，會同時存在兩個版本造成混亂）
- format plugin 的 `go.mod` 原本沒有 `wit-bindgen-go` 工具依賴，需先執行 `go get go.bytecodealliance.org/cmd/wit-bindgen-go@latest` 或直接呼叫已安裝的 `$GOPATH/bin/wit-bindgen-go` 二進位

### WIT 介面升級（parsed-entry 與 log-entry id）

- 新版 WIT 將 parser 的回傳型別從 `list<log-entry>` 改為 `list<parsed-entry>`（不含 `id` 欄位），**id 由 host 在 parse 後統一分配**，plugin 不應也不能設定 id
- Host 端（`runtime.rs`）需在收到 `Vec<ParsedEntry>` 後將每筆資料加上 id 轉換為 `Vec<LogEntry>` 再傳入後續 plugin；id 命名規則建議使用 `seq * MAX_BATCH + index` 確保批次間全域唯一
- format plugin 的 `LogEntry` 多了 `id: u64` 欄位，但 format 邏輯本身不需要使用 id，不填入格式化輸出是正確行為

### Rust host bindgen（wasmtime::component::bindgen!）

- 刪除 `wit/log_host.wit` 並改用整個 `wit/` 目錄（`path: "wit"`）後，`format_bindings` 的 `with` 區塊必須使用正確的 Rust 模組路徑 `local::log_process::pipeline_process`（底線分隔，對應 WIT 的 kebab-case `local:log-process/pipeline-process`）
- 兩個 bindgen（parser 與 format）共用同一份 `pipeline-process` 型別的前提是 `format_bindings` 的 `with` 設定正確指向 parser bindgen 生成的模組，否則 channel 傳遞 `Vec<LogEntry>` 時會出現型別不相容錯誤
