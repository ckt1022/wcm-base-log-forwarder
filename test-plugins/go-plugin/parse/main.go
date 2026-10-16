package main

import (
	"encoding/json"
	"runtime"
	"strconv"
	"strings"
	"time"

	parserplugin "example.com/internal/local/log-process/parser-plugin"
	pipelineprocess "example.com/internal/local/log-process/pipeline-process"
	"go.bytecodealliance.org/cm"
)

// ── 預分配 pool ─────────────────────────────────────────────────────────────
//
// 設計原則：Reset() 只把 tagCursor 歸 0，不呼叫 GC。
// 下一批 parse 直接從 offset 0 開始覆蓋 entryPool / tagPool，
// 不產生任何新的 heap allocation（熱路徑 zero-alloc）。
//
// 記憶體估算（max_batch_lines = 50_000）：
//   entryPool : 50001 × ~56B ≈ 2.8 MB
//   tagPool   : 50001 × 20 × 32B ≈ 32 MB

const (
	maxEntriesPerBatch = 60000
	maxTagsPerBatch    = maxEntriesPerBatch * 15
)

var (
	entryPool   [maxEntriesPerBatch]parserplugin.ParsedEntry
	tagPool     [maxTagsPerBatch][2]string
	tagCursor   int
	globalKvBuf = make([][2]string, 0, 32)
)

const debugLifecycle = false

var (
	lastExecNs    int64
	batchSeq      uint64
	lastEntryUsed int
	lastTagUsed   int
	lastRawBytes  int
)

type Self_log struct {
	Ts    string            `json:"ts"`
	Level string            `json:"level"`
	Msg   string            `json:"msg"`
	Att   map[string]string `json:"att"`
}

func init() {
	//parserplugin.Exports.Parse = ParseJson
	// 手寫 JSON parser 測試時，改成：
	parserplugin.Exports.Parse = ParseLogfmt
	parserplugin.Exports.ReportUsage = ReportUsage
}

// Reset 僅重置 pool cursor，不呼叫 GC。
// 下一批 parse() 直接覆蓋 entryPool / tagPool 相同位置。
func Reset() {
	releaseRetainedBatch("reset")
	runtime.GC()
}

// ── ParseJson ────────────────────────────────────────────────────────────────
// JSON 路徑因 json.Unmarshal 內部無法避免 alloc，維持原實作。

func ParseJson(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	lastExecNs = 0
	start_ := time.Now()
	rawSlice := rawData.Slice() //不是這段花時間

	rawBytes := beginBatch(rawSlice)
	tagCursor = 0
	entries := entryPool[:0]
	var skipCount int

	for _, rawStr := range rawSlice {
		start := time.Now()
		var log Self_log
		if err := json.Unmarshal([]byte(rawStr), &log); err != nil {
			skipCount++
			continue
		}
		lastExecNs += time.Since(start).Nanoseconds()

		level := parseLogLevel(log.Level)

		tagStart := tagCursor
		if tagCursor < len(tagPool) {
			tagPool[tagCursor] = [2]string{"lang", "Go"}
			tagCursor++
		}
		for key, value := range log.Att {
			if tagCursor < len(tagPool) {
				tagPool[tagCursor] = [2]string{key, value}
				tagCursor++
			}
		}

		entries = append(entries, parserplugin.ParsedEntry{
			Timestamp: log.Ts,
			Level:     level,
			Message:   log.Msg,
			Tags:      cm.ToList(tagPool[tagStart:tagCursor]),
			Targettag: routeTag(level),
		})
	}

	finishBatch(len(entries), tagCursor, skipCount, rawBytes, start_)
	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](cm.ToList(entries))
}

// ── ParseJsonManual ─────────────────────────────────────────────────────────
// 手寫 parser 版本，針對 generator 目前格式：
// {"ts":"...","level":"...","msg":"...","att":{"k":"v",...}}
//
// lastExecNs 只量測每行 JSON 掃描與欄位取出的時間，用來對照 ParseJson 中
// json.Unmarshal 的量測範圍。ParsedEntry 建立與 cm.ToList 仍維持在計時外。

