// Native Go log parsing throughput benchmark.
//
// Usage:
//
//	go run tools/gen/main.go -mode json-simple  | go run benchmark/go/main.go -mode json
//	go run tools/gen/main.go -mode logfmt-mixed | go run benchmark/go/main.go -mode logfmt
//	go run tools/gen/main.go -mode syslog-complex | go run benchmark/go/main.go -mode syslog
package main

import (
	"bufio"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"runtime"
	"strconv"
	"strings"
	"time"
)

type SelfLog struct {
	Ts    string            `json:"ts"`
	Level string            `json:"level"`
	Msg   string            `json:"msg"`
	Att   map[string]string `json:"att"`
}

// ------------------------------------------------------------------ JSON

func parseJSON(line string) (SelfLog, error) {
	var log SelfLog
	err := json.Unmarshal([]byte(line), &log)
	return log, err
}

// ------------------------------------------------------------------ logfmt

// parseQuoted extracts a Go-quoted string from the start of s.
// Returns the unquoted value and the remaining string after the closing quote.
func parseQuoted(s string) (val, rest string, err error) {
	i := 1 // skip opening "
	for i < len(s) {
		switch s[i] {
		case '\\':
			i += 2
		case '"':
			i++
			val, err = strconv.Unquote(s[:i])
			return val, s[i:], err
		default:
			i++
		}
	}
	return "", s, fmt.Errorf("unterminated quoted string")
}

func parseLogfmt(line string) (SelfLog, error) {
	var log SelfLog
	log.Att = make(map[string]string)

	rest := line
	for len(rest) > 0 {
		rest = strings.TrimLeft(rest, " \t")
		if len(rest) == 0 {
			break
		}

		eqIdx := strings.IndexByte(rest, '=')
		if eqIdx < 0 {
			break
		}
		key := rest[:eqIdx]
		rest = rest[eqIdx+1:]

		var val string
		if len(rest) > 0 && rest[0] == '"' {
			var err error
			val, rest, err = parseQuoted(rest)
			if err != nil {
				return log, fmt.Errorf("logfmt: key %q: %w", key, err)
			}
		} else {
			spIdx := strings.IndexByte(rest, ' ')
			if spIdx < 0 {
				val = rest
				rest = ""
			} else {
				val = rest[:spIdx]
				rest = rest[spIdx:]
			}
		}

		switch key {
		case "ts":
			log.Ts = val
		case "level":
			log.Level = val
		case "msg":
			log.Msg = val
		default:
			log.Att[key] = val
		}
	}

	return log, nil
}

// ------------------------------------------------------------------ syslog
//
// Format produced by tools/gen/main.go:
//   <PRI>1 TS HOST APPNAME - - - MSG level=val code=val [region=... ...]

func parseSyslog(line string) (SelfLog, error) {
	var log SelfLog
	log.Att = make(map[string]string)

	if len(line) == 0 || line[0] != '<' {
		return log, fmt.Errorf("syslog: missing '<'")
	}
	closeAngle := strings.IndexByte(line, '>')
	if closeAngle < 0 {
		return log, fmt.Errorf("syslog: missing '>'")
	}
	rest := line[closeAngle+1:] // "1 TS HOST APPNAME - - - ..."

	// consume version token
	rest = skipField(rest)
	if rest == "" {
		return log, fmt.Errorf("syslog: truncated after version")
	}

	// timestamp
	spIdx := strings.IndexByte(rest, ' ')
	if spIdx < 0 {
		return log, fmt.Errorf("syslog: missing timestamp")
	}
	log.Ts = rest[:spIdx]
	rest = rest[spIdx+1:]

	// hostname (skip — may also appear in kv pairs for complex lines)
	rest = skipField(rest)
	if rest == "" {
		return log, fmt.Errorf("syslog: truncated after hostname")
	}

	// appname (skip — appears in kv pairs as "service" for simple lines)
	rest = skipField(rest)
	if rest == "" {
		return log, fmt.Errorf("syslog: truncated after appname")
	}

	// skip three dash tokens: procid, msgid, structured-data
	for range [3]struct{}{} {
		rest = skipField(rest)
		if rest == "" {
			return log, fmt.Errorf("syslog: truncated in dash fields")
		}
	}

	// rest = "MSG level=val ..."
	// find " level=" which always immediately follows the message text
	levelSep := strings.Index(rest, " level=")
	if levelSep < 0 {
		log.Msg = rest
		return log, nil
	}
	log.Msg = rest[:levelSep]
	kvStr := rest[levelSep+1:] // "level=val code=val ..."

	// reuse logfmt key=value parser for the trailing pairs
	kvLog, err := parseLogfmt(kvStr)
	if err != nil {
		return log, err
	}
	log.Level = kvLog.Level
	for k, v := range kvLog.Att {
		log.Att[k] = v
	}
	// absorb ts/msg if they appeared in kv (they shouldn't, but be safe)
	if kvLog.Ts != "" {
		log.Ts = kvLog.Ts
	}

	return log, nil
}

