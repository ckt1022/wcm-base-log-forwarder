// HTTP log receiver — latency benchmark server (Go 版本).
//
// 取代 server.py，功能完全相同，但無 GIL，可承受高並發 transport workers。
//
// Usage:
//
//	go run server.go [port]      (預設 port = 5000)
//
// 輸入：POST /ingest，body 為 NDJSON（每行一個 JSON 物件，含 "timestamp" 欄位）
// 輸出：
//   - 每批次打印吞吐量 + 延遲 p50/p99
//   - 收到 SIGINT/SIGTERM 時寫出 server_log/latency.csv + 打印摘要
package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"net/http"
	"os"
	"os/signal"
	"sort"
	"strconv"
	"sync"
	"syscall"
	"time"
)

// ── 全域狀態 ─────────────────────────────────────────────────────────────────

var (
	mu           sync.Mutex
	totalBatches int64
	totalLines   int64
	startMono    time.Time
	lastMono     time.Time
	zeroMono     bool = true // 尚未收到第一個 request

	// latencyBuf: (ts_str, receive_unix float64, latency_ms float64)
	latencyBuf []latRecord
)

type latRecord struct {
	tsStr     string
	receiveNs int64   // time.Now().UnixNano() — 用 int64 避免 float64 精度損失
	latencyMs float64
	slot      string  // _slot 欄位（json-fixed5 模式下為 "1"~"5"，其他模式為 ""）
}

// ── 百分位（線性插值） ────────────────────────────────────────────────────────

func percentile(sorted []float64, p float64) float64 {
	n := len(sorted)
	if n == 0 {
		return 0
	}
	k := (float64(n - 1)) * p / 100.0
	lo := int(math.Floor(k))
	hi := lo + 1
	if hi >= n {
		hi = n - 1
	}
	return sorted[lo] + (k-float64(lo))*(sorted[hi]-sorted[lo])
}

// ── RFC3339Nano 解析 → int64 nanoseconds（避免 float64 精度損失）────────────

func parseRFC3339Ns(s string) int64 {
	t, err := time.Parse(time.RFC3339Nano, s)
	if err != nil {
		return 0
	}
	return t.UnixNano()
}

// ── p50 時序折線圖 ────────────────────────────────────────────────────────────
//
// 在 shutdown 時印出 ASCII 折線圖，y 軸為 p50 延遲，x 軸為時間。
// 固定 y 範圍 -2000ms ~ 3500ms，每格 3s。
// 標記：* 正常  + >800ms  ^ >2000ms  v 負值
// 同時列出所有異常事件（負值 / 尖峰）及其間隔，方便觀察規律。

