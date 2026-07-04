// transport_impl.c — raw-socket transport-plugin (HTTP/1.1 over wasi:sockets TCP)
//
// 邏輯對應 rust-plugin/transport/src/lib.rs：
// init() 驗證/儲存設定、send() 依 max-batch-bytes 分批並帶重試地送出、
// report-usage() 回傳累積送出位元組數。
//
// 與 Rust 版最大的差異：這個 world 只 import 了 wasi:sockets（沒有 wasi:http），
// 所以 HTTP request/response 全部用原始 TCP socket 手刻，僅支援 http://（無 TLS）。

#include "transport_plugin.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <stdarg.h>

// ───────────────────────────────────────────────────────────
// 動態位元組緩衝區
// ───────────────────────────────────────────────────────────

typedef struct {
    uint8_t *data;
    size_t len;
    size_t cap;
} byte_buf_t;

static void bb_init(byte_buf_t *b) {
    b->data = NULL;
    b->len = 0;
    b->cap = 0;
}

static void bb_reserve(byte_buf_t *b, size_t extra) {
    if (b->len + extra <= b->cap) {
        return;
    }
    size_t ncap = b->cap ? b->cap * 2 : 256;
    while (ncap < b->len + extra) {
        ncap *= 2;
    }
    b->data = (uint8_t*)realloc(b->data, ncap);
    b->cap = ncap;
}

static void bb_append(byte_buf_t *b, const uint8_t *p, size_t l) {
    if (l == 0) {
        return;
    }
    bb_reserve(b, l);
    memcpy(b->data + b->len, p, l);
    b->len += l;
}

static void bb_append_str(byte_buf_t *b, const char *s) {
    bb_append(b, (const uint8_t*)s, strlen(s));
}

static void bb_free(byte_buf_t *b) {
    free(b->data);
    b->data = NULL;
    b->len = 0;
    b->cap = 0;
}

typedef struct {
    byte_buf_t *items;
    size_t count;
    size_t cap;
} batch_list_t;

static void bl_push(batch_list_t *bl, byte_buf_t bb) {
    if (bl->count == bl->cap) {
        bl->cap = bl->cap ? bl->cap * 2 : 4;
        bl->items = (byte_buf_t*)realloc(bl->items, bl->cap * sizeof(byte_buf_t));
    }
    bl->items[bl->count++] = bb;
}

// ───────────────────────────────────────────────────────────
// 插件狀態
// ───────────────────────────────────────────────────────────

typedef struct {
    uint8_t tag; // LOCAL_LOG_PROCESS_TRANSPORT_TYPES_AUTH_METHOD_*
    char *bearer_token;
    char *basic_username;
    char *basic_password;
    char *apikey_header;
    char *apikey_key;
} auth_state_t;

typedef struct {
    char **names;
    char **values;
    size_t count;
} header_list_t;

typedef struct {
    bool initialized;

    char *host;
    uint16_t port;
    char *path;

    auth_state_t auth;

    bool has_retry;
    uint32_t max_retries;
    uint32_t initial_backoff_ms;
    uint32_t max_backoff_ms;

    uint32_t connect_timeout_ms;
    uint32_t request_timeout_ms;
    uint32_t max_batch_bytes;

    header_list_t extra_headers;

    uint64_t bytes_sent;
} state_t;

static state_t g_state = {0};

static void free_state(state_t *s) {
    free(s->host);
    free(s->path);
    free(s->auth.bearer_token);
    free(s->auth.basic_username);
    free(s->auth.basic_password);
    free(s->auth.apikey_header);
    free(s->auth.apikey_key);
    for (size_t i = 0; i < s->extra_headers.count; i++) {
        free(s->extra_headers.names[i]);
        free(s->extra_headers.values[i]);
    }
    free(s->extra_headers.names);
    free(s->extra_headers.values);
    memset(s, 0, sizeof(*s));
}

// ───────────────────────────────────────────────────────────
// 小工具：字串複製 / 錯誤建構 / base64 / 時鐘
// ───────────────────────────────────────────────────────────

static char* dup_wstr(transport_plugin_string_t s) {
    char *r = (char*)malloc(s.len + 1);
    memcpy(r, s.ptr, s.len);
    r[s.len] = '\0';
    return r;
}

