# Fluent Bit Lua 錯誤注入測試

測試 Lua filter 發生各種錯誤時 Fluent Bit 的行為，並觀察是否能繼續處理後續 log。

## 環境需求

- Fluent Bit v5：`/opt/fluent-bit/bin/fluent-bit`
- Python 3（loggen）
- 測試 log 檔：`../test-logs.log`（腳本自動建立）

## 執行測試

```bash
./script/run_crash.sh <test>
```

| `<test>` | 說明 |
|---|---|
| `loop` | 無限迴圈，永久 block engine |
| `io` | blocking I/O，永久 block engine |
| `cpu` | 100% CPU 佔用 10 秒後返回 |
| `mem` | 無限分配記憶體直到 OOM kill |
| `parse` | 解析 malformed JSON 時 crash |

範例：

```bash
./script/run_crash.sh loop
./script/run_crash.sh cpu
./script/run_crash.sh mem
```

## 測試流程

腳本啟動 Fluent Bit 後，自動依序注入：

```
t=+3s   seq:1  正常 log（應出現在 stdout）
t=+6s   seq:2  觸發錯誤情境
t=+9s   seq:3  正常 log（loop/io/mem 測試不應出現）
t=+12s  loggen 開始持續輸入 30 秒（觀察錯誤後是否能恢復）
```

完成後按 `Ctrl+C` 結束。

## 各測試預期行為

| 測試 | seq:1 | seq:2 後 | seq:3 | loggen | Fluent Bit |
|---|---|---|---|---|---|
| `loop` | 出現 | 靜默（engine 卡死）| 不出現 | 靜默 | 存活但無反應 |
| `io` | 出現 | 靜默（engine 卡死）| 不出現 | 靜默 | 存活但無反應 |
| `cpu` | 出現 | 靜默約 10 秒 | 延遲出現 | 正常 | 恢復正常 |
| `mem` | 出現 | 記憶體快速上升 | 不出現 | 靜默 | **process 被 OOM kill** |
| `parse` | 出現 | error log，seq:2 被 drop | 出現 | 正常 | 恢復正常（per-record 隔離）|

## protected_mode 說明

腳本會自動設定 `protected_mode`：

- `loop / io / cpu / parse` → `protected_mode true`：Lua error 被 catch，pipeline 繼續
- `mem` → `protected_mode false`：OOM 不被攔截，process 真正被 kill

## 檔案結構

```
lua/
├── conf/
│   └── fluentbit_error_tests.conf   # Fluent Bit 設定檔
├── script/
│   └── run_crash.sh                 # 測試執行腳本
├── test_errors.lua                  # 5 種錯誤注入函式
└── README.md
```

## 相關工具

| 工具 | 路徑 | 說明 |
|---|---|---|
| loggen | `../../tools/loggen.py` | 產生測試 log，支援 `--error-rate` |
| sink server | `../../tools/sink_server.py` | HTTP sink，監聽 port 8080 |