// skipField advances past the next space-delimited token and returns the remainder.
func skipField(s string) string {
	spIdx := strings.IndexByte(s, ' ')
	if spIdx < 0 {
		return ""
	}
	return s[spIdx+1:]
}

// ------------------------------------------------------------------ main

func main() {
	mode := flag.String("mode", "auto", "parse mode: auto | json | logfmt | syslog")
	limit := flag.Int("limit", 50000, "maximum records to benchmark; <=0 means all stdin")
	chunkSize := flag.Int("chunk", 50000, "print logic processing time for each N records; <=0 disables chunk output")
	cpu := flag.Int("cpu", 1, "GOMAXPROCS value; 1 limits Go execution to about one CPU core (100%)")
	repeat := flag.Int("repeat", 1, "number of timed parse passes over the loaded input")
	flag.Parse()

	if *cpu < 1 {
		*cpu = 1
	}
	prevCPU := runtime.GOMAXPROCS(*cpu)
	if *repeat < 1 {
		*repeat = 1
	}

	parseFn, err := parserForMode(*mode)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}

	fmt.Fprintf(
		os.Stderr,
		"[bench] mode=%s limit=%d chunk=%d repeat=%d GOMAXPROCS=%d(previous=%d) reading stdin...\n",
		*mode,
		*limit,
		*chunkSize,
		*repeat,
		*cpu,
		prevCPU,
	)

	readStart := time.Now()
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 1<<20), 1<<20)

	runtime.GC()
	var before, after runtime.MemStats
	runtime.ReadMemStats(&before)

	var inputLines, parsed, errors uint64
	var chunkTimes []time.Duration
	var parseElapsed time.Duration
	chunkCap := maxInt(*chunkSize, 1)
	lines := make([]string, 0, chunkCap)

	flushChunk := func(final bool) {
		if len(lines) == 0 {
			return
		}
		chunkIndex := len(chunkTimes) + 1
		chunkStartLine := inputLines - uint64(len(lines))
		chunkEndLine := inputLines - 1
		chunkStarted := time.Now()

		var chunkParsed, chunkErrors uint64
		for pass := 0; pass < *repeat; pass++ {
			p, e := parseLines(lines, parseFn)
			chunkParsed += p
			chunkErrors += e
		}

		chunkElapsed := time.Since(chunkStarted)
		parseElapsed += chunkElapsed
		parsed += chunkParsed
		errors += chunkErrors
		chunkTimes = append(chunkTimes, chunkElapsed)

		label := "chunk"
		if final {
			label = "final"
		}
		attempts := len(lines) * *repeat
		fmt.Fprintf(
			os.Stderr,
			"[%s] #%d range=%d..%d lines=%d repeat=%d attempts=%d logic=%.3f ms logic_per_50000=%.3f ms throughput=%.0f entries/s\n",
			label,
			chunkIndex,
			chunkStartLine,
			chunkEndLine,
			len(lines),
			*repeat,
			attempts,
			float64(chunkElapsed.Nanoseconds())/1e6,
			float64(chunkElapsed.Nanoseconds())/float64(attempts)*50000.0/1e6,
			float64(attempts)/chunkElapsed.Seconds(),
		)
		lines = lines[:0]
	}

	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			continue
		}
		lines = append(lines, line)
		inputLines++
		if *chunkSize > 0 && len(lines) >= *chunkSize {
			flushChunk(false)
		}
		if *limit > 0 && inputLines >= uint64(*limit) {
			break
		}
	}
	flushChunk(true)

	if err := scanner.Err(); err != nil {
		fmt.Fprintf(os.Stderr, "[bench] scan error: %v\n", err)
	}
	readElapsed := time.Since(readStart)
	runtime.ReadMemStats(&after)

	if inputLines == 0 {
		fmt.Fprintln(os.Stderr, "[bench] no input lines")
		os.Exit(1)
	}

	totalAttempts := inputLines * uint64(*repeat)
	parseSeconds := parseElapsed.Seconds()
	if parseSeconds == 0 {
		parseSeconds = 1e-9
	}

	fmt.Fprintf(os.Stderr, "\n")
	fmt.Fprintf(os.Stderr, "=== native-go-parse-result ===\n")
	fmt.Fprintf(os.Stderr, "mode              : %s\n", *mode)
	fmt.Fprintf(os.Stderr, "gomaxprocs        : %d (CPU limit ~= 100%% of one core)\n", *cpu)
	fmt.Fprintf(os.Stderr, "input_lines        : %d\n", inputLines)
	fmt.Fprintf(os.Stderr, "repeat            : %d\n", *repeat)
	fmt.Fprintf(os.Stderr, "attempts          : %d\n", totalAttempts)
	fmt.Fprintf(os.Stderr, "parsed            : %d\n", parsed)
	fmt.Fprintf(os.Stderr, "errors            : %d\n", errors)
	fmt.Fprintf(os.Stderr, "read_elapsed      : %.3f ms (not included in parse timing)\n", float64(readElapsed.Nanoseconds())/1e6)
	fmt.Fprintf(os.Stderr, "parse_elapsed     : %.3f ms\n", float64(parseElapsed.Nanoseconds())/1e6)
	fmt.Fprintf(os.Stderr, "logic_per_50000   : %.3f ms\n", float64(parseElapsed.Nanoseconds())/float64(totalAttempts)*50000.0/1e6)
	if len(chunkTimes) > 0 {
		minChunk, maxChunk, avgChunk := chunkStats(chunkTimes)
		fmt.Fprintf(os.Stderr, "chunk_count       : %d\n", len(chunkTimes))
		fmt.Fprintf(os.Stderr, "chunk_avg         : %.3f ms\n", float64(avgChunk.Nanoseconds())/1e6)
		fmt.Fprintf(os.Stderr, "chunk_min         : %.3f ms\n", float64(minChunk.Nanoseconds())/1e6)
		fmt.Fprintf(os.Stderr, "chunk_max         : %.3f ms\n", float64(maxChunk.Nanoseconds())/1e6)
	}
	fmt.Fprintf(os.Stderr, "avg_per_attempt   : %.1f ns\n", float64(parseElapsed.Nanoseconds())/float64(totalAttempts))
	fmt.Fprintf(os.Stderr, "throughput        : %.0f entries/s\n", float64(totalAttempts)/parseSeconds)
	fmt.Fprintf(os.Stderr, "alloc_bytes       : %d\n", after.TotalAlloc-before.TotalAlloc)
	fmt.Fprintf(os.Stderr, "mallocs           : %d\n", after.Mallocs-before.Mallocs)
	fmt.Fprintf(os.Stderr, "frees             : %d\n", after.Frees-before.Frees)
	fmt.Fprintf(os.Stderr, "gc_count          : %d\n", after.NumGC-before.NumGC)
}

