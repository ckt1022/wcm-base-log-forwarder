package main

import (
	"encoding/json"
	"runtime"

	parseprocess "example.com/internal/local/log-process/parse-process"
	parserplugin "example.com/internal/local/log-process/parser-plugin"
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
	parserplugin.Exports.Parse = Parse
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
func Parse(rawData cm.List[cm.List[uint8]]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.LogEntry], parserplugin.ParseError] {
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
	entries := make([]parserplugin.LogEntry, 0, len(rawSlice))

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

		entries = append(entries, parserplugin.LogEntry{
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

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.LogEntry], parserplugin.ParseError]](result)
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
func parseLogLevel(s string) parseprocess.LogLevel {
	var level parseprocess.LogLevel
	if err := level.UnmarshalText([]byte(s)); err != nil {
		return parseprocess.LogLevelInfo
	}
	return level
}

func main() {}
