use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicUsize, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::capability::{CapabilityConfig, Stage};

// ── App-level config (YAML) ───────────────────────────────────────────────────

/// Top-level configuration loaded from forwarder.yaml.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub plugins: PluginsConfig,
    pub stages: PipelineStages,
    pub input: InputConfig,
    pub batch: BatchConfigRaw,
    pub endpoint: EndpointSet,
    #[serde(default = "default_reload_secs")]
    pub config_reload_secs: u64,
    /// 每個 stage 的 WASI 能力明確授權（未設定＝全部拒絕，見 capability.rs）。
    #[serde(default)]
    pub capabilities: CapabilityConfig,
}

fn default_reload_secs() -> u64 { 10 }
fn default_pipeline_channel_capacity() -> usize { 20_000 }
fn default_stage_timeout_ms() -> u64 { 30_000 }

/// Paths to each WASM plugin. Relative paths are resolved against the config file's directory.
///
/// 欄位留空、省略或填 `native` 時，該 stage 改用 host 內建的 Rust 原生實作
/// （native.rs，邏輯與 test-plugins/rust-plugin 相同、不經過 WASM 邊界）；
/// 有填 .wasm 路徑時維持原本的 WCM plugin 路徑，行為完全不變。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PluginsConfig {
    #[serde(default)]
    pub parse: String,
    pub parse_noop: Option<String>,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub transport: String,
}

/// 代表「使用 host 內建原生 stage」的保留字。
pub const NATIVE_STAGE: &str = "native";

/// plugin 欄位是否代表內建原生 stage（空字串或 `native`，不分大小寫）。
pub fn is_native_stage(path: &str) -> bool {
    let p = path.trim();
    p.is_empty() || p.eq_ignore_ascii_case(NATIVE_STAGE)
}

/// Which pipeline stages are active.
#[derive(Debug, Clone, Deserialize)]
pub struct PipelineStages {
    #[serde(default)]
    pub filter: bool,
    #[serde(default)]
    pub format: bool,
    #[serde(default)]
    pub transport: bool,
}

/// Input source configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct InputConfig {
    pub mode: InputMode,
    pub tcp: Option<TcpInputConfig>,
    pub tail: Option<TailInputConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    Tcp,
    Tail,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TcpInputConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TailInputConfig {
    pub path: String,
}

/// YAML-friendly batch config (Duration expressed as milliseconds).
#[derive(Debug, Clone, Deserialize)]
pub struct BatchConfigRaw {
    pub mem_limit_mb: usize,
    /// parse dispatcher 累積 buffer 的觸發閾值（MB），達到即 flush。
    /// 必須比 mem_limit_mb 小 30% 以上，保留 WASM parse 中間配置空間。
    pub buffer_size_mb: usize,
    pub max_wait_ms: u64,
    pub channel_capacity: usize,
    pub max_format_chunk: usize,
    #[serde(default)]
    //pub transport_endpoint: Option<String>,
    pub max_transport_bytes: usize,
    pub transport_workers: usize,
    /// parse→filter / filter→format / format→transport channel 容量。
    #[serde(default = "default_pipeline_channel_capacity")]
    pub pipeline_channel_capacity: usize,
    /// 每個階段 WASM 呼叫的超時上限（毫秒）；超過後自動重試，重試 2 次均失敗則寫入 error.txt。
    #[serde(default = "default_stage_timeout_ms")]
    pub stage_timeout_ms: u64,
    /// format / transport 是否重複使用 WASM 實例（parse/filter 一律重用）。
    /// true（預設）＝常駐重用，省去每批 instantiate 開銷；
    /// false ＝每批用完即銷毀重建，防止 GC 語言插件（Go/C# 等）記憶體累積導致 OOM。
    #[serde(default = "default_plugin_instance_reuse")]
    pub plugin_instance_reuse: bool,
    /// parse 失敗時是否以二分法逐筆隔離（isolation.rs）。
    /// true（預設）＝只拒絕真正 malformed／poison 的行，其餘照常輸出；
    /// false ＝舊行為（parse-error 整批丟棄、trap 重試後整批寫入 error.txt），
    /// 保留此開關只為了在同一個 binary 上重現修正前的 C3 結果做前後對照。
    #[serde(default = "default_parse_isolation")]
    pub parse_isolation: bool,
    /// 處理模式：batch（預設，舊行為）＝累積原始輸入到 buffer_size_mb／max_wait_ms 才整批處理；
    /// streaming ＝輸入端以 micro-batch 立即處理，輸出端在 router 依 max_wait_ms 週期 flush。
    #[serde(default)]
    pub processing_mode: ProcessingMode,
    /// streaming 模式下每個 micro-batch 的最大筆數（達到即送 parse）。
    #[serde(default = "default_micro_batch_records")]
    pub micro_batch_records: usize,
    /// streaming 模式下 micro-batch 自第一筆到達起最多等待的毫秒數。
    #[serde(default = "default_micro_batch_max_wait_ms")]
    pub micro_batch_max_wait_ms: u64,
    /// 是否以 epoch interruption 作為 stage watchdog（預設 true，行為不變）。
    /// false ＝ engine 不開啟 epoch interruption（不插入 epoch 檢查、不啟動 ticker、
    /// 不設任何 deadline），stage_timeout_ms 因此失效（可設 0）。只供量測 epoch 開銷
    /// 與「無 watchdog」消融實驗使用：此時卡死的 plugin 會讓該 stage 永久停住。
    #[serde(default = "default_epoch_interruption")]
    pub epoch_interruption: bool,
}

