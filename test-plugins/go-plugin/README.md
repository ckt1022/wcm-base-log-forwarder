# Go Plugin 開發指南

從零開始開發 Go WASM Component Model plugin 的完整流程。
你只需要 `log_plugin.wit`，其餘都由工具生成。

---

## 前置工具安裝

| 工具 | 用途 | 安裝指令 |
|------|------|---------|
| **Go 1.19–1.25** | tinygo 目前支援的版本範圍 | [go.dev/dl](https://go.dev/dl/) |
| **tinygo ≥ 0.40** | 編譯 Go → WASM Component | [tinygo.org/getting-started](https://tinygo.org/getting-started/install/) |
| **wkg** | 管理 WIT 依賴、編譯 WIT 套件 | `cargo install wkg` |
| **wasm-tools** | 驗證、檢查 WASM Component | `cargo install wasm-tools` |
| **wit-bindgen-go** | 從 WIT 生成 Go binding | `go install go.bytecodealliance.org/cmd/wit-bindgen-go@latest` |

安裝 `wit-bindgen-go` 後，執行檔位於 `$(go env GOPATH)/bin/`。
建議加入 PATH：

```bash
export PATH="$(go env GOPATH)/bin:$PATH"
```

---

## 步驟總覽

```
log_plugin.wit
    │
    ├─ Step 1  建立 Go 專案（go mod init）
    ├─ Step 2  建立 wit/ 目錄，放入 WIT 檔案
    ├─ Step 3  wkg wit fetch   → wkg.lock（下載外部 WIT 依賴）
    ├─ Step 4  wkg wit build   → local:log-process.wasm
    ├─ Step 5  wit-bindgen-go  → internal/（Go binding，勿手動修改）
    ├─ Step 6  go get 安裝 cm 套件 → go.mod / go.sum
    ├─ Step 7  實作 main.go
    └─ Step 8  tinygo build    → parser.wasm ✓
```

---

## Step 1：建立 Go 專案

```bash
mkdir my-parser && cd my-parser

# ⚠ 必須用 Go 1.25 以下的版本初始化
# 若系統預設版本 > 1.25，tinygo 會因 go.mod 的 `go` 指令版本過高而拒絕編譯
# 使用 /usr/local/go（安裝路徑依實際情況調整）
/usr/local/go/bin/go mod init example.com
```

初始化後，`go.mod` 的第一行應為 `go 1.25.x`（不可高於 1.25）：

```
module example.com

go 1.25.0
```

> **為什麼版本有限制？**
> tinygo 在編譯前會讀取 `go.mod` 的 `go` 指令，若版本超出支援範圍（目前 1.19–1.25）直接報錯。
> 這與 `GOROOT` 環境變數無關——`go.mod` 的宣告版本本身就必須符合。

---

## Step 2：放入 WIT 介面定義

```bash
mkdir wit
cp /path/to/log_plugin.wit wit/
```

`wit/log_plugin.wit` 定義了 plugin 與 host 之間的契約：

```wit
package local:log-process;

interface parse-process {
    record log-entry {
        timestamp: string,
        level: log-level,
        message: string,
        tags: list<tuple<string, string>>,
    }
    enum log-level { debug, info, warn, error, crit, alert, emerg }
    variant parse-error {
        invalid-format(string),
        unsupported-version(u16),
        corrupted-data,
    }
}

world parser-plugin {
    include wasi:cli/imports@0.2.0;   // 外部依賴，需要 Step 3 下載
    use parse-process.{ log-entry, parse-error };
    export parse: func(raw-data: list<list<u8>>) -> result<list<log-entry>, parse-error>;
    export report-usage: func() -> u64;
}
```

---

## Step 3：下載外部 WIT 依賴

WIT 中的 `include wasi:cli/imports@0.2.0` 是外部套件，需要從 registry 下載。

```bash
wkg wit fetch --wit-dir wit/
```

成功後產生 `wkg.lock`，記錄依賴的版本與 digest：

```toml
[[packages]]
name = "wasi:cli"
registry = "wasi.dev"
[[packages.versions]]
version = "0.2.0"
digest = "sha256:e7e854..."
```

> `wkg.lock` 已存在時可跳過此步驟（離線環境亦可直接進行 Step 4）。

---

## Step 4：將 WIT 編譯為套件 binary

把 `wit/` 目錄下的 WIT 定義（含下載的依賴）打包成一個 `.wasm` binary。
這個檔案不是可執行程式，而是供後續工具讀取的型別資訊。

```bash
wkg wit build --wit-dir wit/
# 輸出檔名由 WIT package 宣告決定：local:log-process.wasm
```

驗證內容：

```bash
wasm-tools component wit local:log-process.wasm
```

---

## Step 5：生成 Go Binding

從 WIT 套件 binary 自動生成 Go 程式碼。
生成的程式碼處理所有 Component Model ABI 細節。

```bash
wit-bindgen-go generate \
  --world parser-plugin \
  --out internal/ \
  --package-root example.com/internal \
  local:log-process.wasm
```

產生的目錄結構：

```
internal/
└── local/log-process/
    ├── parse-process/
    │   └── parse-process.wit.go   # LogEntry, LogLevel, ParseError 型別
    └── parser-plugin/
        ├── parser-plugin.wit.go   # Exports struct（綁定入口）
        ├── parser-plugin.exports.go
        ├── parser-plugin.wasm.go  # ABI 底層實作
        └── abi.go
```

> `internal/` 為自動生成，請勿手動修改。修改 WIT 後需重新執行此步驟。

---

## Step 6：安裝 Go 依賴套件

生成的 `internal/` 程式碼 import 了 `go.bytecodealliance.org/cm`（Component Model 工具函式庫），
需要加入 `go.mod`。

```bash
# ⚠ 同樣需要用 Go 1.25 執行，避免 go.mod 被更新為 1.26
# 直接指定版本
go get go.bytecodealliance.org/cm@v0.3.0  
```

執行後 `go.mod` 會新增：

```
require go.bytecodealliance.org/cm v0.x.x
```

同時產生 `go.sum`。

---

## Step 7：實作 Plugin 邏輯

建立 `main.go`，實作 WIT 定義的 export 函數：

```go
package main

import (
    "encoding/json"

    parseprocess "example.com/internal/local/log-process/parse-process"
    parserplugin  "example.com/internal/local/log-process/parser-plugin"
    "go.bytecodealliance.org/cm"
)

type RawLog struct {
    Ts    string            `json:"ts"`
    Level string            `json:"level"`
    Msg   string            `json:"msg"`
    Att   map[string]string `json:"att"`
}

func init() {
    // ⚠ 必須在 init() 中綁定，WASM Component 初始化時執行
    // main() 不會被呼叫
    parserplugin.Exports.Parse = Parse
    parserplugin.Exports.ReportUsage = ReportUsage
}

func Parse(rawData cm.List[cm.List[uint8]]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.LogEntry], parserplugin.ParseError] {
    rawSlice := rawData.Slice()
    entries := make([]parserplugin.LogEntry, 0, len(rawSlice))

    for _, rawBuf := range rawSlice {
        data := rawBuf.Slice() // 直接取底層 []byte，不複製

        var raw RawLog
        if err := json.Unmarshal(data, &raw); err != nil {
            continue // 跳過無效行，不中止整個 batch
        }

        // 將 JSON level 字串對應至 WIT enum
        var level parseprocess.LogLevel
        if err := level.UnmarshalText([]byte(raw.Level)); err != nil {
            level = parseprocess.LogLevelInfo // 無法識別時預設 info
        }

        pairs := make([][2]string, 0, len(raw.Att))
        for k, v := range raw.Att {
            pairs = append(pairs, [2]string{k, v})
        }

        entries = append(entries, parserplugin.LogEntry{
            Timestamp: raw.Ts,
            Level:     level,
            Message:   raw.Msg,
            Tags:      cm.ToList(pairs),
        })
    }

    return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.LogEntry], parserplugin.ParseError]](
        cm.ToList(entries),
    )
}

func ReportUsage() uint64 {
    return 0 // 可實作記憶體峰值回報，詳見 parser/ 目錄的完整範例
}

// WASM Component 不呼叫 main()，但必須宣告
func main() {}
```

**關鍵注意事項：**

| 細節 | 說明 |
|------|------|
| `init()` 綁定 | WASM Component 的實際入口，`main()` 不會執行 |
| `.Slice()` | 從 Component Model list 取底層 slice，避免複製 |
| 跳過錯誤行 | `continue` 而非 `return Err`，保留已解析結果 |
| Level 對應 | 使用 binding 生成的 `UnmarshalText`，與 WIT enum 保持一致 |

---

## Step 8：編譯為 WASM Component

```bash
# GOROOT：指向 Go 1.19–1.25 的安裝目錄
# PATH  ：確保 tinygo 呼叫的 `go` 是 1.25 版本（光設 GOROOT 不夠）
GOROOT=/usr/local/go \
PATH="/usr/local/go/bin:$PATH" \
tinygo build \
  -target=wasip2 \
  -o parser.wasm \
  --wit-package local:log-process.wasm \
  --wit-world parser-plugin \
  main.go
```

驗證輸出：

```bash
# 確認為合法的 WASM Component（非 core module）
wasm-tools validate --features component-model parser.wasm

# 查看 export 介面是否符合 WIT 定義
wasm-tools component wit parser.wasm
```

---

## 修改後的快速重建

**只改 `main.go`（不動 WIT）：**

```bash
GOROOT=/usr/local/go PATH="/usr/local/go/bin:$PATH" \
tinygo build -target=wasip2 -o parser.wasm \
  --wit-package local:log-process.wasm --wit-world parser-plugin main.go
```

**修改了 `wit/log_plugin.wit`：**

```bash
wkg wit build --wit-dir wit/
wit-bindgen-go generate --world parser-plugin --out internal/ \
  --package-root example.com/internal local:log-process.wasm
GOROOT=/usr/local/go PATH="/usr/local/go/bin:$PATH" \
tinygo build -target=wasip2 -o parser.wasm \
  --wit-package local:log-process.wasm --wit-world parser-plugin main.go
```

---

## 常見錯誤

| 錯誤訊息 | 原因 | 解決方式 |
|---------|------|---------|
| `requires go version 1.19 through 1.25` | `go.mod` 的 `go` 指令版本 > 1.25 | 用 `/usr/local/go/bin/go mod init` 初始化，或手動將 `go.mod` 中的版本改為 `1.25.0` |
| `requires go version 1.19 through 1.25` 即使設了 GOROOT | `PATH` 仍指向舊 go，tinygo 呼叫 PATH 裡的 `go` 檢查版本 | 同時設定 `PATH="/usr/local/go/bin:$PATH"` |
| `export not found: parse` | `init()` 中未綁定 `Exports.Parse` | 確認 `parserplugin.Exports.Parse = Parse` |
| `undefined: cm.List` | `go.bytecodealliance.org/cm` 未加入依賴 | 執行 Step 6 的 `go get` |
| `failed to decode component` | 產生的是 core WASM module 非 Component | 確認 tinygo 使用 `-target=wasip2`（不是 `wasi`） |
| binding 編譯錯誤 | WIT 修改後未重新生成 `internal/` | 重新執行 `wit-bindgen-go generate` |
