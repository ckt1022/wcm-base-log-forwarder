#include "parser_plugin.h"
#include "cJSON.h"
#include <stdlib.h>
#include <string.h>

static uint64_t g_processed_records = 0;

/**
 * 輔助函數：將 cJSON 物件中的欄位轉換為 Tags 列表
 * 包含：1. 根目錄下的非保留欄位 2. "att" 物件內的所有欄位
 */
static void collect_all_tags(cJSON *root, parser_plugin_list_tuple2_string_string_t *ret_tags) {
    int total_tags = 0;
    cJSON *att_obj = cJSON_GetObjectItemCaseSensitive(root, "att");

    // 第一階段：計算總標籤數
    cJSON *item = NULL;
    cJSON_ArrayForEach(item, root) {
        // 跳過保留鍵與 att 物件本身
        if (strcmp(item->string, "ts") == 0 || 
            strcmp(item->string, "level") == 0 || 
            strcmp(item->string, "msg") == 0 ||
            strcmp(item->string, "att") == 0) {
            continue;
        }
        total_tags++;
    }

    if (cJSON_IsObject(att_obj)) {
        total_tags += cJSON_GetArraySize(att_obj);
    }

    ret_tags->len = total_tags;
    if (total_tags == 0) {
        ret_tags->ptr = NULL;
        return;
    }

    ret_tags->ptr = (parser_plugin_tuple2_string_string_t*)malloc(total_tags * sizeof(parser_plugin_tuple2_string_string_t));

    int i = 0;
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
    
    size_t num_elements = raw_data->len;
    ret->ptr = (parser_plugin_parsed_entry_t*)calloc(num_elements, sizeof(parser_plugin_parsed_entry_t));
    ret->len = num_elements;

    for (size_t i = 0; i < num_elements; i++) {
        cJSON *root = cJSON_ParseWithLength((const char *)raw_data->ptr[i].ptr, raw_data->ptr[i].len);

        if (root == NULL) {
            err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PARSE_ERROR_INVALID_FORMAT;
            parser_plugin_string_dup(&err->val.invalid_format, "JSON Parse Error");
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

        cJSON_Delete(root);
        g_processed_records++;
    }

    return true;
}

uint64_t exports_parser_plugin_report_usage(void) {
    return g_processed_records;
}