/// 處理模式（見 BatchConfigRaw::processing_mode）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessingMode {
    #[default]
    Batch,
    Streaming,
}

fn default_micro_batch_records() -> usize { 64 }
fn default_micro_batch_max_wait_ms() -> u64 { 5 }

fn default_plugin_instance_reuse() -> bool {
   true
}

fn default_parse_isolation() -> bool {
    true
}

fn default_epoch_interruption() -> bool {
    true
}

impl From<BatchConfigRaw> for BatchConfig {
    fn from(r: BatchConfigRaw) -> Self {
        BatchConfig {
            mem_limit_mb: r.mem_limit_mb,
            buffer_size_mb: r.buffer_size_mb,
            max_wait: Duration::from_millis(r.max_wait_ms),
            channel_capacity: r.channel_capacity,
            max_format_chunk: r.max_format_chunk,
            // transport_endpoint: r.transport_endpoint,
            max_transport_bytes: r.max_transport_bytes,
            transport_workers: r.transport_workers,
            pipeline_channel_capacity: r.pipeline_channel_capacity,
            stage_timeout_ms: r.stage_timeout_ms,
            plugin_instance_reuse: r.plugin_instance_reuse,
            parse_isolation: r.parse_isolation,
            processing_mode: r.processing_mode,
            micro_batch_records: r.micro_batch_records,
            micro_batch_max_wait: Duration::from_millis(r.micro_batch_max_wait_ms),
            epoch_interruption: r.epoch_interruption,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EndpointSet(pub HashMap<String, String>);

/// Load and parse forwarder.yaml (or any path).
pub fn load_app_config(path: &str) -> Result<AppConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read config '{}': {}", path, e))?;
    let cfg: AppConfig = serde_yaml::from_str(&content)
        .map_err(|e| format!("cannot parse config '{}': {}", path, e))?;
    cfg.capabilities
        .validate()
        .map_err(|e| format!("invalid config '{}': {}", path, e))?;
    Ok(cfg)
}

/// Shared references to each plugin's live runtime slot.
/// Passed to `spawn_config_watcher` so it can rebuild plugins in-place on path change.
/// 使用內建原生 stage 的欄位為 None，watcher 不會對它熱更換。
pub struct PluginSlots {
    pub parse: Option<crate::app::SharedPlugin>,
    pub parse_noop: Option<crate::app::SharedPlugin>,
    pub filter: Option<crate::app::SharedPlugin>,
    pub format: Option<crate::app::SharedPlugin>,
    pub transport: Option<crate::app::SharedPlugin>,
}

/// Spawn a background thread that reloads the config file every `config_reload_secs` seconds.
///
/// - Batch parameters take effect on the next batch.
/// - Plugin path changes OR mtime changes (same-path wasm replacement) trigger
///   an in-place rebuild; the pipeline detects the incremented version and swaps
///   automatically between batches.
pub fn spawn_config_watcher(path: String, shared: Arc<RwLock<AppConfig>>, slots: PluginSlots) {
    // Use filter to avoid turning an empty parent ("") into "/" when joining.
    let config_dir = Path::new(&path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    thread::spawn(move || {
        // Use Path::join so an empty config_dir doesn't produce a spurious leading '/'.
        let resolve = |p: &str| -> String {
            let pp = Path::new(p);
            if pp.is_absolute() {
                p.to_string()
            } else {
                Path::new(&config_dir).join(pp).to_string_lossy().to_string()
            }
        };

        let file_mtime = |p: &str| -> Option<std::time::SystemTime> {
            std::fs::metadata(p).ok()?.modified().ok()
        };

        // Record initial resolved paths and mtimes.
        // Hot-swap triggers when either the path string or the file mtime changes.
        let (mut cur_parse, mut cur_parse_noop, mut cur_filter, mut cur_format, mut cur_transport) = {
            let cfg = shared.read().unwrap();
            (
                resolve(&cfg.plugins.parse),
                cfg.plugins.parse_noop.as_deref().map(|p| resolve(p)),
                resolve(&cfg.plugins.filter),
                resolve(&cfg.plugins.format),
                resolve(&cfg.plugins.transport),
            )
        };
        let mut mt_parse      = file_mtime(&cur_parse);
        let mut mt_parse_noop = cur_parse_noop.as_deref().and_then(|p| file_mtime(p));
        let mut mt_filter     = file_mtime(&cur_filter);
        let mut mt_format     = file_mtime(&cur_format);
        let mut mt_transport  = file_mtime(&cur_transport);

        loop {
            let reload_secs = shared.read().unwrap().config_reload_secs;
            thread::sleep(Duration::from_secs(reload_secs));

            match load_app_config(&path) {
                Ok(new_cfg) => {
                    // 先驗證新設定，通過才套用；否則印出錯誤並繼續以舊設定執行。
                    if let Err(e) = BatchConfig::from(new_cfg.batch.clone()).validate_and_describe() {
                        eprintln!("[config] reload rejected（沿用舊設定）: {}", e);
                        continue;
                    }
                    let new_plugins = new_cfg.plugins.clone();
                    let caps = new_cfg.capabilities.clone();
                    *shared.write().unwrap() = new_cfg;

                    let mut any_plugin_changed = false;

                    // ── parse ──────────────────────────────────────────────────
                    {
                        let new = resolve(&new_plugins.parse);
                        let new_mt = file_mtime(&new);
                        if let Some(slot) = slots.parse.as_ref().filter(|_| new != cur_parse || new_mt != mt_parse) {
                            any_plugin_changed = true;
                            if crate::app::rebuild_shared_slot(slot, &new, Stage::Parse, caps.parse, "parse") {
                                cur_parse = new;
                                mt_parse = new_mt;
                            }
                        }
                    }

                    // ── parse_noop ─────────────────────────────────────────────
                    if let (Some(slot), Some(new_raw)) =
                        (&slots.parse_noop, &new_plugins.parse_noop)
                    {
                        let new = resolve(new_raw);
                        let new_mt = file_mtime(&new);
                        let old = cur_parse_noop.as_deref().unwrap_or("");
                        if new != old || new_mt != mt_parse_noop {
                            any_plugin_changed = true;
                            if crate::app::rebuild_shared_slot(slot, &new, Stage::Parse, caps.parse, "parse_noop") {
                                cur_parse_noop = Some(new);
                                mt_parse_noop = new_mt;
                            }
                        }
                    }

                    // ── filter ─────────────────────────────────────────────────
                    if let Some(ref slot) = slots.filter {
                        let new = resolve(&new_plugins.filter);
                        let new_mt = file_mtime(&new);
                        if new != cur_filter || new_mt != mt_filter {
                            any_plugin_changed = true;
                            if crate::app::rebuild_shared_slot(slot, &new, Stage::Filter, caps.filter, "filter") {
                                cur_filter = new;
                                mt_filter = new_mt;
                            }
                        }
                    }

                    // ── format ─────────────────────────────────────────────────
                    if let Some(ref slot) = slots.format {
                        let new = resolve(&new_plugins.format);
                        let new_mt = file_mtime(&new);
                        if new != cur_format || new_mt != mt_format {
                            any_plugin_changed = true;
                            if crate::app::rebuild_shared_slot(slot, &new, Stage::Format, caps.format, "format") {
                                cur_format = new;
                                mt_format = new_mt;
                            }
                        }
                    }

                    // ── transport ──────────────────────────────────────────────
                    if let Some(ref slot) = slots.transport {
                        let new = resolve(&new_plugins.transport);
                        let new_mt = file_mtime(&new);
                        if new != cur_transport || new_mt != mt_transport {
                            any_plugin_changed = true;
                            if crate::app::rebuild_shared_slot(slot, &new, Stage::Transport, caps.transport, "transport") {
                                cur_transport = new;
                                mt_transport = new_mt;
                            }
                        }
                    }

                    if any_plugin_changed {
                        eprintln!("[config] plugin swap(s) complete — pipeline will apply on next batch");
                    } else {
                        eprintln!("[config] batch params reloaded (no plugin changes)");
                    }
                }
                Err(e) => eprintln!("[config] reload failed: {}", e),
            }
        }
    });
}

#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub mem_limit_mb: usize,
    /// parse dispatcher 累積 buffer 的觸發閾值（MB），達到即 flush。
    pub buffer_size_mb: usize,
    pub max_wait: Duration,
    pub channel_capacity: usize,
    /// 每次呼叫 format plugin 的最大 entry 數。
    /// TinyGo GC 在大批次時無法及時回收中間字串緩衝區，需分批呼叫。
    pub max_format_chunk: usize,
    /// Transport plugin 目標端點 URL（已由 endpoint map 取代，保留供相容）。
    // pub transport_endpoint: Option<String>,
    /// 每次呼叫 transport send() 的最大累積 byte 數（一次對應一個 HTTP POST）。
    /// WASI blocking-write-and-flush 單次寫入上限為 4096 B（sync/async 皆適用）；
    /// plugin 必須在內部分批寫入 HTTP body（每次 ≤ 4096 B）。
    /// 此值控制每次 send() 傳入的資料總量，預設 32 KB，確保 plugin 寫入安全。
    /// 若 plugin 已正確實作分批寫入，可大幅調高（例如 256 KB）以減少 POST 次數。
    pub max_transport_bytes: usize,
    /// 平行 transport worker 數量。每個 worker 持有獨立 WASM store，
    /// 共用同一個 FormattedBatch channel，允許同時進行多個 HTTP POST。
    pub transport_workers: usize,
    /// parse→filter / filter→format / format→transport 三條 pipeline channel 的共用容量。
    pub pipeline_channel_capacity: usize,
    /// 每個階段 WASM 呼叫的超時上限（毫秒）；超過後自動重試（最多 2 次），均失敗則寫入 error.txt。
    pub stage_timeout_ms: u64,
    /// format / transport 是否重複使用 WASM 實例（parse/filter 一律重用）。
    /// true（預設）＝常駐重用；false ＝每批銷毀重建（GC 語言插件防 OOM 用）。
    pub plugin_instance_reuse: bool,
    /// parse 失敗時是否以二分法逐筆隔離（false ＝舊版整批丟棄，僅供前後對照）。
    pub parse_isolation: bool,
    /// batch（預設）或 streaming（micro-batch 輸入 + router 週期 flush）。
    pub processing_mode: ProcessingMode,
    /// streaming：micro-batch 筆數上限。
    pub micro_batch_records: usize,
    /// streaming：micro-batch 自第一筆起的等待上限。
    pub micro_batch_max_wait: Duration,
    /// epoch interruption watchdog（false ＝ 停用，stage_timeout_ms 失效；僅供消融實驗）。
    pub epoch_interruption: bool,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            mem_limit_mb: 256,
            buffer_size_mb: 128,
            max_wait: Duration::from_millis(5000),
            channel_capacity: 150_000,
            max_format_chunk: 50_000,
            // transport_endpoint: None,
            max_transport_bytes: 128 * 1024,
            transport_workers: 5,
            pipeline_channel_capacity: 20_000,
            stage_timeout_ms: 30_000,
            plugin_instance_reuse: true,
            parse_isolation: true,
            processing_mode: ProcessingMode::Batch,
            micro_batch_records: 64,
            micro_batch_max_wait: Duration::from_millis(5),
            epoch_interruption: true,
        }
    }
}

