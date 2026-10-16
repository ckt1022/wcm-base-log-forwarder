// Subprocess baseline 的 child：從 stdin 讀 length-prefixed request，
// 以與 host 內建原生 stage 相同的 Rust 函式（src/native.rs）處理，再寫回 stdout。
// framing 格式見 src/common.rs。
//
// 這個 process 提供的隔離：獨立位址空間（記憶體隔離）、crash 不會帶走 host、
// 可以被 kill／設 rlimit／cgroup；但預設沒有任何 capability 限制——它擁有啟動者的
// 檔案系統、網路與環境變數權限，除非另外套用 seccomp／namespace／landlock。

#![allow(dead_code)]

#[path = "../shim.rs"]
mod app;
#[path = "../common.rs"]
mod common;
#[path = "../../../../../src/native.rs"]
mod native;

use std::io::{self, BufReader, BufWriter};

use common::*;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut r = BufReader::with_capacity(1 << 20, stdin.lock());
    let mut w = BufWriter::with_capacity(1 << 20, stdout.lock());

    while let Some((op, payload)) = read_request(&mut r)? {
        match op {
            OP_QUIT => break,
            OP_PING => {
                let t = now_ns();
                write_response(&mut w, t, t, 0, &[])?;
            }
            OP_PARSE => {
                let lines = decode_lines(&payload);
                let h1 = now_ns();
                let res = native::parse_batch_strict(&lines);
                let h2 = now_ns();
                match res {
                    Ok(ps) => write_response(&mut w, h1, h2, 0, &encode_parsed(&ps))?,
                    Err(e) => {
                        let mut b = Vec::new();
                        put_str(&mut b, &e);
                        write_response(&mut w, h1, h2, 1, &b)?
                    }
                }
            }
            OP_FILTER => {
                let entries = decode_entries(&payload);
                let h1 = now_ns();
                let rs = native::filter_batch(&entries);
                let h2 = now_ns();
                write_response(&mut w, h1, h2, 0, &encode_filter_results(&rs))?;
            }
            OP_FORMAT => {
                let entries = decode_entries(&payload);
                let h1 = now_ns();
                let bytes = native::format_batch(&entries);
                let h2 = now_ns();
                write_response(&mut w, h1, h2, 0, &bytes)?;
            }
            other => {
                let mut b = Vec::new();
                put_str(&mut b, &format!("unknown op {other}"));
                write_response(&mut w, 0, 0, 1, &b)?;
            }
        }
    }
    Ok(())
}
