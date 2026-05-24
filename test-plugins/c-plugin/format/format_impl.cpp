// format_impl.cpp — compile-time selectable log encoding
//
// All encodings share the same wire framing:
//   [4-byte LE length][entry bytes] repeated per entry
// This lets the host decode entries uniformly regardless of encoding.
//
// Select encoding via -D flag (only one):
//   -DFORMAT_JSON_FLAT       flat JSON object per entry (default)
//   -DFORMAT_JSON_NESTED     JSON object with tags under "attributes"
//   -DFORMAT_SYSLOG_RFC5424  RFC 5424 syslog line (includes trailing \n)
//   -DFORMAT_PROTOBUF        protobuf binary per entry
//
// Usage: make ENCODING=json-flat|json-nested|syslog|protobuf

#include "format_plugin.h"
#include <cstdlib>
#include <cstring>
#include <cstdio>
#include <cstdint>
#include <string_view>
#include <time.h>

#if !defined(FORMAT_JSON_FLAT) && !defined(FORMAT_JSON_NESTED) && \
    !defined(FORMAT_SYSLOG_RFC5424) && !defined(FORMAT_PROTOBUF)
#define FORMAT_JSON_FLAT
#endif

// ─── common helpers ───────────────────────────────────────────────────────────

static inline std::string_view sv(const format_plugin_string_t& s) {
    return { reinterpret_cast<const char*>(s.ptr), s.len };
}

static const char* level_name(uint8_t level) {
    static const char* names[] = { "debug","info","warn","error","crit","alert","emerg" };
    return (level < 7) ? names[level] : "unknown";
}

// ─── growable byte buffer ─────────────────────────────────────────────────────

struct Buf {
    uint8_t* data = nullptr;
    size_t   len  = 0;
    size_t   cap  = 0;

    ~Buf() { free(data); }

    uint8_t* release(size_t& out_len) {
        out_len = len;
        uint8_t* p = data;
        data = nullptr; len = 0; cap = 0;
        return p;
    }

    void reserve(size_t n) {
        if (len + n <= cap) return;
        size_t nc = cap ? cap * 2 : 4096;
        while (nc < len + n) nc *= 2;
        data = static_cast<uint8_t*>(realloc(data, nc));
        cap = nc;
    }

    void write(const void* src, size_t n) {
        reserve(n);
        memcpy(data + len, src, n);
        len += n;
    }

    void write_char(char c)            { write(&c, 1); }
    void write_sv(std::string_view s)  { write(s.data(), s.size()); }
    void write_cstr(const char* s)     { write(s, strlen(s)); }

    void write_u64_dec(uint64_t v) {
        char tmp[24];
        int n = snprintf(tmp, sizeof(tmp), "%llu", (unsigned long long)v);
        write(tmp, (size_t)n);
    }
};

// Emit [4-byte LE length][data] into `out`, consuming `entry`.
static void write_frame(Buf& out, Buf& entry) {
    uint32_t sz = (uint32_t)entry.len;
    uint8_t hdr[4] = {
        (uint8_t)(sz        & 0xFF),
        (uint8_t)((sz >> 8) & 0xFF),
        (uint8_t)((sz >>16) & 0xFF),
        (uint8_t)((sz >>24) & 0xFF)
    };
    out.write(hdr, 4);
    size_t n;
    uint8_t* p = entry.release(n);
    out.write(p, n);
    free(p);
}

// ─── JSON helpers ─────────────────────────────────────────────────────────────
#if defined(FORMAT_JSON_FLAT) || defined(FORMAT_JSON_NESTED)

static void json_escape(Buf& out, std::string_view s) {
    out.write_char('"');
    for (unsigned char c : s) {
        switch (c) {
        case '"':  out.write_cstr("\\\""); break;
        case '\\': out.write_cstr("\\\\"); break;
        case '\n': out.write_cstr("\\n");  break;
        case '\r': out.write_cstr("\\r");  break;
        case '\t': out.write_cstr("\\t");  break;
        default:
            if (c < 0x20) {
                char tmp[8];
                snprintf(tmp, sizeof(tmp), "\\u%04x", c);
                out.write_cstr(tmp);
            } else {
                out.write_char((char)c);
            }
        }
    }
    out.write_char('"');
}