impl BatchConfig {
    /// 驗證所有參數是否合理，並說明每個欄位實際影響哪條程式碼路徑。
    ///
    /// 回傳 `Err(說明)` 若有明顯錯誤設定，否則印出每個參數的使用狀況後回傳 `Ok(())`。
    pub fn validate_and_describe(&self) -> Result<(), String> {
        // ── 範圍驗證 ──────────────────────────────────────────────────────
        // 每個斷言後面的 "(used at ...)" 說明這個值真正被消費的地方，
        // 方便日後確認某個欄位是否還有在用。

        // mem_limit_mb
        // used at: runtime.rs run_pipeline → mem_limit_bytes = mem_limit_mb * 1024 * 1024
        //          → MyLimiter::new(mem_limit_bytes) 限制每個 WASM store 的線性記憶體上限
        if self.mem_limit_mb < 64 {
            return Err(format!(
                "mem_limit_mb={} 過小（建議 >= 64 MB）",
                self.mem_limit_mb
            ));
        }

        // buffer_size_mb
        // used at: runtime.rs parse_dispatcher → buffer_budget = buffer_size_mb * 1024 * 1024
        //          → 當 batch 累積 bytes > buffer_budget 時觸發 flush（size trigger）
        // 必須比 mem_limit_mb 小 30% 以上：整個 batch 會送進單一 WASM store parse，
        // 需保留空間給 parse 過程的中間配置，否則 memory.grow 被 MyLimiter 拒絕導致 trap。
        if self.buffer_size_mb == 0 {
            return Err("buffer_size_mb 不能為 0".to_string());
        }
        if (self.buffer_size_mb as f64) > (self.mem_limit_mb as f64) * 0.7 {
            return Err(format!(
                "buffer_size_mb={} 與 mem_limit_mb={} 差距不足 30%，\
                 buffer_size_mb 需 <= mem_limit_mb 的 70%（本例上限 {} MB），\
                 以保留 WASM parse 中間配置空間",
                self.buffer_size_mb,
                self.mem_limit_mb,
                (self.mem_limit_mb as f64 * 0.7) as usize
            ));
        }

        // max_wait
        // used at: runtime.rs parse_loop → rx.recv_timeout(max_wait) 的 timeout 值
        //          → 觸發 time-based flush（即使 batch 未滿也會送出）
        if self.max_wait.is_zero() {
            return Err("max_wait 不能為 0（parse loop 會無限 spin）".to_string());
        }

        // channel_capacity
        // used at: main.rs → sync_channel::<LineItem>(channel_capacity)
        //          → 只控制 stdin→parse 的 LineItem channel 容量
        // 警告: ParsedBatch channel (parse→format) 和 FormattedBatch channel (format→transport)
        //       目前在 runtime.rs:56-57 硬編碼為 20_000，不受此參數控制。
        if self.channel_capacity == 0 {
            return Err("channel_capacity 不能為 0".to_string());
        }

        // max_format_chunk
        // used at: runtime.rs format_loop → pb.entries.chunks(max_format_chunk)
        //          → 將大批次拆成小塊，每塊各自建立新 WASM store 呼叫 format plugin
        //          → 目的：避免 TinyGo GC 在大批次時無法回收中間字串，導致 WASM OOM
        if self.max_format_chunk == 0 {
            return Err("max_format_chunk 不能為 0".to_string());
        }

        // transport_endpoint（選填，路由改由 endpoint map 負責）
        /* 
        if let Some(ep) = &self.transport_endpoint {
            if !ep.starts_with("http://") && !ep.starts_with("https://") {
                return Err(format!(
                    "transport_endpoint='{}' 不是合法的 HTTP/HTTPS URL",
                    ep
                ));
            }
        }
        */

        // max_transport_bytes
        // used at: runtime.rs transport_worker → while acc_bytes + line_len <= max_transport_bytes
        //          → 控制每次呼叫 transport plugin send() 的最大 byte 數
        //          → 每次 send() 對應一個 HTTP POST
        // 注意: transport WASM plugin 內部仍須以 ≤ 4096 B 分批呼叫 blocking_write_and_flush
        if self.max_transport_bytes < 4096 {
            return Err(format!(
                "max_transport_bytes={} 過小（至少需 4096 B，等於一次 WASI write）",
                self.max_transport_bytes
            ));
        }

        // transport_workers
        // used at: runtime.rs run_pipeline → for i in 0..transport_workers { spawn worker }
        //          → 每個 worker 持有獨立 WASM store，共用 rx_formatted channel（Mutex 保護）
        //          → 允許同時進行多個 HTTP POST
        if self.transport_workers == 0 {
            return Err("transport_workers 不能為 0".to_string());
        }

        // pipeline_channel_capacity
        // used at: runtime.rs run_pipeline → sync_channel::<ParsedBatch>(pipeline_channel_capacity)
        //          → 控制 parse→filter / filter→format / format→transport 三條 channel 容量
        if self.pipeline_channel_capacity == 0 {
            return Err("pipeline_channel_capacity 不能為 0".to_string());
        }

        // stage_timeout_ms / epoch_interruption
        // used at: runtime.rs filter_loop / format_loop / transport_worker
        //          → 每個 WASM 呼叫的超時上限；超過後重試（最多 2 次），均失敗則寫入 error.txt
        //          app.rs build_runtime / build_transport_runtime → Config::epoch_interruption
        // 停用 watchdog 的唯一開關是 epoch_interruption: false；此時 stage_timeout_ms 不被使用
        // （可為 0）。epoch_interruption: true 時 stage_timeout_ms 必須 > 0（deadline 0 tick 會立即 trap）。
        if self.epoch_interruption {
            if self.stage_timeout_ms == 0 {
                return Err("stage_timeout_ms 不能為 0（要停用 watchdog 請設 epoch_interruption: false）".to_string());
            }
        } else {
            eprintln!(
                "[config] WARNING epoch_interruption=false：stage watchdog 停用（無 epoch 檢查、無 deadline），\
                 stage_timeout_ms={} 不生效；卡死的 plugin 會讓該 stage 永久停住（僅供消融實驗）",
                self.stage_timeout_ms
            );
        }

        // processing_mode / micro_batch_records / micro_batch_max_wait_ms
        // used at: runtime.rs DispatchParams::from_config → parse_dispatcher 的 flush 條件
        //          runtime.rs transport_router → streaming 時改為每 max_wait_ms 週期 flush
        // batch 模式下兩個 micro_batch_* 欄位不被使用。
        if self.processing_mode == ProcessingMode::Streaming {
            if self.micro_batch_records == 0 || self.micro_batch_records as u64 >= 100_000 {
                return Err(format!(
                    "micro_batch_records={} 需介於 1..100000（host id 每批跨距 100,000）",
                    self.micro_batch_records
                ));
            }
            if self.micro_batch_max_wait.is_zero() {
                return Err("micro_batch_max_wait_ms 不能為 0".to_string());
            }
            eprintln!(
                "[config] processing_mode=streaming：輸入端 micro-batch（{} 筆 或 第一筆後 {} ms 即送 parse）；\
                 輸出端 router 每 max_wait_ms={} ms 週期 flush（同 Fluent Bit 全域 Flush），\
                 或 endpoint buffer 達 max_transport_bytes={} B 時立即送出",
                self.micro_batch_records,
                self.micro_batch_max_wait.as_millis(),
                self.max_wait.as_millis(),
                self.max_transport_bytes,
            );
        } else {
            eprintln!(
                "[config] processing_mode=batch：累積原始輸入至 buffer_size_mb={} MB 或 max_wait_ms={} ms 後整批處理\
                 （micro_batch_* 不使用）",
                self.buffer_size_mb,
                self.max_wait.as_millis(),
            );
        }

        Ok(())
    }