func printLatencyChart(buf []latRecord) {
	if len(buf) == 0 {
		return
	}

	const (
		chartH    = 22
		bucketSec = 3.0
		yMin      = -2000.0
		yMax      = 3500.0
	)

	t0 := float64(buf[0].receiveNs) / 1e9
	tEnd := float64(buf[len(buf)-1].receiveNs) / 1e9
	span := tEnd - t0
	if span <= 0 {
		return
	}

	chartW := int(span/bucketSec) + 2
	if chartW > 90 {
		chartW = 90
	}
	if chartW < 10 {
		chartW = 10
	}

	// ── 每個 bucket 的 p50 ──────────────────────────────────────────────────
	bktLats := make([][]float64, chartW)
	for _, r := range buf {
		idx := int((float64(r.receiveNs)/1e9 - t0) / bucketSec)
		if idx >= 0 && idx < chartW {
			bktLats[idx] = append(bktLats[idx], r.latencyMs)
		}
	}
	p50s := make([]float64, chartW)
	hasData := make([]bool, chartW)
	for i, lats := range bktLats {
		if len(lats) == 0 {
			continue
		}
		sort.Float64s(lats)
		p50s[i] = percentile(lats, 50)
		hasData[i] = true
	}

	// ── y 軸換算 ─────────────────────────────────────────────────────────────
	yRange := yMax - yMin
	rowOf := func(v float64) int {
		r := int(float64(chartH-1) * (yMax - v) / yRange)
		if r < 0 {
			r = 0
		}
		if r >= chartH {
			r = chartH - 1
		}
		return r
	}

	// ── 建立 rune 格子 ────────────────────────────────────────────────────────
	grid := make([][]rune, chartH)
	for r := range grid {
		grid[r] = make([]rune, chartW)
		for c := range grid[r] {
			grid[r][c] = ' '
		}
	}

	// 零線（·）
	zeroRow := rowOf(0)
	for c := range grid[zeroRow] {
		grid[zeroRow][c] = '·'
	}

	// 描點及垂直連線
	prevRow := -1
	for col := 0; col < chartW; col++ {
		if !hasData[col] {
			prevRow = -1
			continue
		}
		v := p50s[col]
		row := rowOf(v)

		// 相鄰點間的垂直橋接
		if prevRow >= 0 {
			lo, hi := prevRow, row
			if lo > hi {
				lo, hi = hi, lo
			}
			for r := lo + 1; r < hi; r++ {
				if grid[r][col] == ' ' {
					grid[r][col] = '|'
				}
			}
		}

		var ch rune
		switch {
		case v < -100:
			ch = 'v'
		case v > 2000:
			ch = '^'
		case v > 800:
			ch = '+'
		default:
			ch = '*'
		}
		grid[row][col] = ch
		prevRow = row
	}

	// ── 印出圖表 ──────────────────────────────────────────────────────────────
	fmt.Printf("\n[server] ── p50 延遲時序圖（每格 %.0fs｜* ≤800ms  + >800ms  ^ >2000ms  v 負值）──\n", bucketSec)
	for row := 0; row < chartH; row++ {
		yVal := yMax - float64(row)/float64(chartH-1)*yRange
		fmt.Printf(" %6.0fms|%s\n", yVal, string(grid[row]))
	}

	// x 軸底線
	xAxis := make([]rune, chartW)
	for i := range xAxis {
		xAxis[i] = '-'
	}
	fmt.Printf("        +%s\n", string(xAxis))

	// x 軸時間標籤
	xLabels := make([]rune, chartW)
	for i := range xLabels {
		xLabels[i] = ' '
	}
	for t := 0; ; t += 30 {
		col := int(float64(t) / bucketSec)
		if col >= chartW {
			break
		}
		for k, c := range fmt.Sprintf("%d", t) {
			if col+k < chartW {
				xLabels[col+k] = c
			}
		}
	}
	fmt.Printf("         %s (s)\n", string(xLabels))

	// ── 異常事件清單 ──────────────────────────────────────────────────────────
	type anomaly struct {
		tSec float64
		p50  float64
		kind string
	}
	var anomalies []anomaly
	for col := 0; col < chartW; col++ {
		if !hasData[col] {
			continue
		}
		v := p50s[col]
		tSec := (float64(col) + 0.5) * bucketSec
		if v < -100 {
			anomalies = append(anomalies, anomaly{tSec, v, "NEG  "})
		} else if v > 1000 {
			anomalies = append(anomalies, anomaly{tSec, v, "SPIKE"})
		}
	}
	if len(anomalies) == 0 {
		return
	}
	fmt.Printf("\n[server] ── 異常事件（NEG=負值  SPIKE=尖峰>1000ms，共 %d 個）──\n", len(anomalies))
	fmt.Printf("  %6s  %9s  %5s  %s\n", "T(s)", "p50(ms)", "類型", "距上次(s)")
	prevT := -1.0
	for _, a := range anomalies {
		gap := "    ─"
		if prevT >= 0 {
			gap = fmt.Sprintf("%5.0f", a.tSec-prevT)
		}
		fmt.Printf("  %6.0f  %8.0f   %s  %s\n", a.tSec, a.p50, a.kind, gap)
		prevT = a.tSec
	}
}

// ── HTTP handler ─────────────────────────────────────────────────────────────

type logHandler struct{}