#endif

// ─── protobuf helpers ────────────────────────────────────────────────────────
#ifdef FORMAT_PROTOBUF

static void pb_varint(Buf& out, uint64_t v) {
    do {
        uint8_t b = v & 0x7F;
        v >>= 7;
        if (v) b |= 0x80;
        out.write_char((char)b);
    } while (v);
}

static void pb_tag(Buf& out, uint32_t field, uint8_t wtype) {
    pb_varint(out, ((uint64_t)field << 3) | wtype);
}

static void pb_uint64(Buf& out, uint32_t field, uint64_t v) {
    if (v == 0) return;
    pb_tag(out, field, 0);
    pb_varint(out, v);
}

static void pb_uint32(Buf& out, uint32_t field, uint32_t v) {
    if (v == 0) return;
    pb_tag(out, field, 0);
    pb_varint(out, v);
}

static void pb_bytes(Buf& out, uint32_t field, const void* data, size_t n) {
    pb_tag(out, field, 2);
    pb_varint(out, n);
    out.write(data, n);
}

static void pb_string(Buf& out, uint32_t field, std::string_view s) {
    pb_bytes(out, field, s.data(), s.size());
}

#endif

// ─── per-entry encoders ───────────────────────────────────────────────────────
// Each encoder builds one entry into a local Buf, then calls write_frame().

// ── 1. JSON flat ──────────────────────────────────────────────────────────────
#ifdef FORMAT_JSON_FLAT
static void encode_entry(Buf& out, const format_plugin_log_entry_t& e) {
    Buf entry;
    entry.write_cstr("{\"id\":");
    entry.write_u64_dec(e.id);
    entry.write_cstr(",\"timestamp\":");
    json_escape(entry, sv(e.timestamp));
    entry.write_cstr(",\"level\":\"");
    entry.write_cstr(level_name(e.level));
    entry.write_cstr("\",\"message\":");
    json_escape(entry, sv(e.message));
    for (size_t i = 0; i < e.tags.len; i++) {
        entry.write_char(',');
        json_escape(entry, sv(e.tags.ptr[i].f0));
        entry.write_char(':');
        json_escape(entry, sv(e.tags.ptr[i].f1));
    }
    entry.write_char('}');
    write_frame(out, entry);
}
#endif

// ── 2. JSON nested ────────────────────────────────────────────────────────────
#ifdef FORMAT_JSON_NESTED
static void encode_entry(Buf& out, const format_plugin_log_entry_t& e) {
    Buf entry;
    entry.write_cstr("{\"id\":");
    entry.write_u64_dec(e.id);
    entry.write_cstr(",\"timestamp\":");
    json_escape(entry, sv(e.timestamp));
    entry.write_cstr(",\"level\":\"");
    entry.write_cstr(level_name(e.level));
    entry.write_cstr("\",\"message\":");
    json_escape(entry, sv(e.message));
    entry.write_cstr(",\"attributes\":{");
    for (size_t i = 0; i < e.tags.len; i++) {
        if (i > 0) entry.write_char(',');
        json_escape(entry, sv(e.tags.ptr[i].f0));
        entry.write_char(':');
        json_escape(entry, sv(e.tags.ptr[i].f1));
    }
    entry.write_cstr("}}");
    write_frame(out, entry);
}
#endif

// ── 3. RFC 5424 syslog ────────────────────────────────────────────────────────
#ifdef FORMAT_SYSLOG_RFC5424
static int level_to_pri(uint8_t level) {
    static const int sev[] = { 7, 6, 4, 3, 2, 1, 0 };
    return 1 * 8 + ((level < 7) ? sev[level] : 6);
}

