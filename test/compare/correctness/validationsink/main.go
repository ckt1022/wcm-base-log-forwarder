// validationsink validates every WCM output record for correctness experiments.
package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/signal"
	"sort"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

var metadata = map[string]string{
	"region": "ap-southeast-1", "zone": "ap-southeast-1a",
	"environment": "production", "hostname": "worker-node-17",
	"namespace": "payments", "pod_name": "payment-api-7f84",
	"container_name": "payment-api", "cluster": "prod-taipei-01",
	"version": "2026.07.31", "trace_id": "4bf92f3577b34da6",
	"span_id": "00f067aa0ba902b7", "request_id": "req-01J4M7V8K2",
	"method": "POST", "path": "/api/v2/orders", "protocol": "HTTP/1.1",
	"status_code": "503", "latency_ms": "1240", "client_ip": "192.168.10.42",
	"user_agent": "checkout-service", "tenant_id": "tenant-1042",
	"account_id": "acct-728194", "operation": "create-order",
	"component": "http-handler", "datacenter": "tpe-dc-02",
	"release": "payment-api-r42", "team": "platform-observe",
}

var templates = [5]struct{ level, msg, service, code string }{
	{"info", "Request completed successfully", "api-gateway", "200"},
	{"warn", "Upstream timeout after 30s waiting for response", "payment-svc", "503"},
	{"error", "Token validation failed: signature mismatch", "auth-service", "401"},
	{"debug", "Health check passed", "notify-svc", "200"},
	{"info", "Database query executed in 3ms", "order-svc", "200"},
}

// malformedPerMillion < 0：舊的固定 5% 樣式（seq%100 ∈ {0,21,42,63,84}，預設、行為不變）。
// >= 0：由 -malformed-rate（百分比）設定，以 seq 的乘法雜湊決定，比例 = rate%（generator 與 sink 必須用同一值）。
var malformedPerMillion int64 = -1

