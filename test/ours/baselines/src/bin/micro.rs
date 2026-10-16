// No-WASM 微基準：與 copy_overhead 的 WCM／Core JSON／Core Direct 相同的輸入與取樣方式，
// 量測兩個新方法：
//   native     ：同一 process 內直接呼叫 src/native.rs（借用輸入，copy-in／copy-out = 0）
//   subprocess ：常駐 child process（stage_child），經 stdin／stdout pipe 以
//                length-prefixed framing 傳遞批次（framing 見 src/common.rs）
//   spawn      ：（附加）child 從 spawn 到回應第一個 ping 的時間，對照 WCM 的 instance 建立成本
//
// 計時點與 WCM 實驗對齊：
//   H0 host 開始送出輸入前（subprocess：開始編碼 request 前）
//   H1 業務邏輯開始（subprocess：child 讀完並解碼成 owned 結構後）
//   H2 業務邏輯完成（subprocess：child 開始編碼回應前）
//   H3 host 取得 owned 結果後（subprocess：讀完並解碼回應後）
// copy_in = H1-H0、guest = H2-H1、copy_out = H3-H2、total = H3-H0。
//
// 用法：micro --method native|subprocess|spawn --stage parse|filter|format --run N
//              [--rounds 100] [--warmup 10] [--batches 64,128,...] [--child PATH]
// 輸出：CSV 到 stdout。

#![allow(dead_code)]

#[path = "../shim.rs"]
mod app;
#[path = "../common.rs"]
mod common;
#[path = "../../../../../src/native.rs"]
mod native;

use std::hint::black_box;
use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use common::*;

struct Args {
    method: String,
    stage: String,
    run: u32,
    rounds: u32,
    warmup: u32,
    batches: Vec<usize>,
    child: String,
}

fn parse_args() -> Args {
    let mut a = Args {
        method: "native".into(),
        stage: "parse".into(),
        run: 1,
        rounds: 100,
        warmup: 10,
        batches: vec![64, 128, 256, 512, 1024, 2048, 4096, 8192],
        child: std::env::current_exe()
            .unwrap()
            .with_file_name("stage_child")
            .to_string_lossy()
            .to_string(),
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let val = v.get(i + 1).cloned().unwrap_or_default();
        match v[i].as_str() {
            "--method" => a.method = val,
            "--stage" => a.stage = val,
            "--run" => a.run = val.parse().unwrap(),
            "--rounds" => a.rounds = val.parse().unwrap(),
            "--warmup" => a.warmup = val.parse().unwrap(),
            "--batches" => a.batches = val.split(',').map(|s| s.parse().unwrap()).collect(),
            "--child" => a.child = val,
            other => panic!("unknown arg {other}"),
        }
        i += 2;
    }
    a
}

struct Sample {
    copy_in: u64,
    guest: u64,
    copy_out: u64,
    total: u64,
    output_count: usize,
}

/// 同 process 直接呼叫原生函式。
fn native_sample(stage: &str, lines: &[String], entries: &[app::local::log_process::pipeline_process::LogEntry]) -> Sample {
    let h0 = now_ns();
    let count = match stage {
        "parse" => {
            let r = native::parse_batch_strict(black_box(lines)).expect("fixture parses");
            let n = r.len();
            black_box(&r);
            n
        }
        "filter" => {
            let r = native::filter_batch(black_box(entries));
            let n = r.len();
            black_box(&r);
            n
        }
        "format" => {
            let r = native::format_batch(black_box(entries));
            let n = r.len();
            black_box(&r);
            n
        }
        _ => unreachable!(),
    };
    let h3 = now_ns();
    Sample { copy_in: 0, guest: h3 - h0, copy_out: 0, total: h3 - h0, output_count: count }
}

struct ChildProc {
    child: Child,
    w: BufWriter<ChildStdin>,
    r: BufReader<ChildStdout>,
}

