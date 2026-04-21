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
	mode := flag.String("mode", "json", "parse mode: json | logfmt | syslog")
	flag.Parse()

	var parseFn func(string) (SelfLog, error)
	switch *mode {
	case "json":
		parseFn = parseJSON
	case "logfmt":
		parseFn = parseLogfmt
	case "syslog":
		parseFn = parseSyslog
	default:
		fmt.Fprintln(os.Stderr, "mode must be one of: json | logfmt | syslog")
		os.Exit(1)
	}

	fmt.Fprintf(os.Stderr, "[bench] mode=%s  reading from stdin...\n", *mode)

	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 1<<20), 1<<20)

	var total, errors, reportBase uint64
	start := time.Now()
	lastReport := start

	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			continue
		}

		_, err := parseFn(line)
		total++
		if err != nil {
			errors++
		}

		now := time.Now()
		if now.Sub(lastReport) >= time.Second {
			window := total - reportBase
			elapsed := now.Sub(lastReport).Seconds()
			fmt.Fprintf(os.Stderr,
				"[bench] total=%-9d inst=%-9.0f avg=%-9.0f errors=%d\n",
				total,
				float64(window)/elapsed,
				float64(total)/now.Sub(start).Seconds(),
				errors,
			)
			lastReport = now
			reportBase = total
		}
	}

	if err := scanner.Err(); err != nil {
		fmt.Fprintf(os.Stderr, "[bench] scan error: %v\n", err)
	}

	elapsed := time.Since(start).Seconds()
	if elapsed == 0 {
		elapsed = 1e-9
	}
	throughput := float64(total) / elapsed

	fmt.Fprintf(os.Stderr, "\n")
	fmt.Fprintf(os.Stderr, "=== result ===\n")
	fmt.Fprintf(os.Stderr, "mode       : %s\n", *mode)
	fmt.Fprintf(os.Stderr, "total      : %d\n", total)
	fmt.Fprintf(os.Stderr, "errors     : %d\n", errors)
	fmt.Fprintf(os.Stderr, "elapsed    : %.3f s\n", elapsed)
	fmt.Fprintf(os.Stderr, "throughput : %.0f entries/s\n", throughput)
}