func malformedInput(seq uint64) bool {
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

type result struct {
	Expected        uint64   `json:"expected"`
	Received        uint64   `json:"received"`
	Unique          uint64   `json:"unique"`
	Missing         uint64   `json:"missing"`
	Duplicate       uint64   `json:"duplicate"`
	Corrupt         uint64   `json:"corrupt"`
	Unexpected      uint64   `json:"unexpected"`
	OrderInversions uint64   `json:"order_inversions"`
	BadJSON         uint64   `json:"bad_output_json"`
	HTTPBatches     uint64   `json:"http_batches"`
	FirstMissing    []uint64 `json:"first_missing_ids"`
	Errors          []string `json:"first_errors"`
}

type validator struct {
	mu               sync.Mutex
	inputCount       uint64
	malformed        bool
	edge             bool
	expectedLang     string
	requireUniqueID  bool
	logFirstPerBatch bool
	seen             []bool
	idSeen           map[uint64]bool
	lastSeq          uint64
	haveLastSeq      bool
	res              result
	secLines         uint64
	secLats          []float64
	series           []secRow
	seriesStop       chan struct{}
	seriesDone       chan struct{}
	firstLog         string
	latestBatchLog   string
}

type secRow struct {
	Unix       float64
	Lines      uint64
	Cumulative uint64
	P50        float64
	P99        float64
	P999       float64
}

type liveStatus struct {
	Status          string  `json:"status"`
	ServerUnix      float64 `json:"server_time_unix_s"`
	ReceivedTotal   uint64  `json:"received_total"`
	UniqueTotal     uint64  `json:"unique_total"`
	Last1sEPS       uint64  `json:"last_1s_eps"`
	Rolling10sEPS   float64 `json:"rolling_10s_eps"`
	Last1sP50       float64 `json:"last_1s_p50_ms"`
	Last1sP99       float64 `json:"last_1s_p99_ms"`
	Last1sP999      float64 `json:"last_1s_p99_9_ms"`
	HTTPBatches     uint64  `json:"http_batches"`
	LatestBatch     uint64  `json:"latest_batch"`
	LatestBatchLog  string  `json:"latest_batch_first_log,omitempty"`
	Duplicate       uint64  `json:"duplicate"`
	Corrupt         uint64  `json:"corrupt"`
	Unexpected      uint64  `json:"unexpected"`
	OrderInversions uint64  `json:"order_inversions"`
	BadJSON         uint64  `json:"bad_output_json"`
	SampleLog       string  `json:"sample_log,omitempty"`
}

func quantile(values []float64, q float64) float64 {
	if len(values) == 0 {
		return 0
	}
	idx := int(q * float64(len(values)-1))
	return values[idx]
}

func (v *validator) snapshot(now time.Time) {
	v.mu.Lock()
	defer v.mu.Unlock()
	sort.Float64s(v.secLats)
	v.series = append(v.series, secRow{Unix: float64(now.UnixNano()) / 1e9,
		Lines: v.secLines, Cumulative: v.res.Received,
		P50: quantile(v.secLats, .50), P99: quantile(v.secLats, .99), P999: quantile(v.secLats, .999)})
	v.secLines = 0
	v.secLats = v.secLats[:0]
}

func (v *validator) startSeries() {
	v.seriesStop = make(chan struct{})
	v.seriesDone = make(chan struct{})
	go func() {
		defer close(v.seriesDone)
		ticker := time.NewTicker(time.Second)
		defer ticker.Stop()
		for {
			select {
			case now := <-ticker.C:
				v.snapshot(now)
			case <-v.seriesStop:
				v.snapshot(time.Now())
				return
			}
		}
	}()
}

func (v *validator) addError(message string) {
	v.res.Corrupt++
	if len(v.res.Errors) < 50 {
		v.res.Errors = append(v.res.Errors, message)
	}
}

func asString(m map[string]any, key string) (string, bool) {
	v, ok := m[key].(string)
	return v, ok
}

func (v *validator) validateLine(line []byte) {
	var m map[string]any
	if err := json.Unmarshal(line, &m); err != nil {
		v.res.BadJSON++
		if len(v.res.Errors) < 50 {
			v.res.Errors = append(v.res.Errors, "bad output JSON: "+err.Error())
		}
		return
	}
	eventID, ok := asString(m, "event_id")
	if !ok {
		v.res.Unexpected++
		if len(v.res.Errors) < 50 {
			v.res.Errors = append(v.res.Errors, "missing/non-string event_id")
		}
		return
	}
	seq, err := strconv.ParseUint(eventID, 10, 64)
	if err != nil || seq >= v.inputCount {
		v.res.Unexpected++
		if len(v.res.Errors) < 50 {
			v.res.Errors = append(v.res.Errors, "invalid event_id="+eventID)
		}
		return
	}
	if v.seen[seq] {
		v.res.Duplicate++
	} else {
		v.seen[seq] = true
		v.res.Unique++
	}
	if v.haveLastSeq && seq <= v.lastSeq {
		v.res.OrderInversions++
	}
	v.lastSeq, v.haveLastSeq = seq, true

	t := templates[seq%5]
	if t.level != "warn" && t.level != "error" {
		v.res.Unexpected++
		if len(v.res.Errors) < 50 {
			v.res.Errors = append(v.res.Errors, fmt.Sprintf("filtered level emitted event_id=%s level=%s", eventID, t.level))
		}
	}
	checks := map[string]string{
		"event_id": eventID, "level": t.level, "message": t.msg,
		"service": t.service, "code": t.code, "lang": v.expectedLang,
	}
	for key, value := range metadata {
		checks[key] = value
	}
	for key, want := range checks {
		if got, exists := asString(m, key); !exists || got != want {
			v.addError(fmt.Sprintf("event_id=%s field=%s got=%v want=%q", eventID, key, m[key], want))
		}
	}
	if ts, exists := asString(m, "timestamp"); !exists {
		v.addError("event_id=" + eventID + " missing/non-string timestamp")
	} else if parsed, err := time.Parse(time.RFC3339Nano, ts); err != nil {
		v.addError("event_id=" + eventID + " bad timestamp=" + ts)
	} else {
		v.secLats = append(v.secLats, float64(time.Now().Sub(parsed))/float64(time.Millisecond))
	}
	if id, exists := m["id"].(float64); !exists || id < 0 || id != float64(uint64(id)) {
		v.addError(fmt.Sprintf("event_id=%s invalid id=%v", eventID, m["id"]))
	} else {
		uid := uint64(id)
		if v.requireUniqueID {
			if v.idSeen[uid] {
				v.addError(fmt.Sprintf("event_id=%s duplicate output id=%d", eventID, uid))
			}
			v.idSeen[uid] = true
		}
	}
	if v.edge {
		edgeChecks := map[string]string{"edge_bool": "true", "edge_int": "-42", "edge_float": "12.5", "edge_null": "null"}
		for key, want := range edgeChecks {
			if got, exists := asString(m, key); !exists || got != want {
				v.addError(fmt.Sprintf("event_id=%s field=%s got=%v want=%q", eventID, key, m[key], want))
			}
		}
		prefix := "台北🚀 \"quote\" \\ slash "
		got, exists := asString(m, "edge_unicode")
		padding := strings.TrimPrefix(got, prefix)
		if !exists || !strings.HasPrefix(got, prefix) || padding == "" || strings.Trim(padding, "x") != "" {
			v.addError(fmt.Sprintf("event_id=%s invalid edge_unicode", eventID))
		}
	} else {
		for i := 0; i < 5; i++ {
			key := fmt.Sprintf("detail_%02d", i)
			got, exists := asString(m, key)
			if !exists || len(got) == 0 || strings.Trim(got, "x") != "" {
				v.addError(fmt.Sprintf("event_id=%s invalid %s", eventID, key))
			}
		}
	}
	allowed := map[string]bool{"id": true, "timestamp": true, "level": true, "message": true,
		"lang": true, "event_id": true, "service": true, "code": true}
	for key := range metadata {
		allowed[key] = true
	}
	if v.edge {
		for _, key := range []string{"edge_bool", "edge_int", "edge_float", "edge_null", "edge_unicode"} {
			allowed[key] = true
		}
	} else {
		for i := 0; i < 5; i++ {
			allowed[fmt.Sprintf("detail_%02d", i)] = true
		}
	}
	for key := range m {
		if !allowed[key] {
			v.addError(fmt.Sprintf("event_id=%s unexpected field=%s", eventID, key))
		}
	}
}

func (v *validator) handler(w http.ResponseWriter, r *http.Request) {
	if r.Method == http.MethodGet && r.URL.Path == "/status" {
		v.statusHandler(w)
		return
	}
	if r.Method != http.MethodPost || r.URL.Path != "/ingest" {
		http.NotFound(w, r)
		return
	}
	body, err := io.ReadAll(r.Body)
	if err != nil {
		http.Error(w, "read", http.StatusBadRequest)
		return
	}
	v.mu.Lock()
	v.res.HTTPBatches++
	batchID := v.res.HTTPBatches
	loggedBatchFirst := false
	for _, line := range bytes.Split(body, []byte{'\n'}) {
		if len(bytes.TrimSpace(line)) == 0 {
			continue
		}
		if v.firstLog == "" {
			v.firstLog = string(line)
			if !v.logFirstPerBatch {
				fmt.Printf("[validationsink] first_received_log=%s\n", line)
			}
		}
		if !loggedBatchFirst {
			v.latestBatchLog = string(line)
			if v.logFirstPerBatch {
				fmt.Printf("[validationsink] batch=%d first_received_log=%s\n", batchID, line)
			}
			loggedBatchFirst = true
		}
		v.res.Received++
		v.secLines++
		v.validateLine(line)
	}
	v.mu.Unlock()
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte(`{"status":"ok"}`))
}

