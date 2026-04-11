package main

import (
	"encoding/json"
	"runtime"
	"strconv"
	"strings"

	parserplugin "example.com/internal/local/log-process/parser-plugin"
	pipelineprocess "example.com/internal/local/log-process/pipeline-process"
	"go.bytecodealliance.org/cm"
)

// 定義與 JSON 格式對應的內部結構
type Self_log struct {
	Ts    string            `json:"ts"`
	Level string            `json:"level"`
	Msg   string            `json:"msg"`
	Att   map[string]string `json:"att"`
}

func init() {
	// ParseLogfmt // Parse // ParseSys
	parserplugin.Exports.Parse = ParseSys
	parserplugin.Exports.ReportUsage = ReportUsage
}

// peak_mem 記錄本次 Parse 呼叫期間 Go heap 的峰值用量（bytes）。
//
// ⚠ 測量範圍限制：
//   - 本值來自 runtime.MemStats.HeapInuse，僅代表 Go runtime heap 層面的記憶體壓力
//   - 不包含 goroutine stacks、runtime GC metadata、WASM globals 等非 heap 記憶體
//   - 因此本值 < 實際 WASM 線性記憶體總量
//   - 完整的 WASM 線性記憶體峰值請從 host 端讀取（Rust：MyLimiter.wasm_mem_peak）
var peak_mem uint64

// Parse 將 rawData（list<list<u8>>）解析為 LogEntry 列表。
//
// 修正事項：
//  1. 單行格式錯誤時跳過該行並繼續（原本 return Err 會中止整個 batch）
//  2. 正確對應 JSON level 字串至 WIT LogLevel enum（原本永遠填 LogLevelDebug）
//  3. 採樣點移至 cm.ToList 之後，確保捕捉到最終分配峰值
func Parse(rawData cm.List[cm.List[uint8]]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	// ── 記憶體採樣策略說明 ────────────────────────────────────────────────
	// runtime.ReadMemStats() 每次呼叫都觸發 GC stop-the-world（STW）暫停，
	// 會直接拉高 parse latency（host 側測到的 parse_ms 含 STW 成本）。
	// 採樣點控制在三個關鍵位置：
	//   ① 進入 Parse 前（基準線，確認 runtime 底層 overhead）
	//   ② json.Unmarshal 後（loop 內最大分配點，須在 loop 內採樣才不會漏掉）
	//   ③ cm.ToList(entries) 後（所有分配完成後的真實峰值）
	// ─────────────────────────────────────────────────────────────────────

	peak_mem = 0
	samplePeakMem() // ① 基準線

	rawSlice := rawData.Slice()
	// 預分配容量 = 輸入行數，避免 append 多次觸發 realloc
	entries := make([]parserplugin.ParsedEntry, 0, len(rawSlice))

	var skipCount int // 格式錯誤被跳過的行數

	for _, rawBuf := range rawSlice {
		// .Slice() 直接取得底層 []byte 指針，不複製資料至 Go heap
		data := rawBuf.Slice()

		var log Self_log
		if err := json.Unmarshal(data, &log); err != nil {
			// [修正] 跳過無效行，繼續處理剩餘行
			// 原本：return cm.Err(...) → 整個 batch 失敗，已解析的 entry 全丟
			// 現在：記錄跳過數量，保留已解析結果
			skipCount++
			continue
		}

		// ② 主分配點採樣：json.Unmarshal 為 loop 內最大分配（臨時 struct + map）
		// 必須在 loop 內採樣，否則 GC 可能在 loop 結束後 sweep，導致漏掉峰值
		//samplePeakMem()

		// [修正] 正確對應 JSON level 字串至 WIT LogLevel enum
		// 原本：永遠填 parseprocess.LogLevelDebug，忽略實際 level 值
		level := parseLogLevel(log.Level)

		// 預分配 pairs 容量 = att map 大小，減少 append realloc
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

	// [修正] 先完成 cm.ToList(entries) 再採樣
	// 原本採樣在 return 前（cm.ToList 之前），若 ToList 有額外分配會被漏掉
	result := cm.ToList(entries)
	samplePeakMem() // ③ 最終峰值（所有分配完成後）

	_ = skipCount // 可改為輸出至 stderr 追蹤解析失敗率

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](result)
}

