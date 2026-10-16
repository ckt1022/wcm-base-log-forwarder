// Parsers.cs — pure parsing logic with no WIT dependencies.
// JSON / syslog / logfmt → ParseResult, which ParserPluginWorldExportsImpl converts
// into the WIT-generated ParsedEntry type.

using System.Text.Json;

namespace ParserPlugin;

public enum LogLevel : byte
{
    Debug = 0,
    Info  = 1,
    Warn  = 2,
    Error = 3,
    Crit  = 4,
    Alert = 5,
    Emerg = 6,
}

public struct ParseResult
{
    public string Timestamp;
    public LogLevel Level;
    public string Message;
    public List<(string Key, string Value)> Tags;
    public string Targettag;
}

public static class Router
{
    public static string RouteTag(LogLevel level) => level switch
    {
        LogLevel.Error or LogLevel.Crit or LogLevel.Alert or LogLevel.Emerg => "AB",
        LogLevel.Warn => "BC",
        _ => "C",
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON  {"ts":"…","level":"…","msg":"…","att":{…}}
// ─────────────────────────────────────────────────────────────────────────────

public static class JsonParser
{
    public static bool TryParse(string raw, out ParseResult result)
    {
        result = default;
        if (string.IsNullOrWhiteSpace(raw)) return false;

        JsonDocument doc;
        try { doc = JsonDocument.Parse(raw); }
        catch { return false; }

        using (doc)
        {
            var root = doc.RootElement;

            var ts  = root.TryGetProperty("ts",    out var tsEl)    && tsEl.ValueKind    == JsonValueKind.String ? tsEl.GetString()    ?? "" : "";
            var msg = root.TryGetProperty("msg",   out var msgEl)   && msgEl.ValueKind   == JsonValueKind.String ? msgEl.GetString()   ?? "" : "";
            var lvl = root.TryGetProperty("level", out var levelEl) && levelEl.ValueKind == JsonValueKind.String ? levelEl.GetString() ?? "" : "";

            var tags = new List<(string, string)> { ("lang", "C#") };

            foreach (var prop in root.EnumerateObject())
            {
                if (prop.Name is "ts" or "level" or "msg" or "att") continue;
                tags.Add((prop.Name, prop.Value.ValueKind == JsonValueKind.String
                    ? prop.Value.GetString() ?? ""
                    : prop.Value.GetRawText()));
            }

            if (root.TryGetProperty("att", out var attEl) && attEl.ValueKind == JsonValueKind.Object)
            {
                foreach (var kv in attEl.EnumerateObject())
                    tags.Add((kv.Name, kv.Value.ValueKind == JsonValueKind.String
                        ? kv.Value.GetString() ?? ""
                        : kv.Value.GetRawText()));
            }

            var parsedLevel = MapLevel(lvl);
            result = new ParseResult
            {
                Timestamp = ts,
                Level     = parsedLevel,
                Message   = msg,
                Tags      = tags,
                Targettag = Router.RouteTag(parsedLevel),
            };
            return true;
        }
    }

    private static LogLevel MapLevel(string s) => s.ToLowerInvariant() switch
    {
        "debug" => LogLevel.Debug,
        "info"  => LogLevel.Info,
        "warn"  => LogLevel.Warn,
        "error" => LogLevel.Error,
        "crit"  => LogLevel.Crit,
        "alert" => LogLevel.Alert,
        "emerg" => LogLevel.Emerg,
        _       => LogLevel.Info,
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// logfmt  ts=… level=… msg="…" key=value
// ─────────────────────────────────────────────────────────────────────────────

public static class LogfmtParser
{
    public static bool TryParse(string raw, out ParseResult result)
    {
        result = default;
        if (string.IsNullOrWhiteSpace(raw)) return false;

        var kvBuf = new List<(string, string)>(16);
        if (!ParseFields(raw.AsSpan(), kvBuf)) return false;

        var ts  = GetKv(kvBuf, "ts");
        var msg = GetKv(kvBuf, "msg");
        var lvl = GetKv(kvBuf, "level");

        var tags = new List<(string, string)>(kvBuf.Count + 1) { ("lang", "C#") };
        foreach (var (k, v) in kvBuf)
        {
            if (k is "ts" or "level" or "msg") continue;
            tags.Add((k, v));
        }

        var parsedLevel = MapLevel(lvl);
        result = new ParseResult
        {
            Timestamp = ts,
            Level     = parsedLevel,
            Message   = msg,
            Tags      = tags,
            Targettag = Router.RouteTag(parsedLevel),
        };
        return true;
    }

    public static bool ParseFields(ReadOnlySpan<char> s, List<(string, string)> out_)
    {
        int i = 0, n = s.Length;
        while (i < n)
        {
            while (i < n && s[i] == ' ') i++;
            if (i >= n) break;

            int keyStart = i;
            while (i < n && s[i] != '=' && s[i] != ' ') i++;
            if (i >= n || s[i] != '=') return false;

            string key = s[keyStart..i].ToString();
            i++;

            string val;
            if (i < n && s[i] == '"')
            {
                i++;
                var sb = new System.Text.StringBuilder();
                while (i < n)
                {
                    if (s[i] == '\\' && i + 1 < n)
                    {
                        i++;
                        sb.Append(s[i] switch { '\\' => '\\', '"' => '"', 'n' => '\n', 't' => '\t', 'r' => '\r', _ => s[i] });
                    }
                    else if (s[i] == '"') { i++; break; }
                    else sb.Append(s[i]);
                    i++;
                }
                val = sb.ToString();
            }
            else
            {
                int valStart = i;
                while (i < n && s[i] != ' ') i++;
                val = s[valStart..i].ToString();
            }

            out_.Add((key, val));
        }
        return true;
    }

    private static string GetKv(List<(string, string)> kvs, string key)
    {
        foreach (var (k, v) in kvs) if (k == key) return v;
        return "";
    }

    private static LogLevel MapLevel(string s) => s.ToLowerInvariant() switch
    {
        "debug" => LogLevel.Debug,
        "info"  => LogLevel.Info,
        "warn"  => LogLevel.Warn,
        "error" => LogLevel.Error,
        "crit"  => LogLevel.Crit,
        "alert" => LogLevel.Alert,
        "emerg" => LogLevel.Emerg,
        _       => LogLevel.Info,
    };
}
