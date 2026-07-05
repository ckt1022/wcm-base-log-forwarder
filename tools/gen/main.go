// Log generator for wcm-base-log-forwarder testing and benchmarking.
//
// Usage:
//
//	go run main.go [flags] | ../target/debug/wcm-base-log-forwarder
//
// Modes:
//
//	json-simple    - 固定格式 JSON，少量 att 欄位（基本效能測試）
//	json-complex   - JSON 含多個 att 欄位與較長訊息（壓力測試）
//	json-mixed     - json-simple + json-complex 混合（接近真實情況）
//	json-fixed5    - 5 條固定內容循環（ts 每次更新；_slot=1~5 標識種類，用於排查特定 log 造成的延遲）
//	invalid        - 混入無法解析的行（測試 parser 錯誤處理；valid line 為 json-simple）
//	logfmt-simple  - 扁平 key=value 格式，基本效能測試
//	logfmt-complex - logfmt 含較多欄位與較長訊息
//	logfmt-mixed   - logfmt simple + complex 混合
//	syslog-simple  - RFC5424 風格 syslog，基本欄位
//	syslog-complex - RFC5424 風格 syslog，附帶延伸 key=value 欄位
//	syslog-mixed   - syslog simple + complex 混合
//
// Traffic shapes (-traffic):
//
//	flat   - 固定速率（預設）
//	wave   - 正弦波形，高峰低谷交替，長期平均等於 -rate（適合可控實驗）
//	bursty - 正弦波底層＋隨機突發尖峰，模擬真實流量（適合壓力測試）
package main

import (
	"bufio"
	"flag"
	"fmt"
	"math"
	"math/rand"
	"net"
	"os"
	"strconv"
	"strings"
	"time"
)

var (
	rate        = flag.Int("rate", 5000, "target lines per second")
	duration    = flag.Int("duration", 30, "run duration in seconds, 0=forever")
	mode        = flag.String("mode", "json-simple", "log mode: json-simple|json-complex|json-mixed|invalid|logfmt-simple|logfmt-complex|logfmt-mixed|syslog-simple|syslog-complex|syslog-mixed")
	invalidRate = flag.Float64("invalid-rate", 0.05, "fraction of invalid/malformed lines when mode=invalid (0.0-1.0)")
	bufferSize  = flag.Int("buffer", 1<<20, "stdout buffer size in bytes (default 1MB)")
	flushEvery  = flag.Int("flush-ms", 100, "flush stdout interval in milliseconds")
	seed        = flag.Int64("seed", 0, "random seed, 0=use current time")

	// Traffic shape flags
	trafficShape = flag.String("traffic", "flat", "traffic shape: flat|wave|bursty")
	waveAmp      = flag.Float64("wave-amp", 0.6, "sine wave amplitude 0.0-0.9 (wave/bursty); rate varies between rate*(1-amp) and rate*(1+amp)")
	wavePeriod   = flag.Float64("wave-period", 60.0, "sine wave period in seconds")
	spikeMult    = flag.Float64("spike-mult", 3.0, "spike peak multiplier applied on top of wave (bursty)")
	spikeFreqPM  = flag.Float64("spike-freq", 2.0, "average spikes per minute (bursty)")
	spikeDurSec  = flag.Float64("spike-dur", 5.0, "spike duration in seconds (bursty)")

	// Output flags
	logFile = flag.String("log-file", "gen.log", "write generator diagnostics to this file; use '-' for stderr")

	// TCP output flags
	outputDest = flag.String("output", "stdout", "output destination: stdout | tcp")
	tcpAddr    = flag.String("tcp-addr", "127.0.0.1:5140", "forwarder TCP address when output=tcp")
	sendUnit   = flag.String("send-unit", "line", "send granularity when output=tcp: line (每產生一條立即送出) | batch (累積 N 行再送)")
	batchLines = flag.Int("batch-lines", 100, "lines per batch when send-unit=batch")
)

// -----------------------------------------------------------------
// Log field pools
// -----------------------------------------------------------------

var levels = []string{"debug", "info", "info", "info", "warn", "error"}

var services = []string{"api-gateway", "auth-service", "payment-svc", "order-svc", "notify-svc"}