func (v *validator) statusHandler(w http.ResponseWriter) {
	v.mu.Lock()
	status := liveStatus{
		Status:          "waiting",
		ServerUnix:      float64(time.Now().UnixNano()) / 1e9,
		ReceivedTotal:   v.res.Received,
		UniqueTotal:     v.res.Unique,
		HTTPBatches:     v.res.HTTPBatches,
		LatestBatch:     v.res.HTTPBatches,
		LatestBatchLog:  v.latestBatchLog,
		Duplicate:       v.res.Duplicate,
		Corrupt:         v.res.Corrupt,
		Unexpected:      v.res.Unexpected,
		OrderInversions: v.res.OrderInversions,
		BadJSON:         v.res.BadJSON,
		SampleLog:       v.firstLog,
	}
	if v.res.Received > 0 {
		status.Status = "receiving"
	}
	if len(v.series) > 0 {
		last := v.series[len(v.series)-1]
		status.Last1sEPS = last.Lines
		status.Last1sP50 = last.P50
		status.Last1sP99 = last.P99
		status.Last1sP999 = last.P999
		start := len(v.series) - 10
		if start < 0 {
			start = 0
		}
		var lines uint64
		for _, row := range v.series[start:] {
			lines += row.Lines
		}
		status.Rolling10sEPS = float64(lines) / float64(len(v.series)-start)
	}
	v.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	if err := enc.Encode(status); err != nil {
		fmt.Fprintln(os.Stderr, "status response:", err)
	}
}