static char* dup_cstr(const char *s) {
    size_t l = strlen(s);
    char *r = (char*)malloc(l + 1);
    memcpy(r, s, l + 1);
    return r;
}

static void set_err_config(transport_plugin_plugin_error_t *err, const char *msg) {
    err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PLUGIN_ERROR_CONFIG_ERROR;
    transport_plugin_string_dup(&err->val.config_error, msg);
}

static void set_err_processing(transport_plugin_plugin_error_t *err, const char *msg) {
    err->tag = LOCAL_LOG_PROCESS_PIPELINE_PROCESS_PLUGIN_ERROR_PROCESSING_FAILED;
    transport_plugin_string_dup(&err->val.processing_failed, msg);
}

static void set_err_processing_fmt(transport_plugin_plugin_error_t *err, const char *fmt, ...) {
    char buf[256];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    set_err_processing(err, buf);
}

static uint64_t monotonic_now(void) {
    return wasi_clocks_monotonic_clock_now();
}

static void sleep_ms(uint64_t ms) {
    wasi_clocks_monotonic_clock_own_pollable_t p = wasi_clocks_monotonic_clock_subscribe_duration(ms * 1000000ULL);
    wasi_io_poll_borrow_pollable_t pb = wasi_io_poll_borrow_pollable(p);
    wasi_io_poll_method_pollable_block(pb);
    wasi_io_poll_pollable_drop_own(p);
}

static char* base64_encode(const uint8_t *input, size_t len) {
    static const char *TABLE = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    size_t out_len = ((len + 2) / 3) * 4;
    char *out = (char*)malloc(out_len + 1);
    size_t oi = 0;
    for (size_t i = 0; i < len; i += 3) {
        uint8_t b0 = input[i];
        uint8_t b1 = (i + 1 < len) ? input[i + 1] : 0;
        uint8_t b2 = (i + 2 < len) ? input[i + 2] : 0;
        out[oi++] = TABLE[b0 >> 2];
        out[oi++] = TABLE[((b0 & 0x3) << 4) | (b1 >> 4)];
        out[oi++] = (i + 1 < len) ? TABLE[((b1 & 0xf) << 2) | (b2 >> 6)] : '=';
        out[oi++] = (i + 2 < len) ? TABLE[b2 & 0x3f] : '=';
    }
    out[oi] = '\0';
    return out;
}

// ───────────────────────────────────────────────────────────
// endpoint 解析：只接受 http://host[:port][/path]
// ───────────────────────────────────────────────────────────

static bool parse_endpoint(const char *endpoint, char **out_host, uint16_t *out_port, char **out_path,
                            transport_plugin_plugin_error_t *err) {
    const char *rest;
    if (strncmp(endpoint, "http://", 7) == 0) {
        rest = endpoint + 7;
    } else if (strncmp(endpoint, "https://", 8) == 0) {
        set_err_config(err, "https is not supported: C transport plugin only has raw wasi:sockets access (no TLS)");
        return false;
    } else {
        set_err_config(err, "unsupported scheme in endpoint (only http:// is supported)");
        return false;
    }

    const char *slash = strchr(rest, '/');
    size_t authority_len = slash ? (size_t)(slash - rest) : strlen(rest);
    const char *path_src = slash ? slash : "/";

    if (authority_len == 0) {
        set_err_config(err, "endpoint is missing a host");
        return false;
    }

    char authority[256];
    if (authority_len >= sizeof(authority)) {
        authority_len = sizeof(authority) - 1;
    }
    memcpy(authority, rest, authority_len);
    authority[authority_len] = '\0';

    char *colon = strrchr(authority, ':');
    uint16_t port = 80;
    char host[256];
    if (colon) {
        size_t hlen = (size_t)(colon - authority);
        if (hlen >= sizeof(host)) {
            hlen = sizeof(host) - 1;
        }
        memcpy(host, authority, hlen);
        host[hlen] = '\0';
        int parsed_port = atoi(colon + 1);
        if (parsed_port > 0 && parsed_port <= 65535) {
            port = (uint16_t)parsed_port;
        }
    } else {
        strncpy(host, authority, sizeof(host) - 1);
        host[sizeof(host) - 1] = '\0';
    }

    if (host[0] == '\0') {
        set_err_config(err, "endpoint host must not be empty");
        return false;
    }

    *out_host = dup_cstr(host);
    *out_port = port;
    *out_path = dup_cstr(path_src);
    return true;
}