var simpleMessages = []string{
	"Connection accepted from 192.168.1.100",
	"Request completed successfully",
	"Cache hit for key user_session",
	"Database query executed in 3ms",
	"Health check passed",
}

var complexMessages = []string{
	"Upstream timeout after 30s waiting for response from backend pool eu-west-2",
	"Token validation failed: signature mismatch, expected HS256 but received RS256 algorithm",
	"Retry attempt 3/5 for POST /api/v2/orders after connection reset by peer",
	"Circuit breaker OPEN for service payment-svc: 15 failures in last 60s window",
	"Slow query detected: SELECT * FROM events WHERE user_id=? AND created_at>? took 1240ms",
}

var httpCodes = []string{"200", "201", "400", "401", "403", "404", "429", "500", "502", "503"}
var regions = []string{"us-east-1", "eu-west-1", "ap-southeast-1"}
var traceIDs = []string{"4bf92f3577b34da6", "00f067aa0ba902b7", "e457b5a2e4d86bd1", "1234567890abcdef"}

// -----------------------------------------------------------------
// Line builders - JSON
// -----------------------------------------------------------------

// simpleJSONLineLen is the fixed byte length for every json-simple line.
// Computed from max natural content (149 B) + padding field overhead (11 B) + margin (12 B).
// All lines are padded to exactly this length via a trailing "_p" field.
const simpleJSONLineLen = 172

// simpleJSONPad is a pre-allocated space string used for padding; length must be >= 50.
const simpleJSONPad = "                                                                " // 64 spaces

func buildSimpleJSONLine(dst []byte, ts, level, svc, code string) []byte {
	dst = dst[:0]
	dst = append(dst, `{"ts":"`...)
	dst = append(dst, ts...)
	dst = append(dst, `","level":"`...)
	dst = append(dst, level...)
	dst = append(dst, `","msg":"`...)
	dst = append(dst, simpleMessages[rand.Intn(len(simpleMessages))]...)
	dst = append(dst, `","att":{"service":"`...)
	dst = append(dst, svc...)
	dst = append(dst, `","code":"`...)
	dst = append(dst, code...)
	dst = append(dst, `"},"_p":"`...)
	padLen := simpleJSONLineLen - len(dst) - 2 // -2 for closing `"}`
	if padLen < 0 {
		padLen = 0
	}
	dst = append(dst, simpleJSONPad[:padLen]...)
	dst = append(dst, `"}`...)
	return dst
}

func buildComplexJSONLine(dst []byte, ts, level, svc, code, region, traceID string, latencyMs int) []byte {
	dst = dst[:0]
	dst = append(dst, `{"ts":"`...)
	dst = append(dst, ts...)
	dst = append(dst, `","level":"`...)
	dst = append(dst, level...)
	dst = append(dst, `","msg":"`...)
	dst = append(dst, complexMessages[rand.Intn(len(complexMessages))]...)
	dst = append(dst, `","att":{"service":"`...)
	dst = append(dst, svc...)
	dst = append(dst, `","code":"`...)
	dst = append(dst, code...)
	dst = append(dst, `","region":"`...)
	dst = append(dst, region...)
	dst = append(dst, `","trace_id":"`...)
	dst = append(dst, traceID...)
	dst = fmt.Appendf(dst, `","latency_ms":"%d","host":"worker-%02d","env":"production"}}`, latencyMs, rand.Intn(32))
	return dst
}

// -----------------------------------------------------------------
// json-fixed5: 5 條固定內容循環（ts 每次刷新，_slot 標識種類）
// -----------------------------------------------------------------

// fixed5Tpl 每個元素對應一條固定 log 的靜態部分（不含 ts）。
// 格式：{"ts":"<ts>","level":"<level>","msg":"<msg>","att":{"service":"<svc>","code":"<code>"},"_slot":"<N>"}
var fixed5Tpl = [5]struct{ level, msg, svc, code string }{
	{"info", "Request completed successfully", "api-gateway", "200"},
	{"warn", "Upstream timeout after 30s waiting for response", "payment-svc", "503"},
	{"error", "Token validation failed: signature mismatch", "auth-service", "401"},
	{"debug", "Health check passed", "notify-svc", "200"},
	{"info", "Database query executed in 3ms", "order-svc", "200"},
}

