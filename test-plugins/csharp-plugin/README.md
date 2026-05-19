## C# build Wasm component 

Guest C#
To generate the bindings:

wit-bindgen csharp -w command -r native-aot --generate-stub wit/
Now you create a c# project file:

dotnet new console -o MyApp
cd MyApp
dotnet new nugetconfig
In the nuget.config after <clear />make sure you have:

<add key="dotnet-experimental" value="https://pkgs.dev.azure.com/dnceng/public/_packaging/dotnet-experimental/nuget/v3/index.json" />
<add key="nuget" value="https://api.nuget.org/v3/index.json" />
In the MyApp.csproj add the following to the property group:

<RuntimeIdentifier>wasi-wasm</RuntimeIdentifier>
<UseAppHost>false</UseAppHost>
<PublishTrimmed>true</PublishTrimmed>
<InvariantGlobalization>true</InvariantGlobalization>
<SelfContained>true</SelfContained>
<AllowUnsafeBlocks>true</AllowUnsafeBlocks>
<WASI_SDK_PATH>path/to/wasi-sdk</WASI_SDK_PATH>
Add the native-aot compiler (substitute win-x64 for linux-x64 on Linux):

dotnet add package Microsoft.DotNet.ILCompiler.LLVM --prerelease
dotnet add package runtime.win-x64.Microsoft.DotNet.ILCompiler.LLVM --prerelease
Now you can build with:

dotnet publish
Check out componentize-dotnet for a simplified experience.

## 從0開始建立C# component的步驟

### 前置需求

| 工具 | 安裝 |
|------|------|
| .NET SDK 9.0+ | https://dotnet.microsoft.com/download |
| wit-bindgen 0.57+ | `cargo install wit-bindgen-cli` |
| wasi-sdk 24+ | https://github.com/WebAssembly/wasi-sdk/releases |
| wasm-tools | `cargo install wasm-tools` |
| WASI preview1 adapter | 見下方 |

```bash
# Linux (WSL2) 快速安裝 .NET 9
wget https://dot.net/v1/dotnet-install.sh && chmod +x dotnet-install.sh
./dotnet-install.sh --channel 9.0
export PATH="$HOME/.dotnet:$PATH"

# 下載 WASI preview1→preview2 adapter（放在 parse/ 目錄）
wget -P parse/ \
  https://github.com/bytecodealliance/wasmtime/releases/latest/download/wasi_snapshot_preview1.reactor.wasm
```

---

### 目錄結構

```
parse/
├── internal/                             # wit-bindgen 生成的所有檔案（勿手動修改）
│   ├── ParserPlugin.cs                   # ABI 膠水 [UnmanagedCallersOnly] exports
│   ├── ParserPluginWorld.wit.Imports.*.cs # 型別定義 + WASI 互動程式 (57 個檔案)
│   ├── ParserPluginWorld_component_type.wit
│   └── ParserPluginWorld_wasm_import_linkage_attribute.cs
│
├── ParserPluginWorldExportsImpl.cs       # ★ 填入實作的檔案（由 stub 產生，使用者編輯）
├── Parsers.cs                            # JSON / syslog / logfmt 解析邏輯
├── Program.cs                            # 空 Main()，entry point
│
├── wit/
│   ├── log_plugin.wit                    # WIT world 定義
│   └── deps/                             # WASI 0.2.0 WIT 依賴
│
├── ParserPlugin.csproj
├── nuget.config
├── Makefile
└── wasi_snapshot_preview1.reactor.wasm  # preview1→preview2 adapter
```

---

### 流程總覽

```
wit/log_plugin.wit
       │
       │  make bindgen
       │  (wit-bindgen csharp -w parser-plugin -r native-aot --generate-stub wit/ --out-dir internal/)
       ▼
internal/ParserPlugin.cs                  ← ABI 膠水（勿修改）
internal/ParserPluginWorld.wit.Imports.*  ← 型別定義（勿修改）
ParserPluginWorldExportsImpl.cs           ← stub → 填入 Parse / ReportUsage / Reset
       │
       │  make publish
       │  (WASI_SDK_PATH=… dotnet publish -c Release)
       ▼
publish/ParserPlugin.wasm                 ← WASM 模組（帶 WASI preview1 imports）
       │
       │  make component
       │  (wasm-tools component new … --adapt wasi_snapshot_preview1.reactor.wasm)
       ▼
parser_csharp.wasm                        ← ★ 最終 WASM Component
```

