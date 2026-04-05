use std::cell::RefCell;

wit_bindgen::generate!({
    path: "wit",
    world: "transport-plugin",
    generate_all,
});

// After generate_all, types from the WIT file are accessible as:
//   local::log_process::pipeline_process::PluginError
//   local::log_process::transport_types::{TransportConfig, AuthMethod, ...}
//   wasi::http::outgoing_handler
//   wasi::http::types::{Fields, OutgoingRequest, ...}
//   wasi::io::streams::OutputStream
//   wasi::clocks::monotonic_clock

use local::log_process::transport_types::{AuthMethod, RetryConfig};
use wasi::clocks::monotonic_clock;
use wasi::http::outgoing_handler;
use wasi::http::types::{FieldKey, FieldValue, Fields, Method, OutgoingBody, RequestOptions, Scheme, OutgoingRequest};
use wasi::io::streams::StreamError;

// ── Global state (WASM is single-threaded) ─────────────────────────────────

struct State {
    config: Option<TransportConfig>,
    bytes_sent: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        config: None,
        bytes_sent: 0,
    });
}

// ── Helper: parse endpoint URL ─────────────────────────────────────────────

fn parse_endpoint(endpoint: &str) -> Result<(Scheme, String, String), PluginError> {
    let (scheme, rest) = if let Some(r) = endpoint.strip_prefix("https://") {
        (Scheme::Https, r)
    } else if let Some(r) = endpoint.strip_prefix("http://") {
        (Scheme::Http, r)
    } else {
        return Err(PluginError::ConfigError(format!(
            "unsupported scheme in endpoint: {}",
            endpoint
        )));
    };

    let (authority, path) = if let Some(idx) = rest.find('/') {
        (&rest[..idx], rest[idx..].to_string())
    } else {
        (rest, "/".to_string())
    };

    Ok((scheme, authority.to_string(), path))
}

// ── Helper: build HTTP headers ─────────────────────────────────────────────

fn build_headers(auth: &AuthMethod, extra: &[(String, String)], body_len: usize) -> Result<Fields, PluginError> {
    let fields = Fields::new();

    set_header(&fields, "content-type", b"application/octet-stream")?;
    set_header(&fields, "content-length", body_len.to_string().as_bytes())?;

    match auth {
        AuthMethod::None => {}
        AuthMethod::BearerToken(token) => {
            let value = format!("Bearer {}", token);
            set_header(&fields, "authorization", value.as_bytes())?;
        }
        AuthMethod::BasicAuth(cred) => {
            let encoded = base64_encode(
                format!("{}:{}", cred.username, cred.password).as_bytes(),
            );
            let value = format!("Basic {}", encoded);
            set_header(&fields, "authorization", value.as_bytes())?;
        }
        AuthMethod::ApiKey(api) => {
            set_header(&fields, &api.header_name, api.key.as_bytes())?;
        }
    }

    for (k, v) in extra {
        set_header(&fields, k, v.as_bytes())?;
    }

    Ok(fields)
}

fn set_header(fields: &Fields, name: &str, value: &[u8]) -> Result<(), PluginError> {
    fields
        .set(
            &FieldKey::from(name),
            &[FieldValue::from(value)],
        )
        .map_err(|e| PluginError::ProcessingFailed(format!("header error: {:?}", e)))
}

// ── Helper: minimal base64 encoding (no external crate) ───────────────────

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i];
        let b1 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] } else { 0 };

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[((b0 & 0x3) << 4 | b1 >> 4) as usize] as char);
        out.push(if i + 1 < input.len() {
            TABLE[((b1 & 0xf) << 2 | b2 >> 6) as usize] as char
        } else {
            '='
        });
        out.push(if i + 2 < input.len() {
            TABLE[(b2 & 0x3f) as usize] as char
        } else {
            '='
        });
        i += 3;
    }
    out
}

// ── Helper: send one HTTP POST batch ───────────────────────────────────────