    /// 以表格形式印出每個參數的值、用途、以及是否為預設值。
    pub fn print_config_table(&self) {
        let default = BatchConfig::default();
        let rows: &[(&str, String, &str, bool)] = &[
            (
                "mem_limit_mb",
                format!("{} MB", self.mem_limit_mb),
                "每個 WASM store 線性記憶體上限 (runtime.rs MyLimiter)",
                self.mem_limit_mb == default.mem_limit_mb,
            ),
            (
                "buffer_size_mb",
                format!("{} MB", self.buffer_size_mb),
                "parse buffer 大小觸發 flush 的閾值 (runtime.rs parse_dispatcher)",
                self.buffer_size_mb == default.buffer_size_mb,
            ),
            (
                "max_wait",
                format!("{} ms", self.max_wait.as_millis()),
                "parse 時間觸發 flush 的等待上限 (runtime.rs parse_loop recv_timeout)",
                self.max_wait == default.max_wait,
            ),
            (
                "channel_capacity",
                format!("{}", self.channel_capacity),
                "stdin→parse LineItem channel 容量",
                self.channel_capacity == default.channel_capacity,
            ),
            (
                "max_format_chunk",
                format!("{}", self.max_format_chunk),
                "format plugin 每次呼叫的最大 entry 數 (runtime.rs format_loop chunks)",
                self.max_format_chunk == default.max_format_chunk,
            ),
            /*
            (
                "transport_endpoint",
                self.transport_endpoint.clone().unwrap_or_else(|| "(via endpoint map)".to_string()),
                "transport plugin init() 目標 URL（已由 endpoint map 取代）",
                self.transport_endpoint == default.transport_endpoint,
            ),
            */
            (
                "max_transport_bytes",
                format!("{} KB", self.max_transport_bytes / 1024),
                "每次 transport send() 的最大 byte 數 = 一個 HTTP POST 大小",
                self.max_transport_bytes == default.max_transport_bytes,
            ),
            (
                "transport_workers",
                format!("{}", self.transport_workers),
                "並行 transport worker 數量，每個持有獨立 WASM store",
                self.transport_workers == default.transport_workers,
            ),
            (
                "pipeline_channel_capacity",
                format!("{}", self.pipeline_channel_capacity),
                "parse→filter / filter→format / format→transport 三條 channel 容量",
                self.pipeline_channel_capacity == default.pipeline_channel_capacity,
            ),
            (
                "stage_timeout_ms",
                format!("{} ms", self.stage_timeout_ms),
                "每階段 WASM 呼叫超時上限，超過後重試 2 次，均失敗寫 error.txt",
                self.stage_timeout_ms == default.stage_timeout_ms,
            ),
            (
                "epoch_interruption",
                format!("{}", self.epoch_interruption),
                "epoch watchdog；false 時 stage_timeout_ms 不生效（消融實驗用）",
                self.epoch_interruption == default.epoch_interruption,
            ),
        ];

        eprintln!("┌{:─<28}┬{:─<14}┬{:─<7}┬{:─<58}┐", "", "", "", "");
        eprintln!(
            "│ {:26} │ {:12} │ {:5} │ {:56} │",
            "參數", "值", "預設", "用途"
        );
        eprintln!("├{:─<28}┼{:─<14}┼{:─<7}┼{:─<58}┤", "", "", "", "");
        for (name, val, desc, is_default) in rows {
            eprintln!(
                "│ {:26} │ {:12} │ {:5} │ {:56} │",
                name,
                val,
                if *is_default { "✓" } else { "●" },
                desc
            );
        }
        eprintln!("└{:─<28}┴{:─<14}┴{:─<7}┴{:─<58}┘", "", "", "", "");
    }
}