func (logHandler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/ingest" || r.Method != http.MethodPost {
		http.NotFound(w, r)
		return
	}

	// 讀完整個 body，再打時間戳
	var buf bytes.Buffer
	_, _ = buf.ReadFrom(r.Body)
	receiveMono := time.Now()
	receiveNs := receiveMono.UnixNano()

	body := buf.Bytes()
	nLines := bytes.Count(body, []byte("\n"))

	// ── 延遲計算（鎖外，避免拉長 critical section）───────────────────────────
	type lineRec struct {
		tsStr     string
		latencyMs float64
		slot      string
	}
	var lineRecs []lineRec

	scanner := bufio.NewScanner(bytes.NewReader(body))
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var obj map[string]json.RawMessage
		if err := json.Unmarshal(line, &obj); err != nil {
			continue
		}
		rawTs, ok := obj["timestamp"]
		if !ok {
			continue
		}
		var tsStr string
		if err := json.Unmarshal(rawTs, &tsStr); err != nil {
			continue
		}
		tsNs := parseRFC3339Ns(tsStr)
		if tsNs == 0 {
			continue
		}
		latMs := float64(receiveNs-tsNs) / 1e6

		var slot string
		if rawSlot, ok2 := obj["_slot"]; ok2 {
			_ = json.Unmarshal(rawSlot, &slot)
		}
		lineRecs = append(lineRecs, lineRec{tsStr: tsStr, latencyMs: latMs, slot: slot})
	}

	// ── 更新全域狀態（持鎖）──────────────────────────────────────────────────
	mu.Lock()
	if zeroMono {
		startMono = receiveMono
		lastMono = receiveMono
		zeroMono = false
	}

	var batchTput float64
	if dt := receiveMono.Sub(lastMono).Seconds(); dt > 0 {
		batchTput = float64(nLines) / dt
	}
	lastMono = receiveMono

	totalBatches++
	totalLines += int64(nLines)
	batchNum := totalBatches
	tl := totalLines

	elapsed := receiveMono.Sub(startMono).Seconds()
	avgTput := 0.0
	if elapsed > 0 {
		avgTput = float64(tl) / elapsed
	}

	for _, rec := range lineRecs {
		latencyBuf = append(latencyBuf, latRecord{
			tsStr:     rec.tsStr,
			receiveNs: receiveNs,
			latencyMs: rec.latencyMs,
			slot:      rec.slot,
		})
	}
	mu.Unlock()

	// ── 每批次輸出 ───────────────────────────────────────────────────────────
	latInfo := "  (no timestamp)"
	if len(lineRecs) > 0 {
		batchLats := make([]float64, len(lineRecs))
		for i, r := range lineRecs {
			batchLats[i] = r.latencyMs
		}
		sort.Float64s(batchLats)
		latInfo = fmt.Sprintf("  lat p50=%6.1fms  p99=%6.1fms",
			percentile(batchLats, 50), percentile(batchLats, 99))
	}

	fmt.Printf("[server] #%5d  %6d lines  %8.0f/s  avg=%8.0f/s%s\n",
		batchNum, nLines, batchTput, avgTput, latInfo)

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte(`{"status":"ok"}`))
}

// ── 關閉：寫 CSV + 摘要 ───────────────────────────────────────────────────────

