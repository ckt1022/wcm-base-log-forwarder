#nullable enable

using System.Buffers;
using System.Buffers.Binary;
using System.Text.Json;
using PP = global::FormatPluginWorld.wit.Imports.local.logProcess.v0_3_0.IPipelineProcessImports;

namespace FormatPluginWorld;

public partial class FormatPluginWorldExportsImpl : IFormatPluginWorldExports
{

    public static byte[] Format(global::System.Collections.Generic.List<global::FormatPluginWorld.wit.Imports.local.logProcess.v0_3_0.IPipelineProcessImports.LogEntry> structData)
    {
        // H1: bindings have decoded the complete LogEntry list into managed objects.
        IFormatPluginWorldImports.TimeSign();

        using var output = new MemoryStream();
        var header = new byte[4];
        foreach (var entry in structData)
        {
            var buffer = new ArrayBufferWriter<byte>();
            using (var json = new Utf8JsonWriter(buffer))
            {
                json.WriteStartObject();
                json.WriteNumber("id", entry.id);
                json.WriteString("timestamp", entry.timestamp);
                json.WriteString("level", LevelName(entry.level));
                json.WriteString("message", entry.message);
                foreach (var (key, value) in entry.tags)
                {
                    json.WriteString(key, value);
                }
                json.WriteEndObject();
            }

            BinaryPrimitives.WriteUInt32LittleEndian(header, (uint)buffer.WrittenCount);
            output.Write(header);
            output.Write(buffer.WrittenSpan);
        }

        var result = output.ToArray();
        // H2: all framed flat-JSON output bytes have been constructed.
        IFormatPluginWorldImports.TimeSign();
        return result;
    }

    public static int FormatCallback(/* TODO: event arg */)
    {
        throw new NotImplementedException();
    }

    public static ulong ReportUsage()
    {
        return 0;
    }

    public static int ReportUsageCallback(/* TODO: event arg */)
    {
        throw new NotImplementedException();
    }

    private static string LevelName(PP.LogLevel level) => level switch
    {
        PP.LogLevel.DEBUG => "debug",
        PP.LogLevel.INFO => "info",
        PP.LogLevel.WARN => "warn",
        PP.LogLevel.ERROR => "error",
        PP.LogLevel.CRIT => "crit",
        PP.LogLevel.ALERT => "alert",
        PP.LogLevel.EMERG => "emerg",
        _ => "unknown",
    };
}