func parseLines(lines []string, parseFn func(string) (SelfLog, error)) (parsed, errors uint64) {
	for _, line := range lines {
		if _, err := parseFn(line); err != nil {
			errors++
		} else {
			parsed++
		}
	}
	return parsed, errors
}

func chunkStats(chunks []time.Duration) (min, max, avg time.Duration) {
	if len(chunks) == 0 {
		return 0, 0, 0
	}

	min = chunks[0]
	max = chunks[0]
	var total time.Duration
	for _, chunk := range chunks {
		if chunk < min {
			min = chunk
		}
		if chunk > max {
			max = chunk
		}
		total += chunk
	}
	return min, max, total / time.Duration(len(chunks))
}

func parserForMode(mode string) (func(string) (SelfLog, error), error) {
	switch mode {
	case "auto":
		return parseAuto, nil
	case "json":
		return parseJSON, nil
	case "logfmt":
		return parseLogfmt, nil
	case "syslog":
		return parseSyslog, nil
	default:
		return nil, fmt.Errorf("mode must be one of: auto | json | logfmt | syslog")
	}
}

func parseAuto(line string) (SelfLog, error) {
	line = strings.TrimSpace(line)
	if line == "" {
		return SelfLog{}, nil
	}
	switch line[0] {
	case '{':
		return parseJSON(line)
	case '<':
		return parseSyslog(line)
	default:
		return parseLogfmt(line)
	}
}

func maxInt(a, b int) int {
	if a > b {
		return a
	}
	return b
}