// ───────────────────────────────────────────────────────────
// DNS 解析 + TCP 連線（wasi:sockets）
// ───────────────────────────────────────────────────────────

static bool parse_ipv4_literal(const char *host, wasi_sockets_network_ipv4_address_t *out) {
    unsigned a, b, c, d;
    int consumed = 0;
    if (sscanf(host, "%u.%u.%u.%u%n", &a, &b, &c, &d, &consumed) != 4) {
        return false;
    }
    if (host[consumed] != '\0') {
        return false;
    }
    if (a > 255 || b > 255 || c > 255 || d > 255) {
        return false;
    }
    out->f0 = (uint8_t)a;
    out->f1 = (uint8_t)b;
    out->f2 = (uint8_t)c;
    out->f3 = (uint8_t)d;
    return true;
}

static bool resolve_ipv4(wasi_sockets_network_borrow_network_t net, const char *host, uint64_t deadline,
                          wasi_sockets_network_ipv4_address_t *out, transport_plugin_plugin_error_t *err) {
    if (parse_ipv4_literal(host, out)) {
        return true;
    }

    transport_plugin_string_t name;
    transport_plugin_string_set(&name, host);

    wasi_sockets_ip_name_lookup_own_resolve_address_stream_t stream_own;
    wasi_sockets_ip_name_lookup_error_code_t rerr;
    if (!wasi_sockets_ip_name_lookup_resolve_addresses(net, &name, &stream_own, &rerr)) {
        set_err_processing_fmt(err, "DNS resolve start failed (code %u)", rerr);
        return false;
    }
    wasi_sockets_ip_name_lookup_borrow_resolve_address_stream_t stream_b =
        wasi_sockets_ip_name_lookup_borrow_resolve_address_stream(stream_own);

    bool found = false;
    wasi_sockets_network_ip_address_t got;

    for (;;) {
        wasi_sockets_ip_name_lookup_option_ip_address_t opt;
        wasi_sockets_ip_name_lookup_error_code_t nerr;
        bool ok = wasi_sockets_ip_name_lookup_method_resolve_address_stream_resolve_next_address(stream_b, &opt, &nerr);
        if (!ok) {
            if (nerr == WASI_SOCKETS_NETWORK_ERROR_CODE_WOULD_BLOCK) {
                if (deadline && monotonic_now() >= deadline) {
                    wasi_sockets_ip_name_lookup_resolve_address_stream_drop_own(stream_own);
                    set_err_processing(err, "DNS resolve timed out");
                    return false;
                }
                wasi_sockets_ip_name_lookup_own_pollable_t p =
                    wasi_sockets_ip_name_lookup_method_resolve_address_stream_subscribe(stream_b);
                wasi_io_poll_borrow_pollable_t pb = wasi_io_poll_borrow_pollable(p);
                wasi_io_poll_method_pollable_block(pb);
                wasi_io_poll_pollable_drop_own(p);
                continue;
            }
            wasi_sockets_ip_name_lookup_resolve_address_stream_drop_own(stream_own);
            set_err_processing_fmt(err, "DNS resolve failed (code %u)", nerr);
            return false;
        }
        if (!opt.is_some) {
            break;
        }
        got = opt.val;
        found = true;
        break;
    }
    wasi_sockets_ip_name_lookup_resolve_address_stream_drop_own(stream_own);

    if (!found) {
        set_err_processing(err, "DNS resolve returned no addresses");
        return false;
    }
    if (got.tag != WASI_SOCKETS_NETWORK_IP_ADDRESS_IPV4) {
        set_err_processing(err, "only IPv4 addresses are supported");
        return false;
    }
    *out = got.val.ipv4;
    return true;
}