/// 追蹤 channel 目前積壓的條數與 byte 數。
/// Clone 後共享同一組 Atomic，可分別交給 sender thread 和 receiver thread。
#[derive(Clone)]
pub struct ChannelStats {
    count: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
}

impl ChannelStats {
    pub fn new() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
            bytes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// 送出一條後呼叫（send() 成功回傳後才呼叫，以反映真實 buffer 狀態）
    pub fn on_send(&self, byte_len: usize) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(byte_len, Ordering::Relaxed);
    }

    /// 收到一條後立刻呼叫（取出時就減，反映已離開 buffer）
    pub fn on_recv(&self, byte_len: usize) {
        self.count.fetch_sub(1, Ordering::Relaxed);
        self.bytes.fetch_sub(byte_len, Ordering::Relaxed);
    }

    /// 回傳 (條數, byte 數) 快照
    pub fn snapshot(&self) -> (usize, usize) {
        (
            self.count.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        )
    }
}

pub struct Batch {
    pub lines: Vec<String>,
    pub bytes: usize,
    pub created_at: Instant,
}

impl Batch {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            bytes: 0,
            created_at: Instant::now(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
    pub fn len(&self) -> usize {
        self.lines.len()
    }
    pub fn elapsed(&self) -> Duration {
        self.created_at.elapsed()
    }
    pub fn push(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push(line);
    }
    pub fn clear(&mut self) {
        self.lines.clear();
        self.bytes = 0;
        self.created_at = Instant::now();
    }
}

pub struct FlushReason {
    pub size: bool,
    pub time: bool,
    pub eof: bool,
}

// ── 統計結構 ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
pub struct ParseDiffTiming {
    pub noop_elapsed_ns: u64,
    pub noop_component_ns: u64,
    pub copy_in_ns: u64,
    pub guest_ns: u64,
    pub copy_out_ns: u64,
}

#[derive(Default)]
pub struct ParseStats {
    pub total_batches: u64,
    pub total_input_lines: u64,
    pub total_input_bytes: u64,
    pub total_output_entries: u64,
    pub total_elapsed: Duration,
    /// Go heap 峰值（各 batch 最大值）
    pub go_heap_peak_max: u64,
    /// WASM 線性記憶體峰值（各 batch 最大值）
    pub wasm_mem_peak_max: usize,
    /// 累計 memory.grow 觸發次數（各 batch 加總）
    pub total_grow_count: u64,
    /// 累計 memory.grow 增量 bytes（各 batch 加總）
    pub total_grow_delta_bytes: u64,
    /// 累計 component 內部回報的執行時間 ns（各 batch 加總）
    pub total_component_ns: u64,
    /// 差分量測成功的 batch 數。
    pub total_diff_batches: u64,
    /// no-op parser 的整段 call_parse() 時間，用於估算 host→WCM copy-in。
    pub total_noop_elapsed_ns: u64,
    /// no-op parser 回報的 guest 端空邏輯時間。
    pub total_noop_component_ns: u64,
    /// 差分估算的 host→WCM copy-in 時間。
    pub total_copy_in_ns: u64,
    /// 差分估算的 WCM→host copy-out 時間。
    pub total_copy_out_ns: u64,
    /// 觸發逐筆隔離（二分法）的批次數。
    pub total_isolated_batches: u64,
    /// 隔離後被保留下來、正常往下游送的行數（只計入有錯誤的批次）。
    pub total_salvaged_lines: u64,
    /// 隔離後被拒絕、寫入 error.txt 的行數（malformed／poison／預算耗盡）。
    pub total_rejected_lines: u64,
    /// 隔離過程額外呼叫 parse 的次數（不含最初失敗的整批呼叫）。
    pub total_isolation_calls: u64,
    /// trap 隔離預算耗盡、整段拒絕的次數。
    pub total_isolation_budget_exhausted: u64,
}

#[derive(Default)]
pub struct FormatStats {
    pub total_batches: u64,
    pub total_input_entries: u64,
    pub total_output_lines: u64,
    pub total_elapsed: Duration,
    /// WASM 線性記憶體峰值（各 batch 最大值）
    pub wasm_mem_peak_max: usize,
    /// 累計 component 內部回報的編碼邏輯時間 ns（各 batch 加總）
    pub total_component_ns: u64,
}

#[derive(Default)]
pub struct TransportStats {
    pub total_batches: u64,
    /// 傳送的格式化行數
    pub total_input_lines: u64,
    /// 傳送的總 byte 數（格式化輸出大小）
    pub total_input_bytes: u64,
    /// transport plugin report-usage() 回報的累計 byte 數
    pub total_bytes_reported: u64,
    pub total_elapsed: Duration,
    /// WASM 線性記憶體峰值
    pub wasm_mem_peak_max: usize,
}

#[derive(Default)]
pub struct FilterStats {
    pub total_batches: u64,
    pub total_input_entries: u64,
    pub total_kept_entries: u64,
    pub total_dropped_entries: u64,
    pub total_elapsed: Duration,
    pub wasm_mem_peak_max: usize,
    pub total_component_ns: u64,
}

#[cfg(test)]
mod epoch_switch_tests {
    use super::*;

