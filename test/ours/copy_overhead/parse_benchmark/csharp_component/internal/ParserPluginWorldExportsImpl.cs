#nullable enable

using System.Text.Json;
using PP = global::ParserPluginWorld.wit.Imports.local.logProcess.v0_3_0.IPipelineProcessImports;

namespace ParserPluginWorld;

public partial class ParserPluginWorldExportsImpl : IParserPluginWorldExports
{
    public static List<PP.ParsedEntry> Parse(List<string> rawData)
    {
        // H1: generated bindings have decoded list<string> into managed strings.
        IParserPluginWorldImports.TimeSign();

        var results = new List<PP.ParsedEntry>(rawData.Count);
        foreach (var raw in rawData)
        {
            try
            {
                using var document = JsonDocument.Parse(raw);
                var root = document.RootElement;
                var timestamp = ReadString(root, "ts");
                var message = ReadString(root, "msg");
                var level = ReadLevel(root);
                var tags = CollectTags(root);

                results.Add(new PP.ParsedEntry(
                    timestamp,
                    level,
                    message,
                    tags,
                    "A"));
            }
            catch (JsonException)
            {
                throw new WitException<PP.ParseError>(
                    PP.ParseError.InvalidFormat("JSON Parse Error"),
                    0);
            }
        }

        // H2: JSON parsing and managed ParsedEntry construction are complete.
        IParserPluginWorldImports.TimeSign();
        return results;
    }

    public static ulong ReportUsage() => 0;

    private static string ReadString(JsonElement root, string name)
    {
        return root.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString() ?? string.Empty
            : string.Empty;
    }

    private static PP.LogLevel ReadLevel(JsonElement root)
    {
        if (!root.TryGetProperty("level", out var value))
        {
            return PP.LogLevel.INFO;
        }
        if (value.ValueKind == JsonValueKind.Number && value.TryGetInt32(out var numeric))
        {
            return numeric is >= 0 and <= 6 ? (PP.LogLevel)numeric : PP.LogLevel.INFO;
        }
        if (value.ValueKind != JsonValueKind.String)
        {
            return PP.LogLevel.INFO;
        }

        return value.GetString()?.ToLowerInvariant() switch
        {
            "debug" => PP.LogLevel.DEBUG,
            "warn" => PP.LogLevel.WARN,
            "error" => PP.LogLevel.ERROR,
            "crit" => PP.LogLevel.CRIT,
            "alert" => PP.LogLevel.ALERT,
            "emerg" => PP.LogLevel.EMERG,
            _ => PP.LogLevel.INFO,
        };
    }

    private static List<(string, string)> CollectTags(JsonElement root)
    {
        // Keep the exact Rust benchmark output bytes for a controlled comparison.
        var tags = new List<(string, string)> { ("lang", "Rust") };
        foreach (var property in root.EnumerateObject())
        {
            if (property.Name is "ts" or "level" or "msg" or "att")
            {
                continue;
            }
            tags.Add((property.Name, JsonValue(property.Value)));
        }
        if (root.TryGetProperty("att", out var attributes) &&
            attributes.ValueKind == JsonValueKind.Object)
        {
            foreach (var property in attributes.EnumerateObject())
            {
                tags.Add((property.Name, JsonValue(property.Value)));
            }
        }
        return tags;
    }

    private static string JsonValue(JsonElement value) =>
        value.ValueKind == JsonValueKind.String
            ? value.GetString() ?? string.Empty
            : value.GetRawText();
}