func shutdown() {
	mu.Lock()
	buf := make([]latRecord, len(latencyBuf))
	copy(buf, latencyBuf)
	tl := totalLines
	elapsed := 0.0
	if !zeroMono {
		elapsed = time.Since(startMono).Seconds()
	}
	mu.Unlock()

	// 寫 CSV
	outDir := "server_log"
	_ = os.MkdirAll(outDir, 0755)
	csvPath := outDir + "/latency_worker_1.csv"
	f, err := os.Create(csvPath)
	if err == nil {
		_, _ = fmt.Fprintln(f, "ts,receive_unix,latency_ms")
		for _, r := range buf {
			_, _ = fmt.Fprintf(f, "%s,%.6f,%.3f\n", r.tsStr, float64(r.receiveNs)/1e9, r.latencyMs)
		}
		f.Close()
		fmt.Printf("\n[server] latency.csv 寫出：%s  (%d 筆)\n", csvPath, len(buf))
	} else {
		fmt.Fprintf(os.Stderr, "[server] 無法寫出 CSV：%v\n", err)
	}

	// 延遲摘要
	if len(buf) == 0 {
		fmt.Println("[server] 無延遲資料可分析。")
	} else {
		lats := make([]float64, len(buf))
		for i, r := range buf {
			lats[i] = r.latencyMs
		}
		sort.Float64s(lats)
		n := len(lats)
		fmt.Printf("[server] ── 延遲分布摘要（全部 %d 筆，含 warmup）──\n", n)
		for _, p := range []float64{50, 95, 99, 99.9} {
			fmt.Printf("  p%-5g = %8.2f ms\n", p, percentile(lats, p))
		}
		fmt.Printf("  min    = %8.2f ms\n", lats[0])
		fmt.Printf("  max    = %8.2f ms\n", lats[n-1])
		fmt.Println("  ※ warmup 過濾（前 15 秒）請用 analyze.py 重新計算。")

		// ── 各 slot 延遲分布（json-fixed5 模式）───────────────────────────────
		slotLats := make(map[string][]float64)
		for _, r := range buf {
			if r.slot != "" {
				slotLats[r.slot] = append(slotLats[r.slot], r.latencyMs)
			}
		}
		if len(slotLats) > 0 {
			slots := make([]string, 0, len(slotLats))
			for s := range slotLats {
				slots = append(slots, s)
			}
			sort.Strings(slots)

			// 對應 fixed5Tpl 的說明
			slotDesc := map[string]string{
				"1": "info  / Request completed / api-gateway  200",
				"2": "warn  / Upstream timeout  / payment-svc  503",
				"3": "error / Token validation  / auth-service 401",
				"4": "debug / Health check      / notify-svc   200",
				"5": "info  / DB query 3ms      / order-svc    200",
			}

			fmt.Printf("\n[server] ── 各 slot 延遲分布（json-fixed5）──\n")
			fmt.Printf("  %-4s  %-6s  %8s  %8s  %8s  %8s  %8s  %s\n",
				"slot", "count", "p50", "p95", "p99", "max", "min", "內容")
			for _, s := range slots {
				sl := slotLats[s]
				sort.Float64s(sl)
				m := len(sl)
				desc := slotDesc[s]
				fmt.Printf("  %-4s  %-6d  %8.2f  %8.2f  %8.2f  %8.2f  %8.2f  %s\n",
					s, m,
					percentile(sl, 50), percentile(sl, 95), percentile(sl, 99),
					sl[m-1], sl[0],
					desc)
			}
		}

		printLatencyChart(buf)
	}

	finalAvg := 0.0
	if elapsed > 0 {
		finalAvg = float64(tl) / elapsed
	}
	fmt.Printf("[server] 停止。  batches=%d  lines=%d  avg=%.0f lines/s\n",
		totalBatches, tl, finalAvg)
}

// ── main ─────────────────────────────────────────────────────────────────────

func main() {
	port := 5000
	if len(os.Args) > 1 {
		if p, err := strconv.Atoi(os.Args[1]); err == nil {
			port = p
		}
	}
	addr := fmt.Sprintf("127.0.0.1:%d", port)

	srv := &http.Server{
		Addr:    addr,
		Handler: logHandler{},
	}

	fmt.Printf("[server] Listening on http://%s/ingest (Go, no GIL)\n", addr)
	fmt.Println("[server] Press Ctrl+C to stop.\n")

	// SIGINT / SIGTERM → 關閉 HTTP server，main() 再呼叫 shutdown()
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-sigCh
		_ = srv.Close() // 讓 ListenAndServe 返回 ErrServerClosed
	}()

	if err := srv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		fmt.Fprintf(os.Stderr, "[server] fatal: %v\n", err)
		os.Exit(1)
	}

	// ListenAndServe 返回後（SIGTERM 觸發 Close），在 main goroutine 執行 shutdown
	shutdown()
}