func buildFixed5Line(dst []byte, ts string, slot int) []byte {
	t := fixed5Tpl[slot]
	dst = dst[:0]
	dst = append(dst, `{"ts":"`...)
	dst = append(dst, ts...)
	dst = append(dst, `","level":"`...)
	dst = append(dst, t.level...)
	dst = append(dst, `","msg":"`...)
	dst = append(dst, t.msg...)
	dst = append(dst, `","att":{"service":"`...)
	dst = append(dst, t.svc...)
	dst = append(dst, `","code":"`...)
	dst = append(dst, t.code...)
	dst = append(dst, `"},"_slot":"`...)
	dst = append(dst, byte('1'+slot))
	dst = append(dst, '"', '}')
	return dst
}

// -----------------------------------------------------------------
// Helper functions
// -----------------------------------------------------------------

func appendLogfmtValue(dst []byte, v string) []byte {
	needQuote := false
	for i := 0; i < len(v); i++ {
		switch v[i] {
		case ' ', '\t', '\n', '\r', '"', '=':
			needQuote = true
		}
		if needQuote {
			break
		}
	}

	if !needQuote {
		return append(dst, v...)
	}

	return strconv.AppendQuote(dst, v)
}

func levelToSyslogSeverity(level string) int {
	switch level {
	case "debug":
		return 7
	case "info":
		return 6
	case "warn":
		return 4
	case "error":
		return 3
	default:
		return 6
	}
}

// -----------------------------------------------------------------
// Line builders - logfmt
// -----------------------------------------------------------------

func buildSimpleLogfmtLine(dst []byte, ts, level, svc, code string) []byte {
	dst = dst[:0]

	dst = append(dst, "ts="...)
	dst = appendLogfmtValue(dst, ts)

	dst = append(dst, " level="...)
	dst = appendLogfmtValue(dst, level)

	dst = append(dst, " msg="...)
	dst = appendLogfmtValue(dst, simpleMessages[rand.Intn(len(simpleMessages))])

	dst = append(dst, " service="...)
	dst = appendLogfmtValue(dst, svc)

	dst = append(dst, " code="...)
	dst = appendLogfmtValue(dst, code)

	return dst
}

func buildComplexLogfmtLine(dst []byte, ts, level, svc, code, region, traceID string, latencyMs int) []byte {
	dst = dst[:0]

	dst = append(dst, "ts="...)
	dst = appendLogfmtValue(dst, ts)

	dst = append(dst, " level="...)
	dst = appendLogfmtValue(dst, level)

	dst = append(dst, " msg="...)
	dst = appendLogfmtValue(dst, complexMessages[rand.Intn(len(complexMessages))])

	dst = append(dst, " service="...)
	dst = appendLogfmtValue(dst, svc)

	dst = append(dst, " code="...)
	dst = appendLogfmtValue(dst, code)

	dst = append(dst, " region="...)
	dst = appendLogfmtValue(dst, region)

	dst = append(dst, " trace_id="...)
	dst = appendLogfmtValue(dst, traceID)

	dst = append(dst, " latency_ms="...)
	dst = strconv.AppendInt(dst, int64(latencyMs), 10)

	dst = append(dst, " host="...)
	dst = fmt.Appendf(dst, "worker-%02d", rand.Intn(32))

	dst = append(dst, " env=production"...)

	return dst
}

// -----------------------------------------------------------------
// Line builders - syslog (RFC5424-like)
// -----------------------------------------------------------------

func buildSimpleSyslogLine(dst []byte, ts, level, svc, code string) []byte {
	dst = dst[:0]

	severity := levelToSyslogSeverity(level)
	priority := 8 + severity // facility=1(user-level)
	host := fmt.Sprintf("worker-%02d", rand.Intn(32))
	msg := simpleMessages[rand.Intn(len(simpleMessages))]

	dst = append(dst, '<')
	dst = strconv.AppendInt(dst, int64(priority), 10)
	dst = append(dst, '>', '1', ' ')

	dst = append(dst, ts...)
	dst = append(dst, ' ')

	dst = append(dst, host...)
	dst = append(dst, ' ')

	dst = append(dst, svc...)
	dst = append(dst, ' ')

	dst = append(dst, '-', ' ', '-', ' ', '-', ' ')

	dst = append(dst, msg...)
	dst = append(dst, ' ')

	dst = append(dst, "level="...)
	dst = appendLogfmtValue(dst, level)

	dst = append(dst, " code="...)
	dst = appendLogfmtValue(dst, code)

	dst = append(dst, " service="...)
	dst = appendLogfmtValue(dst, svc)

	return dst
}

