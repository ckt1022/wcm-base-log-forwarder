wit_bindgen::generate!({
    path: "../wit",
    world: "parser-plugin",
});

use local::log_process::pipeline_process::LogLevel;
use serde_json::Value;

struct ParseBenchmark;

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn map_level(value: Option<&Value>) -> LogLevel {
    match value {
        Some(Value::Number(number)) => match number.as_u64().unwrap_or(1) {
            0 => LogLevel::Debug,
            1 => LogLevel::Info,
            2 => LogLevel::Warn,
            3 => LogLevel::Error,
            4 => LogLevel::Crit,
            5 => LogLevel::Alert,
            6 => LogLevel::Emerg,
            _ => LogLevel::Info,
        },
        Some(Value::String(level)) if level.eq_ignore_ascii_case("debug") => LogLevel::Debug,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("warn") => LogLevel::Warn,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("error") => LogLevel::Error,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("crit") => LogLevel::Crit,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("alert") => LogLevel::Alert,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("emerg") => LogLevel::Emerg,
        _ => LogLevel::Info,
    }
}

fn collect_tags(root: &Value) -> Vec<(String, String)> {
    let mut tags = vec![("lang".to_string(), "Rust".to_string())];
    if let Value::Object(fields) = root {
        for (key, value) in fields {
            if !matches!(key.as_str(), "ts" | "level" | "msg" | "att") {
                tags.push((key.clone(), value_to_string(value)));
            }
        }
        if let Some(Value::Object(attributes)) = fields.get("att") {
            for (key, value) in attributes {
                tags.push((key.clone(), value_to_string(value)));
            }
        }
    }
    tags
}

fn parse_json(raw_data: &[String]) -> Result<Vec<ParsedEntry>, ParseError> {
    #[cfg(feature = "experiment-one-message")]
    let mut combined_message = String::with_capacity(raw_data.len() * 917);
    let mut entries = Vec::with_capacity(raw_data.len());
    for raw in raw_data {
        let root: Value = serde_json::from_str(raw)
            .map_err(|_| ParseError::InvalidFormat("JSON Parse Error".to_string()))?;

        #[cfg(feature = "experiment-one-message")]
        {
            combined_message.push_str(root.get("msg").and_then(Value::as_str).unwrap_or(""));
            continue;
        }

        #[cfg(feature = "experiment-empty-list")]
        {
            // Parse and validate every input, but return no ABI output. This
            // isolates H2..H3 when there is no result list to lower/lift.
            drop(root);
            continue;
        }

        #[cfg(feature = "experiment-compact-record")]
        {
            // Preserve one ParsedEntry per input while removing all dynamic
            // string and nested-list payloads.
            drop(root);
            entries.push(ParsedEntry {
                timestamp: String::new(),
                level: LogLevel::Info,
                message: String::new(),
                tags: Vec::new(),
                targettag: String::new(),
            });
            continue;
        }

        #[cfg(any(
            feature = "experiment-message-only",
            feature = "experiment-message-256"
        ))]
        {
            // Preserve the large message payload but remove the other strings
            // and the nested tag list.
            let full_message = root.get("msg").and_then(Value::as_str).unwrap_or("");
            #[cfg(feature = "experiment-message-only")]
            let message = full_message.to_string();
            #[cfg(feature = "experiment-message-256")]
            let message = full_message
                .get(..256)
                .expect("the deterministic ASCII message exceeds 256 bytes")
                .to_string();
            entries.push(ParsedEntry {
                timestamp: String::new(),
                level: LogLevel::Info,
                message,
                tags: Vec::new(),
                targettag: String::new(),
            });
            continue;
        }

        #[cfg(not(any(
            feature = "experiment-empty-list",
            feature = "experiment-compact-record",
            feature = "experiment-message-only",
            feature = "experiment-message-256",
            feature = "experiment-one-message"
        )))]
        {
            let timestamp = root
                .get("ts")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let level = map_level(root.get("level"));
            let message = root
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let tags = collect_tags(&root);
            entries.push(ParsedEntry {
                timestamp,
                level,
                message,
                tags,
                targettag: "A".to_string(),
            });
        }
    }

    #[cfg(feature = "experiment-one-message")]
    if !raw_data.is_empty() {
        entries.push(ParsedEntry {
            timestamp: String::new(),
            level: LogLevel::Info,
            message: combined_message,
            tags: Vec::new(),
            targettag: String::new(),
        });
    }

    Ok(entries)
}

impl Guest for ParseBenchmark {
    fn parse(raw_data: Vec<String>) -> Result<Vec<ParsedEntry>, ParseError> {
        // Marker 1: all input lowering/binding work has completed.
        time_sign();
        let result = parse_json(&raw_data);
        // Marker 2: JSON parsing and output construction have completed;
        // output lowering/lifting begins after this function returns.
        time_sign();
        result
    }

    fn report_usage() -> u64 {
        0
    }
}

export!(ParseBenchmark);
