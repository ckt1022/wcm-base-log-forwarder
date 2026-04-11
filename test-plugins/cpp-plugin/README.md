# C++ Plugin 開發指南

從零開始開發 C++ WASM Component Model plugin 的完整流程。  
你只需要 `log_plugin.wit`，其餘都由工具生成。

---

## 目錄結構總覽

```
test-plugins/cpp-plugin/
├── README.md                  ← 本文件
├── format/                    ← format-plugin 範例（已完整實作）
│   ├── wit/
│   │   ├── log_plugin.wit     ← WIT 介面定義（從 host 複製過來的）
│   │   └── deps/              ← WASI 依賴（wit-bindgen 會用到）
│   ├── format_plugin.h        ← 【自動生成，勿手改】C ABI 型別定義
│   ├── format_plugin.c        ← 【自動生成，勿手改】ABI 膠水層
│   ├── format_plugin_component_type.o  ← 【自動生成，勿手改】WIT 型別嵌入
│   ├── format_impl.cpp        ← 【你寫這裡】實際邏輯
│   ├── Makefile
│   └── format.wasm            ← 最終產出（WASM Component）
└── parse/                     ← parser-plugin 參考實作（C 語言）
```

---

## 步驟總覽

```
log_plugin.wit
    │
    ├─ Step 0  安裝工具鏈（wasi-sdk、wasm-tools、wit-bindgen）
    ├─ Step 1  建立 wit/ 目錄，放入 WIT 檔 + WASI deps
    ├─ Step 2  wit-bindgen c  → 生成 .h / .c / _component_type.o
    ├─ Step 3  建立 your_impl.cpp，實作 exports_* 函式
    └─ Step 4  make           → 產出 plugin.wasm ✓
```

---

## Step 0：安裝工具鏈

### wasi-sdk（C/C++ → wasm32-wasip1 編譯器）

wasi-sdk 是唯一官方支援的 C/C++ WASM 工具鏈，**系統的 clang 無法直接使用**。

```bash
# 下載最新版本（檢查 https://github.com/WebAssembly/wasi-sdk/releases 取得最新版號）
curl -L -o /tmp/wasi-sdk.tar.gz \
  "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-32/wasi-sdk-32.0-x86_64-linux.tar.gz"

tar -xf /tmp/wasi-sdk.tar.gz -C ~ --one-top-level=wasi-sdk --strip-components=1

# 確認安裝成功
~/wasi-sdk/bin/clang++ --version
# 應輸出：clang version 22.x.x-wasi-sdk  Target: wasm32-unknown-wasip1
```

> **雷點**：wasi-sdk 解壓後的根目錄名稱因版本而異（例如 `wasi-sdk-32.0-x86_64-linux`）。  
> 務必使用 `--one-top-level=wasi-sdk` 讓它統一解壓到 `~/wasi-sdk`，否則 Makefile 的路徑會對不上。

### wasm-tools 與 wit-bindgen

```bash
cargo install wasm-tools       # 版本 ≥ 1.200
cargo install wit-bindgen-cli  # 版本 = 0.53.x（與 host 的 wit-bindgen 版本需一致）
```

---

## Step 1：建立 wit/ 目錄

```
your-plugin/
└── wit/
    ├── log_plugin.wit      ← 從 host 的 wit/ 複製對應的 world 定義
    └── deps/               ← WASI 依賴，wit-bindgen 解析 WIT 時需要
```

**複製 WASI deps（只需做一次）：**

```bash
mkdir -p wit/deps
cp -r /path/to/wcm-base-log-forwarder/test-plugins/go-plugin/format/wit/deps/* wit/deps/
```

deps 目錄包含：`wasi-cli-0.2.0`、`wasi-clocks-0.2.0`、`wasi-io-0.2.0` 等 WASI 標準介面定義。  
如果 WIT 檔裡有 `include wasi:cli/imports@0.2.0;`，就一定要有這些 deps，否則 `wit-bindgen` 會報 `package not found`。

---

## Step 2：生成 C binding

```bash
wit-bindgen c --world <world-name> --out-dir . wit/
```

以 format-plugin 為例：

```bash
wit-bindgen c --world format-plugin --out-dir . wit/
```

**產出三個檔案（全部自動生成，不要手動修改）：**

| 檔案 | 說明 |
|---|---|
| `format_plugin.h` | 所有 WIT 型別的 C struct 定義 + 你需要實作的函式宣告 |
| `format_plugin.c` | ABI 膠水層：負責把 WASM linear memory 的 offset 轉成 C struct |
| `format_plugin_component_type.o` | 將 WIT 型別資訊嵌入 WASM binary，`wasm-tools component new` 需要它 |

> **注意**：world name 要與 WIT 檔中 `world xxx {` 的名稱完全對應，例如 WIT 寫 `world format-plugin`，指令就用 `--world format-plugin`（用連字號，不是底線）。

