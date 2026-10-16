// probe.rs — pattern_gaps G6 的能力探測（filter_chaos／format_chaos／transport_chaos 共用同一份內容）。
//
// 與 parse_chaos 的 FAULT_FILE／FAULT_ENV／FAULT_NET 語意相同，只是在其他 stage 觸發：
//   FAULT_FILE  讀 /etc/passwd（std::fs → wasi:filesystem）；inert 時沒有 preopen，應失敗
//   FAULT_ENV   列出環境變數（wasi:cli/environment）；host 不注入任何值，應為空
//   FAULT_NET   TCP connect（std::net → wasi:sockets）到 127.0.0.1:8080（實驗 sink，確實在 listen）
//               與 127.0.0.1:8081（不在 endpoint map）
// 每次探測把實際觀察寫到 stderr（host 繼承 → dut.log）：
//   [<plugin>] PROBE <kind> target=... outcome=DENIED|LEAK|DENIED(empty)|CONNECTED detail=...
// 因為 std::fs／std::net 被使用，component 會 import wasi:filesystem 與 wasi:sockets，
// 預設 deny 時整個 plugin 應在載入期被拒絕（與 parse_chaos 相同）。

pub fn probe_file(who: &str) {
    let r = std::fs::read_to_string("/etc/passwd");
    let detail = match &r {
        Ok(s) => format!("read {} bytes of /etc/passwd", s.len()),
        Err(e) => format!("kind={:?} err={e}", e.kind()),
    };
    eprintln!("[{who}] PROBE file path=/etc/passwd outcome={} detail={detail}",
              if r.is_ok() { "LEAK" } else { "DENIED" });
}

pub fn probe_env(who: &str) {
    let vars: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    let bytes: usize = vars.iter().map(|s| s.len()).sum();
    eprintln!("[{who}] PROBE env vars={} bytes={} outcome={}", vars.len(), bytes,
              if vars.is_empty() { "DENIED(empty)" } else { "LEAK" });
}

/// raw TCP（wasi:sockets）探測；回傳是否連上。
pub fn probe_tcp(who: &str, target: &str, label: &str) -> bool {
    let r = std::net::TcpStream::connect(target);
    let detail = match &r {
        Ok(s) => format!("connected local={:?}", s.local_addr().ok()),
        Err(e) => format!("kind={:?} err={e}", e.kind()),
    };
    eprintln!("[{who}] PROBE net_tcp target={target} ({label}) outcome={} detail={detail}",
              if r.is_ok() { "CONNECTED" } else { "DENIED" });
    r.is_ok()
}

pub fn probe_net(who: &str) {
    probe_tcp(who, "127.0.0.1:8080", "listed-endpoint");
    probe_tcp(who, "127.0.0.1:8081", "not-listed");
}

/// 依 marker 觸發探測；回傳是否有任何 marker。
pub fn probe_marker(who: &str, msg: &str) -> bool {
    if msg.contains("FAULT_NET") {
        probe_net(who);
        true
    } else if msg.contains("FAULT_FILE") {
        probe_file(who);
        true
    } else if msg.contains("FAULT_ENV") {
        probe_env(who);
        true
    } else {
        false
    }
}
