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
	dst = append(dst, `"}}`...)
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

	w := bufio.NewWriterSize(os.Stdout, *bufferSize)
	defer w.Flush()

	start := time.Now()
	lastFlush := start
	lastReport := start

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

		// 在第 20 秒時先在 stdout 印出訊號，再注入一筆含 "LOOP" 的觸發 log
		if !loopSent && now.Sub(start) >= 10*time.Second {
			loopSent = true
			if err := w.Flush(); err != nil {
				fmt.Fprintf(logOut, "flush error: %v\n", err)
				os.Exit(1)
			}
			fmt.Fprintln(os.Stdout, "[gen-signal] t=20s: about to emit LOOP trigger log")
			ts := now.Format(time.RFC3339Nano)
			loopLog := fmt.Sprintf(`{"ts":"%s","level":"warn","msg":"LOOP trigger","loop":"true"}`, ts)
			if _, err := fmt.Fprintln(w, loopLog); err != nil {
				fmt.Fprintf(logOut, "write error: %v\n", err)
				os.Exit(1)
			}
			if err := w.Flush(); err != nil {
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

			if _, err := w.Write(lineBuf); err != nil {
				fmt.Fprintf(logOut, "write error: %v\n", err)
				os.Exit(1)
			}
			if err := w.WriteByte('\n'); err != nil {
				fmt.Fprintf(logOut, "write error: %v\n", err)
				os.Exit(1)
			}

			total++
			produced++
		}

		if now.Sub(lastFlush) >= time.Duration(*flushEvery)*time.Millisecond {
			if err := w.Flush(); err != nil {
				fmt.Fprintf(logOut, "flush error: %v\n", err)
				os.Exit(1)
			}
			lastFlush = now
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

	if err := w.Flush(); err != nil {
		fmt.Fprintf(logOut, "flush error: %v\n", err)
		os.Exit(1)
	}

	elapsed := time.Since(start).Seconds()
	fmt.Fprintf(logOut,
		"[gen] done: total=%d elapsed=%.3fs avg=%.0f/s\n",
		total, elapsed, float64(total)/elapsed,
	)
}