func buildComplexSyslogLine(dst []byte, ts, level, svc, code, region, traceID string, latencyMs int) []byte {
	dst = dst[:0]

	severity := levelToSyslogSeverity(level)
	priority := 8 + severity // facility=1(user-level)
	host := fmt.Sprintf("worker-%02d", rand.Intn(32))
	msg := complexMessages[rand.Intn(len(complexMessages))]

	dst = append(dst, '<')
	dst = strconv.AppendInt(dst, int64(priority), 10)
	dst = append(dst, '>', '1', ' ')

	dst = append(dst, ts...)
	dst = append(dst, ' ')

	dst = append(dst, host...)
	dst = append(dst, ' ')

	dst = append(dst, svc...)
	dst = append(dst, ' ')

	dst = append(dst, '-', ' ', '-', ' ', '-', ' ')

	dst = append(dst, msg...)
	dst = append(dst, ' ')

	dst = append(dst, "level="...)
	dst = appendLogfmtValue(dst, level)

	dst = append(dst, " code="...)
	dst = appendLogfmtValue(dst, code)

	dst = append(dst, " region="...)
	dst = appendLogfmtValue(dst, region)

	dst = append(dst, " trace_id="...)
	dst = appendLogfmtValue(dst, traceID)

	dst = append(dst, " latency_ms="...)
	dst = strconv.AppendInt(dst, int64(latencyMs), 10)

	dst = append(dst, " host="...)
	dst = appendLogfmtValue(dst, host)

	dst = append(dst, " env=production"...)

	return dst
}

// invalidLines covers several failure scenarios for the parser
var invalidLines = []string{
	`not json at all`,
	`{"ts":"2024-01-01","broken":`,
	``,
	`   `,
	`{"ts":null,"level":123,"msg":true}`,
	strings.Repeat("a", 8192), // 超長行
}

func pickInvalidLine() []byte {
	return []byte(invalidLines[rand.Intn(len(invalidLines))])
}

// -----------------------------------------------------------------
// lineOut — 輸出目標抽象
//
//   writeLine(b)  寫一行（不含換行，內部自行補 \n）
//   flush()       強制排清（用於 LOOP trigger 後的即時發送）
//   tick(now)     週期性排清檢查（每個外層迴圈迭代呼叫一次）
//   close()       最終排清並關閉
// -----------------------------------------------------------------

type lineOut interface {
	writeLine(b []byte) error
	flush() error
	tick(now time.Time) error
	close() error
}

// ── stdoutOut：原有行為（buffered stdout + 週期排清）────────────────────────

type stdoutOut struct {
	w          *bufio.Writer
	flushEvery time.Duration
	lastFlush  time.Time
}

func (s *stdoutOut) writeLine(b []byte) error {
	if _, err := s.w.Write(b); err != nil {
		return err
	}
	return s.w.WriteByte('\n')
}

func (s *stdoutOut) flush() error { return s.w.Flush() }

func (s *stdoutOut) tick(now time.Time) error {
	if now.Sub(s.lastFlush) >= s.flushEvery {
		s.lastFlush = now
		return s.w.Flush()
	}
	return nil
}

func (s *stdoutOut) close() error { return s.w.Flush() }

// ── tcpLineOut：每產生一行立刻送出（send-unit=line）─────────────────────────

type tcpLineOut struct {
	conn net.Conn
	buf  []byte
}

func (t *tcpLineOut) writeLine(b []byte) error {
	t.buf = append(t.buf[:0], b...)
	t.buf = append(t.buf, '\n')
	_, err := t.conn.Write(t.buf)
	return err
}

func (t *tcpLineOut) flush() error               { return nil }
func (t *tcpLineOut) tick(_ time.Time) error      { return nil }
func (t *tcpLineOut) close() error                { return t.conn.Close() }

