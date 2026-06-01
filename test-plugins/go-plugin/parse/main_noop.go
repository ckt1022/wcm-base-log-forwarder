package main

import (
	"time"

	parserplugin "example.com/internal/local/log-process/parser-plugin"
	"go.bytecodealliance.org/cm"
)

var noopLastExecNs int64

func init() {
	parserplugin.Exports.Parse = ParseNoop
	parserplugin.Exports.ReportUsage = ReportNoopUsage
	parserplugin.Exports.Reset = ResetNoop
}

func ParseNoop(rawData cm.List[string]) cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError] {
	start := time.Now()
	_ = rawData.Slice()
	noopLastExecNs = time.Since(start).Nanoseconds()

	return cm.OK[cm.Result[parserplugin.ParseErrorShape, cm.List[parserplugin.ParsedEntry], parserplugin.ParseError]](
		cm.ToList([]parserplugin.ParsedEntry{}),
	)
}

func ReportNoopUsage() uint64 {
	return uint64(noopLastExecNs)
}

func ResetNoop() {
	noopLastExecNs = 0
}

func main() {}