static bool tcp_connect(const char *host, uint16_t port, uint32_t connect_timeout_ms,
                         wasi_sockets_tcp_own_tcp_socket_t *out_sock,
                         wasi_io_streams_own_input_stream_t *out_in,
                         wasi_io_streams_own_output_stream_t *out_out,
                         transport_plugin_plugin_error_t *err) {
    uint64_t deadline = connect_timeout_ms ? monotonic_now() + (uint64_t)connect_timeout_ms * 1000000ULL : 0;

    wasi_sockets_instance_network_own_network_t net_own = wasi_sockets_instance_network_instance_network();
    wasi_sockets_network_borrow_network_t net_b = wasi_sockets_network_borrow_network(net_own);

    wasi_sockets_network_ipv4_address_t ipv4;
    if (!resolve_ipv4(net_b, host, deadline, &ipv4, err)) {
        wasi_sockets_network_network_drop_own(net_own);
        return false;
    }

    wasi_sockets_tcp_create_socket_own_tcp_socket_t sock_own;
    wasi_sockets_tcp_create_socket_error_code_t cerr;
    if (!wasi_sockets_tcp_create_socket_create_tcp_socket(WASI_SOCKETS_NETWORK_IP_ADDRESS_FAMILY_IPV4, &sock_own, &cerr)) {
        wasi_sockets_network_network_drop_own(net_own);
        set_err_processing_fmt(err, "create tcp socket failed (code %u)", cerr);
        return false;
    }
    wasi_sockets_tcp_borrow_tcp_socket_t sock_b = wasi_sockets_tcp_borrow_tcp_socket(sock_own);

    wasi_sockets_network_ip_socket_address_t remote;
    remote.tag = WASI_SOCKETS_NETWORK_IP_SOCKET_ADDRESS_IPV4;
    remote.val.ipv4.port = port;
    remote.val.ipv4.address = ipv4;

    wasi_sockets_tcp_error_code_t serr;
    if (!wasi_sockets_tcp_method_tcp_socket_start_connect(sock_b, net_b, &remote, &serr)) {
        wasi_sockets_tcp_tcp_socket_drop_own(sock_own);
        wasi_sockets_network_network_drop_own(net_own);
        set_err_processing_fmt(err, "tcp connect start failed (code %u)", serr);
        return false;
    }
    wasi_sockets_network_network_drop_own(net_own);

    wasi_sockets_tcp_tuple2_own_input_stream_own_output_stream_t streams;
    for (;;) {
        if (deadline && monotonic_now() >= deadline) {
            wasi_sockets_tcp_tcp_socket_drop_own(sock_own);
            set_err_processing(err, "tcp connect timed out");
            return false;
        }
        wasi_sockets_tcp_own_pollable_t p = wasi_sockets_tcp_method_tcp_socket_subscribe(sock_b);
        wasi_io_poll_borrow_pollable_t pb = wasi_io_poll_borrow_pollable(p);
        wasi_io_poll_method_pollable_block(pb);
        wasi_io_poll_pollable_drop_own(p);

        wasi_sockets_tcp_error_code_t ferr;
        bool ok = wasi_sockets_tcp_method_tcp_socket_finish_connect(sock_b, &streams, &ferr);
        if (ok) {
            break;
        }
        if (ferr == WASI_SOCKETS_NETWORK_ERROR_CODE_WOULD_BLOCK) {
            continue;
        }
        wasi_sockets_tcp_tcp_socket_drop_own(sock_own);
        set_err_processing_fmt(err, "tcp connect failed (code %u)", ferr);
        return false;
    }

    *out_sock = sock_own;
    *out_in = streams.f0;
    *out_out = streams.f1;
    return true;
}

static void cleanup_connection(wasi_sockets_tcp_own_tcp_socket_t sock,
                                wasi_io_streams_own_input_stream_t in,
                                wasi_io_streams_own_output_stream_t out) {
    wasi_sockets_tcp_error_code_t shut_err;
    wasi_sockets_tcp_method_tcp_socket_shutdown(wasi_sockets_tcp_borrow_tcp_socket(sock),
                                                 WASI_SOCKETS_TCP_SHUTDOWN_TYPE_BOTH, &shut_err);
    wasi_io_streams_input_stream_drop_own(in);
    wasi_io_streams_output_stream_drop_own(out);
    wasi_sockets_tcp_tcp_socket_drop_own(sock);
}

// ───────────────────────────────────────────────────────────
// HTTP/1.1 request 組裝
// ───────────────────────────────────────────────────────────

static void append_header(byte_buf_t *req, const char *name, const char *value) {
    bb_append_str(req, name);
    bb_append_str(req, ": ");
    bb_append_str(req, value);
    bb_append_str(req, "\r\n");
}

