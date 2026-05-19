// Filters.cs — 純過濾邏輯，不依賴任何 WIT 生成型別。
// ReductionPluginWorldExportsImpl.cs 呼叫此處，並將 WIT 型別轉換後傳入。

namespace ReductionPlugin;

/// <summary>
/// 對應 WIT log-level enum（index 與 WIT 定義相同）。
/// </summary>
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

/// <summary>
/// Level-based filter：保留 >= MinLevel 的 log，丟棄其餘。
/// 編譯期常數，不需 runtime config，適合論文的 ablation 量測。
/// </summary>
public static class LevelFilter
{
#if FILTER_ERROR
    private const LogLevel MinLevel = LogLevel.Error;
#elif FILTER_CRIT
    private const LogLevel MinLevel = LogLevel.Crit;
#else
    // 預設：保留 Warn 以上（丟棄 Debug / Info）
    private const LogLevel MinLevel = LogLevel.Warn;
#endif

    public static bool ShouldKeep(LogLevel level) => level >= MinLevel;
}
