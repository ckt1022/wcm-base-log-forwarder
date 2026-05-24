// wit-bindgen stub — 由此填入實作。
// 其餘生成檔案在 internal/，請勿手動修改。
#nullable enable

using PP = global::ReductionPluginWorld.wit.Imports.local.logProcess.v0_3_0.IPipelineProcessImports;

namespace ReductionPluginWorld;

public partial class ReductionPluginWorldExportsImpl : IReductionPluginWorldExports
{
    private static ulong _lastFilterNs;

    // WIT: filter(struct-data: list<log-entry>) -> result<list<filter-result>, plugin-error>
    public static global::System.Collections.Generic.List<PP.FilterResult> Filter(
        global::System.Collections.Generic.List<PP.LogEntry> structData)
    {
        var started = global::System.Diagnostics.Stopwatch.GetTimestamp();
        var results = new global::System.Collections.Generic.List<PP.FilterResult>(structData.Count);

        foreach (var entry in structData)
        {
            var level = (global::ReductionPlugin.LogLevel)(byte)entry.level;
            results.Add(new PP.FilterResult(
                id:   entry.id,
                keep: global::ReductionPlugin.LevelFilter.ShouldKeep(level)
            ));
        }

        _lastFilterNs = StopwatchTicksToNs(
            global::System.Diagnostics.Stopwatch.GetTimestamp() - started);
        return results;
    }

    public static int FilterCallback(/* TODO: event arg */) =>
        throw new global::System.NotImplementedException();

    // WIT: report-usage() -> u64
    public static ulong ReportUsage() => _lastFilterNs;

    public static int ReportUsageCallback(/* TODO: event arg */) =>
        throw new global::System.NotImplementedException();

    private static ulong StopwatchTicksToNs(long ticks) =>
        (ulong)((double)ticks * 1_000_000_000.0 / global::System.Diagnostics.Stopwatch.Frequency);
}