func ParseJsonManual(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	lastExecNs = 0
	start := time.Now()
	rawSlice := rawData.Slice()

	rawBytes := beginBatch(rawSlice)
	tagCursor = 0
	entries := entryPool[:0]
	var skipCount int

	for _, rawStr := range rawSlice {
		tagStart := tagCursor
		if tagCursor < len(tagPool) {
			tagPool[tagCursor] = [2]string{"lang", "Go"}
			tagCursor++
		}

		ts, levelStr, msg, err := parseJSONLineManual(rawStr)

		if err != nil {
			tagCursor = tagStart
			skipCount++
			continue
		}

		level := parseLogLevel(levelStr)
		entries = append(entries, parserplugin.ParsedEntry{
			Timestamp: ts,
			Level:     level,
			Message:   msg,
			Tags:      cm.ToList(tagPool[tagStart:tagCursor]),
			Targettag: routeTag(level),
		})
	}

	finishBatch(len(entries), tagCursor, skipCount, rawBytes, start)
	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](cm.ToList(entries))
}

// ── ParseLogfmt ──────────────────────────────────────────────────────────────
// 零 alloc 熱路徑：
//   - entryPool[:0]  取代 make([]ParsedEntry, ...)
//   - globalKvBuf    取代 make([][2]string, ...)（跨批次重用）
//   - tagPool        取代 make([][2]string, ...) for pairs（cursor reset）
//   - rawStr 直接傳入 parseLogfmtFields（省去 []byte 轉換）

func ParseLogfmt(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	start := time.Now()

	rawSlice := rawData.Slice()
	rawBytes := beginBatch(rawSlice)
	tagCursor = 0
	entries := entryPool[:0]
	kvBuf := globalKvBuf[:0]
	var skipCount int

	for _, rawStr := range rawSlice {
		kvBuf = kvBuf[:0]
		var err error
		kvBuf, err = parseLogfmtFields(rawStr, kvBuf)
		if err != nil {
			skipCount++
			continue
		}

		ts := kvGet(kvBuf, "ts")
		msg := kvGet(kvBuf, "msg")
		level := parseLogLevel(kvGet(kvBuf, "level"))

		tagStart := tagCursor
		if tagCursor < len(tagPool) {
			tagPool[tagCursor] = [2]string{"lang", "Go"}
			tagCursor++
		}
		for _, kv := range kvBuf {
			switch kv[0] {
			case "ts", "level", "msg":
				continue
			default:
				if tagCursor < len(tagPool) {
					tagPool[tagCursor] = kv
					tagCursor++
				}
			}
		}

		entries = append(entries, parserplugin.ParsedEntry{
			Timestamp: ts,
			Level:     level,
			Message:   msg,
			Tags:      cm.ToList(tagPool[tagStart:tagCursor]),
			Targettag: routeTag(level),
		})
	}

	globalKvBuf = kvBuf
	finishBatch(len(entries), tagCursor, skipCount, rawBytes, start)
	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](cm.ToList(entries))
}

// ── ParseSys ─────────────────────────────────────────────────────────────────
// 零 alloc 熱路徑（同 ParseLogfmt 原則），額外消除：
//   - strings.SplitN → splitInto8（stack-allocated [8]string）
//   - parseLogLevel([]byte(s)) → switch on string（省去 []byte 轉換）

func ParseSys(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	start := time.Now()

	rawSlice := rawData.Slice()
	rawBytes := beginBatch(rawSlice)
	tagCursor = 0
	entries := entryPool[:0]
	kvBuf := globalKvBuf[:0]
	var skipCount int

	for _, rawStr := range rawSlice {
		kvBuf = kvBuf[:0]
		entry, kv, err := parseSysEntry(rawStr, kvBuf)
		kvBuf = kv
		if err != nil {
			skipCount++
			continue
		}
		entries = append(entries, entry)
	}

	globalKvBuf = kvBuf
	finishBatch(len(entries), tagCursor, skipCount, rawBytes, start)
	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](cm.ToList(entries))
}

func ReportUsage() uint64 {
	return uint64(lastExecNs)
}

