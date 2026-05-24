#include "../parse/parser_plugin.h"
#include <stdint.h>
#define __USE_POSIX199309
#include <time.h>

static uint64_t g_last_exec_ns = 0;

static uint64_t now_ns(void) {
    struct timespec ts;

    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        return 0;
    }

    return ((uint64_t)ts.tv_sec * 1000000000ULL) + (uint64_t)ts.tv_nsec;
}

bool exports_parser_plugin_parse(parser_plugin_list_string_t *raw_data,
                                 parser_plugin_list_parsed_entry_t *ret,
                                 parser_plugin_parse_error_t *err) {
    (void)err;

    uint64_t start_ns = now_ns();
    volatile size_t input_len = raw_data->len;
    (void)input_len;

    ret->ptr = NULL;
    ret->len = 0;

    uint64_t end_ns = now_ns();
    g_last_exec_ns = end_ns >= start_ns ? end_ns - start_ns : 0;

    return true;
}

uint64_t exports_parser_plugin_report_usage(void) {
    return g_last_exec_ns;
}

void exports_parser_plugin_reset(void) {
    g_last_exec_ns = 0;
}
