// correctness generator emits deterministic 1024-byte/36-field JSON records.
package main

import (
	"bufio"
	"encoding/json"
	"flag"
	"fmt"
	"net"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync/atomic"
	"time"
)

var metadata = [][2]string{
	{"region", "ap-southeast-1"}, {"zone", "ap-southeast-1a"},
	{"environment", "production"}, {"hostname", "worker-node-17"},
	{"namespace", "payments"}, {"pod_name", "payment-api-7f84"},
	{"container_name", "payment-api"}, {"cluster", "prod-taipei-01"},
	{"version", "2026.07.31"}, {"trace_id", "4bf92f3577b34da6"},
	{"span_id", "00f067aa0ba902b7"}, {"request_id", "req-01J4M7V8K2"},
	{"method", "POST"}, {"path", "/api/v2/orders"},
	{"protocol", "HTTP/1.1"}, {"status_code", "503"},
	{"latency_ms", "1240"}, {"client_ip", "192.168.10.42"},
	{"user_agent", "checkout-service"}, {"tenant_id", "tenant-1042"},
	{"account_id", "acct-728194"}, {"operation", "create-order"},
	{"component", "http-handler"}, {"datacenter", "tpe-dc-02"},
	{"release", "payment-api-r42"}, {"team", "platform-observe"},
}

var templates = [5]struct{ level, msg, service, code string }{
	{"info", "Request completed successfully", "api-gateway", "200"},
	{"warn", "Upstream timeout after 30s waiting for response", "payment-svc", "503"},
	{"error", "Token validation failed: signature mismatch", "auth-service", "401"},
	{"debug", "Health check passed", "notify-svc", "200"},
	{"info", "Database query executed in 3ms", "order-svc", "200"},
}

func addString(b []byte, key, value string) []byte {
	b = append(b, ',', '"')
	b = append(b, key...)
	b = append(b, `":"`...)
	b = append(b, value...)
	return append(b, '"')
}

func validLine(dst []byte, timestamp string, seq uint64) []byte {
	t := templates[seq%5]
	dst = dst[:0]
	dst = append(dst, `{"ts":"`...)
	dst = append(dst, timestamp...)
	dst = append(dst, `","level":"`...)
	dst = append(dst, t.level...)
	dst = append(dst, `","msg":"`...)
	dst = append(dst, t.msg...)
	dst = append(dst, `","att":{"service":"`...)
	dst = append(dst, t.service...)
	dst = append(dst, `","code":"`...)
	dst = append(dst, t.code...)
	dst = append(dst, `"}`...)
	dst = addString(dst, "event_id", fmt.Sprintf("%020d", seq))
	for _, item := range metadata {
		dst = addString(dst, item[0], item[1])
	}

	const details = 5 // 5 base + 26 metadata + 5 detail = 36 top-level fields
	overhead := 1     // final }
	for i := 0; i < details; i++ {
		overhead += len(fmt.Sprintf(`,"detail_%02d":""`, i))
	}
	valueBytes := 1024 - len(dst) - overhead
	if valueBytes < 0 {
		panic("correctness record cannot fit in 1024 bytes")
	}
	for i := 0; i < details; i++ {
		n := valueBytes / (details - i)
		dst = append(dst, fmt.Sprintf(`,"detail_%02d":"`, i)...)
		dst = append(dst, strings.Repeat("x", n)...)
		dst = append(dst, '"')
		valueBytes -= n
	}
	dst = append(dst, '}')
	if len(dst) != 1024 {
		panic(fmt.Sprintf("record length=%d, want 1024", len(dst)))
	}
	return dst
}

