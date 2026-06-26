#include "parser_plugin.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/*
 * parse_access plugin – WCM IO capability access test
 *
 * 目的：在沒有授予 IO 權限的情況下，呼叫 WASI filesystem 和 socket API，
 * 驗證 WCM 能力模型是否能阻擋未授權的外部 IO 存取。
 *
 * 測試項目：
 *   1. Filesystem – 取得 preopens 並嘗試開啟/建立檔案
 *   2. Network    – 嘗試建立 TCP socket
 *
 * 每次 parse() 呼叫都會回傳一個 parsed-entry，其 tags 記錄了每項測試的結果。
 * 若 WCM 阻擋了存取，相關 API 會回傳錯誤碼（not-permitted / access-denied）；
 * 若未阻擋，結果會顯示 "allowed"，表示存取成功。
 */

static uint64_t g_last_exec_ns = 0;

/* 測試 1: 嘗試透過 WASI filesystem preopens 存取檔案系統 */
static const char *try_filesystem_access(void) {
    wasi_filesystem_preopens_list_tuple2_own_descriptor_string_t dirs;
    wasi_filesystem_preopens_get_directories(&dirs);

    if (dirs.len == 0) {
        /* WCM 沒有給予任何 preopen 目錄 → 無法存取 filesystem */
        wasi_filesystem_preopens_list_tuple2_own_descriptor_string_free(&dirs);
        return "fs:no-preopens(blocked)";
    }

    /* 取得 preopens 成功：嘗試在第一個目錄下建立/開啟檔案 */
    wasi_filesystem_types_borrow_descriptor_t root_borrow =
        wasi_filesystem_types_borrow_descriptor(dirs.ptr[0].f0);

    parser_plugin_string_t path;
    parser_plugin_string_dup(&path, "access_test_probe.txt");

    wasi_filesystem_types_own_descriptor_t file_fd;
    wasi_filesystem_types_error_code_t fs_err;

    bool opened = wasi_filesystem_types_method_descriptor_open_at(
        root_borrow,
        WASI_FILESYSTEM_TYPES_PATH_FLAGS_SYMLINK_FOLLOW,
        &path,
        WASI_FILESYSTEM_TYPES_OPEN_FLAGS_CREATE,
        WASI_FILESYSTEM_TYPES_DESCRIPTOR_FLAGS_WRITE,
        &file_fd,
        &fs_err
    );

    parser_plugin_string_free(&path);
    wasi_filesystem_preopens_list_tuple2_own_descriptor_string_free(&dirs);

    if (opened) {
        wasi_filesystem_types_descriptor_drop_own(file_fd);
        return "fs:write-open-ok(allowed)";
    }

    /* 有 preopen 但開檔被拒 */
    switch (fs_err) {
        case WASI_FILESYSTEM_TYPES_ERROR_CODE_NOT_PERMITTED: return "fs:open-not-permitted";
        case WASI_FILESYSTEM_TYPES_ERROR_CODE_ACCESS:        return "fs:open-access-denied";
        case WASI_FILESYSTEM_TYPES_ERROR_CODE_READ_ONLY:     return "fs:open-read-only";
        default:                                              return "fs:open-error(other)";
    }
}

/* 測試 2: 嘗試建立 TCP socket */
static const char *try_network_access(void) {
    wasi_sockets_tcp_create_socket_own_tcp_socket_t sock;
    wasi_sockets_tcp_create_socket_error_code_t net_err;

    bool created = wasi_sockets_tcp_create_socket_create_tcp_socket(
        WASI_SOCKETS_NETWORK_IP_ADDRESS_FAMILY_IPV4,
        &sock,
        &net_err
    );

    if (created) {
        wasi_sockets_tcp_tcp_socket_drop_own(sock);
        return "net:tcp-socket-ok(allowed)";
    }

    switch (net_err) {
        case WASI_SOCKETS_NETWORK_ERROR_CODE_ACCESS_DENIED:   return "net:tcp-access-denied(blocked)";
        case WASI_SOCKETS_NETWORK_ERROR_CODE_NOT_SUPPORTED:   return "net:tcp-not-supported";
        default:                                               return "net:tcp-error(other)";
    }
}

bool exports_parser_plugin_parse(parser_plugin_list_string_t *raw_data,
                                  parser_plugin_list_parsed_entry_t *ret,
                                  parser_plugin_parse_error_t *err) {
    (void)err;

    const char *fs_result  = try_filesystem_access();
    const char *net_result = try_network_access();

    /* 回傳一個 parsed entry，記錄此次 IO 存取測試的結果 */
    ret->len = 1;
    ret->ptr = (parser_plugin_parsed_entry_t *)calloc(1, sizeof(parser_plugin_parsed_entry_t));

    parser_plugin_string_dup(&ret->ptr[0].timestamp, "");
    ret->ptr[0].level = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN;

    char msg[512];
    snprintf(msg, sizeof(msg),
             "[WCM-IO-TEST] input=%zu | %s | %s",
             raw_data->len, fs_result, net_result);
    parser_plugin_string_dup(&ret->ptr[0].message, msg);

    /* Tags: fs_access 與 net_access 分別記錄兩項測試結果 */
    ret->ptr[0].tags.len = 2;
    ret->ptr[0].tags.ptr = (parser_plugin_tuple2_string_string_t *)malloc(
        2 * sizeof(parser_plugin_tuple2_string_string_t)
    );
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[0].f0, "fs_access");
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[0].f1, fs_result);
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[1].f0, "net_access");
    parser_plugin_string_dup(&ret->ptr[0].tags.ptr[1].f1, net_result);

    parser_plugin_string_dup(&ret->ptr[0].targettag, "C");

    return true;
}

uint64_t exports_parser_plugin_report_usage(void) {
    return g_last_exec_ns;
}