---

## Step 3：實作 your_impl.cpp

### 找到需要實作的函式

打開生成的 `.h` 檔，搜尋 `Exported Functions`：

```bash
grep -A5 "Exported Functions" format_plugin.h
```

輸出範例（format-plugin）：

```c
// Exported Functions from `format-plugin`
bool exports_format_plugin_format(
    format_plugin_list_log_entry_t *struct_data,
    format_plugin_list_list_u8_t *ret,
    format_plugin_plugin_error_t *err
);
uint64_t exports_format_plugin_report_usage(void);
```

**這兩個函式就是你需要實作的目標。**  
函式命名規則：`exports_<world-name>_<function-name>`，連字號轉底線。

### 實作範本

```cpp
// your_impl.cpp
#include "format_plugin.h"   // ← 生成的 header，只 include 這一個
#include <cstdlib>
#include <cstring>

// 成功回傳 true，填入 *ret
// 失敗回傳 false，填入 *err（用 err->tag 標明錯誤種類）
extern "C" bool exports_format_plugin_format(
    format_plugin_list_log_entry_t* struct_data,
    format_plugin_list_list_u8_t*   ret,
    format_plugin_plugin_error_t*   err
) {
    size_t n = struct_data->len;
    ret->len = n;
    ret->ptr = (format_plugin_list_u8_t*)malloc(n * sizeof(format_plugin_list_u8_t));

    for (size_t i = 0; i < n; i++) {
        format_plugin_log_entry_t& e = struct_data->ptr[i];
        // ... 你的邏輯 ...
        // ret->ptr[i].ptr = 指向 malloc 出來的 bytes
        // ret->ptr[i].len = byte 數
    }
    return true;
}

extern "C" uint64_t exports_format_plugin_report_usage(void) {
    return 0;  // 回傳 plugin 的 heap 用量，除錯用；不需要就回傳 0
}
```

### 型別對應速查

| WIT 型別 | C struct | 存取方式 |
|---|---|---|
| `string` | `format_plugin_string_t` | `.ptr`（`uint8_t*`）、`.len`（`size_t`）|
| `list<T>` | `format_plugin_list_T_t` | `.ptr`（`T*`）、`.len`（`size_t`）|
| `tuple<string, string>` | `format_plugin_tuple2_string_string_t` | `.f0`（key）、`.f1`（value）|
| `log-entry` | `format_plugin_log_entry_t` | `.id`、`.timestamp`、`.level`、`.message`、`.tags`|
| `log-level` enum | `uint8_t` 常數 | `LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_DEBUG` 等 |
| `result<ok, err>` | `is_err` + `union { ok, err }` | 函式回傳 `bool`，`false` 時填 `*err` |

### 記憶體管理規則

- **輸入（`struct_data`）**：host 擁有，plugin 可以讀，**不能** `free`。
- **輸出（`ret`）**：plugin 必須用 `malloc` 分配，所有權移交給 host，host 呼叫 `cabi_post_*` 後會 `free`。
- 輸出的每一個 `list_u8_t.ptr` 都必須是獨立的 `malloc`，不能指向輸入的記憶體或 stack。

---

## Step 4：編譯

### 使用 Makefile（推薦）

複製 `format/Makefile` 到你的目錄，修改以下兩行：

```makefile
FINAL_WASM := your_plugin.wasm
# 以及 bindings: 目標裡的 --world 名稱
```

```bash
make          # 建置 your_plugin.wasm
make clean    # 清除中間產物
make bindings # WIT 改動後重新生成 C binding
```

### 手動編譯步驟（了解原理）

```bash
WASI_SDK=~/wasi-sdk
ADAPTER=$(find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" | head -1)

# 1. 編譯 C++ 實作
$WASI_SDK/bin/clang++ \
    --target=wasm32-wasip1 \
    --sysroot=$WASI_SDK/share/wasi-sysroot \
    -fno-exceptions -O2 -I. \
    -c your_impl.cpp -o your_impl.o

# 2. 編譯生成的 C binding（用 clang，不是 clang++）
$WASI_SDK/bin/clang \
    --target=wasm32-wasip1 \
    --sysroot=$WASI_SDK/share/wasi-sysroot \
    -O2 -I. \
    -c format_plugin.c -o format_plugin.o

# 3. 連結成 WASM core module
$WASI_SDK/bin/clang \
    --target=wasm32-wasip1 \
    --sysroot=$WASI_SDK/share/wasi-sysroot \
    -Wl,--no-entry -Wl,--export-dynamic \
    your_impl.o format_plugin.o format_plugin_component_type.o \
    -o plugin_core.wasm

# 4. 包裝成 WIT Component（把 preview1 轉換成 preview2）
wasm-tools component new plugin_core.wasm \
    --adapt wasi_snapshot_preview1="$ADAPTER" \
    -o your_plugin.wasm

# 5. 驗證
wasm-tools validate your_plugin.wasm
```

