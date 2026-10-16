// filter_impl.c — level-based reduction-plugin
//
// 邏輯對應 csharp-plugin/filter/Filters.cs 的 LevelFilter：
// 保留 level >= Warn 的 log（丟棄 debug / info），純常數版本、不讀 runtime config。

#include "reduction_plugin.h"
#include <stdlib.h>
#include <time.h>

#define MIN_KEEP_LEVEL LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN

static uint64_t g_last_exec_ns = 0;

static uint64_t now_ns(void) {
    struct timespec ts;

    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return 0;
    }

    return ((uint64_t)ts.tv_sec * 1000000000ULL) + (uint64_t)ts.tv_nsec;
}

bool exports_reduction_plugin_filter(reduction_plugin_list_log_entry_t *struct_data,
                                      reduction_plugin_list_filter_result_t *ret,
                                      reduction_plugin_plugin_error_t *err) {
    (void)err;
    uint64_t start_ns = now_ns();

    size_t num_elements = struct_data->len;
    ret->ptr = (reduction_plugin_filter_result_t*)malloc(num_elements * sizeof(reduction_plugin_filter_result_t));
    ret->len = num_elements;

    for (size_t i = 0; i < num_elements; i++) {
        ret->ptr[i].id   = struct_data->ptr[i].id;
        ret->ptr[i].keep = struct_data->ptr[i].level >= MIN_KEEP_LEVEL;
    }

    uint64_t end_ns = now_ns();
    g_last_exec_ns = end_ns >= start_ns ? end_ns - start_ns : 0;

    return true;
}

uint64_t exports_reduction_plugin_report_usage(void) {
    return g_last_exec_ns;
}