    const BASE: &str = "mem_limit_mb: 256\nbuffer_size_mb: 128\nmax_wait_ms: 100\nchannel_capacity: 1000\n\
max_format_chunk: 1000\nmax_transport_bytes: 1048576\ntransport_workers: 1\n";

    fn parse(extra: &str) -> BatchConfig {
        let raw: BatchConfigRaw = serde_yaml::from_str(&format!("{}{}", BASE, extra)).unwrap();
        BatchConfig::from(raw)
    }

    #[test]
    fn epoch_interruption_defaults_to_true() {
        let b = parse("");
        assert!(b.epoch_interruption);
        assert_eq!(b.stage_timeout_ms, 30_000);
        assert!(BatchConfig::default().epoch_interruption);
        assert!(b.validate_and_describe().is_ok());
    }

    #[test]
    fn zero_stage_timeout_rejected_while_watchdog_on() {
        let b = parse("stage_timeout_ms: 0\n");
        let err = b.validate_and_describe().unwrap_err();
        assert!(err.contains("epoch_interruption"), "{}", err);
    }

    #[test]
    fn zero_stage_timeout_allowed_when_watchdog_off() {
        let b = parse("stage_timeout_ms: 0\nepoch_interruption: false\n");
        assert!(!b.epoch_interruption);
        assert!(b.validate_and_describe().is_ok());
    }
}