fn send_batch(body_bytes: &[u8], config: &TransportConfig) -> Result<(), PluginError> {
    let (scheme, authority, path) = parse_endpoint(&config.endpoint)?;
    let headers = build_headers(&config.auth, &config.extra_headers, body_bytes.len())?;

    let request = OutgoingRequest::new(headers);
    request
        .set_method(&Method::Post)
        .map_err(|_| PluginError::ProcessingFailed("set method failed".into()))?;
    request
        .set_scheme(Some(&scheme))
        .map_err(|_| PluginError::ProcessingFailed("set scheme failed".into()))?;
    request
        .set_authority(Some(&authority))
        .map_err(|_| PluginError::ProcessingFailed("set authority failed".into()))?;
    request
        .set_path_with_query(Some(&path))
        .map_err(|_| PluginError::ProcessingFailed("set path failed".into()))?;

    // Request options (timeouts in nanoseconds)
    let opts = RequestOptions::new();
    if config.connect_timeout_ms > 0 {
        let _ = opts.set_connect_timeout(Some(config.connect_timeout_ms as u64 * 1_000_000));
    }
    if config.request_timeout_ms > 0 {
        let ns = config.request_timeout_ms as u64 * 1_000_000;
        let _ = opts.set_first_byte_timeout(Some(ns));
        let _ = opts.set_between_bytes_timeout(Some(ns));
    }

    // Write body via blocking write
    {
        let out_body = request
            .body()
            .map_err(|_| PluginError::ProcessingFailed("get request body failed".into()))?;
        let stream = out_body
            .write()
            .map_err(|_| PluginError::ProcessingFailed("get write stream failed".into()))?;

        stream
            .blocking_write_and_flush(body_bytes)
            .map_err(|e| match e {
                StreamError::LastOperationFailed(err) => {
                    PluginError::ProcessingFailed(format!("write error: {}", err.to_debug_string()))
                }
                StreamError::Closed => {
                    PluginError::ProcessingFailed("stream closed prematurely".into())
                }
            })?;

        drop(stream);
        OutgoingBody::finish(out_body, None)
            .map_err(|e| PluginError::ProcessingFailed(format!("finish body: {:?}", e)))?;
    }

    // Send and wait for response
    let future_resp = outgoing_handler::handle(request, Some(opts))
        .map_err(|e| PluginError::ProcessingFailed(format!("http handle error: {:?}", e)))?;

    let pollable = future_resp.subscribe();
    pollable.block();

    let incoming = future_resp
        .get()
        .ok_or_else(|| PluginError::ProcessingFailed("no response after poll".into()))?
        .map_err(|_| PluginError::ProcessingFailed("future response error".into()))?
        .map_err(|e| PluginError::ProcessingFailed(format!("http error: {:?}", e)))?;

    let status = incoming.status();
    if !(200..300).contains(&status) {
        return Err(PluginError::ProcessingFailed(format!(
            "server returned HTTP {}",
            status
        )));
    }

    Ok(())
}

// ── Helper: sleep using WASI monotonic clock ───────────────────────────────

fn sleep_ms(ms: u64) {
    let pollable = monotonic_clock::subscribe_duration(ms * 1_000_000);
    pollable.block();
}

// ── Plugin struct and Guest implementation ────────────────────────────────

struct TransportPlugin;

impl Guest for TransportPlugin {
    fn init(config: TransportConfig) -> Result<(), PluginError> {
        if config.endpoint.is_empty() {
            return Err(PluginError::ConfigError(
                "endpoint must not be empty".into(),
            ));
        }
        parse_endpoint(&config.endpoint)?;

        STATE.with(|s| {
            let mut state = s.borrow_mut();
            state.config = Some(config);
            state.bytes_sent = 0;
        });

        Ok(())
    }

    fn send(output_data: Vec<Vec<u8>>) -> Result<(), PluginError> {
        let config = STATE.with(|s| {
            s.borrow()
                .config
                .clone()
                .ok_or_else(|| PluginError::ConfigError("init() not called before send()".into()))
        })?;

        if output_data.is_empty() {
            return Ok(());
        }

        let max_batch_bytes = config.max_batch_bytes;
        let retry_cfg = config.retry.clone();

        // Split into HTTP batches respecting max_batch_bytes (0 = unlimited)
        let batches: Vec<Vec<u8>> = if max_batch_bytes == 0 {
            let mut buf = Vec::new();
            for entry in &output_data {
                buf.extend_from_slice(entry);
                if !entry.ends_with(b"\n") {
                    buf.push(b'\n');
                }
            }
            vec![buf]
        } else {
            let limit = max_batch_bytes as usize;
            let mut batches: Vec<Vec<u8>> = Vec::new();
            let mut current: Vec<u8> = Vec::new();
            for entry in &output_data {
                let entry_size = entry.len() + 1; // +1 for newline
                if !current.is_empty() && current.len() + entry_size > limit {
                    batches.push(std::mem::take(&mut current));
                }
                current.extend_from_slice(entry);
                if !entry.ends_with(b"\n") {
                    current.push(b'\n');
                }
            }
            if !current.is_empty() {
                batches.push(current);
            }
            batches
        };

        let mut total_sent: u64 = 0;

        for batch in &batches {
            let max_retries = retry_cfg.as_ref().map(|r| r.max_retries).unwrap_or(0);
            let initial_backoff = retry_cfg
                .as_ref()
                .map(|r| r.initial_backoff_ms as u64)
                .unwrap_or(100);
            let max_backoff = retry_cfg
                .as_ref()
                .map(|r| r.max_backoff_ms as u64)
                .unwrap_or(5000);

            let mut attempt = 0u32;
            let mut backoff_ms = initial_backoff;

            loop {
                match send_batch(batch, &config) {
                    Ok(()) => {
                        total_sent += batch.len() as u64;
                        break;
                    }
                    Err(e) if attempt < max_retries => {
                        attempt += 1;
                        sleep_ms(backoff_ms);
                        backoff_ms = (backoff_ms * 2).min(max_backoff);
                    }
                    Err(e) => return Err(e),
                }
            }
        }

        STATE.with(|s| {
            s.borrow_mut().bytes_sent += total_sent;
        });

        Ok(())
    }

    fn report_usage() -> u64 {
        STATE.with(|s| s.borrow().bytes_sent)
    }
}

export!(TransportPlugin);
