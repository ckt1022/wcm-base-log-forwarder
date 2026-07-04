#!/usr/bin/env python3
"""
Simple HTTP log receiver for testing the transport WASM component.
Listens on port 8080, accepts POST /ingest, prints per-batch summary.
Uses ThreadingHTTPServer to handle concurrent requests from multiple workers.

延遲量測：
  format 插件輸出的每行 JSON 含 "timestamp" 欄位（RFC3339Nano）。
  server 在 body 完整讀取後記錄 receive_wall = time.time()，計算
    latency_ms = (receive_wall - ts_unix) * 1000
  所有 latency 累積在記憶體；關閉時（Ctrl+C 或 SIGTERM）寫出
  server_log/latency.csv 並列印 p50 / p95 / p99 / p999。
"""

import json
import os
import re
import signal
import sys
import time
import threading
from datetime import datetime, timezone, timedelta
from http.server import BaseHTTPRequestHandler, HTTPServer
from socketserver import ThreadingMixIn

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 5000

# ── 吞吐量計數器（monotonic 時鐘，不受系統時間調整影響）────────────────────
_lock          = threading.Lock()
_total_batches = 0
_total_bytes   = 0
_total_lines   = 0
_start_mono    = None   # 第一個 request 的 monotonic 時間
_last_mono     = None   # 上一個 request 的 monotonic 時間（批次吞吐量用）

# ── 延遲量測緩衝：每筆 = (ts_str, receive_unix: float, latency_ms: float) ───
_latency_buffer: list = []


# ── RFC3339Nano 解析 ──────────────────────────────────────────────────────────
# Go 的 time.RFC3339Nano 格式：2006-01-02T15:04:05.999999999Z07:00
# Python datetime 最多支援微秒（6 位），此函式處理到奈秒（9 位）。

_RFC3339_RE = re.compile(
    r'(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})'
    r'(?:\.(\d+))?'           # 小數秒（可選）
    r'(Z|[+-]\d{2}:\d{2})'   # 時區
)

def _parse_rfc3339(ts: str) -> float:
    """RFC3339Nano 字串 → Unix 時間戳（float 秒）。解析失敗回傳 0.0。"""
    m = _RFC3339_RE.match(ts)
    if not m:
        return 0.0
    yr, mo, dy, hr, mn, sc, frac, tz = m.groups()

    # 小數秒：補齊 / 截斷至 9 位後除以 1e9
    frac_sec = 0.0
    if frac:
        frac_sec = int(frac.ljust(9, '0')[:9]) / 1_000_000_000.0

    if tz == 'Z':
        tz_offset = timedelta(0)
    else:
        sign = 1 if tz[0] == '+' else -1
        th, tm_ = int(tz[1:3]), int(tz[4:6])
        tz_offset = timedelta(hours=sign * th, minutes=sign * tm_)

    dt = datetime(int(yr), int(mo), int(dy),
                  int(hr), int(mn), int(sc),
                  tzinfo=timezone(tz_offset))
    # dt.timestamp() 回傳整秒的 Unix float；frac_sec 補上小數部分
    return dt.timestamp() + frac_sec


def _percentile(sorted_data: list, p: float) -> float:
    """線性插值百分位數。sorted_data 必須已升序排列。"""
    n = len(sorted_data)
    if n == 0:
        return 0.0
    k  = (n - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, n - 1)
    return sorted_data[lo] + (k - lo) * (sorted_data[hi] - sorted_data[lo])


# ── 關閉時：寫出 CSV + 列印摘要 ──────────────────────────────────────────────

def _write_latency_csv() -> list:
    out_dir  = "server_log"
    os.makedirs(out_dir, exist_ok=True)
    csv_path = os.path.join(out_dir, "latency.csv")

    with _lock:
        buf = list(_latency_buffer)   # 取快照，避免鎖住太久

    with open(csv_path, "w") as f:
        f.write("ts,receive_unix,latency_ms\n")
        for ts_str, recv_unix, lat_ms in buf:
            f.write(f"{ts_str},{recv_unix:.6f},{lat_ms:.3f}\n")

    print(f"\n[server] latency.csv 寫出：{csv_path}  ({len(buf)} 筆)", flush=True)
    return buf