---

### 步驟 1：產生綁定（首次或 WIT 修改後）

```bash
cd parse/
make bindgen
```

`internal/` 下會產生約 59 個 .cs 檔。
若根目錄尚無 `ParserPluginWorldExportsImpl.cs`，Makefile 會自動把 stub 搬到根目錄。

#### 關鍵生成型別（來自 `internal/ParserPluginWorld.wit.Imports.*.cs`）

```csharp
// using PP = ParserPluginWorld.wit.Imports.local.logProcess.v0_1_0.IPipelineProcessImports;

PP.ParsedEntry          // struct { string timestamp; LogLevel level; string message; List<(string,string)> tags; }
PP.LogLevel             // enum { DEBUG, INFO, WARN, ERROR, CRIT, ALERT, EMERG }
PP.ParseError           // PP.ParseError.InvalidFormat("msg") / .UnsupportedVersion(v) / .CorruptedData()
```

#### `ParserPluginWorldExportsImpl.cs` stub 結構

```csharp
public partial class ParserPluginWorldExportsImpl : IParserPluginWorldExports
{
    public static List<PP.ParsedEntry> Parse(List<string> rawData) { /* 填入 */ }
    public static ulong ReportUsage() { /* 填入 */ }
    public static void Reset()        { /* 填入 */ }
}
```

回傳錯誤用：`throw new WitException<PP.ParseError>(PP.ParseError.InvalidFormat("reason"), 0);`

---

### 步驟 2：還原套件

```bash
dotnet restore
# 需要 dotnet-experimental feed（nuget.config 已設定）
# 下載 Microsoft.DotNet.ILCompiler.LLVM（NativeAOT LLVM WASM backend）
```

---

### 步驟 3：編譯 WASM 模組

```bash
export WASI_SDK_PATH=/opt/wasi-sdk      # 依實際路徑設定
make publish
# 等同: WASI_SDK_PATH=… dotnet publish -c Release -o publish/
```

輸出：`publish/ParserPlugin.wasm`（WASM 模組，包含 WASI preview1 imports）

---

### 步驟 4：包裝成 WASM Component

```bash
make component
# 等同:
# wasm-tools component new publish/ParserPlugin.wasm \
#   --adapt wasi_snapshot_preview1.reactor.wasm \
#   -o parser_csharp.wasm
```

輸出：`parser_csharp.wasm` ← 最終可載入的 WASM Component。

---

### 一鍵完整建置

```bash
export WASI_SDK_PATH=/opt/wasi-sdk
make all        # restore → publish → component
```

---

### 步驟 5：驗證

```bash
# 確認 WIT interface 匯出
wasm-tools component wit parser_csharp.wasm

# 預期輸出：
# package root:component;
# world root {
#   export parse: func(raw-data: list<string>) -> result<list<parsed-entry>, parse-error>;
#   export report-usage: func() -> u64;
#   export reset: func();
# }
```

---

### 解析格式

| 格式 | 首字元偵測 | 範例 |
|------|-----------|------|
| JSON | `{` | `{"ts":"…","level":"info","msg":"…","att":{"k":"v"}}` |
| Syslog (RFC5424-like) | `<` | `<134>1 2024-01-01T00:00:00Z host app 1 - msg level=info k=v` |
| logfmt | 其他 | `ts=… level=info msg="hello" k=v` |

無法解析的行 silent skip（與 Go plugin 行為一致）。

---

### 重新產生綁定後合併實作

WIT 修改後重跑 `make bindgen`，`internal/` 下的生成檔會被覆蓋。
`ParserPluginWorldExportsImpl.cs` **不會**被覆蓋（Makefile 的保護邏輯），
但若型別有異動需手動更新。

```bash
make bindgen          # 重新生成 internal/
git diff              # 確認型別有無破壞性變更
dotnet build          # 驗證編譯
```