fn spawn_child(path: &str) -> ChildProc {
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn stage_child");
    let w = BufWriter::with_capacity(1 << 20, child.stdin.take().unwrap());
    let r = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
    ChildProc { child, w, r }
}

impl ChildProc {
    fn quit(mut self) {
        let _ = write_request(&mut self.w, OP_QUIT, &[]);
        let _ = self.child.wait();
    }
}

/// 經 pipe 往返一次（含 host 端編碼與解碼成 owned 結構）。
fn subprocess_sample(
    cp: &mut ChildProc,
    stage: &str,
    lines: &[String],
    entries: &[app::local::log_process::pipeline_process::LogEntry],
) -> Sample {
    let h0 = now_ns();
    let (op, payload) = match stage {
        "parse" => (OP_PARSE, encode_lines(lines)),
        "filter" => (OP_FILTER, encode_entries(entries)),
        "format" => (OP_FORMAT, encode_entries(entries)),
        _ => unreachable!(),
    };
    write_request(&mut cp.w, op, &payload).expect("write request");
    let resp = read_response(&mut cp.r).expect("read response");
    assert_eq!(resp.status, 0, "child returned error");
    let count = match stage {
        "parse" => {
            let v = decode_parsed(&resp.payload);
            let n = v.len();
            black_box(&v);
            n
        }
        "filter" => {
            let v = decode_filter_results(&resp.payload);
            let n = v.len();
            black_box(&v);
            n
        }
        "format" => {
            // format 回應即為 frame 串（與 WCM 的 list<u8> 相同），payload 已是 owned Vec<u8>
            resp.payload.len()
        }
        _ => unreachable!(),
    };
    let h3 = now_ns();
    Sample {
        copy_in: resp.h1.saturating_sub(h0),
        guest: resp.h2.saturating_sub(resp.h1),
        copy_out: h3.saturating_sub(resp.h2),
        total: h3 - h0,
        output_count: count,
    }
}

fn main() {
    let a = parse_args();
    println!("method,stage,run,batch_count,round,input_bytes,copy_in_ns,guest_ns,copy_out_ns,total_ns,output_count");

    if a.method == "spawn" {
        // child 冷啟動：spawn → 第一個 ping 回應
        for round in 0..(a.warmup + a.rounds) {
            let h0 = now_ns();
            let mut cp = spawn_child(&a.child);
            write_request(&mut cp.w, OP_PING, &[]).unwrap();
            let resp = read_response(&mut cp.r).unwrap();
            let h3 = now_ns();
            cp.quit();
            if round >= a.warmup {
                println!(
                    "spawn,spawn,{},0,{},0,{},0,{},{},0",
                    a.run,
                    round - a.warmup,
                    resp.h1.saturating_sub(h0),
                    h3.saturating_sub(resp.h1),
                    h3 - h0
                );
            }
        }
        return;
    }

    let mut cp = if a.method == "subprocess" { Some(spawn_child(&a.child)) } else { None };
    for &batch in &a.batches {
        let lines = input_lines(batch);
        let entries = input_entries(batch);
        let input_bytes: usize = lines.iter().map(|l| l.len()).sum();
        for round in 0..(a.warmup + a.rounds) {
            let s = match cp.as_mut() {
                Some(cp) => subprocess_sample(cp, &a.stage, &lines, &entries),
                None => native_sample(&a.stage, &lines, &entries),
            };
            if a.stage != "format" {
                assert_eq!(s.output_count, batch);
            }
            if round >= a.warmup {
                println!(
                    "{},{},{},{},{},{},{},{},{},{},{}",
                    a.method,
                    a.stage,
                    a.run,
                    batch,
                    round - a.warmup,
                    input_bytes,
                    s.copy_in,
                    s.guest,
                    s.copy_out,
                    s.total,
                    s.output_count
                );
            }
        }
    }
    if let Some(cp) = cp {
        cp.quit();
    }
}