---

## 常見雷點

### 1. 用系統 clang 編譯

**現象**：連結時找不到 `wasm-ld`，或編譯出 x86_64 binary。  
**原因**：系統 clang（`/usr/bin/clang`）沒有 WASM sysroot，也沒有對應的 libc。  
**解決**：一律使用 `~/wasi-sdk/bin/clang++` 和 `~/wasi-sdk/bin/clang`。

---

### 2. 把生成的 `.c` 用 `clang++` 編譯

**現象**：大量 `[-Wdeprecated]` 警告，嚴重時連結失敗。  
**原因**：`wit-bindgen` 生成的 `format_plugin.c` 是標準 C（C11），裡面有些 C-only 語法在 C++ 模式下視為 deprecated。  
**解決**：`.c` 用 `clang`，你的 `.cpp` 用 `clang++`，連結也用 `clang`（如 Makefile 所示）。

---

### 3. 跳過 `_component_type.o`

**現象**：連結成功，但 `wasm-tools component new` 失敗：
```
error: module was not valid
  undefined symbol: __component_type_object_force_link_...
```
**原因**：`format_plugin.c` 的最後幾行呼叫了定義在 `_component_type.o` 裡的函式。沒有它，WASM binary 缺少 WIT 型別描述，`wasm-tools` 無法打包成 component。  
**解決**：連結時永遠帶上 `format_plugin_component_type.o`。

---

### 4. wit-bindgen 版本不符

**現象**：`wasm-tools validate` 失敗，或 host 載入時報 `import not found`。  
**原因**：host 用的 `wit-bindgen` 版本（本專案為 0.53.1）與你生成 binding 的版本不同，ABI layout 可能有差。  
**解決**：確認版本一致：
```bash
wit-bindgen --version   # 應輸出 wit-bindgen-cli 0.53.1
```
如需更新 host，記得同步重新生成所有 plugin 的 binding。

---

### 5. 在輸出 buffer 用 stack 或輸入指標

**現象**：host 在取得結果後發生記憶體錯誤或資料損毀。  
**原因**：WIT C binding 的記憶體所有權嚴格：輸出的 `list<u8>` 必須是用 `malloc` 分配的獨立 heap 記憶體，host 的 `cabi_post_*` 清理函式會 `free` 它。若你指向 stack 或輸入的指標，free 時就爆。  
**規則**：每一個 `ret->ptr[i].ptr` 都必須是 `malloc` 的結果。

---

### 6. 使用 C++ 例外（exceptions）

**現象**：編譯時警告 `exception handling disabled`，執行時 `unreachable`。  
**原因**：WASI 環境不支援 C++ 例外機制，`throw`/`catch` 在 WASM 中未定義。  
**解決**：編譯旗標加 `-fno-exceptions`（Makefile 已包含），改用回傳碼處理錯誤。

---

### 7. `wit/deps` 缺少 WASI 依賴

**現象**：執行 `wit-bindgen c` 時報：
```
Error: package not found  --> wit/log_plugin.wit:89:13
  include wasi:cli/imports@0.2.0;
```
**原因**：WIT 中用了 `include wasi:cli/imports@0.2.0`，但 `wit/deps/` 沒有對應的 package 定義。  
**解決**：從 `test-plugins/go-plugin/format/wit/deps/` 整個複製到你的 `wit/deps/`。

---

## 驗證 plugin 輸出格式

快速確認 plugin 是否符合 pipeline 預期（以 format plugin 為例）：

```bash
# 切換 host 使用你的 plugin（改 src/main.rs 中的 format 路徑）
# 然後：
go run tools/gen/main.go -rate 1000 -duration 3 | ./target/debug/wcm-base-log-forwarder 2>/dev/null | head -5
```

format plugin 的期望輸出格式：

```
2024-01-15T12:00:00Z INFO user logged in service=auth region=us-east
2024-01-15T12:00:00Z ERROR connection timeout host=db-01 retry=3
```

格式：`timestamp LEVEL message key=value ...`（空格分隔，與 Go plugin 完全相同）。

---

## 參考

| 資源 | 說明 |
|---|---|
| [wit/log_plugin.wit](format/wit/log_plugin.wit) | 本專案的 WIT 介面定義 |
| [format_impl.cpp](format/format_impl.cpp) | 完整 C++ 實作範例 |
| [parse/main.c](parse/main.c) | C 語言實作範例（logic 參考） |
| [wasi-sdk releases](https://github.com/WebAssembly/wasi-sdk/releases) | 下載工具鏈 |
| [wit-bindgen C 文件](https://github.com/bytecodealliance/wit-bindgen) | binding 生成工具說明 |