static void build_request(byte_buf_t *req, const uint8_t *body, size_t body_len) {
    bb_init(req);
    bb_append_str(req, "POST ");
    bb_append_str(req, g_state.path);
    bb_append_str(req, " HTTP/1.1\r\n");

    char host_header[600];
    if (g_state.port == 80) {
        snprintf(host_header, sizeof(host_header), "%s", g_state.host);
    } else {
        snprintf(host_header, sizeof(host_header), "%s:%u", g_state.host, g_state.port);
    }
    append_header(req, "Host", host_header);
    append_header(req, "Content-Type", "application/octet-stream");

    char len_buf[32];
    snprintf(len_buf, sizeof(len_buf), "%zu", body_len);
    append_header(req, "Content-Length", len_buf);
    append_header(req, "Connection", "close");

    switch (g_state.auth.tag) {
        case LOCAL_LOG_PROCESS_TRANSPORT_TYPES_AUTH_METHOD_BEARER_TOKEN: {
            char *v = (char*)malloc(strlen(g_state.auth.bearer_token) + 8);
            sprintf(v, "Bearer %s", g_state.auth.bearer_token);
            append_header(req, "Authorization", v);
            free(v);
            break;
        }
        case LOCAL_LOG_PROCESS_TRANSPORT_TYPES_AUTH_METHOD_BASIC_AUTH: {
            char cred[512];
            snprintf(cred, sizeof(cred), "%s:%s", g_state.auth.basic_username, g_state.auth.basic_password);
            char *enc = base64_encode((uint8_t*)cred, strlen(cred));
            char *v = (char*)malloc(strlen(enc) + 8);
            sprintf(v, "Basic %s", enc);
            append_header(req, "Authorization", v);
            free(v);
            free(enc);
            break;
        }
        case LOCAL_LOG_PROCESS_TRANSPORT_TYPES_AUTH_METHOD_API_KEY:
            append_header(req, g_state.auth.apikey_header, g_state.auth.apikey_key);
            break;
        default:
            break;
    }

    for (size_t i = 0; i < g_state.extra_headers.count; i++) {
        append_header(req, g_state.extra_headers.names[i], g_state.extra_headers.values[i]);
    }

    bb_append_str(req, "\r\n");
    bb_append(req, body, body_len);
}

// ───────────────────────────────────────────────────────────
// 送出一個 batch（一次連線，失敗不重試 — 重試由呼叫端負責）
// ───────────────────────────────────────────────────────────

