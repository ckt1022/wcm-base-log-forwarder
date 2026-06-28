#include "parser_plugin.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/*
 * parse_bad – WCM 「未授權 import」示範插件
 *
 * 這個插件在 WIT 中宣告了 `local:log-process/forbidden-capability`，
 * 這是一個完全虛構的介面，任何合法的 host linker 都不會提供它。
 *
 * 預期行為：
 *   - 編譯與打包成 WASM component：成功
 *   - host 嘗試實例化（ParsePool::create_one）：立即失敗
 *   - runtime.rs replenish() 捕獲錯誤並印出 [pool] 錯誤訊息
 *   - WIT 層的能力阻擋在這裡變得「可見」，不同於 POSIX 層的靜默失敗
 *
 * 若 host 意外成功載入此插件，代表 linker 安全設定有漏洞。
 */

bool exports_parser_plugin_parse(parser_plugin_list_string_t *raw_data,
                                  parser_plugin_list_parsed_entry_t *ret,
                                  parser_plugin_parse_error_t *err)
{
    (void)err;

    /*
     * 呼叫自定義介面的函式。
     * 這些呼叫在實際執行中永遠不會到達，因為插件在實例化時就已失敗。
     * 但它們必須存在於程式碼中，才能讓 WASM linker 將
     * forbidden-capability 的 import 嵌入最終的 component binary 中。
     */
    parser_plugin_list_string_t path;
    parser_plugin_string_t path_str;
    parser_plugin_string_dup(&path_str, "/etc");
    path.ptr = &path_str;
    path.len = 1;

    parser_plugin_list_string_t files;
    local_log_process_forbidden_capability_probe_host_files(&path_str, &files);
    parser_plugin_list_string_free(&files);

    bool connected = local_log_process_forbidden_capability_connect_to_host(
        &path_str, 8080);

    parser_plugin_string_free(&path_str);
    (void)connected;

    /* 以下程式碼理論上永遠不執行 */
    ret->len = 1;
    ret->ptr = (parser_plugin_parsed_entry_t *)calloc(
        1, sizeof(parser_plugin_parsed_entry_t));
    parser_plugin_string_dup(&ret->ptr[0].timestamp, "");
    ret->ptr[0].level = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ERROR;
    parser_plugin_string_dup(&ret->ptr[0].message,
        "[BUG] parse_bad was instantiated — host linker security check failed!");
    ret->ptr[0].tags.len = 0;
    ret->ptr[0].tags.ptr = NULL;
    parser_plugin_string_dup(&ret->ptr[0].targettag, "C");
    return true;
}

uint64_t exports_parser_plugin_report_usage(void)
{
    return 0;
}
