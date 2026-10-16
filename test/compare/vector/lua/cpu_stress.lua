-- lua/cpu_stress.lua
-- Vector Lua transform — inject="cputest" 時進入無限數學迴圈 (CPU 100%)
-- 對應 Fluentd: plugins/filter_cpu_test.rb

function process(event, emit)
    local inject = event.log["inject"]
    if inject == "cputest" then
        io.stderr:write("[CPU TEST] 觸發 CPU 耗盡，進入無限數學迴圈（block transform thread）\n")
        local x = 0.0
        while true do
            x = x + math.sqrt(x + 1)
        end
    end
    emit(event)
end
