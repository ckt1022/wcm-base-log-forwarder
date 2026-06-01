#include "parser_plugin.h"
#include "cJSON.h"
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#define __USE_POSIX199309
#include <time.h>

static uint64_t g_processed_records = 0;
static uint64_t g_last_exec_ns = 0;

static uint64_t now_ns(void) {
    struct timespec ts;

    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return 0;
    }

    return ((uint64_t)ts.tv_sec * 1000000000ULL) + (uint64_t)ts.tv_nsec;
}

/**
 * 輔助函數：將 cJSON 物件中的欄位轉換為 Tags 列表
 * 包含：1. 根目錄下的非保留欄位 2. "att" 物件內的所有欄位
 */
static void collect_all_tags(cJSON *root, parser_plugin_list_tuple2_string_string_t *ret_tags) {
    int other_tags = 0;
    cJSON *att_obj = cJSON_GetObjectItemCaseSensitive(root, "att");

    // 第一階段：計算除 lang 以外的標籤數
    cJSON *item = NULL;
    cJSON_ArrayForEach(item, root) {
        if (strcmp(item->string, "ts") == 0 ||
            strcmp(item->string, "level") == 0 ||
            strcmp(item->string, "msg") == 0 ||
            strcmp(item->string, "att") == 0) {
            continue;
        }
        other_tags++;
    }

    if (cJSON_IsObject(att_obj)) {
        other_tags += cJSON_GetArraySize(att_obj);
    }

    int total_tags = 1 + other_tags;  // +1 for lang=C
    ret_tags->len = total_tags;
    ret_tags->ptr = (parser_plugin_tuple2_string_string_t*)malloc(total_tags * sizeof(parser_plugin_tuple2_string_string_t));

    // 第一個 tag：lang=C（標識此批次由 C 插件解析）
    parser_plugin_string_dup(&ret_tags->ptr[0].f0, "lang");
    parser_plugin_string_dup(&ret_tags->ptr[0].f1, "C");

    int i = 1;
    // 第二階段：填充根目錄下的欄位
    cJSON_ArrayForEach(item, root) {
        if (strcmp(item->string, "ts") == 0 ||
            strcmp(item->string, "level") == 0 ||
            strcmp(item->string, "msg") == 0 ||
            strcmp(item->string, "att") == 0) {
            continue;
        }
        parser_plugin_string_dup(&ret_tags->ptr[i].f0, item->string);
        parser_plugin_string_dup(&ret_tags->ptr[i].f1, cJSON_IsString(item) ? item->valuestring : cJSON_PrintUnformatted(item));
        i++;
    }

    // 第三階段：填充 att 內的欄位
    if (cJSON_IsObject(att_obj)) {
        cJSON *att_item = NULL;
        cJSON_ArrayForEach(att_item, att_obj) {
            parser_plugin_string_dup(&ret_tags->ptr[i].f0, att_item->string);
            parser_plugin_string_dup(&ret_tags->ptr[i].f1, cJSON_IsString(att_item) ? att_item->valuestring : cJSON_PrintUnformatted(att_item));
            i++;
        }
    }
}

/**
 * 根據 log level 回傳 route 標籤（同 Go plugin 的 routeTag 邏輯）
 */
static const char* route_tag_c(uint8_t level) {
    switch (level) {
    case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ERROR:
    case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_CRIT:
    case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ALERT:
    case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_EMERG:
        return "AB";
    case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN:
        return "BC";
    default:
        return "C";
    }
}

/**
 * 映射字串 Log Level 到系統定義的數值
 */
static uint8_t map_level(const char *level_str) {
    if (level_str == NULL) return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_INFO;
    if (strcasecmp(level_str, "debug") == 0) return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_DEBUG;
    if (strcasecmp(level_str, "info") == 0)  return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_INFO;
    if (strcasecmp(level_str, "warn") == 0)  return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN;
    if (strcasecmp(level_str, "error") == 0) return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ERROR;
    return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_INFO;
}

bool exports_parser_plugin_parse(parser_plugin_list_string_t *raw_data, 
                                 parser_plugin_list_parsed_entry_t *ret, 
                                 parser_plugin_parse_error_t *err) {
    uint64_t start_ns = now_ns();
    
    size_t num_elements = raw_data->len;
    ret->ptr = (parser_plugin_parsed_entry_t*)calloc(num_elements, sizeof(parser_plugin_parsed_entry_t));
    ret->len = num_elements;

    for (size_t i = 0; i < num_elements; i++) {
        cJSON *root = cJSON_ParseWithLength((const char *)raw_data->ptr[i].ptr, raw_data->ptr[i].len);

        if (root == NULL) {
            err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PARSE_ERROR_INVALID_FORMAT;
            parser_plugin_string_dup(&err->val.invalid_format, "JSON Parse Error");
            uint64_t end_ns = now_ns();
            g_last_exec_ns = end_ns >= start_ns ? end_ns - start_ns : 0;
            return false;
        }

        // 1. 解析 ts (timestamp)
        cJSON *ts = cJSON_GetObjectItemCaseSensitive(root, "ts");
        parser_plugin_string_dup(&ret->ptr[i].timestamp, cJSON_IsString(ts) ? ts->valuestring : "");

        // 2. 解析 level (處理字串轉數值)
        cJSON *lv = cJSON_GetObjectItemCaseSensitive(root, "level");
        if (cJSON_IsNumber(lv)) {
            ret->ptr[i].level = (uint8_t)lv->valueint;
        } else {
            ret->ptr[i].level = map_level(cJSON_IsString(lv) ? lv->valuestring : NULL);
        }

        // 3. 解析 msg (message)
        cJSON *msg = cJSON_GetObjectItemCaseSensitive(root, "msg");
        parser_plugin_string_dup(&ret->ptr[i].message, cJSON_IsString(msg) ? msg->valuestring : "");

        // 4. 自動收集所有標籤 (含根目錄非保留欄位與 att 物件)
        collect_all_tags(root, &ret->ptr[i].tags);

        // 5. 設定 route 標籤
        parser_plugin_string_dup(&ret->ptr[i].targettag, route_tag_c(ret->ptr[i].level));

        cJSON_Delete(root);
        g_processed_records++;
    }

    uint64_t end_ns = now_ns();
    g_last_exec_ns = end_ns >= start_ns ? end_ns - start_ns : 0;

    return true;
}

uint64_t exports_parser_plugin_report_usage(void) {
    return g_last_exec_ns;
}

void exports_parser_plugin_reset(void) {
    g_processed_records = 0;
    g_last_exec_ns = 0;
}