def _print_latency_summary(buf: list):
    if not buf:
        print("[server] 無延遲資料可分析。", flush=True)
        return

    lats = sorted(r[2] for r in buf)
    n    = len(lats)
    print(f"[server] ── 延遲分布摘要（全部 {n} 筆，含 warmup）──", flush=True)
    for pct in (50, 95, 99, 99.9):
        print(f"  p{pct:<5} = {_percentile(lats, pct):>8.2f} ms", flush=True)
    print(f"  min    = {lats[0]:>8.2f} ms", flush=True)
    print(f"  max    = {lats[-1]:>8.2f} ms", flush=True)
    print("  ※ warmup 過濾（前 15 秒）請用 analyze.py 重新計算。", flush=True)


def _shutdown():
    buf = _write_latency_csv()
    _print_latency_summary(buf)
    with _lock:
        elapsed   = (time.monotonic() - _start_mono) if _start_mono else 0
        final_avg = _total_lines / elapsed if elapsed > 0 else 0
    print(
        f"[server] 停止。"
        f"  batches={_total_batches}"
        f"  lines={_total_lines}"
        f"  avg={final_avg:.0f} lines/s",
        flush=True,
    )


# SIGTERM：latency.sh cleanup 會送此信號
def _sigterm_handler(signum, frame):
    _shutdown()
    sys.exit(0)

signal.signal(signal.SIGTERM, _sigterm_handler)


# ── HTTP handler ──────────────────────────────────────────────────────────────

class ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    """每個 request 在獨立 thread 處理。"""
    daemon_threads = True


class LogReceiver(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/ingest":
            self.send_response(404)
            self.end_headers()
            return

        length_header = self.headers.get("Content-Length")
        if length_header is not None:
            body = self.rfile.read(int(length_header))
        else:
            chunks = []
            while True:
                chunk = self.rfile.read(4096)
                if not chunk:
                    break
                chunks.append(chunk)
            body = b"".join(chunks)

        # body 完整讀取後才打時間戳——代表 server 已收到整批資料
        receive_wall = time.time()      # wall clock，與 generator 同機，無 clock skew
        receive_mono = time.monotonic()
        n_lines      = body.count(b"\n")

        # ── 延遲計算（鎖外，避免拉長 critical section）──────────────────────
        # format 插件輸出 "timestamp" 欄位（RFC3339Nano），非原始 "ts" 欄位
        line_latencies = []
        for raw in body.splitlines():
            if not raw:
                continue
            try:
                obj    = json.loads(raw)
                ts_str = obj.get("timestamp", "")
                if not ts_str:
                    continue
                ts_unix = _parse_rfc3339(ts_str)
                if ts_unix == 0.0:
                    continue
                lat_ms = (receive_wall - ts_unix) * 1000.0
                line_latencies.append((ts_str, receive_wall, lat_ms))
            except Exception:
                pass

        # ── 更新全域狀態（持鎖）──────────────────────────────────────────────
        global _total_batches, _total_bytes, _total_lines, _start_mono, _last_mono
        with _lock:
            if _start_mono is None:
                _start_mono = receive_mono

            if _last_mono is not None and receive_mono > _last_mono:
                batch_tput = n_lines / (receive_mono - _last_mono)
            else:
                batch_tput = 0.0
            _last_mono = receive_mono

            _total_batches += 1
            _total_bytes   += len(body)
            _total_lines   += n_lines
            batch_num = _total_batches
            tl        = _total_lines

            elapsed  = receive_mono - _start_mono
            avg_tput = tl / elapsed if elapsed > 0 else 0.0

            _latency_buffer.extend(line_latencies)

        # ── 每批次輸出：吞吐量 + 當批延遲 ───────────────────────────────────
        if line_latencies:
            batch_lats = sorted(r[2] for r in line_latencies)
            lat_info = (
                f"  lat p50={_percentile(batch_lats, 50):>6.1f}ms"
                f"  p99={_percentile(batch_lats, 99):>6.1f}ms"
            )
        else:
            lat_info = "  (no timestamp)"

        print(
            f"[server] #{batch_num:>5}  {n_lines:>6} lines"
            f"  {batch_tput:>8.0f}/s  avg={avg_tput:>8.0f}/s"
            f"{lat_info}",
            flush=True,
        )

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def log_message(self, fmt, *args):
        pass  # 抑制預設的 access log


def main():
    server = ThreadingHTTPServer(("127.0.0.1", PORT), LogReceiver)
    print(f"[server] Listening on http://127.0.0.1:{PORT}/ingest (multi-threaded)", flush=True)
    print("[server] Press Ctrl+C to stop.\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        _shutdown()


if __name__ == "__main__":
    main()