func edgeLine(dst []byte, timestamp string, seq uint64) []byte {
	t := templates[seq%5]
	dst = dst[:0]
	dst = append(dst, `{"ts":"`...)
	dst = append(dst, timestamp...)
	dst = append(dst, `","level":"`...)
	dst = append(dst, t.level...)
	dst = append(dst, `","msg":"`...)
	dst = append(dst, t.msg...)
	dst = append(dst, `","att":{"service":"`...)
	dst = append(dst, t.service...)
	dst = append(dst, `","code":"`...)
	dst = append(dst, t.code...)
	dst = append(dst, `"}`...)
	dst = addString(dst, "event_id", fmt.Sprintf("%020d", seq))
	for _, item := range metadata {
		dst = addString(dst, item[0], item[1])
	}
	dst = append(dst, `,"edge_bool":true,"edge_int":-42,"edge_float":12.5,"edge_null":null,"edge_unicode":`...)
	prefix := "台北🚀 \"quote\" \\ slash "
	quoted := strconv.Quote(prefix)
	quoted = quoted[:len(quoted)-1]
	padding := 1024 - len(dst) - len(quoted) - 2
	if padding < 0 {
		panic("edge correctness record cannot fit in 1024 bytes")
	}
	dst = append(dst, quoted...)
	dst = append(dst, strings.Repeat("x", padding)...)
	dst = append(dst, '"', '}')
	if len(dst) != 1024 {
		panic(fmt.Sprintf("edge record length=%d, want 1024", len(dst)))
	}
	return dst
}

// malformedPerMillion < 0：舊的固定 5% 樣式（seq%100 ∈ {0,21,42,63,84}，預設、行為不變）。
// >= 0：由 -malformed-rate（百分比）設定，以 seq 的乘法雜湊決定，比例 = rate%（generator 與 sink 必須用同一值）。
var malformedPerMillion int64 = -1

func isMalformed(seq uint64) bool {
	if malformedPerMillion >= 0 {
		h := (seq + 1) * 0x9E3779B97F4A7C15
		h ^= h >> 31
		return int64(h%1000000) < malformedPerMillion
	}
	switch seq % 100 {
	case 0, 21, 42, 63, 84:
		return true
	default:
		return false
	}
}

func malformedLine(dst []byte, seq uint64) []byte {
	patterns := []string{"not-json-", "{", `{"ts":`, `{"unterminated":"`, `[] trailing`}
	p := patterns[(seq/20)%uint64(len(patterns))]
	dst = append(dst[:0], p...)
	fill := byte(' ')
	if strings.HasPrefix(p, "not-json") || strings.Contains(p, "unterminated") {
		fill = 'x'
	}
	for len(dst) < 1024 {
		dst = append(dst, fill)
	}
	return dst[:1024]
}

type summary struct {
	Count      uint64  `json:"count"`
	Bytes      uint64  `json:"bytes"`
	Malformed  uint64  `json:"malformed"`
	ElapsedSec float64 `json:"elapsed_sec"`
	AverageEPS float64 `json:"average_eps"`
	LagP99Ms   float64 `json:"send_lag_p99_ms"`
	LagP999Ms  float64 `json:"send_lag_p99_9_ms"`
	LagMaximum float64 `json:"send_lag_max_ms"`
	SizeErrors uint64  `json:"size_errors"`
}

type phase struct {
	Name       string
	Rate       int
	Duration   int
	StartSec   int
	EndSec     int
	StartCount uint64
	EndCount   uint64
}

func parsePhases(spec string) ([]phase, error) {
	if strings.TrimSpace(spec) == "" {
		return nil, nil
	}
	var phases []phase
	startSec := 0
	var startCount uint64
	for _, raw := range strings.Split(spec, ",") {
		parts := strings.Split(strings.TrimSpace(raw), ":")
		if len(parts) != 3 {
			return nil, fmt.Errorf("phase %q must be name:rate:seconds", raw)
		}
		rate, err := strconv.Atoi(parts[1])
		if err != nil || rate <= 0 {
			return nil, fmt.Errorf("invalid phase rate in %q", raw)
		}
		duration, err := strconv.Atoi(parts[2])
		if err != nil || duration <= 0 {
			return nil, fmt.Errorf("invalid phase duration in %q", raw)
		}
		count := uint64(rate * duration)
		phases = append(phases, phase{Name: parts[0], Rate: rate, Duration: duration,
			StartSec: startSec, EndSec: startSec + duration,
			StartCount: startCount, EndCount: startCount + count})
		startSec += duration
		startCount += count
	}
	return phases, nil
}

