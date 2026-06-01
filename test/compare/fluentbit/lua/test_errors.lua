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

-- 3. CPU exhaustion: spins at 100% CPU for 10 seconds then returns.
--    Unlike infinite_loop, the function eventually completes so we can
--    observe how many records were delayed and whether Fluent Bit recovers.
function test_cpu_exhaustion(tag, timestamp, record)
    if should_trigger(record, "cpu") then
        local deadline = os.clock() + 10
        local x = 0
        while os.clock() < deadline do
            x = x + math.sqrt(x + 1)
        end
        record["_cpu_result"] = x
        return 1, timestamp, record
    end
    return 0, timestamp, record
end

-- 4. Memory exhaustion: allocates 1 MB unique strings until OOM.
--    string.rep is called inside the loop so each iteration produces a
--    distinct heap object; LuaJIT cannot share references.
--    Expected result: Linux OOM killer terminates the process.
function test_memory_exhaustion(tag, timestamp, record)
    if should_trigger(record, "mem") then
        local sink = {}
        local i = 0
        while true do
            i = i + 1
            sink[i] = string.rep("X", 1024 * 1024)  -- 1 MB new allocation per iteration
        end
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