static bool send_batch_once(const uint8_t *body, size_t body_len, transport_plugin_plugin_error_t *err) {
    wasi_sockets_tcp_own_tcp_socket_t sock;
    wasi_io_streams_own_input_stream_t in;
    wasi_io_streams_own_output_stream_t out;
    if (!tcp_connect(g_state.host, g_state.port, g_state.connect_timeout_ms, &sock, &in, &out, err)) {
        return false;
    }

    wasi_io_streams_borrow_output_stream_t out_b = wasi_io_streams_borrow_output_stream(out);
    wasi_io_streams_borrow_input_stream_t in_b = wasi_io_streams_borrow_input_stream(in);

    uint64_t deadline = g_state.request_timeout_ms
        ? monotonic_now() + (uint64_t)g_state.request_timeout_ms * 1000000ULL
        : 0;

    byte_buf_t req;
    build_request(&req, body, body_len);

    // WASI io/streams 的 blocking_write_and_flush 每次最多寫 4096 B，需分批寫入。
    const size_t CHUNK = 4096;
    size_t offset = 0;
    bool write_ok = true;
    while (offset < req.len) {
        if (deadline && monotonic_now() >= deadline) {
            set_err_processing(err, "request write timed out");
            write_ok = false;
            break;
        }
        size_t end = offset + CHUNK;
        if (end > req.len) {
            end = req.len;
        }
        transport_plugin_list_u8_t chunk = { .ptr = req.data + offset, .len = end - offset };
        wasi_io_streams_stream_error_t serr;
        if (!wasi_io_streams_method_output_stream_blocking_write_and_flush(out_b, &chunk, &serr)) {
            set_err_processing_fmt(err, "write failed (stream error tag %u)", serr.tag);
            write_ok = false;
            break;
        }
        offset = end;
    }
    bb_free(&req);

    if (!write_ok) {
        cleanup_connection(sock, in, out);
        return false;
    }

    // 讀取回應，僅解析狀態行；Connection: close 讓 server 主動關閉連線。
    uint8_t resp_buf[4096];
    size_t resp_len = 0;
    bool got_status = false;
    int status_code = 0;
    bool read_error = false;

    for (int iter = 0; iter < 64; iter++) {
        if (deadline && monotonic_now() >= deadline) {
            set_err_processing(err, "response read timed out");
            read_error = true;
            break;
        }
        transport_plugin_list_u8_t data;
        wasi_io_streams_stream_error_t rerr;
        bool ok = wasi_io_streams_method_input_stream_blocking_read(in_b, 4096, &data, &rerr);
        if (!ok) {
            if (rerr.tag == WASI_IO_STREAMS_STREAM_ERROR_CLOSED) {
                break;
            }
            set_err_processing_fmt(err, "read failed (stream error tag %u)", rerr.tag);
            read_error = true;
            break;
        }
        if (data.len > 0 && !got_status) {
            size_t copy_len = data.len;
            if (resp_len + copy_len > sizeof(resp_buf) - 1) {
                copy_len = sizeof(resp_buf) - 1 - resp_len;
            }
            memcpy(resp_buf + resp_len, data.ptr, copy_len);
            resp_len += copy_len;
            resp_buf[resp_len] = '\0';
            int major, minor;
            if (sscanf((char*)resp_buf, "HTTP/%d.%d %d", &major, &minor, &status_code) == 3) {
                got_status = true;
            }
        }
        transport_plugin_list_u8_free(&data);
    }

    cleanup_connection(sock, in, out);

    if (read_error) {
        return false;
    }
    if (!got_status) {
        set_err_processing(err, "no HTTP status line received from server");
        return false;
    }
    if (status_code < 200 || status_code >= 300) {
        set_err_processing_fmt(err, "server returned HTTP %d", status_code);
        return false;
    }
    return true;
}

static bool send_with_retry(const uint8_t *body, size_t len, transport_plugin_plugin_error_t *err) {
    uint32_t max_retries = g_state.has_retry ? g_state.max_retries : 0;
    uint64_t backoff_ms = g_state.has_retry ? g_state.initial_backoff_ms : 100;
    uint64_t max_backoff_ms = g_state.has_retry ? g_state.max_backoff_ms : 5000;
    uint32_t attempt = 0;

    for (;;) {
        transport_plugin_plugin_error_t local_err;
        if (send_batch_once(body, len, &local_err)) {
            return true;
        }
        if (attempt < max_retries) {
            attempt++;
            transport_plugin_plugin_error_free(&local_err);
            sleep_ms(backoff_ms);
            backoff_ms *= 2;
            if (backoff_ms > max_backoff_ms) {
                backoff_ms = max_backoff_ms;
            }
            continue;
        }
        *err = local_err;
        return false;
    }
}

// ───────────────────────────────────────────────────────────
// batch 切割（依 max-batch-bytes，0 = 不限制）
// ───────────────────────────────────────────────────────────

static void build_batches(transport_plugin_list_list_u8_t *output_data, uint32_t max_batch_bytes, batch_list_t *out) {
    out->items = NULL;
    out->count = 0;
    out->cap = 0;

    if (max_batch_bytes == 0) {
        byte_buf_t buf;
        bb_init(&buf);
        for (size_t i = 0; i < output_data->len; i++) {
            transport_plugin_list_u8_t *entry = &output_data->ptr[i];
            bb_append(&buf, entry->ptr, entry->len);
            if (entry->len == 0 || entry->ptr[entry->len - 1] != '\n') {
                bb_append(&buf, (const uint8_t*)"\n", 1);
            }
        }
        bl_push(out, buf);
        return;
    }

    size_t limit = max_batch_bytes;
    byte_buf_t cur;
    bb_init(&cur);
    for (size_t i = 0; i < output_data->len; i++) {
        transport_plugin_list_u8_t *entry = &output_data->ptr[i];
        size_t entry_size = entry->len + 1;
        if (cur.len > 0 && cur.len + entry_size > limit) {
            bl_push(out, cur);
            bb_init(&cur);
        }
        bb_append(&cur, entry->ptr, entry->len);
        if (entry->len == 0 || entry->ptr[entry->len - 1] != '\n') {
            bb_append(&cur, (const uint8_t*)"\n", 1);
        }
    }
    if (cur.len > 0) {
        bl_push(out, cur);
    } else {
        bb_free(&cur);
    }
}

