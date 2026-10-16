// 與 host 端 wasmtime bindgen 產生的型別同名、同欄位的純 Rust 結構。
// src/native.rs 以 `crate::app::local::log_process::pipeline_process::*` 引用型別，
// 這個 shim 讓本 crate 能用 #[path] 直接編譯同一份 native.rs，
// 確保微基準量到的邏輯與 host 內建原生 stage 完全相同。
pub mod local {
    pub mod log_process {
        pub mod pipeline_process {
            #[repr(u8)]
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub enum LogLevel {
                Debug,
                Info,
                Warn,
                Error,
                Crit,
                Alert,
                Emerg,
            }

            impl LogLevel {
                pub fn from_u8(v: u8) -> LogLevel {
                    match v {
                        0 => LogLevel::Debug,
                        1 => LogLevel::Info,
                        2 => LogLevel::Warn,
                        3 => LogLevel::Error,
                        4 => LogLevel::Crit,
                        5 => LogLevel::Alert,
                        _ => LogLevel::Emerg,
                    }
                }
            }

            #[derive(Clone, Debug)]
            pub struct ParsedEntry {
                pub timestamp: String,
                pub level: LogLevel,
                pub message: String,
                pub tags: Vec<(String, String)>,
                pub targettag: String,
            }

            #[derive(Clone, Debug)]
            pub struct LogEntry {
                pub id: u64,
                pub timestamp: String,
                pub level: LogLevel,
                pub message: String,
                pub tags: Vec<(String, String)>,
            }

            #[derive(Clone, Copy, Debug)]
            pub struct FilterResult {
                pub id: u64,
                pub keep: bool,
            }
        }
    }
}