// ParseLogfmt 將 rawData（list<list<u8>>）解析為 logfmt 格式的 LogEntry 列表。
//
// 格式範例：
//
//	ts=2026-04-11T12:00:00Z level=info msg="Request completed successfully" service=api-gateway code=200
//
// 行為與 JSON Parse 保持一致：
//  1. 單行格式錯誤時跳過該行並繼續
//  2. level 由 level=... 對應至 WIT LogLevel enum
//  3. ts / level / msg 作為主欄位，其餘 key=value 進入 Tags
func ParseLogfmt(rawData cm.List[cm.List[uint8]]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	peak_mem = 0
	samplePeakMem()

	rawSlice := rawData.Slice()
	entries := make([]parserplugin.ParsedEntry, 0, len(rawSlice))

	var skipCount int

	for _, rawBuf := range rawSlice {
		data := rawBuf.Slice()

		entry, err := parse_logfmt(data)
		if err != nil {
			skipCount++
			continue
		}

		entries = append(entries, entry)
	}

	result := cm.ToList(entries)
	samplePeakMem()

	_ = skipCount

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](result)
}

// ParseSys 將 rawData（list<list<u8>>）解析為 syslog 格式的 LogEntry 列表。
//
// 支援的格式為本專案 generator 產出的 RFC5424-like 單行格式：
//
//	<PRI>1 TIMESTAMP HOST APP-NAME - - - MESSAGE key=value ...
//
// 範例：
//
//	<14>1 2026-04-11T12:00:00Z worker-03 auth-service - - - Health check passed level=info code=200 service=auth-service
//
// 行為：
//  1. 單行格式錯誤時跳過該行並繼續
//  2. 若尾端 key=value 含 level，優先使用該欄位；否則 fallback 至 PRI severity
//  3. host / app_name 與其他 key=value 一併進入 Tags
func ParseSys(rawData cm.List[cm.List[uint8]]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	peak_mem = 0
	samplePeakMem()

	rawSlice := rawData.Slice()
	entries := make([]parserplugin.ParsedEntry, 0, len(rawSlice))

	var skipCount int

	for _, rawBuf := range rawSlice {
		data := rawBuf.Slice()

		entry, err := parse_sys(data)
		if err != nil {
			skipCount++
			continue
		}

		entries = append(entries, entry)
	}

	result := cm.ToList(entries)
	samplePeakMem()

	_ = skipCount

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](result)
}

// ReportUsage 回傳本次 Parse 期間觀測到的 Go heap 峰值用量（bytes）。
//
// ⚠ 此值僅代表 Go runtime heap 層面的記憶體用量（HeapInuse），
// 不等於 WASM 線性記憶體總量。完整測量請參閱 host 端的 MyLimiter.wasm_mem_peak。
func ReportUsage() uint64 {
	return peak_mem
}

// samplePeakMem 讀取目前 Go heap in-use bytes 並更新 peak_mem。
//
// ⚠ 效能警告：runtime.ReadMemStats() 會觸發 GC stop-the-world（STW）暫停，
// 每次約 < 1ms，但在高吞吐量下會累積並拉高可觀測的 parse latency。
//
// 使用 HeapInuse 而非 HeapAlloc 的原因：
//   - HeapAlloc：只含「當前活動物件」的 bytes，GC sweep 後立即下降，容易低估峰值
//   - HeapInuse：含「in-use span 內所有 bytes」（活動物件 + 已 free 但 span 未歸還 OS 的空間）
//     能更穩定地反映 runtime 實際持有的 heap 記憶體壓力，較不受 GC 時機影響
func samplePeakMem() {
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	if m.HeapInuse > peak_mem {
		peak_mem = m.HeapInuse
	}
}

// parseLogLevel 將 JSON level 字串對應至 WIT LogLevel enum。
// 使用 binding 自動生成的 UnmarshalText 保持與 WIT 定義一致。
// 若字串無法識別（如自定義 level 名稱），回傳 LogLevelInfo 作為安全預設值。
func parseLogLevel(s string) pipelineprocess.LogLevel {
	var level pipelineprocess.LogLevel
	if err := level.UnmarshalText([]byte(s)); err != nil {
		return pipelineprocess.LogLevelInfo
	}
	return level
}

// parse_logfmt 解析單行 logfmt，並轉為 ParsedEntry。
//
// 規則：
//   - ts / level / msg 取出作為主欄位
//   - 其餘 key=value 進入 Tags
//   - 支援 quoted value，例如 msg="hello world"
//   - 若缺少 level，則 fallback 為 LogLevelInfo
func parse_logfmt(data []byte) (parserplugin.ParsedEntry, error) {
	fields, err := parseLogfmtFields(string(data))
	if err != nil {
		return parserplugin.ParsedEntry{}, err
	}

	ts := fields["ts"]
	msg := fields["msg"]
	level := parseLogLevel(fields["level"])

	pairs := make([][2]string, 0, len(fields))
	for k, v := range fields {
		switch k {
		case "ts", "level", "msg":
			continue
		default:
			pairs = append(pairs, [2]string{k, v})
		}
	}

	return parserplugin.ParsedEntry{
		Timestamp: ts,
		Level:     level,
		Message:   msg,
		Tags:      cm.ToList(pairs),
	}, nil
}

