package main

import (
	"encoding/json"
	"strconv"
	"strings"
	"time"

	parserplugin "example.com/internal/local/log-process/parser-plugin"
	pipelineprocess "example.com/internal/local/log-process/pipeline-process"
	"go.bytecodealliance.org/cm"
)

type Self_log struct {
	Ts    string            `json:"ts"`
	Level string            `json:"level"`
	Msg   string            `json:"msg"`
	Att   map[string]string `json:"att"`
}

func init() {
	parserplugin.Exports.Parse = ParseLogfmt
	parserplugin.Exports.ReportUsage = ReportUsage
}

var lastExecNs int64

// Parse 將 rawData（list<string>）解析為 JSON 格式的 LogEntry 列表。
func ParseJson(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	start := time.Now()

	rawSlice := rawData.Slice()
	entries := make([]parserplugin.ParsedEntry, 0, len(rawSlice))

	var skipCount int

	for _, rawStr := range rawSlice {
		data := []byte(rawStr)

		var log Self_log
		if err := json.Unmarshal(data, &log); err != nil {
			skipCount++
			continue
		}

		level := parseLogLevel(log.Level)

		pairs := make([][2]string, 0, len(log.Att))
		for key, value := range log.Att {
			pairs = append(pairs, [2]string{key, value})
		}

		entries = append(entries, parserplugin.ParsedEntry{
			Timestamp: log.Ts,
			Level:     level,
			Message:   log.Msg,
			Tags:      cm.ToList(pairs),
		})
	}

	result := cm.ToList(entries)

	_ = skipCount
	lastExecNs = time.Since(start).Nanoseconds()

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](result)
}

// ParseLogfmt 將 rawData（list<string>）解析為 logfmt 格式的 LogEntry 列表。
func ParseLogfmt(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	start := time.Now()

	rawSlice := rawData.Slice()
	entries := make([]parserplugin.ParsedEntry, 0, len(rawSlice))

	kvBuf := make([][2]string, 0, 16)

	var skipCount int

	for _, rawStr := range rawSlice {
		data := []byte(rawStr)

		var entry parserplugin.ParsedEntry
		var err error
		kvBuf = kvBuf[:0]
		entry, kvBuf, err = parse_logfmt(data, kvBuf)
		if err != nil {
			skipCount++
			continue
		}

		entries = append(entries, entry)
	}

	result := cm.ToList(entries)

	_ = skipCount
	lastExecNs = time.Since(start).Nanoseconds()

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](result)
}

// ParseSys 將 rawData（list<string>）解析為 syslog 格式的 LogEntry 列表。
func ParseSys(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	start := time.Now()

	rawSlice := rawData.Slice()
	entries := make([]parserplugin.ParsedEntry, 0, len(rawSlice))

	kvBuf := make([][2]string, 0, 16)

	var skipCount int

	for _, rawStr := range rawSlice {
		data := []byte(rawStr)

		var entry parserplugin.ParsedEntry
		var err error
		kvBuf = kvBuf[:0]
		entry, kvBuf, err = parse_sys(data, kvBuf)
		if err != nil {
			skipCount++
			continue
		}

		entries = append(entries, entry)
	}

	result := cm.ToList(entries)

	_ = skipCount
	lastExecNs = time.Since(start).Nanoseconds()

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](result)
}

func ReportUsage() uint64 {
	return uint64(lastExecNs)
}

func parseLogLevel(s string) pipelineprocess.LogLevel {
	var level pipelineprocess.LogLevel
	if err := level.UnmarshalText([]byte(s)); err != nil {
		return pipelineprocess.LogLevelInfo
	}
	return level
}

func kvGet(kvs [][2]string, key string) string {
	for _, kv := range kvs {
		if kv[0] == key {
			return kv[1]
		}
	}
	return ""
}

func parse_logfmt(data []byte, kvBuf [][2]string) (parserplugin.ParsedEntry, [][2]string, error) {
	var err error
	kvBuf, err = parseLogfmtFields(string(data), kvBuf)
	if err != nil {
		return parserplugin.ParsedEntry{}, kvBuf, err
	}

	ts := kvGet(kvBuf, "ts")
	msg := kvGet(kvBuf, "msg")
	level := parseLogLevel(kvGet(kvBuf, "level"))

	pairs := make([][2]string, 0, len(kvBuf))
	for _, kv := range kvBuf {
		switch kv[0] {
		case "ts", "level", "msg":
			continue
		default:
			pairs = append(pairs, kv)
		}
	}

	entry := parserplugin.ParsedEntry{
		Timestamp: ts,
		Level:     level,
		Message:   msg,
		Tags:      cm.ToList(pairs),
	}
	return entry, kvBuf, nil
}

