-- lua/loop_trigger.lua
-- Vector Lua transform — inject="loop" 時進入無窮迴圈，阻塞 transform thread
-- 對應 Fluentd: plugins/filter_loop_trigger.rb

function process(event, emit)
    local inject = event.log["inject"]
    if inject == "loop" then
        io.stderr:write("[LOOP] 偵測到 inject='loop'，即將進入無窮迴圈\n")
        while true do end
    end
    emit(event)
end
