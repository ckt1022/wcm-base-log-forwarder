#include "parser_plugin.h"
#include "cJSON.h"
#include <stdlib.h>
#include <string.h>
#include <strings.h>
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

static void free_plugin_string(parser_plugin_string_t *s) {
    if (s == NULL) return;

    free(s->ptr);
    s->ptr = NULL;
    s->len = 0;
}

static void free_tags(parser_plugin_list_tuple2_string_string_t *tags) {
    if (tags == NULL || tags->ptr == NULL) return;

    for (size_t i = 0; i < tags->len; i++) {
        free_plugin_string(&tags->ptr[i].f0);
        free_plugin_string(&tags->ptr[i].f1);
    }

    free(tags->ptr);
    tags->ptr = NULL;
    tags->len = 0;
}

static void free_parsed_entries(parser_plugin_parsed_entry_t *entries, size_t len) {
    if (entries == NULL) return;

    for (size_t i = 0; i < len; i++) {
        free_plugin_string(&entries[i].timestamp);
        free_plugin_string(&entries[i].message);
        free_tags(&entries[i].tags);
    }

    free(entries);
}

static void dup_json_value(parser_plugin_string_t *out, cJSON *item) {
    if (cJSON_IsString(item)) {
        parser_plugin_string_dup(out, item->valuestring);
        return;
    }

    char *printed = cJSON_PrintUnformatted(item);
    parser_plugin_string_dup(out, printed != NULL ? printed : "");
    free(printed);
}

static int is_reserved_key(const char *key) {
    if (key == NULL) return 0;

    return strcmp(key, "ts") == 0 ||
           strcmp(key, "level") == 0 ||
           strcmp(key, "msg") == 0 ||
           strcmp(key, "att") == 0;
}

static void collect_all_tags(cJSON *root, parser_plugin_list_tuple2_string_string_t *ret_tags) {
    int total_tags = 0;
    cJSON *att_obj = cJSON_GetObjectItemCaseSensitive(root, "att");

    cJSON *item = NULL;
    cJSON_ArrayForEach(item, root) {
        if (is_reserved_key(item->string)) {
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

    ret_tags->ptr = (parser_plugin_tuple2_string_string_t *)calloc(
        total_tags,
        sizeof(parser_plugin_tuple2_string_string_t)
    );

    if (ret_tags->ptr == NULL) {
        ret_tags->len = 0;
        return;
    }

    int i = 0;

    cJSON_ArrayForEach(item, root) {
        if (is_reserved_key(item->string)) {
            continue;
        }

        parser_plugin_string_dup(&ret_tags->ptr[i].f0, item->string);
        dup_json_value(&ret_tags->ptr[i].f1, item);
        i++;
    }

    if (cJSON_IsObject(att_obj)) {
        cJSON *att_item = NULL;
        cJSON_ArrayForEach(att_item, att_obj) {
            parser_plugin_string_dup(&ret_tags->ptr[i].f0, att_item->string);
            dup_json_value(&ret_tags->ptr[i].f1, att_item);
            i++;
        }
    }
}

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

    ret->ptr = (parser_plugin_parsed_entry_t *)calloc(
        num_elements,
        sizeof(parser_plugin_parsed_entry_t)
    );
    ret->len = num_elements;

    if (ret->ptr == NULL && num_elements > 0) {
        err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PARSE_ERROR_INVALID_FORMAT;
        parser_plugin_string_dup(&err->val.invalid_format, "Out of memory");

        uint64_t end_ns = now_ns();
        g_last_exec_ns = end_ns >= start_ns ? end_ns - start_ns : 0;
        ret->len = 0;
        return false;
    }

    for (size_t i = 0; i < num_elements; i++) {
        cJSON *root = cJSON_ParseWithLength(
            (const char *)raw_data->ptr[i].ptr,
            raw_data->ptr[i].len
        );

        if (root == NULL) {
            free_parsed_entries(ret->ptr, i);
            ret->ptr = NULL;
            ret->len = 0;

            err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PARSE_ERROR_INVALID_FORMAT;
            parser_plugin_string_dup(&err->val.invalid_format, "JSON Parse Error");

            uint64_t end_ns = now_ns();
            g_last_exec_ns = end_ns >= start_ns ? end_ns - start_ns : 0;
            return false;
        }

        cJSON *ts = cJSON_GetObjectItemCaseSensitive(root, "ts");
        parser_plugin_string_dup(
            &ret->ptr[i].timestamp,
            cJSON_IsString(ts) ? ts->valuestring : ""
        );

        cJSON *lv = cJSON_GetObjectItemCaseSensitive(root, "level");
        if (cJSON_IsNumber(lv)) {
            ret->ptr[i].level = (uint8_t)lv->valueint;
        } else {
            ret->ptr[i].level = map_level(cJSON_IsString(lv) ? lv->valuestring : NULL);
        }

        cJSON *msg = cJSON_GetObjectItemCaseSensitive(root, "msg");
        parser_plugin_string_dup(
            &ret->ptr[i].message,
            cJSON_IsString(msg) ? msg->valuestring : ""
        );

        collect_all_tags(root, &ret->ptr[i].tags);

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
