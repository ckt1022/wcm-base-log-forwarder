-- lua/mem_stress.lua
-- Vector Lua transform — inject="memtest" 時啟動記憶體洩漏
-- 參考 Fluent Bit: test_errors.lua::test_memory_exhaustion
--
-- _G._leak 為全域 table，在 transform 初始化後持續存活，GC 無法回收。
-- 記憶體成長速率 ≈ loggen 速率 × 2 KB/record，直到容器 OOM kill (exit 137)。

_G._leak        = {}
_G._leak_active = false

function process(event, emit)
    local inject = event.log["inject"]
    if inject == "memtest" then
        _G._leak_active = true
        io.stderr:write("[MEM TEST] 觸發記憶體洩漏，開始累積 _G._leak...\n")
    end

    if _G._leak_active then
        _G._leak[#_G._leak + 1] = { r = event.log, p = string.rep("X", 1024 * 64) }
        if (#_G._leak % 100) == 0 then
            io.stderr:write("[MEM TEST] leak count=" .. #_G._leak .. "\n")
        end
    end

    emit(event)
end