func beginBatch(rawSlice []string) int {
	releaseRetainedBatch("parse-begin")
	batchSeq++

	rawBytes := 0
	for _, raw := range rawSlice {
		rawBytes += len(raw)
	}

	if debugLifecycle {
		println("parse begin", "batch", batchSeq, "lines", len(rawSlice), "raw-bytes", rawBytes)
	}

	return rawBytes
}

func finishBatch(entries, tags, skipped, rawBytes int, start time.Time) {
	lastEntryUsed = entries
	lastTagUsed = tags
	lastRawBytes = rawBytes
	lastExecNs = time.Since(start).Nanoseconds()

	if debugLifecycle {
		println(
			"parse end",
			"batch", batchSeq,
			"entries", entries,
			"tags", tags,
			"skipped", skipped,
			"raw-bytes", rawBytes,
			"exec-ns", lastExecNs,
		)
	}
}

func releaseRetainedBatch(where string) {
	if debugLifecycle && (lastEntryUsed != 0 || lastTagUsed != 0 || len(globalKvBuf) != 0) {
		println(
			"release",
			where,
			"batch", batchSeq,
			"entries", lastEntryUsed,
			"tags", lastTagUsed,
			"raw-bytes", lastRawBytes,
			"kv-len", len(globalKvBuf),
		)
	}

	// 清除引用
	for i := 0; i < lastEntryUsed && i < len(entryPool); i++ {
		entryPool[i] = parserplugin.ParsedEntry{}
	}
	for i := 0; i < lastTagUsed && i < len(tagPool); i++ {
		tagPool[i] = [2]string{}
	}
	for i := range globalKvBuf {
		globalKvBuf[i] = [2]string{}
	}

	globalKvBuf = globalKvBuf[:0]
	tagCursor = 0
	lastEntryUsed = 0
	lastTagUsed = 0
	lastRawBytes = 0
}

// ── 核心工具函數 ──────────────────────────────────────────────────────────────

// parseLogLevel — 直接 string switch，省去 []byte(s) 轉換（每 entry 一次 alloc）。
func parseLogLevel(s string) pipelineprocess.LogLevel {
	switch s {
	case "debug":
		return pipelineprocess.LogLevelDebug
	case "info":
		return pipelineprocess.LogLevelInfo
	case "warn":
		return pipelineprocess.LogLevelWarn
	case "error":
		return pipelineprocess.LogLevelError
	case "crit":
		return pipelineprocess.LogLevelCrit
	case "alert":
		return pipelineprocess.LogLevelAlert
	case "emerg":
		return pipelineprocess.LogLevelEmerg
	default:
		return pipelineprocess.LogLevelInfo
	}
}

func kvGet(kvs [][2]string, key string) string {
	for _, kv := range kvs {
		if kv[0] == key {
			return kv[1]
		}
	}
	return ""
}

func parseJSONLineManual(s string) (ts, level, msg string, err error) {
	i := 0
	skipJSONSpace(s, &i)
	if i >= len(s) || s[i] != '{' {
		return "", "", "", strconv.ErrSyntax
	}
	i++

	for {
		skipJSONSpace(s, &i)
		if i >= len(s) {
			return "", "", "", strconv.ErrSyntax
		}
		if s[i] == '}' {
			i++
			return ts, level, msg, nil
		}

		key, keyErr := parseJSONStringManual(s, &i)
		if keyErr != nil {
			return "", "", "", keyErr
		}
		skipJSONSpace(s, &i)
		if i >= len(s) || s[i] != ':' {
			return "", "", "", strconv.ErrSyntax
		}
		i++
		skipJSONSpace(s, &i)

		switch key {
		case "ts":
			ts, err = parseJSONStringManual(s, &i)
		case "level":
			level, err = parseJSONStringManual(s, &i)
		case "msg":
			msg, err = parseJSONStringManual(s, &i)
		case "att":
			err = parseJSONAttManual(s, &i)
		default:
			err = skipJSONValueManual(s, &i)
		}
		if err != nil {
			return "", "", "", err
		}

		skipJSONSpace(s, &i)
		if i >= len(s) {
			return "", "", "", strconv.ErrSyntax
		}
		switch s[i] {
		case ',':
			i++
		case '}':
			i++
			return ts, level, msg, nil
		default:
			return "", "", "", strconv.ErrSyntax
		}
	}
}

