#include "parser_plugin.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// 模擬 cJSON 的解析邏輯（實務上請將 cJSON.c 編譯進去）
// 這裡假設你已經包含了 cJSON 庫來處理 JSON
#include "cJSON.h" 

// ---------------------------------------------------------
// 輔助函式：將 cJSON 物件轉化為 WIT 的 LogEntry 結構
// ---------------------------------------------------------
static bool fill_log_entry(cJSON *json, parser_plugin_log_entry_t *entry) {
    cJSON *ts = cJSON_GetObjectItemCaseSensitive(json, "ts");
    cJSON *level = cJSON_GetObjectItemCaseSensitive(json, "level");
    cJSON *msg = cJSON_GetObjectItemCaseSensitive(json, "msg");
    cJSON *att = cJSON_GetObjectItemCaseSensitive(json, "att");

    // 1. 處理 Timestamp
    parser_plugin_string_dup(&entry->timestamp, cJSON_IsString(ts) ? ts->valuestring : "");

    // 2. 處理 Level (Go 版本固定設為 0)
    entry->level = LOCAL_LOG_PROCESS_PARSE_PROCESS_LOG_LEVEL_DEBUG;

    // 3. 處理 Message
    parser_plugin_string_dup(&entry->message, cJSON_IsString(msg) ? msg->valuestring : "");

    // 4. 處理 Tags (對應 Go 的 map[string]string -> list<tuple<string, string>>)
    if (cJSON_IsObject(att)) {
        int count = cJSON_GetArraySize(att);
        entry->tags.len = count;
        entry->tags.ptr = (parser_plugin_tuple2_string_string_t*) malloc(count * sizeof(parser_plugin_tuple2_string_string_t));
        
        int i = 0;
        cJSON *child = NULL;
        cJSON_ArrayForEach(child, att) {
            parser_plugin_string_dup(&entry->tags.ptr[i].f0, child->string);       // Key
            parser_plugin_string_dup(&entry->tags.ptr[i].f1, cJSON_GetStringValue(child)); // Value
            i++;
        }
    } else {
        entry->tags.len = 0;
        entry->tags.ptr = NULL;
    }

    return true;
}

// ---------------------------------------------------------
// 實作導出函數: Parse
// ---------------------------------------------------------
bool exports_parser_plugin_parse(
    parser_plugin_list_list_u8_t *raw_data, 
    parser_plugin_list_log_entry_t *ret, 
    parser_plugin_parse_error_t *err
) {
    // 預先分配回傳列表的空間
    ret->len = raw_data->len;
    ret->ptr = (parser_plugin_log_entry_t*) calloc(ret->len, sizeof(parser_plugin_log_entry_t));

    for (size_t i = 0; i < raw_data->len; i++) {
        // 將 uint8 陣列視為字串進行解析 (需注意 null-terminated)
        char *json_str = (char*) malloc(raw_data->ptr[i].len + 1);
        memcpy(json_str, raw_data->ptr[i].ptr, raw_data->ptr[i].len);
        json_str[raw_data->ptr[i].len] = '\0';

        cJSON *json = cJSON_Parse(json_str);
        free(json_str);

        if (json == NULL) {
            // 解析失敗：設定錯誤訊息並返回 false (對應 Go 的 wit_types.Err)
            err->tag = LOCAL_LOG_PROCESS_PARSE_PROCESS_PARSE_ERROR_INVALID_FORMAT;
            parser_plugin_string_dup(&err->val.invalid_format, "Invalid JSON format");
            
            // 清理已分配的記憶體
            for (size_t j = 0; j < i; j++) {
                local_log_process_parse_process_log_entry_free(&ret->ptr[j]);
            }
            free(ret->ptr);
            parser_plugin_list_list_u8_free(raw_data);
            return false;
        }

        // 填充結構化數據
        fill_log_entry(json, &ret->ptr[i]);
        cJSON_Delete(json);
    }

    // 依照 WIT 規範：必須釋放輸入參數的所有權
    parser_plugin_list_list_u8_free(raw_data);

    return true; // 成功返回
}

// ---------------------------------------------------------
// 實作導出函數: ReportUsage
// ---------------------------------------------------------
#include <malloc.h>

uint64_t exports_parser_plugin_report_usage(void) {
    // 在 WASI 環境中，mallinfo 可提供目前的記憶體分配資訊
    // 對應 Go 的 runtime.ReadMemStats 中的 Alloc
    struct mallinfo mi = mallinfo();
    return (uint64_t)mi.uordblks; // 已經被 malloc 使用掉的總空間 (位元組)
}