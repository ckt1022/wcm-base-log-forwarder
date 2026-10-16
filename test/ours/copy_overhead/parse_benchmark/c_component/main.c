#include "parser_plugin.h"
#include "cJSON.h"

#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

static void duplicate_json_value(parser_plugin_string_t *target, const cJSON *value) {
    if (cJSON_IsString(value) && value->valuestring != NULL) {
        parser_plugin_string_dup(target, value->valuestring);
        return;
    }

    char *printed = cJSON_PrintUnformatted(value);
    if (printed == NULL) {
        parser_plugin_string_dup(target, "");
        return;
    }
    parser_plugin_string_dup(target, printed);
    cJSON_free(printed);
}

static uint8_t map_level(const cJSON *value) {
    if (cJSON_IsNumber(value)) {
        int level = value->valueint;
        if (level >= LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_DEBUG &&
            level <= LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_EMERG) {
            return (uint8_t)level;
        }
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_INFO;
    }

    if (!cJSON_IsString(value) || value->valuestring == NULL) {
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_INFO;
    }
    const char *level = value->valuestring;
    if (strcasecmp(level, "debug") == 0) {
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_DEBUG;
    }
    if (strcasecmp(level, "warn") == 0) {
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN;
    }
    if (strcasecmp(level, "error") == 0) {
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ERROR;
    }
    if (strcasecmp(level, "crit") == 0) {
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_CRIT;
    }
    if (strcasecmp(level, "alert") == 0) {
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ALERT;
    }
    if (strcasecmp(level, "emerg") == 0) {
        return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_EMERG;
    }
    return LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_INFO;
}

static size_t count_tags(const cJSON *root) {
    size_t count = 1; /* lang=Rust, retained for byte-for-byte Rust parity. */
    const cJSON *item = NULL;
    cJSON_ArrayForEach(item, root) {
        if (item->string == NULL || strcmp(item->string, "ts") == 0 ||
            strcmp(item->string, "level") == 0 || strcmp(item->string, "msg") == 0 ||
            strcmp(item->string, "att") == 0) {
            continue;
        }
        count++;
    }

    const cJSON *attributes = cJSON_GetObjectItemCaseSensitive(root, "att");
    if (cJSON_IsObject(attributes)) {
        count += (size_t)cJSON_GetArraySize(attributes);
    }
    return count;
}

static void collect_tags(const cJSON *root, parser_plugin_list_tuple2_string_string_t *tags) {
    tags->len = count_tags(root);
    tags->ptr = calloc(tags->len, sizeof(*tags->ptr));

    size_t index = 0;
    parser_plugin_string_dup(&tags->ptr[index].f0, "lang");
    parser_plugin_string_dup(&tags->ptr[index].f1, "Rust");
    index++;

    const cJSON *item = NULL;
    cJSON_ArrayForEach(item, root) {
        if (item->string == NULL || strcmp(item->string, "ts") == 0 ||
            strcmp(item->string, "level") == 0 || strcmp(item->string, "msg") == 0 ||
            strcmp(item->string, "att") == 0) {
            continue;
        }
        parser_plugin_string_dup(&tags->ptr[index].f0, item->string);
        duplicate_json_value(&tags->ptr[index].f1, item);
        index++;
    }

    const cJSON *attributes = cJSON_GetObjectItemCaseSensitive(root, "att");
    if (cJSON_IsObject(attributes)) {
        const cJSON *attribute = NULL;
        cJSON_ArrayForEach(attribute, attributes) {
            parser_plugin_string_dup(&tags->ptr[index].f0, attribute->string);
            duplicate_json_value(&tags->ptr[index].f1, attribute);
            index++;
        }
    }
}

bool exports_parser_plugin_parse(parser_plugin_list_string_t *raw_data,
                                 parser_plugin_list_parsed_entry_t *ret,
                                 parser_plugin_parse_error_t *err) {
    /* H1: input list/string lowering into C has completed. */
    parser_plugin_time_sign();

    ret->len = raw_data->len;
    ret->ptr = calloc(ret->len, sizeof(*ret->ptr));
    if (ret->len != 0 && ret->ptr == NULL) {
        err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PARSE_ERROR_CORRUPTED_DATA;
        parser_plugin_time_sign();
        return false;
    }

    for (size_t index = 0; index < raw_data->len; index++) {
        cJSON *root = cJSON_ParseWithLength((const char *)raw_data->ptr[index].ptr,
                                            raw_data->ptr[index].len);
        if (root == NULL) {
            parser_plugin_list_parsed_entry_free(ret);
            ret->ptr = NULL;
            ret->len = 0;
            err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PARSE_ERROR_INVALID_FORMAT;
            parser_plugin_string_dup(&err->val.invalid_format, "JSON Parse Error");
            parser_plugin_time_sign();
            return false;
        }

        parser_plugin_parsed_entry_t *entry = &ret->ptr[index];
        const cJSON *timestamp = cJSON_GetObjectItemCaseSensitive(root, "ts");
        const cJSON *level = cJSON_GetObjectItemCaseSensitive(root, "level");
        const cJSON *message = cJSON_GetObjectItemCaseSensitive(root, "msg");

        parser_plugin_string_dup(&entry->timestamp,
                                 cJSON_IsString(timestamp) ? timestamp->valuestring : "");
        entry->level = map_level(level);
        parser_plugin_string_dup(&entry->message,
                                 cJSON_IsString(message) ? message->valuestring : "");
        collect_tags(root, &entry->tags);
        parser_plugin_string_dup(&entry->targettag, "A");
        cJSON_Delete(root);
    }

    /* H2: JSON parse and ParsedEntry construction have completed. */
    parser_plugin_time_sign();
    return true;
}

uint64_t exports_parser_plugin_report_usage(void) {
    return 0;
}