func parseJSONAttManual(s string, i *int) error {
	skipJSONSpace(s, i)
	if *i >= len(s) || s[*i] != '{' {
		return strconv.ErrSyntax
	}
	(*i)++

	for {
		skipJSONSpace(s, i)
		if *i >= len(s) {
			return strconv.ErrSyntax
		}
		if s[*i] == '}' {
			(*i)++
			return nil
		}

		key, err := parseJSONStringManual(s, i)
		if err != nil {
			return err
		}
		skipJSONSpace(s, i)
		if *i >= len(s) || s[*i] != ':' {
			return strconv.ErrSyntax
		}
		(*i)++
		skipJSONSpace(s, i)

		val, err := parseJSONStringManual(s, i)
		if err != nil {
			return err
		}
		if tagCursor < len(tagPool) {
			tagPool[tagCursor] = [2]string{key, val}
			tagCursor++
		}

		skipJSONSpace(s, i)
		if *i >= len(s) {
			return strconv.ErrSyntax
		}
		switch s[*i] {
		case ',':
			(*i)++
		case '}':
			(*i)++
			return nil
		default:
			return strconv.ErrSyntax
		}
	}
}

func parseJSONStringManual(s string, i *int) (string, error) {
	if *i >= len(s) || s[*i] != '"' {
		return "", strconv.ErrSyntax
	}
	(*i)++
	start := *i

	for *i < len(s) {
		switch s[*i] {
		case '"':
			val := s[start:*i]
			(*i)++
			return val, nil
		case '\\':
			return parseJSONStringEscapedManual(s, i, start)
		default:
			(*i)++
		}
	}

	return "", strconv.ErrSyntax
}

func parseJSONStringEscapedManual(s string, i *int, start int) (string, error) {
	var b strings.Builder
	b.WriteString(s[start:*i])

	for *i < len(s) {
		c := s[*i]
		switch c {
		case '"':
			(*i)++
			return b.String(), nil
		case '\\':
			(*i)++
			if *i >= len(s) {
				return "", strconv.ErrSyntax
			}
			switch s[*i] {
			case '"', '\\', '/':
				b.WriteByte(s[*i])
				(*i)++
			case 'b':
				b.WriteByte('\b')
				(*i)++
			case 'f':
				b.WriteByte('\f')
				(*i)++
			case 'n':
				b.WriteByte('\n')
				(*i)++
			case 'r':
				b.WriteByte('\r')
				(*i)++
			case 't':
				b.WriteByte('\t')
				(*i)++
			case 'u':
				if *i+4 >= len(s) {
					return "", strconv.ErrSyntax
				}
				r, ok := parseJSONHex4(s[*i+1 : *i+5])
				if !ok {
					return "", strconv.ErrSyntax
				}
				b.WriteRune(r)
				*i += 5
			default:
				return "", strconv.ErrSyntax
			}
		default:
			b.WriteByte(c)
			(*i)++
		}
	}

	return "", strconv.ErrSyntax
}

func skipJSONValueManual(s string, i *int) error {
	skipJSONSpace(s, i)
	if *i >= len(s) {
		return strconv.ErrSyntax
	}

	switch s[*i] {
	case '"':
		_, err := parseJSONStringManual(s, i)
		return err
	case '{':
		return skipJSONCompositeManual(s, i, '{', '}')
	case '[':
		return skipJSONCompositeManual(s, i, '[', ']')
	default:
		for *i < len(s) {
			switch s[*i] {
			case ',', '}', ']':
				return nil
			case ' ', '\t', '\n', '\r':
				return nil
			default:
				(*i)++
			}
		}
		return nil
	}
}

func skipJSONCompositeManual(s string, i *int, open, close byte) error {
	if *i >= len(s) || s[*i] != open {
		return strconv.ErrSyntax
	}
	depth := 0
	for *i < len(s) {
		switch s[*i] {
		case '"':
			if _, err := parseJSONStringManual(s, i); err != nil {
				return err
			}
			continue
		case open:
			depth++
		case close:
			depth--
			if depth == 0 {
				(*i)++
				return nil
			}
		}
		(*i)++
	}
	return strconv.ErrSyntax
}