func (v *validator) finish(outDir string) error {
	close(v.seriesStop)
	<-v.seriesDone
	v.mu.Lock()
	defer v.mu.Unlock()
	for seq := uint64(0); seq < v.inputCount; seq++ {
		t := templates[seq%5]
		if t.level != "warn" && t.level != "error" {
			continue
		}
		if v.malformed && malformedInput(seq) {
			continue
		}
		v.res.Expected++
		if !v.seen[seq] {
			v.res.Missing++
			if len(v.res.FirstMissing) < 50 {
				v.res.FirstMissing = append(v.res.FirstMissing, seq)
			}
		}
	}
	f, err := os.Create(outDir + "/validation_summary.json")
	if err != nil {
		return err
	}
	defer f.Close()
	enc := json.NewEncoder(f)
	enc.SetIndent("", "  ")
	if err := enc.Encode(v.res); err != nil {
		return err
	}
	seriesFile, err := os.Create(outDir + "/sink_per_sec.csv")
	if err != nil {
		return err
	}
	w := bufio.NewWriter(seriesFile)
	fmt.Fprintln(w, "unix_s,lines,cumulative,p50_ms,p99_ms,p99_9_ms")
	for _, row := range v.series {
		fmt.Fprintf(w, "%.6f,%d,%d,%.3f,%.3f,%.3f\n", row.Unix, row.Lines, row.Cumulative, row.P50, row.P99, row.P999)
	}
	_ = w.Flush()
	return seriesFile.Close()
}

func main() {
	port := flag.String("port", "8080", "listen port")
	outDir := flag.String("out-dir", ".", "output directory")
	inputCount := flag.Uint64("input-count", 100000, "exact generator input count")
	malformed := flag.Bool("malformed", false, "generator uses deterministic malformed pattern")
	malformedRate := flag.Float64("malformed-rate", -1, "with -malformed: percent of records replaced by malformed lines (0-100, e.g. 0.1); default -1 keeps the fixed 5% pattern")
	edge := flag.Bool("edge-types", false, "validate boolean/number/null/Unicode/escaped values")
	expectedLang := flag.String("expected-lang", "Rust", "expected output lang field")
	requireUniqueID := flag.Bool("require-unique-id", true, "require the numeric output id to be unique")
	logFirstPerBatch := flag.Bool("log-first-per-batch", false, "write the first received log of every HTTP batch to stdout")
	flag.Parse()
	if *malformedRate > 100 {
		fmt.Fprintln(os.Stderr, "-malformed-rate must be <= 100")
		os.Exit(2)
	}
	if *malformedRate >= 0 {
		malformedPerMillion = int64(*malformedRate*10000 + 0.5)
	}
	v := &validator{inputCount: *inputCount, malformed: *malformed, edge: *edge,
		expectedLang: *expectedLang, requireUniqueID: *requireUniqueID, logFirstPerBatch: *logFirstPerBatch,
		seen: make([]bool, *inputCount), idSeen: make(map[uint64]bool)}
	v.startSeries()

	srv := &http.Server{Addr: "0.0.0.0:" + *port, Handler: http.HandlerFunc(v.handler)}
	go func() {
		if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
	}()
	fmt.Printf("[validationsink] listening :%s expected-input=%d malformed=%v\n", *port, *inputCount, *malformed)
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	<-sig
	if err := v.finish(*outDir); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	w := bufio.NewWriter(os.Stdout)
	v.mu.Lock()
	fmt.Fprintf(w, "[validationsink] done expected=%d received=%d unique=%d missing=%d duplicate=%d corrupt=%d unexpected=%d reorder=%d bad_json=%d\n",
		v.res.Expected, v.res.Received, v.res.Unique, v.res.Missing, v.res.Duplicate,
		v.res.Corrupt, v.res.Unexpected, v.res.OrderInversions, v.res.BadJSON)
	v.mu.Unlock()
	_ = w.Flush()
}
