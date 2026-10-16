// parse_loop plugin – 實作 parse-plugin WIT world
//
// 正常情況：對輸入進行 JSON / logfmt 解析，回傳 parsed-entry list。
// 觸發條件：當某筆 log 的 msg 欄位包含 "LOOP"（大小寫不限）時，
//           立即進入無窮迴圈，用來驗證 WCM host 是否能偵測並終止掛住的插件。
//
// 觸發範例（JSON）:
//   {"ts":"2024-01-01T00:00:00Z","level":"warn","msg":"LOOP trigger"}
#nullable enable

using PP = global::ParserPluginWorld.wit.Imports.local.logProcess.v0_3_0.IPipelineProcessImports;

namespace ParserPluginWorld;

public partial class ParserPluginWorldExportsImpl : IParserPluginWorldExports
{
    private static ulong _lastParseNs;

    public static global::System.Collections.Generic.List<PP.ParsedEntry> Parse(
        global::System.Collections.Generic.List<string> rawData)
    {
        var started = global::System.Diagnostics.Stopwatch.GetTimestamp();
        var results = new global::System.Collections.Generic.List<PP.ParsedEntry>(rawData.Count);

        foreach (var line in rawData)
        {
            var trimmed = line.Trim();
            if (trimmed.Length == 0) continue;

            global::ParserPlugin.ParseResult r;
            bool ok = trimmed[0] switch
            {
                '{' => global::ParserPlugin.JsonParser.TryParse(trimmed, out r),
                _   => global::ParserPlugin.LogfmtParser.TryParse(trimmed, out r),
            };
            if (!ok) continue;

            // 觸發無窮迴圈：msg 欄位包含 "LOOP" 時進入死循環
            if (r.Message.Contains("LOOP", StringComparison.OrdinalIgnoreCase))
            {
                while (true) { }
            }

            results.Add(new PP.ParsedEntry(
                timestamp: r.Timestamp,
                level:     (PP.LogLevel)(byte)r.Level,
                message:   r.Message,
                tags:      r.Tags,
                targettag: r.Targettag
            ));
        }

        _lastParseNs = StopwatchTicksToNs(
            global::System.Diagnostics.Stopwatch.GetTimestamp() - started);
        return results;
    }

    public static int ParseCallback(/* TODO: event arg */) =>
        throw new global::System.NotImplementedException();

    public static ulong ReportUsage() => _lastParseNs;

    public static int ReportUsageCallback(/* TODO: event arg */) =>
        throw new global::System.NotImplementedException();

    private static ulong StopwatchTicksToNs(long ticks) =>
        (ulong)((double)ticks * 1_000_000_000.0
                / global::System.Diagnostics.Stopwatch.Frequency);
}