func phaseAtTime(phases []phase, elapsed float64) (string, uint64) {
	if len(phases) == 0 {
		return "flat", 0
	}
	for _, p := range phases {
		if elapsed < float64(p.EndSec) {
			inside := elapsed - float64(p.StartSec)
			if inside < 0 {
				inside = 0
			}
			offered := p.StartCount + uint64(inside*float64(p.Rate))
			if offered > p.EndCount {
				offered = p.EndCount
			}
			return p.Name, offered
		}
	}
	return "post-plan", phases[len(phases)-1].EndCount
}

func percentile(values []float64, q float64) float64 {
	if len(values) == 0 {
		return 0
	}
	idx := int(q * float64(len(values)-1))
	return values[idx]
}

func main() {
	rate := flag.Int("rate", 2500, "records per second")
	count := flag.Uint64("count", 100000, "exact records to emit")
	addr := flag.String("tcp-addr", "127.0.0.1:5140", "WCM TCP endpoint")
	malformed := flag.Bool("malformed", false, "replace deterministic 5% with malformed 1024-byte lines")
	malformedRate := flag.Float64("malformed-rate", -1, "with -malformed: percent of records replaced by malformed lines (0-100, e.g. 0.1); default -1 keeps the fixed 5% pattern")
	edge := flag.Bool("edge-types", false, "emit boolean/number/null/Unicode/escaped values")
	phaseSpec := flag.String("phases", "", "comma-separated name:rate:seconds phase plan")
	phaseCSV := flag.String("phase-csv", "", "write absolute phase boundaries to CSV")
	perSecondCSV := flag.String("per-sec-csv", "", "write offered/sent/pending counters each second")
	summaryPath := flag.String("summary", "generator_summary.json", "summary output")
	flag.Parse()
	if *malformedRate > 100 {
		fmt.Fprintln(os.Stderr, "-malformed-rate must be <= 100")
		os.Exit(2)
	}
	if *malformedRate >= 0 {
		malformedPerMillion = int64(*malformedRate*10000 + 0.5)
	}
	if *rate <= 0 || *count == 0 {
		fmt.Fprintln(os.Stderr, "rate and count must be positive")
		os.Exit(2)
	}

	phases, err := parsePhases(*phaseSpec)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	targetCount := *count
	if len(phases) > 0 {
		targetCount = phases[len(phases)-1].EndCount
	}

	conn, err := net.Dial("tcp", *addr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "connect: %v\n", err)
		os.Exit(1)
	}
	w := bufio.NewWriterSize(conn, 1<<20)
	start := time.Now()
	if *phaseCSV != "" && len(phases) > 0 {
		f, err := os.Create(*phaseCSV)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		fmt.Fprintln(f, "phase,rate_eps,duration_s,start_unix,end_unix,start_count,end_count")
		for _, p := range phases {
			fmt.Fprintf(f, "%s,%d,%d,%.6f,%.6f,%d,%d\n", p.Name, p.Rate, p.Duration,
				float64(start.Add(time.Duration(p.StartSec)*time.Second).UnixNano())/1e9,
				float64(start.Add(time.Duration(p.EndSec)*time.Second).UnixNano())/1e9,
				p.StartCount, p.EndCount)
		}
		_ = f.Close()
	}
	var sentAtomic atomic.Uint64
	statsStop := make(chan struct{})
	statsDone := make(chan struct{})
	if *perSecondCSV != "" {
		go func() {
			defer close(statsDone)
			f, err := os.Create(*perSecondCSV)
			if err != nil {
				fmt.Fprintln(os.Stderr, "per-sec CSV:", err)
				return
			}
			defer f.Close()
			fmt.Fprintln(f, "unix_s,elapsed_s,phase,offered_total,sent_total,pending,send_eps")
			ticker := time.NewTicker(time.Second)
			defer ticker.Stop()
			var previous uint64
			for {
				select {
				case now := <-ticker.C:
					elapsed := now.Sub(start).Seconds()
					name, offered := phaseAtTime(phases, elapsed)
					sent := sentAtomic.Load()
					pending := uint64(0)
					if offered > sent {
						pending = offered - sent
					}
					fmt.Fprintf(f, "%.6f,%.3f,%s,%d,%d,%d,%d\n", float64(now.UnixNano())/1e9,
						elapsed, name, offered, sent, pending, sent-previous)
					previous = sent
				case <-statsStop:
					return
				}
			}
		}()
	} else {
		close(statsDone)
	}
	lastFlush := start
	lastReport := start
	batchCount := 0
	line := make([]byte, 0, 1024)
	lags := make([]float64, 0, targetCount)
	var totalBytes, malformedCount, sizeErrors uint64
	phaseIndex := 0

	for seq := uint64(0); seq < targetCount; seq++ {
		currentRate := *rate
		phaseName := "flat"
		phaseStartCount := uint64(0)
		phaseStartSec := 0
		if len(phases) > 0 {
			for phaseIndex+1 < len(phases) && seq >= phases[phaseIndex].EndCount {
				phaseIndex++
			}
			p := phases[phaseIndex]
			currentRate, phaseName = p.Rate, p.Name
			phaseStartCount, phaseStartSec = p.StartCount, p.StartSec
		}
		scheduled := start.Add(time.Duration(phaseStartSec)*time.Second +
			time.Duration(float64(seq-phaseStartCount+1)/float64(currentRate)*float64(time.Second)))
		if wait := time.Until(scheduled); wait > 0 {
			time.Sleep(wait)
		}
		now := time.Now()
		lag := float64(now.Sub(scheduled)) / float64(time.Millisecond)
		if lag < 0 {
			lag = 0
		}
		lags = append(lags, lag)
		if *malformed && isMalformed(seq) {
			line = malformedLine(line, seq)
			malformedCount++
		} else if *edge {
			line = edgeLine(line, scheduled.Format(time.RFC3339Nano), seq)
		} else {
			line = validLine(line, scheduled.Format(time.RFC3339Nano), seq)
		}
		if len(line) != 1024 {
			sizeErrors++
		}
		if _, err := w.Write(line); err != nil {
			fmt.Fprintf(os.Stderr, "write: %v\n", err)
			os.Exit(1)
		}
		if err := w.WriteByte('\n'); err != nil {
			fmt.Fprintf(os.Stderr, "write newline: %v\n", err)
			os.Exit(1)
		}
		totalBytes += uint64(len(line) + 1)
		sentAtomic.Store(seq + 1)
		batchCount++
		if batchCount >= 500 || time.Since(lastFlush) >= 20*time.Millisecond {
			if err := w.Flush(); err != nil {
				fmt.Fprintf(os.Stderr, "flush: %v\n", err)
				os.Exit(1)
			}
			batchCount = 0
			lastFlush = time.Now()
		}
		if time.Since(lastReport) >= time.Second {
			fmt.Printf("[correctgen] phase=%s sent=%d/%d lag=%.3fms\n", phaseName, seq+1, targetCount, lag)
			lastReport = time.Now()
		}
	}
	if err := w.Flush(); err != nil {
		fmt.Fprintf(os.Stderr, "final flush: %v\n", err)
		os.Exit(1)
	}
	_ = conn.Close()
	close(statsStop)
	<-statsDone
	elapsed := time.Since(start).Seconds()
	sort.Float64s(lags)
	s := summary{
		Count: targetCount, Bytes: totalBytes, Malformed: malformedCount,
		ElapsedSec: elapsed, AverageEPS: float64(targetCount) / elapsed,
		LagP99Ms: percentile(lags, .99), LagP999Ms: percentile(lags, .999),
		LagMaximum: percentile(lags, 1), SizeErrors: sizeErrors,
	}
	f, err := os.Create(*summaryPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "summary: %v\n", err)
		os.Exit(1)
	}
	enc := json.NewEncoder(f)
	enc.SetIndent("", "  ")
	_ = enc.Encode(s)
	_ = f.Close()
	fmt.Printf("[correctgen] done count=%d malformed=%d elapsed=%.3fs avg=%.0f/s lag_p99=%.3fms\n",
		s.Count, s.Malformed, s.ElapsedSec, s.AverageEPS, s.LagP99Ms)
}
