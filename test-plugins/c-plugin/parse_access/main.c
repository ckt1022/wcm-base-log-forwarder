#include "parser_plugin.h"
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

/*
 * parse_access – WCM runtime-level blocking demo
 *
 * WIT world: filesystem/types+preopens 與 cli/environment 均有宣告（與正式 parse 相同）。
 * 關鍵：host 的 WasiCtxBuilder 沒有注入任何 preopened dir 或環境變數。
 *
 * 這證明了：WIT 中宣告能力 ≠ 真的能存取資源。
 * 實際能存取什麼，完全由 host 在建立 WasiCtx 時決定。
 *
 *  [A] fopen() – POSIX 層嘗試檔案存取
 *      WIT 有 filesystem 能力，但 host 沒有給 preopened dir。
 *      libc 在 __wasilibc_find_relpath() 短路，回傳 ENOTCAPABLE。
 *      host 完全看不到這次存取嘗試（WASI p2 層從未被呼叫）。
 *
 *  [B] wasi_cli_environment_get_environment() – 直接 WASI p2 呼叫
 *      WIT 有 cli/environment 能力，但 host 沒有呼叫 inherit_env()。
 *      直接呼叫 WASI p2 → 到達 wasmtime 實作 → 回傳空列表。
 *      與 fopen() 不同，此呼叫 host 可以攔截（可見）。
 */

static uint64_t g_last_exec_ns = 0;

/*
 * Test 1 – filesystem access via standard POSIX fopen().
 * Translates through wasip1 path_open → adapter → WasiCtx.
 * Without preopened dirs the libc returns ENOTCAPABLE immediately.
 */
static const char *try_filesystem_access(void)
{
    errno = 0;
    FILE *f = fopen("wcm_capability_probe.txt", "r");
    if (f) {
        fclose(f);
        return "fs:ALLOWED!(unexpected)";
    }
    switch (errno) {
    case 76:      return "fs:ENOTCAPABLE(blocked-no-preopens)";
    case ENOENT:  return "fs:ENOENT(blocked-no-preopens)";
    case EACCES:  return "fs:EACCES(denied)";
    default: {
        static char buf[48];
        snprintf(buf, sizeof(buf), "fs:errno=%d(blocked)", errno);
        return buf;
    }
    }
}

/*
 * Test 2 – 直接呼叫 WASI p2 讀取環境變數
 *
 * WIT 中有宣告 wasi:cli/environment，所以 wit-bindgen 生成了
 * wasi_cli_environment_get_environment() 的 C binding。
 *
 * 此呼叫 直接到達 wasmtime 的 WASI p2 實作（不像 fopen 被 libc 短路）。
 * 因為 WasiCtxBuilder 沒有呼叫 inherit_env()，回傳空列表。
 *
 * 與 Test A（fopen）的核心差異：
 *   Test A：WASI 層完全不可見 → host 無法攔截
 *   Test B：WASI 層可見       → host 可以替換實作並攔截記錄
 */
static const char *try_direct_p2_env_call(void)
{
    parser_plugin_list_tuple2_string_string_t env_list;
    wasi_cli_environment_get_environment(&env_list);

    size_t count = env_list.len;
    parser_plugin_list_tuple2_string_string_free(&env_list);

    if (count == 0) {
        return "p2-env:empty-list(WasiCtxBuilder-no-inherit_env=blocked)";
    }

    static char buf[64];
    snprintf(buf, sizeof(buf), "p2-env:LEAK!count=%zu(host-misconfigured)", count);
    return buf;
}

bool exports_parser_plugin_parse(parser_plugin_list_string_t *raw_data,
                                  parser_plugin_list_parsed_entry_t *ret,
                                  parser_plugin_parse_error_t *err)
{
    (void)err;

    // [A] WIT 有 filesystem 能力，但 host 未提供 preopened dir → POSIX 短路失敗
    const char *fs_result  = try_filesystem_access();

    // [B] WIT 有 environment 能力，但 host 未注入環境變數 → p2 直接呼叫回傳空列表
    const char *env_result = try_direct_p2_env_call();

    ret->len = 1;
    ret->ptr = (parser_plugin_parsed_entry_t *)calloc(
        1, sizeof(parser_plugin_parsed_entry_t));

    parser_plugin_string_dup(&ret->ptr[0].timestamp, "");
    ret->ptr[0].level = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN;

    char msg[480];
    snprintf(msg, sizeof(msg),
             "[WCM-CAP-TEST] input=%zu | %s | %s",
             raw_data->len, fs_result, env_result);
    parser_plugin_string_dup(&ret->ptr[0].message, msg);

    ret->ptr[0].tags.len = 2;
    ret->ptr[0].tags.ptr = (parser_plugin_tuple2_string_string_t *)malloc(
        2 * sizeof(parser_plugin_tuple2_string_string_t));
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[0].f0, "A_fs_access_posix");
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[0].f1, fs_result);
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[1].f0, "B_p2_env_direct");
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[1].f1, env_result);

    parser_plugin_string_dup(&ret->ptr[0].targettag, "C");

    return true;
}

uint64_t exports_parser_plugin_report_usage(void)
{
    return g_last_exec_ns;
}
