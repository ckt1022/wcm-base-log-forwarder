package export_wit_world

import (
	"encoding/json"
	"sort"
	"strings"

	witTypes "go.bytecodealliance.org/pkg/wit/types"
	"wit_component/local_log_process_pipeline_process"
	"wit_component/wit_world"
)

type pipeline = local_log_process_pipeline_process.ParsedEntry

func Parse(rawData []string) witTypes.Result[[]local_log_process_pipeline_process.ParsedEntry, local_log_process_pipeline_process.ParseError] {
	// H1: generated bindings have decoded list<string> into Go strings.
	wit_world.TimeSign()

	entries := make([]local_log_process_pipeline_process.ParsedEntry, 0, len(rawData))
	for _, raw := range rawData {
		entry, err := parseOne(raw)
		if err != nil {
			wit_world.TimeSign()
			return witTypes.Err[[]local_log_process_pipeline_process.ParsedEntry](
				local_log_process_pipeline_process.MakeParseErrorInvalidFormat("JSON Parse Error"),
			)
		}
		entries = append(entries, entry)
	}

	// H2: JSON parsing and Go ParsedEntry construction are complete.
	wit_world.TimeSign()
	return witTypes.Ok[[]local_log_process_pipeline_process.ParsedEntry, local_log_process_pipeline_process.ParseError](entries)
}

func ReportUsage() uint64 {
	return 0
}

func parseOne(raw string) (local_log_process_pipeline_process.ParsedEntry, error) {
	var root map[string]json.RawMessage
	if err := json.Unmarshal([]byte(raw), &root); err != nil {
		return pipeline{}, err
	}

	return pipeline{
		Timestamp: readString(root["ts"]),
		Level:     readLevel(root["level"]),
		Message:   readString(root["msg"]),
		Tags:      collectTags(root),
		Targettag: "A",
	}, nil
}

func readString(raw json.RawMessage) string {
	var value string
	if len(raw) == 0 || json.Unmarshal(raw, &value) != nil {
		return ""
	}
	return value
}

func readLevel(raw json.RawMessage) local_log_process_pipeline_process.LogLevel {
	var numeric uint8
	if json.Unmarshal(raw, &numeric) == nil && numeric <= local_log_process_pipeline_process.LogLevelEmerg {
		return numeric
	}

	switch strings.ToLower(readString(raw)) {
	case "debug":
		return local_log_process_pipeline_process.LogLevelDebug
	case "warn":
		return local_log_process_pipeline_process.LogLevelWarn
	case "error":
		return local_log_process_pipeline_process.LogLevelError
	case "crit":
		return local_log_process_pipeline_process.LogLevelCrit
	case "alert":
		return local_log_process_pipeline_process.LogLevelAlert
	case "emerg":
		return local_log_process_pipeline_process.LogLevelEmerg
	default:
		return local_log_process_pipeline_process.LogLevelInfo
	}
}

func collectTags(root map[string]json.RawMessage) []witTypes.Tuple2[string, string] {
	// Keep the exact Rust benchmark output payload for controlled comparison.
	tags := []witTypes.Tuple2[string, string]{{F0: "lang", F1: "Rust"}}

	keys := sortedKeys(root)
	for _, key := range keys {
		if key == "ts" || key == "level" || key == "msg" || key == "att" {
			continue
		}
		tags = append(tags, witTypes.Tuple2[string, string]{F0: key, F1: jsonValue(root[key])})
	}

	var attributes map[string]json.RawMessage
	if json.Unmarshal(root["att"], &attributes) == nil {
		for _, key := range sortedKeys(attributes) {
			tags = append(tags, witTypes.Tuple2[string, string]{F0: key, F1: jsonValue(attributes[key])})
		}
	}
	return tags
}

func sortedKeys(values map[string]json.RawMessage) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func jsonValue(raw json.RawMessage) string {
	var value string
	if json.Unmarshal(raw, &value) == nil {
		return value
	}
	return string(raw)
}