func parse_sys(data []byte, kvBuf [][2]string) (parserplugin.ParsedEntry, [][2]string, error) {
	line := strings.TrimSpace(string(data))
	if line == "" {
		return parserplugin.ParsedEntry{}, kvBuf, strconv.ErrSyntax
	}

	if !strings.HasPrefix(line, "<") {
		return parserplugin.ParsedEntry{}, kvBuf, strconv.ErrSyntax
	}
	endPRI := strings.IndexByte(line, '>')
	if endPRI <= 1 {
		return parserplugin.ParsedEntry{}, kvBuf, strconv.ErrSyntax
	}

	pri, err := strconv.Atoi(line[1:endPRI])
	if err != nil {
		return parserplugin.ParsedEntry{}, kvBuf, err
	}
	severity := pri % 8

	rest := line[endPRI+1:]

	parts := strings.SplitN(rest, " ", 8)
	if len(parts) < 8 {
		return parserplugin.ParsedEntry{}, kvBuf, strconv.ErrSyntax
	}

	version := parts[0]
	ts := parts[1]
	host := parts[2]
	app := parts[3]
	msgAndMore := parts[7]

	if version != "1" {
		return parserplugin.ParsedEntry{}, kvBuf, strconv.ErrSyntax
	}

	msgAndMore = strings.TrimSpace(msgAndMore)

	msg := msgAndMore
	fieldPart := ""

	if idx := strings.Index(msgAndMore, " level="); idx >= 0 {
		msg = strings.TrimSpace(msgAndMore[:idx])
		fieldPart = strings.TrimSpace(msgAndMore[idx+1:])
	}

	if fieldPart != "" {
		kvBuf, err = parseLogfmtFields(fieldPart, kvBuf)
		if err != nil {
			return parserplugin.ParsedEntry{}, kvBuf, err
		}
	}

	levelStr := kvGet(kvBuf, "level")
	var level pipelineprocess.LogLevel
	if levelStr != "" {
		level = parseLogLevel(levelStr)
	} else {
		level = syslogSeverityToLogLevel(severity)
	}

	pairs := make([][2]string, 0, len(kvBuf)+2)
	pairs = append(pairs,
		[2]string{"host", host},
		[2]string{"app_name", app},
	)
	for _, kv := range kvBuf {
		if kv[0] == "level" {
			continue
		}
		pairs = append(pairs, kv)
	}

	entry := parserplugin.ParsedEntry{
		Timestamp: ts,
		Level:     level,
		Message:   msg,
		Tags:      cm.ToList(pairs),
	}
	return entry, kvBuf, nil
}

func parseLogfmtFields(s string, out [][2]string) ([][2]string, error) {
	i := 0
	n := len(s)

	for i < n {
		for i < n && s[i] == ' ' {
			i++
		}
		if i >= n {
			break
		}

		keyStart := i
		for i < n && s[i] != '=' && s[i] != ' ' {
			i++
		}
		if i >= n || s[i] != '=' {
			return out, strconv.ErrSyntax
		}
		key := s[keyStart:i]
		i++

		if i >= n {
			out = append(out, [2]string{key, ""})
			break
		}

		var val string
		if s[i] == '"' {
			i++

			var b strings.Builder
			for i < n {
				switch s[i] {
				case '\\':
					if i+1 >= n {
						return out, strconv.ErrSyntax
					}
					i++
					switch s[i] {
					case '\\', '"':
						b.WriteByte(s[i])
					case 'n':
						b.WriteByte('\n')
					case 't':
						b.WriteByte('\t')
					case 'r':
						b.WriteByte('\r')
					default:
						b.WriteByte(s[i])
					}
					i++
				case '"':
					i++
					val = b.String()
					goto doneValue
				default:
					b.WriteByte(s[i])
					i++
				}
			}
			return out, strconv.ErrSyntax
		} else {
			valStart := i
			for i < n && s[i] != ' ' {
				i++
			}
			val = s[valStart:i]
		}

	doneValue:
		out = append(out, [2]string{key, val})

		for i < n && s[i] == ' ' {
			i++
		}
	}

	return out, nil
}

func syslogSeverityToLogLevel(sev int) pipelineprocess.LogLevel {
	switch sev {
	case 0, 1, 2, 3:
		return pipelineprocess.LogLevelError
	case 4:
		return pipelineprocess.LogLevelWarn
	case 5, 6:
		return pipelineprocess.LogLevelInfo
	case 7:
		return pipelineprocess.LogLevelDebug
	default:
		return pipelineprocess.LogLevelInfo
	}
}

func main() {}