func skipJSONSpace(s string, i *int) {
	for *i < len(s) {
		switch s[*i] {
		case ' ', '\t', '\n', '\r':
			(*i)++
		default:
			return
		}
	}
}

func parseJSONHex4(s string) (rune, bool) {
	var v rune
	for i := 0; i < 4; i++ {
		c := s[i]
		switch {
		case c >= '0' && c <= '9':
			v = v*16 + rune(c-'0')
		case c >= 'a' && c <= 'f':
			v = v*16 + rune(c-'a'+10)
		case c >= 'A' && c <= 'F':
			v = v*16 + rune(c-'A'+10)
		default:
			return 0, false
		}
	}
	return v, true
}

// splitInto8 將 s 以 sep 切分至多 8 段，結果存入呼叫方的 stack array。
// 零 heap allocation；取代 strings.SplitN(s, " ", 8) 的每行一次 slice alloc。
func splitInto8(s string, sep byte, out *[8]string) int {
	n := 0
	for n < 7 {
		i := strings.IndexByte(s, sep)
		if i < 0 {
			break
		}
		out[n] = s[:i]
		s = s[i+1:]
		n++
	}
	out[n] = s
	return n + 1
}

// parseSysEntry 解析單行 syslog（RFC5424-like），寫入全域 tagPool，零 alloc。
func parseSysEntry(rawStr string, kvBuf [][2]string) (parserplugin.ParsedEntry, [][2]string, error) {
	line := strings.TrimSpace(rawStr)
	if len(line) == 0 || line[0] != '<' {
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

	var parts [8]string // stack-allocated，不進 heap
	if splitInto8(rest, ' ', &parts) < 8 {
		return parserplugin.ParsedEntry{}, kvBuf, strconv.ErrSyntax
	}

	if parts[0] != "1" {
		return parserplugin.ParsedEntry{}, kvBuf, strconv.ErrSyntax
	}
	ts := parts[1]
	host := parts[2]
	app := parts[3]
	msgAndMore := strings.TrimSpace(parts[7])

	msg := msgAndMore
	fieldPart := ""
	if idx := strings.Index(msgAndMore, " level="); idx >= 0 {
		msg = strings.TrimSpace(msgAndMore[:idx])
		fieldPart = msgAndMore[idx+1:]
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

	tagStart := tagCursor
	if tagCursor < len(tagPool) {
		tagPool[tagCursor] = [2]string{"lang", "Go"}
		tagCursor++
	}
	if tagCursor < len(tagPool) {
		tagPool[tagCursor] = [2]string{"host", host}
		tagCursor++
	}
	if tagCursor < len(tagPool) {
		tagPool[tagCursor] = [2]string{"app_name", app}
		tagCursor++
	}
	for _, kv := range kvBuf {
		if kv[0] == "level" {
			continue
		}
		if tagCursor < len(tagPool) {
			tagPool[tagCursor] = kv
			tagCursor++
		}
	}

	return parserplugin.ParsedEntry{
		Timestamp: ts,
		Level:     level,
		Message:   msg,
		Tags:      cm.ToList(tagPool[tagStart:tagCursor]),
		Targettag: routeTag(level),
	}, kvBuf, nil
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

// routeTag 根據解析結果決定要送往哪些後端（對應 YAML endpoint 的 key）。
// 回傳值可包含多個 key 的組合，host 端逐字元拆分後 fan-out 發送。
// 範例："A" = 只送A，"AB" = 同時送A和B，"ABC" = 送全部。
//
// 規則（由上往下，第一個符合的優先）：
//
//	error / crit / alert / emerg → "AB"（嚴重錯誤，A+B 雙備援）
//	warn                         → "BC"（警告，B+C 同時記錄）
//	debug / info（其他）          → "C" （一般流量，只送C）
func routeTag(level pipelineprocess.LogLevel) string {
	switch level {
	case pipelineprocess.LogLevelError,
		pipelineprocess.LogLevelCrit,
		pipelineprocess.LogLevelAlert,
		pipelineprocess.LogLevelEmerg:
		return "AB"
	case pipelineprocess.LogLevelWarn:
		return "BC"
	default:
		return "C"
	}
}

func main() {}
