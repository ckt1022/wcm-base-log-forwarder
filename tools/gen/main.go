// Log generator for wcm-base-log-forwarder testing and benchmarking.
//
// Usage:
//
//	go run main.go [flags] | ../target/debug/wcm-base-log-forwarder
//
// Modes:
//
//	simple   - 固定格式 JSON，少量 att 欄位（基本效能測試）
//	complex  - JSON 含多個 att 欄位與較長訊息（壓力測試）
//	mixed    - simple + complex 混合（接近真實情況）
//	invalid  - 混入無法解析的行（測試 parser 錯誤處理）
package main

import (
	"bufio"
	"flag"
	"fmt"
	"math/rand"
	"os"
	"strings"
	"time"
)

var (
	rate        = flag.Int("rate", 5000, "target lines per second")
	duration    = flag.Int("duration", 30, "run duration in seconds, 0=forever")
	mode        = flag.String("mode", "simple", "log mode: simple|complex|mixed|invalid")
	invalidRate = flag.Float64("invalid-rate", 0.05, "fraction of invalid/malformed lines when mode=invalid (0.0-1.0)")
	bufferSize  = flag.Int("buffer", 1<<20, "stdout buffer size in bytes (default 1MB)")
	flushEvery  = flag.Int("flush-ms", 100, "flush stdout interval in milliseconds")
	seed        = flag.Int64("seed", 0, "random seed, 0=use current time")
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
// Line builders
// -----------------------------------------------------------------

func buildSimpleLine(dst []byte, ts, level, svc, code string) []byte {
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

func buildComplexLine(dst []byte, ts, level, svc, code, region, traceID string, latencyMs int) []byte {
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

	validModes := map[string]bool{"simple": true, "complex": true, "mixed": true, "invalid": true}
	if !validModes[*mode] {
		fmt.Fprintln(os.Stderr, "mode must be one of: simple, complex, mixed, invalid")
		os.Exit(1)
	}

	w := bufio.NewWriterSize(os.Stdout, *bufferSize)
	defer w.Flush()

	start := time.Now()
	lastFlush := start
	lastReport := start

	var total, reportBase uint64
	lineBuf := make([]byte, 0, 512)

	fmt.Fprintf(os.Stderr,
		"[gen] start: rate=%d/s duration=%ds mode=%s invalid-rate=%.2f\n",
		*rate, *duration, *mode, *invalidRate,
	)

	for {
		now := time.Now()

		if *duration > 0 && now.Sub(start) >= time.Duration(*duration)*time.Second {
			break
		}

		shouldHaveSent := uint64(now.Sub(start).Nanoseconds()) * uint64(*rate) / 1_000_000_000

		var produced int
		for total < shouldHaveSent && produced < 50000 {
			ts := now.Format(time.RFC3339Nano)
			level := levels[rand.Intn(len(levels))]
			svc := services[rand.Intn(len(services))]
			code := httpCodes[rand.Intn(len(httpCodes))]

			// 決定這一行的內容
			switch *mode {
			case "simple":
				lineBuf = buildSimpleLine(lineBuf, ts, level, svc, code)

			case "complex":
				region := regions[rand.Intn(len(regions))]
				traceID := traceIDs[rand.Intn(len(traceIDs))]
				latency := rand.Intn(2000)
				lineBuf = buildComplexLine(lineBuf, ts, level, svc, code, region, traceID, latency)

			case "mixed":
				if rand.Intn(2) == 0 {
					lineBuf = buildSimpleLine(lineBuf, ts, level, svc, code)
				} else {
					region := regions[rand.Intn(len(regions))]
					traceID := traceIDs[rand.Intn(len(traceIDs))]
					latency := rand.Intn(2000)
					lineBuf = buildComplexLine(lineBuf, ts, level, svc, code, region, traceID, latency)
				}

			case "invalid":
				if rand.Float64() < *invalidRate {
					lineBuf = lineBuf[:0]
					lineBuf = append(lineBuf, pickInvalidLine()...)
				} else {
					lineBuf = buildSimpleLine(lineBuf, ts, level, svc, code)
				}
			}

			if _, err := w.Write(lineBuf); err != nil {
				fmt.Fprintf(os.Stderr, "write error: %v\n", err)
				os.Exit(1)
			}
			_ = w.WriteByte('\n')

			total++
			produced++
		}

		// 定期 flush stdout
		if now.Sub(lastFlush) >= time.Duration(*flushEvery)*time.Millisecond {
			_ = w.Flush()
			lastFlush = now
		}

		// 定期在 stderr 印進度
		if now.Sub(lastReport) >= time.Second {
			windowCount := total - reportBase
			windowSecs := now.Sub(lastReport).Seconds()
			fmt.Fprintf(os.Stderr,
				"[gen] total=%d inst=%.0f/s avg=%.0f/s\n",
				total,
				float64(windowCount)/windowSecs,
				float64(total)/now.Sub(start).Seconds(),
			)
			lastReport = now
			reportBase = total
		}

		if total >= shouldHaveSent {
			time.Sleep(200 * time.Microsecond)
		}
	}

	_ = w.Flush()
	elapsed := time.Since(start).Seconds()
	fmt.Fprintf(os.Stderr,
		"[gen] done: total=%d elapsed=%.3fs avg=%.0f/s\n",
		total, elapsed, float64(total)/elapsed,
	)
}
