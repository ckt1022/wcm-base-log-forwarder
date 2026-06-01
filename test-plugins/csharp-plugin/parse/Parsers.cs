// Parsers.cs — pure parsing logic with no WIT dependencies.
// All three modes (JSON / syslog / logfmt) produce ParseResult structs
// that Program.cs converts into the WIT-generated ParsedEntry type.

using System.Text.Json;

namespace ParserPlugin;

// ─────────────────────────────────────────────────────────────────────────────
// Internal types mirroring the WIT schema (no unsafe / interop)
// ─────────────────────────────────────────────────────────────────────────────

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

            // Fields outside reserved keys go to tags
            foreach (var prop in root.EnumerateObject())
            {
                if (prop.Name is "ts" or "level" or "msg" or "att") continue;
                tags.Add((prop.Name, prop.Value.ValueKind == JsonValueKind.String
                    ? prop.Value.GetString() ?? ""
                    : prop.Value.GetRawText()));
            }

            // Nested "att" object — all its fields become tags
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
// Syslog  RFC 5424-like:  <PRI>1 TIMESTAMP HOST APP PID MSGID STRUCTURED-DATA MSG
// Generator format:       <34>1 2024-01-01T00:00:00Z host app 1234 - message level=info
// ─────────────────────────────────────────────────────────────────────────────

public static class SyslogParser
{
    public static bool TryParse(string raw, out ParseResult result)
    {
        result = default;
        var line = raw.AsSpan().Trim();

        if (line.IsEmpty || line[0] != '<') return false;

        int endPri = line.IndexOf('>');
        if (endPri <= 1) return false;

        if (!int.TryParse(line[1..endPri], out int pri)) return false;
        int severity = pri % 8;

        var rest = line[(endPri + 1)..];

        // Expect at least 8 space-delimited tokens: version ts host app pid msgid structuredData [msg...]
        Span<Range> parts = stackalloc Range[8];
        int count = SplitN(rest, ' ', parts);
        if (count < 8) return false;

        if (!rest[parts[0]].SequenceEqual("1")) return false;

        var ts   = rest[parts[1]].ToString();
        var host = rest[parts[2]].ToString();
        var app  = rest[parts[3]].ToString();

        // Everything from token 7 onward is the message (may contain embedded logfmt fields)
        int msgStart = rest[..parts[7].Start].LastIndexOf(' ') + 1;
        // simpler: reassemble from token index 7
        var msgAndMore = rest[parts[7].Start..].ToString().Trim();

        string msg       = msgAndMore;
        string fieldPart = "";

        int levelIdx = msgAndMore.IndexOf(" level=", StringComparison.Ordinal);
        if (levelIdx >= 0)
        {
            msg       = msgAndMore[..levelIdx].Trim();
            fieldPart = msgAndMore[(levelIdx + 1)..];
        }

        var kvBuf = new List<(string, string)>();
        if (fieldPart.Length > 0)
            LogfmtParser.ParseFields(fieldPart.AsSpan(), kvBuf);

        string levelStr = GetKv(kvBuf, "level");
        var level = levelStr.Length > 0 ? MapLogfmtLevel(levelStr) : SeverityToLevel(severity);

        var tags = new List<(string, string)>(kvBuf.Count + 3) { ("lang", "C#"), ("host", host), ("app_name", app) };
        foreach (var (k, v) in kvBuf)
        {
            if (k != "level") tags.Add((k, v));
        }

        result = new ParseResult { Timestamp = ts, Level = level, Message = msg, Tags = tags, Targettag = Router.RouteTag(level) };
        return true;
    }

    private static int SplitN(ReadOnlySpan<char> s, char sep, Span<Range> parts)
    {
        int count = 0;
        int start = 0;
        for (int i = 0; i <= s.Length; i++)
        {
            if (i == s.Length || s[i] == sep)
            {
                if (count < parts.Length)
                    parts[count] = new Range(start, i);
                count++;
                start = i + 1;
                if (count == parts.Length - 1) // last slot gets the rest
                {
                    parts[count] = new Range(start, s.Length);
                    return count + 1;
                }
            }
        }
        return count;
    }

    private static string GetKv(List<(string, string)> kvs, string key)
    {
        foreach (var (k, v) in kvs)
            if (k == key) return v;
        return "";
    }

    private static LogLevel SeverityToLevel(int sev) => sev switch
    {
        0 or 1 or 2 or 3 => LogLevel.Error,
        4                 => LogLevel.Warn,
        5 or 6            => LogLevel.Info,
        7                 => LogLevel.Debug,
        _                 => LogLevel.Info,
    };

    private static LogLevel MapLogfmtLevel(string s) => s.ToLowerInvariant() switch
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

    // ParseFields is also called by SyslogParser for embedded logfmt field sections.
    public static bool ParseFields(ReadOnlySpan<char> s, List<(string, string)> out_)
    {
        int i = 0;
        int n = s.Length;

        while (i < n)
        {
            while (i < n && s[i] == ' ') i++;
            if (i >= n) break;

            int keyStart = i;
            while (i < n && s[i] != '=' && s[i] != ' ') i++;
            if (i >= n || s[i] != '=') return false;

            string key = s[keyStart..i].ToString();
            i++; // skip '='

            string val;
            if (i < n && s[i] == '"')
            {
                i++; // skip opening quote
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
