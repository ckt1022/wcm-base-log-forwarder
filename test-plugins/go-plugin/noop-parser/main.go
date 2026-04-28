package main

import (
	"time"

	parserplugin "example.com/internal/local/log-process/parser-plugin"
	"go.bytecodealliance.org/cm"
)

func init() {
	parserplugin.Exports.Parse = NoopParse
	parserplugin.Exports.ReportUsage = ReportUsage
}

var lastExecNs int64

// NoopParse 接受 list<string>（ABI lifting 仍會發生）但不做任何解析，
// 直接回傳空的 list<parsed-entry>。
// 在 host 側計時此呼叫可量測：store init + ABI lifting + ABI lowering（空結果）。
// component_ns ≈ 0，所以 abi+store = host_elapsed - component_ns ≈ 全部成本。
func NoopParse(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	start := time.Now()
	_ = rawData.Slice() // 確保 ABI lifting 完整完成後才開始計時
	empty := cm.ToList([]parserplugin.ParsedEntry{})
	lastExecNs = time.Since(start).Nanoseconds()
	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](empty)
}

func ReportUsage() uint64 {
	return uint64(lastExecNs)
}

func main() {}