// ───────────────────────────────────────────────────────────
// Exports
// ───────────────────────────────────────────────────────────

bool exports_transport_plugin_init(transport_plugin_transport_config_t *config, transport_plugin_plugin_error_t *err) {
    free_state(&g_state);

    if (config->endpoint.len == 0) {
        set_err_config(err, "endpoint must not be empty");
        return false;
    }

    char *endpoint = dup_wstr(config->endpoint);
    char *host = NULL;
    char *path = NULL;
    uint16_t port;
    bool ok = parse_endpoint(endpoint, &host, &port, &path, err);
    free(endpoint);
    if (!ok) {
        return false;
    }

    g_state.host = host;
    g_state.port = port;
    g_state.path = path;

    g_state.auth.tag = config->auth.tag;
    switch (config->auth.tag) {
        case LOCAL_LOG_PROCESS_TRANSPORT_TYPES_AUTH_METHOD_BEARER_TOKEN:
            g_state.auth.bearer_token = dup_wstr(config->auth.val.bearer_token);
            break;
        case LOCAL_LOG_PROCESS_TRANSPORT_TYPES_AUTH_METHOD_BASIC_AUTH:
            g_state.auth.basic_username = dup_wstr(config->auth.val.basic_auth.username);
            g_state.auth.basic_password = dup_wstr(config->auth.val.basic_auth.password);
            break;
        case LOCAL_LOG_PROCESS_TRANSPORT_TYPES_AUTH_METHOD_API_KEY:
            g_state.auth.apikey_header = dup_wstr(config->auth.val.api_key.header_name);
            g_state.auth.apikey_key = dup_wstr(config->auth.val.api_key.key);
            break;
        default:
            break;
    }

    g_state.has_retry = config->retry.is_some;
    if (g_state.has_retry) {
        g_state.max_retries = config->retry.val.max_retries;
        g_state.initial_backoff_ms = config->retry.val.initial_backoff_ms;
        g_state.max_backoff_ms = config->retry.val.max_backoff_ms;
    }

    // tls 設定不支援：此 world 只 import 了 wasi:sockets，沒有 TLS 能力，
    // https 端點已在 parse_endpoint() 中被拒絕，故此處刻意忽略 config->tls。

    g_state.connect_timeout_ms = config->connect_timeout_ms;
    g_state.request_timeout_ms = config->request_timeout_ms;
    g_state.max_batch_bytes = config->max_batch_bytes;

    size_t n = config->extra_headers.len;
    if (n > 0) {
        g_state.extra_headers.names = (char**)malloc(n * sizeof(char*));
        g_state.extra_headers.values = (char**)malloc(n * sizeof(char*));
        for (size_t i = 0; i < n; i++) {
            g_state.extra_headers.names[i] = dup_wstr(config->extra_headers.ptr[i].f0);
            g_state.extra_headers.values[i] = dup_wstr(config->extra_headers.ptr[i].f1);
        }
        g_state.extra_headers.count = n;
    }

    g_state.bytes_sent = 0;
    g_state.initialized = true;
    return true;
}

bool exports_transport_plugin_send(transport_plugin_list_list_u8_t *output_data, transport_plugin_plugin_error_t *err) {
    if (!g_state.initialized) {
        set_err_config(err, "init() not called before send()");
        return false;
    }
    if (output_data->len == 0) {
        return true;
    }

    batch_list_t batches;
    build_batches(output_data, g_state.max_batch_bytes, &batches);

    uint64_t total_sent = 0;
    bool all_ok = true;
    for (size_t i = 0; i < batches.count; i++) {
        byte_buf_t *b = &batches.items[i];
        if (!send_with_retry(b->data, b->len, err)) {
            all_ok = false;
            break;
        }
        total_sent += b->len;
    }

    for (size_t i = 0; i < batches.count; i++) {
        bb_free(&batches.items[i]);
    }
    free(batches.items);

    if (!all_ok) {
        return false;
    }

    g_state.bytes_sent += total_sent;
    return true;
}

uint64_t exports_transport_plugin_report_usage(void) {
    return g_state.bytes_sent;
}
