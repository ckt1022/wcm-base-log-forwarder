-- Five error-injection scenarios for Fluent Bit Lua filter testing.
-- Each function only triggers when the record contains the matching
-- "inject" field value (or the raw log string contains it).
-- Normal records always pass through so the before/after contrast is visible.

local function should_trigger(record, value)
    if record["inject"] == value then return true end
    local raw = tostring(record["log"] or "")
    return raw:find('"inject"%s*:%s*"' .. value .. '"') ~= nil
end

-- 1. Infinite loop
function test_infinite_loop(tag, timestamp, record)
    if should_trigger(record, "loop") then
        while true do end
    end
    return 0, timestamp, record
end

-- 2. I/O blocking
function test_io_blocking(tag, timestamp, record)
    if should_trigger(record, "io") then
        io.read("*l")
    end
    return 0, timestamp, record
end

-- 3. CPU exhaustion: spins at ~100% CPU indefinitely (stopped externally after 3 min).
--    Records the sustained spike; container is stopped by run_crash.sh after TEST_DURATION.
function test_cpu_exhaustion(tag, timestamp, record)
    if should_trigger(record, "cpu") then
        local x = 0
        while true do
            x = x + math.sqrt(x + 1)
        end
    end
    return 0, timestamp, record
end

-- 4. Memory exhaustion: accumulates every record into a global table after trigger.
--    _G._leak persists across all filter invocations — GC cannot reclaim it because
--    the global reference is always live. Memory grows proportionally to throughput,
--    modelling a realistic supply-chain attack where a plugin silently retains all logs.
--    Expected result: memory grows monotonically until container is OOM killed (exit 137).
_G._leak        = {}
_G._leak_active = false

function test_memory_exhaustion(tag, timestamp, record)
    if should_trigger(record, "mem") then
        _G._leak_active = true
    end
    if _G._leak_active then
        _G._leak[#_G._leak + 1] = { r = record, p = string.rep("X", 2048) }
    end
    return 0, timestamp, record
end

-- 5. Single parse error: no error handling — crashes on malformed log field.
--    Trigger: send a line whose log field is not valid JSON (e.g. "{broken").
function test_parse_error(tag, timestamp, record)
    local raw = record["log"]
    local first_key = raw:match('"(%w+)"')
    record["first_key"] = first_key:upper()  -- crashes if match returns nil
    return 1, timestamp, record
end


-- 新增filter函數