// parse_sys 解析單行 syslog，並轉為 ParsedEntry。
//
// 支援格式：
//
//	<PRI>1 TIMESTAMP HOST APP-NAME - - - MESSAGE key=value ...
//
// 注意：
//   - 這裡針對目前 generator 產出的 RFC5424-like 格式做解析
//   - MESSAGE 後尾端若帶有 logfmt 欄位（例如 level=info code=200），會進一步拆解
//   - 若 level 欄位不存在，則使用 PRI 推導 severity 對應的 LogLevel
func parse_sys(data []byte) (parserplugin.ParsedEntry, error) {
	line := strings.TrimSpace(string(data))
	if line == "" {
		return parserplugin.ParsedEntry{}, strconv.ErrSyntax
	}

	// PRI：例如 <14>
	if !strings.HasPrefix(line, "<") {
		return parserplugin.ParsedEntry{}, strconv.ErrSyntax
	}
	endPRI := strings.IndexByte(line, '>')
	if endPRI <= 1 {
		return parserplugin.ParsedEntry{}, strconv.ErrSyntax
	}

	pri, err := strconv.Atoi(line[1:endPRI])
	if err != nil {
		return parserplugin.ParsedEntry{}, err
	}
	severity := pri % 8

	rest := line[endPRI+1:]

	// VERSION TIMESTAMP HOST APP-NAME PROCID MSGID STRUCTURED-DATA MSG
	// 本 generator 固定輸出：
	//   1 ts host svc - - - message level=... code=...
	parts := strings.SplitN(rest, " ", 8)
	if len(parts) < 8 {
		return parserplugin.ParsedEntry{}, strconv.ErrSyntax
	}

	version := parts[0]
	ts := parts[1]
	host := parts[2]
	app := parts[3]
	procid := parts[4]
	msgid := parts[5]
	structuredData := parts[6]
	msgAndMore := parts[7]

	if version != "1" {
		return parserplugin.ParsedEntry{}, strconv.ErrSyntax
	}

	// 目前 generator 固定為 "- - -"
	_ = procid
	_ = msgid
	_ = structuredData

	msgAndMore = strings.TrimSpace(msgAndMore)

	// 以 " level=" 作為 message 與尾端 key=value 的切點
	msg := msgAndMore
	fieldPart := ""

	if idx := strings.Index(msgAndMore, " level="); idx >= 0 {
		msg = strings.TrimSpace(msgAndMore[:idx])
		fieldPart = strings.TrimSpace(msgAndMore[idx+1:])
	}

	fields := map[string]string{}
	if fieldPart != "" {
		fields, err = parseLogfmtFields(fieldPart)
		if err != nil {
			return parserplugin.ParsedEntry{}, err
		}
	}

	levelStr := fields["level"]
	var level pipelineprocess.LogLevel
	if levelStr != "" {
		level = parseLogLevel(levelStr)
	} else {
		level = syslogSeverityToLogLevel(severity)
	}

	pairs := make([][2]string, 0, len(fields)+2)
	pairs = append(pairs,
		[2]string{"host", host},
		[2]string{"app_name", app},
	)

	for k, v := range fields {
		if k == "level" {
			continue
		}
		pairs = append(pairs, [2]string{k, v})
	}

	return parserplugin.ParsedEntry{
		Timestamp: ts,
		Level:     level,
		Message:   msg,
		Tags:      cm.ToList(pairs),
	}, nil
}

// parseLogfmtFields 將 logfmt 字串解析為 map[string]string。
//
// 支援：
//   - key=value
//   - key="value with spaces"
//   - 常見 escape：\\ \" \n \t \r
//
// 限制：
//   - 不做型別推斷，全部保留為字串
//   - 遇到不合法語法直接回傳錯誤
func parseLogfmtFields(s string) (map[string]string, error) {
	fields := make(map[string]string)
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
			return nil, strconv.ErrSyntax
		}
		key := s[keyStart:i]
		i++ // skip '='

		if i >= n {
			fields[key] = ""
			break
		}

		var val string
		if s[i] == '"' {
			i++ // skip opening quote

			var b strings.Builder
			for i < n {
				switch s[i] {
				case '\\':
					if i+1 >= n {
						return nil, strconv.ErrSyntax
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
						// 保守接受未知 escape，避免 parser 過度嚴格
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
			return nil, strconv.ErrSyntax
		} else {
			valStart := i
			for i < n && s[i] != ' ' {
				i++
			}
			val = s[valStart:i]
		}

	doneValue:
		fields[key] = val

		for i < n && s[i] == ' ' {
			i++
		}
	}

	return fields, nil
}

// syslogSeverityToLogLevel 將 syslog PRI 的 severity 映射至 WIT LogLevel enum。
//
// 對應規則：
//   - 0~3 => Error
//   - 4   => Warn
//   - 5~6 => Info
//   - 7   => Debug
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
