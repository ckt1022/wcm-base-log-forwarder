#nullable enable

using PP = global::ReductionPluginWorld.wit.Imports.local.logProcess.v0_3_0.IPipelineProcessImports;

namespace ReductionPluginWorld;

public partial class ReductionPluginWorldExportsImpl : IReductionPluginWorldExports
{
    public static List<PP.FilterResult> Filter(List<PP.LogEntry> structData)
    {
        // H1: bindings have decoded the complete LogEntry list into managed objects.
        IReductionPluginWorldImports.TimeSign();

        var results = new List<PP.FilterResult>(structData.Count);
        foreach (var entry in structData)
        {
            results.Add(new PP.FilterResult(
                entry.id,
                (byte)entry.level >= (byte)PP.LogLevel.WARN));
        }

        // H2: all decisions and managed FilterResult objects are complete.
        IReductionPluginWorldImports.TimeSign();
        return results;
    }

    public static int FilterCallback(/* TODO: event arg */)
        => throw new NotImplementedException();

    public static ulong ReportUsage() => 0;

    public static int ReportUsageCallback(/* TODO: event arg */)
        => throw new NotImplementedException();
}
