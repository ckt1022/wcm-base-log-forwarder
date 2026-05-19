// wit-bindgen 生成的 stub — 由此填入實作。
// 其餘生成檔案在 internal/，請勿手動修改。
#nullable enable

using PP = global::ParserPluginWorld.wit.Imports.local.logProcess.v0_1_0.IPipelineProcessImports;

namespace ParserPluginWorld;

public partial class ParserPluginWorldExportsImpl : IParserPluginWorldExports
{
    private static ulong _count;
    private static ulong _lastParseNs;

    // WIT: parse(raw-data: list<string>) -> result<list<parsed-entry>, parse-error>
    // 解析失敗的行 silent skip，與 Go plugin 行為一致。
    // 如需回傳 fatal error，改為: throw new WitException<PP.ParseError>(PP.ParseError.InvalidFormat("reason"), 0);
    public static global::System.Collections.Generic.List<PP.ParsedEntry> Parse(
        global::System.Collections.Generic.List<string> rawData)
    {
        var started = global::System.Diagnostics.Stopwatch.GetTimestamp();
#if NOOP_PARSER
        _ = rawData.Count;
        var results = new global::System.Collections.Generic.List<PP.ParsedEntry>(0);
#else
        var results = new global::System.Collections.Generic.List<PP.ParsedEntry>(rawData.Count);

        foreach (var line in rawData)
        {
            var trimmed = line.Trim();
            if (trimmed.Length == 0) continue;

            global::ParserPlugin.ParseResult r;
            bool ok = trimmed[0] switch
            {
                '{'  => global::ParserPlugin.JsonParser.TryParse(trimmed, out r),
                '<'  => global::ParserPlugin.SyslogParser.TryParse(trimmed, out r),
                _    => global::ParserPlugin.LogfmtParser.TryParse(trimmed, out r),
            };
            if (!ok) continue;

            results.Add(new PP.ParsedEntry(
                timestamp: r.Timestamp,
                level:     (PP.LogLevel)(byte)r.Level,
                message:   r.Message,
                tags:      r.Tags
            ));
            _count++;
        }
#endif

        _lastParseNs = StopwatchTicksToNs(global::System.Diagnostics.Stopwatch.GetTimestamp() - started);
        return results;
    }

    public static int ParseCallback(/* TODO: event arg */) =>
        throw new global::System.NotImplementedException();

    // WIT: report-usage() -> u64
    public static ulong ReportUsage() => _lastParseNs;

    public static int ReportUsageCallback(/* TODO: event arg */) =>
        throw new global::System.NotImplementedException();

    // WIT: reset()
    public static void Reset()
    {
        _count = 0;
        _lastParseNs = 0;
    }

    private static ulong StopwatchTicksToNs(long ticks) =>
        (ulong)((double)ticks * 1_000_000_000.0 / global::System.Diagnostics.Stopwatch.Frequency);

    public static int ResetCallback(/* TODO: event arg */) =>
        throw new global::System.NotImplementedException();
}
