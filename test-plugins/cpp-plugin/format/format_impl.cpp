// C++ implementation of the format plugin
// Output format: timestamp LEVEL message key=value key=value ...
// Matches the TinyGo format plugin behaviour exactly.

#include "format_plugin.h"
#include <cstdlib>
#include <cstring>

// ─────────────────────────────────────────────────────────────────
// Helper: map log-level enum → uppercase ASCII string
// ─────────────────────────────────────────────────────────────────
static const char* level_str(local_log_process_pipeline_process_log_level_t level) {
    switch (level) {
        case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_DEBUG: return "DEBUG";
        case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_INFO:  return "INFO";
        case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_WARN:  return "WARN";
        case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ERROR: return "ERROR";
        case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_CRIT:  return "CRIT";
        case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_ALERT: return "ALERT";
        case LOCAL_LOG_PROCESS_PIPELINE_PROCESS_LOG_LEVEL_EMERG: return "EMERG";
        default:                                                  return "UNKNOWN";
    }
}

// ─────────────────────────────────────────────────────────────────
// Helper: append bytes to a growable buffer
// ─────────────────────────────────────────────────────────────────
struct Buf {
    uint8_t* data;
    size_t   len;
    size_t   cap;

    Buf() : data(nullptr), len(0), cap(0) {}

    void ensure(size_t extra) {
        if (len + extra <= cap) return;
        size_t new_cap = cap == 0 ? 64 : cap * 2;
        while (new_cap < len + extra) new_cap *= 2;
        data = static_cast<uint8_t*>(realloc(data, new_cap));
        cap  = new_cap;
    }

    void push_str(const char* s, size_t n) {
        ensure(n);
        memcpy(data + len, s, n);
        len += n;
    }

    void push_str(const format_plugin_string_t& s) {
        push_str(reinterpret_cast<const char*>(s.ptr), s.len);
    }

    void push_char(char c) {
        ensure(1);
        data[len++] = static_cast<uint8_t>(c);
    }

    // Transfer ownership to a format_plugin_list_u8_t; caller must not free
    // this Buf afterwards (ownership moves to the returned struct).
    format_plugin_list_u8_t take() {
        format_plugin_list_u8_t out;
        // Trim to exact length (optional, saves a tiny bit of memory).
        data = static_cast<uint8_t*>(realloc(data, len == 0 ? 1 : len));
        out.ptr = data;
        out.len = len;
        // Prevent double-free by clearing our own pointer.
        data = nullptr;
        len  = 0;
        cap  = 0;
        return out;
    }

    ~Buf() { free(data); }
};

// ─────────────────────────────────────────────────────────────────
// Exported: format
// ─────────────────────────────────────────────────────────────────
extern "C" bool exports_format_plugin_format(
    format_plugin_list_log_entry_t* struct_data,
    format_plugin_list_list_u8_t*   ret,
    format_plugin_plugin_error_t*   /* err */
) {
    size_t n = struct_data->len;

    // Allocate output array of list<u8>
    ret->len = n;
    ret->ptr = static_cast<format_plugin_list_u8_t*>(
        malloc(n * sizeof(format_plugin_list_u8_t))
    );
    if (ret->ptr == nullptr && n > 0) {
        ret->len = 0;
        return false;
    }

    for (size_t i = 0; i < n; i++) {
        format_plugin_log_entry_t& e = struct_data->ptr[i];
        Buf buf;

        // timestamp
        buf.push_str(e.timestamp);
        buf.push_char(' ');

        // LEVEL
        const char* lvl = level_str(e.level);
        buf.push_str(lvl, strlen(lvl));
        buf.push_char(' ');

        // message
        buf.push_str(e.message);

        // key=value tags
        for (size_t t = 0; t < e.tags.len; t++) {
            buf.push_char(' ');
            buf.push_str(e.tags.ptr[t].f0);   // key
            buf.push_char('=');
            buf.push_str(e.tags.ptr[t].f1);   // value
        }

        ret->ptr[i] = buf.take();
    }

    return true;
}

// ─────────────────────────────────────────────────────────────────
// Exported: report-usage
// Returns 0 – format plugin does not track heap usage separately.
// ─────────────────────────────────────────────────────────────────
extern "C" uint64_t exports_format_plugin_report_usage(void) {
    return 0;
}