// ── tcpBatchOut：累積 N 行再送出（send-unit=batch）──────────────────────────

type tcpBatchOut struct {
	conn       net.Conn
	w          *bufio.Writer
	batchLines int
	count      int
	flushEvery time.Duration
	lastFlush  time.Time
}

func (t *tcpBatchOut) writeLine(b []byte) error {
	if _, err := t.w.Write(b); err != nil {
		return err
	}
	if err := t.w.WriteByte('\n'); err != nil {
		return err
	}
	t.count++
	if t.count >= t.batchLines {
		t.count = 0
		return t.w.Flush()
	}
	return nil
}

func (t *tcpBatchOut) flush() error {
	t.count = 0
	return t.w.Flush()
}

func (t *tcpBatchOut) tick(now time.Time) error {
	if now.Sub(t.lastFlush) >= t.flushEvery {
		t.lastFlush = now
		return t.flush()
	}
	return nil
}

func (t *tcpBatchOut) close() error {
	if err := t.w.Flush(); err != nil {
		return err
	}
	return t.conn.Close()
}

// -----------------------------------------------------------------
// Main loop
// -----------------------------------------------------------------

func main() {
	flag.Parse()

	if *seed != 0 {
		rand.Seed(*seed)
	} else {
		rand.Seed(time.Now().UnixNano())
	}

	if *rate <= 0 {
		fmt.Fprintln(os.Stderr, "rate must be > 0")
		os.Exit(1)
	}

	// Open diagnostic log output.
	var logOut *os.File
	if *logFile == "-" {
		logOut = os.Stderr
	} else {
		var err error
		logOut, err = os.OpenFile(*logFile, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0644)
		if err != nil {
			fmt.Fprintf(os.Stderr, "cannot open log file %s: %v\n", *logFile, err)
			os.Exit(1)
		}
		defer logOut.Close()
	}

	validModes := map[string]bool{
		"json-simple":    true,
		"json-complex":   true,
		"json-mixed":     true,
		"json-fixed5":    true,
		"invalid":        true,
		"logfmt-simple":  true,
		"logfmt-complex": true,
		"logfmt-mixed":   true,
		"syslog-simple":  true,
		"syslog-complex": true,
		"syslog-mixed":   true,
	}
	if !validModes[*mode] {
		fmt.Fprintln(os.Stderr, "mode must be one of: json-simple, json-complex, json-mixed, invalid, logfmt-simple, logfmt-complex, logfmt-mixed, syslog-simple, syslog-complex, syslog-mixed")
		os.Exit(1)
	}
	if *trafficShape != "flat" && *trafficShape != "wave" && *trafficShape != "bursty" {
		fmt.Fprintln(os.Stderr, "traffic must be one of: flat, wave, bursty")
		os.Exit(1)
	}
	if *waveAmp < 0 || *waveAmp >= 1.0 {
		fmt.Fprintln(os.Stderr, "wave-amp must be in [0.0, 1.0)")
		os.Exit(1)
	}

	start := time.Now()
	lastReport := start

	// ── 建立輸出目標 ───────────────────────────────────────────────────────────
	var out lineOut
	switch *outputDest {
	case "tcp":
		conn, err := net.Dial("tcp", *tcpAddr)
		if err != nil {
			fmt.Fprintf(os.Stderr, "[gen] cannot connect to %s: %v\n", *tcpAddr, err)
			os.Exit(1)
		}
		fmt.Fprintf(logOut, "[gen] output=tcp  addr=%s  send-unit=%s", *tcpAddr, *sendUnit)
		if *sendUnit == "batch" {
			fmt.Fprintf(logOut, "  batch-lines=%d", *batchLines)
			out = &tcpBatchOut{
				conn:       conn,
				w:          bufio.NewWriterSize(conn, *bufferSize),
				batchLines: *batchLines,
				flushEvery: time.Duration(*flushEvery) * time.Millisecond,
				lastFlush:  start,
			}
		} else {
			out = &tcpLineOut{conn: conn}
		}
		fmt.Fprintln(logOut)
	default: // stdout
		fmt.Fprintf(logOut, "[gen] output=stdout\n")
		out = &stdoutOut{
			w:          bufio.NewWriterSize(os.Stdout, *bufferSize),
			flushEvery: time.Duration(*flushEvery) * time.Millisecond,
			lastFlush:  start,
		}
	}

	var total, reportBase uint64
	lineBuf := make([]byte, 0, 512)
	loopSent := false

	// Traffic shaping state
	var cumTarget float64
	lastTick := start
	currentMult := 1.0

	// Bursty spike state
	inSpike := false
	var spikeEnd time.Time
	var nextSpike time.Time
	if *trafficShape == "bursty" {
		intervalSec := 60.0 / *spikeFreqPM
		// First spike after 0.5–1.5× interval to avoid hitting at t=0
		delay := intervalSec * (0.5 + rand.Float64())
		nextSpike = start.Add(time.Duration(delay * float64(time.Second)))
	}

	fmt.Fprintf(logOut,
		"[gen] start: rate=%d/s duration=%ds mode=%s traffic=%s invalid-rate=%.2f\n",
		*rate, *duration, *mode, *trafficShape, *invalidRate,
	)

	for {
		now := time.Now()

		if *duration > 0 && now.Sub(start) >= time.Duration(*duration)*time.Second {
			break
		}

		// Compute instantaneous rate multiplier and advance the cumulative target.
		{
			dt := now.Sub(lastTick).Seconds()
			lastTick = now
			elapsed := now.Sub(start).Seconds()

			switch *trafficShape {
			case "wave":
				// Pure sine wave: integrates to exactly rate*T over full periods.
				currentMult = 1.0 + *waveAmp*math.Sin(2*math.Pi*elapsed / *wavePeriod)
			case "bursty":
				baseMult := 1.0 + *waveAmp*math.Sin(2*math.Pi*elapsed / *wavePeriod)
				// Transition: spike → normal
				if inSpike && now.After(spikeEnd) {
					inSpike = false
					intervalSec := 60.0 / *spikeFreqPM
					jitter := (rand.Float64()*2 - 1) * intervalSec * 0.4
					next := intervalSec + jitter
					if next < 3.0 {
						next = 3.0
					}
					nextSpike = now.Add(time.Duration(next * float64(time.Second)))
				}
				// Transition: normal → spike
				if !inSpike && !nextSpike.IsZero() && !now.Before(nextSpike) {
					inSpike = true
					spikeEnd = now.Add(time.Duration(*spikeDurSec * float64(time.Second)))
				}
				if inSpike {
					currentMult = baseMult * *spikeMult
				} else {
					currentMult = baseMult
				}
			default: // flat
				currentMult = 1.0
			}

			// Clamp to avoid negative rates at extreme amplitudes.
			if currentMult < 0.05 {
				currentMult = 0.05
			}

			cumTarget += float64(*rate) * currentMult * dt
		}

		shouldHaveSent := uint64(cumTarget)

		// 在第 15 秒時注入一筆含 "LOOP" 的觸發 log；訊號寫到 logOut，不污染資料流
		if !loopSent && now.Sub(start) >= 15*time.Second {
			loopSent = true
			fmt.Fprintln(logOut, "[gen-signal] t=15s: about to emit LOOP trigger log")
			ts := now.Format(time.RFC3339Nano)
			loopLog := fmt.Sprintf(`{"ts":"%s","level":"warn","msg":"LOOP trigger","loop":"true"}`, ts)
			if err := out.writeLine([]byte(loopLog)); err != nil {
				fmt.Fprintf(logOut, "write error: %v\n", err)
				os.Exit(1)
			}
			if err := out.flush(); err != nil {
				fmt.Fprintf(logOut, "flush error: %v\n", err)
				os.Exit(1)
			}
			fmt.Fprintf(logOut, "[gen] LOOP trigger emitted at t=%.2fs\n", now.Sub(start).Seconds())
		}

		var produced int
		for total < shouldHaveSent && produced < 50000 {
			ts := now.Format(time.RFC3339Nano)
			level := levels[rand.Intn(len(levels))]
			svc := services[rand.Intn(len(services))]
			code := httpCodes[rand.Intn(len(httpCodes))]

			switch *mode {
			case "json-simple":
				lineBuf = buildSimpleJSONLine(lineBuf, ts, level, svc, code)

			case "json-fixed5":
				lineBuf = buildFixed5Line(lineBuf, ts, int(total%5))

			case "json-complex":
				region := regions[rand.Intn(len(regions))]
				traceID := traceIDs[rand.Intn(len(traceIDs))]
				latency := rand.Intn(2000)
				lineBuf = buildComplexJSONLine(lineBuf, ts, level, svc, code, region, traceID, latency)

			case "json-mixed":
				if rand.Intn(2) == 0 {
					lineBuf = buildSimpleJSONLine(lineBuf, ts, level, svc, code)
				} else {
					region := regions[rand.Intn(len(regions))]
					traceID := traceIDs[rand.Intn(len(traceIDs))]
					latency := rand.Intn(2000)
					lineBuf = buildComplexJSONLine(lineBuf, ts, level, svc, code, region, traceID, latency)
				}

			case "logfmt-simple":
				lineBuf = buildSimpleLogfmtLine(lineBuf, ts, level, svc, code)

			case "logfmt-complex":
				region := regions[rand.Intn(len(regions))]
				traceID := traceIDs[rand.Intn(len(traceIDs))]
				latency := rand.Intn(2000)
				lineBuf = buildComplexLogfmtLine(lineBuf, ts, level, svc, code, region, traceID, latency)

			case "logfmt-mixed":
				if rand.Intn(2) == 0 {
					lineBuf = buildSimpleLogfmtLine(lineBuf, ts, level, svc, code)
				} else {
					region := regions[rand.Intn(len(regions))]
					traceID := traceIDs[rand.Intn(len(traceIDs))]
					latency := rand.Intn(2000)
					lineBuf = buildComplexLogfmtLine(lineBuf, ts, level, svc, code, region, traceID, latency)
				}

			case "syslog-simple":
				lineBuf = buildSimpleSyslogLine(lineBuf, ts, level, svc, code)

			case "syslog-complex":
				region := regions[rand.Intn(len(regions))]
				traceID := traceIDs[rand.Intn(len(traceIDs))]
				latency := rand.Intn(2000)
				lineBuf = buildComplexSyslogLine(lineBuf, ts, level, svc, code, region, traceID, latency)

			case "syslog-mixed":
				if rand.Intn(2) == 0 {
					lineBuf = buildSimpleSyslogLine(lineBuf, ts, level, svc, code)
				} else {
					region := regions[rand.Intn(len(regions))]
					traceID := traceIDs[rand.Intn(len(traceIDs))]
					latency := rand.Intn(2000)
					lineBuf = buildComplexSyslogLine(lineBuf, ts, level, svc, code, region, traceID, latency)
				}

			case "invalid":
				if rand.Float64() < *invalidRate {
					lineBuf = lineBuf[:0]
					lineBuf = append(lineBuf, pickInvalidLine()...)
				} else {
					lineBuf = buildSimpleJSONLine(lineBuf, ts, level, svc, code)
				}
			}

			if err := out.writeLine(lineBuf); err != nil {
				fmt.Fprintf(logOut, "write error: %v\n", err)
				os.Exit(1)
			}

			total++
			produced++
		}

		if err := out.tick(now); err != nil {
			fmt.Fprintf(logOut, "flush error: %v\n", err)
			os.Exit(1)
		}

		if now.Sub(lastReport) >= time.Second {
			windowCount := total - reportBase
			windowSecs := now.Sub(lastReport).Seconds()
			spike := ""
			if inSpike {
				spike = " [SPIKE]"
			}
			fmt.Fprintf(logOut,
				"[gen] total=%d inst=%.0f/s avg=%.0f/s mult=%.2fx%s\n",
				total,
				float64(windowCount)/windowSecs,
				float64(total)/now.Sub(start).Seconds(),
				currentMult,
				spike,
			)
			lastReport = now
			reportBase = total
		}

		if total >= shouldHaveSent {
			time.Sleep(200 * time.Microsecond)
		}
	}

	if err := out.close(); err != nil {
		fmt.Fprintf(logOut, "close error: %v\n", err)
		os.Exit(1)
	}

	elapsed := time.Since(start).Seconds()
	fmt.Fprintf(logOut,
		"[gen] done: total=%d elapsed=%.3fs avg=%.0f/s\n",
		total, elapsed, float64(total)/elapsed,
	)
}
