package main

import (
	"strings"

	formatplugin "example.com/internal/local/log-process/format-plugin"
	"go.bytecodealliance.org/cm"
)

func init() {
	// ⚠ 必須在 init() 中綁定，WASM Component 初始化時執行
	// main() 不會被呼叫
	formatplugin.Exports.Format = Format
	formatplugin.Exports.ReportUsage = ReportUsage
}

func Format(structData cm.List[formatplugin.LogEntry]) (result cm.Result[formatplugin.PluginErrorShape, cm.List[cm.List[uint8]], formatplugin.PluginError]) {
	entries := structData.Slice()
	encoded := make([]cm.List[uint8], 0, len(entries))

	for _, entry := range entries {
		// 格式：timestamp LEVEL message key=value key=value ...
		var sb strings.Builder
		sb.WriteString(entry.Timestamp)
		sb.WriteByte(' ')
		sb.WriteString(strings.ToUpper(entry.Level.String()))
		sb.WriteByte(' ')
		sb.WriteString(entry.Message)

		for _, tag := range entry.Tags.Slice() {
			sb.WriteByte(' ')
			sb.WriteString(tag[0])
			sb.WriteByte('=')
			sb.WriteString(tag[1])
		}

		line := []byte(sb.String())
		encoded = append(encoded, cm.ToList(line))
	}

	return cm.OK[cm.Result[formatplugin.PluginErrorShape, cm.List[cm.List[uint8]], formatplugin.PluginError]](cm.ToList(encoded))
}

// ReportUsage 回傳 0（format plugin 目前不追蹤記憶體）
func ReportUsage() uint64 {
	return 0
}

// WASM Component 不呼叫 main()，但必須宣告
func main() {}
