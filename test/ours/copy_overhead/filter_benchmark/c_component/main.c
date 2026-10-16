#include "reduction_plugin.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define MIN_KEEP_LEVEL LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN

bool exports_reduction_plugin_filter(reduction_plugin_list_log_entry_t *struct_data,
                                     reduction_plugin_list_filter_result_t *ret,
                                     reduction_plugin_plugin_error_t *err) {
    /* H1: input LogEntry lowering into C memory has completed. */
    reduction_plugin_time_sign();

    ret->len = struct_data->len;
    ret->ptr = calloc(ret->len, sizeof(*ret->ptr));
    if (ret->len != 0 && ret->ptr == NULL) {
        err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PLUGIN_ERROR_INTERNAL_ERROR;
        reduction_plugin_time_sign();
        return false;
    }

    for (size_t index = 0; index < struct_data->len; index++) {
        ret->ptr[index].id = struct_data->ptr[index].id;
        ret->ptr[index].keep = struct_data->ptr[index].level >= MIN_KEEP_LEVEL;
    }

    /* H2: all decisions and FilterResult allocations are complete. */
    reduction_plugin_time_sign();
    return true;
}

uint64_t exports_reduction_plugin_report_usage(void) {
    return 0;
}