static void sd_escape(Buf& out, std::string_view s) {
    for (char c : s) {
        if (c == '"' || c == '\\' || c == ']') out.write_char('\\');
        out.write_char(c);
    }
}

static void encode_entry(Buf& out, const format_plugin_log_entry_t& e) {
    Buf entry;

    char pri[8];
    snprintf(pri, sizeof(pri), "<%d>1 ", level_to_pri(e.level));
    entry.write_cstr(pri);

    if (e.timestamp.len > 0) entry.write_sv(sv(e.timestamp));
    else                      entry.write_char('-');
    entry.write_char(' ');

    std::string_view hostname = "-", appname = "-";
    for (size_t i = 0; i < e.tags.len; i++) {
        std::string_view k = sv(e.tags.ptr[i].f0);
        if (k == "host" || k == "hostname") hostname = sv(e.tags.ptr[i].f1);
        else if (k == "app" || k == "appname") appname = sv(e.tags.ptr[i].f1);
    }
    entry.write_sv(hostname);
    entry.write_char(' ');
    entry.write_sv(appname);
    entry.write_cstr(" - - ");

    bool has_sd = false;
    for (size_t i = 0; i < e.tags.len; i++) {
        std::string_view k = sv(e.tags.ptr[i].f0);
        if (k == "host" || k == "hostname" || k == "app" || k == "appname") continue;
        if (!has_sd) { entry.write_cstr("[log@12345"); has_sd = true; }
        entry.write_char(' ');
        entry.write_sv(k);
        entry.write_cstr("=\"");
        sd_escape(entry, sv(e.tags.ptr[i].f1));
        entry.write_char('"');
    }
    if (has_sd) entry.write_char(']');
    else        entry.write_char('-');

    entry.write_char(' ');
    entry.write_sv(sv(e.message));
    entry.write_char('\n');

    write_frame(out, entry);
}
#endif

// ── 4. Protocol Buffers ───────────────────────────────────────────────────────
#ifdef FORMAT_PROTOBUF
/*
  message Tag      { string key = 1; string value = 2; }
  message LogEntry {
    uint64 id = 1;  string timestamp = 2;
    uint32 level = 3;  string message = 4;
    repeated Tag tags = 5;
  }
*/
static void encode_entry(Buf& out, const format_plugin_log_entry_t& e) {
    Buf entry;
    pb_uint64(entry, 1, e.id);
    pb_string(entry, 2, sv(e.timestamp));
    pb_uint32(entry, 3, (uint32_t)e.level);
    pb_string(entry, 4, sv(e.message));
    for (size_t i = 0; i < e.tags.len; i++) {
        Buf tag;
        pb_string(tag, 1, sv(e.tags.ptr[i].f0));
        pb_string(tag, 2, sv(e.tags.ptr[i].f1));
        size_t tag_len;
        uint8_t* tag_data = tag.release(tag_len);
        pb_bytes(entry, 5, tag_data, tag_len);
        free(tag_data);
    }
    write_frame(out, entry);
}
#endif

// ─── exported WIT functions ───────────────────────────────────────────────────

// Accumulated encode time (ns) for the most recent format() call.
static uint64_t g_logic_ns = 0;

extern "C" uint64_t exports_format_plugin_report_usage(void) {
    return g_logic_ns;
}

extern "C" bool exports_format_plugin_format(
    format_plugin_list_log_entry_t* struct_data,
    format_plugin_list_u8_t*        ret,
    format_plugin_plugin_error_t*   err)
{
    if (!struct_data || !ret) {
        err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PLUGIN_ERROR_INTERNAL_ERROR;
        return false;
    }

    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);

    Buf out;
    for (size_t i = 0; i < struct_data->len; i++)
        encode_entry(out, struct_data->ptr[i]);

    clock_gettime(CLOCK_MONOTONIC, &t1);
    g_logic_ns = (uint64_t)((t1.tv_sec - t0.tv_sec) * 1000000000LL
                           + (t1.tv_nsec - t0.tv_nsec));

    ret->ptr = out.release(ret->len);
    return true;
}
