-- lua/file_access.lua
-- Vector Lua transform — inject="filetest" 時嘗試讀取敏感路徑
-- 對應 Fluentd: plugins/filter_file_access_test.rb

local PATHS = { "/etc/passwd", "/etc/shadow", "/proc/1/environ", "/run/secrets" }

function process(event, emit)
    local inject = event.log["inject"]
    if inject == "filetest" then
        local results = {}
        for _, path in ipairs(PATHS) do
            local f, err = io.open(path, "r")
            if f then
                local content = f:read(500) or ""
                f:close()
                results[path] = "readable | preview=" .. content:sub(1, 80)
            else
                local msg = tostring(err or "unknown")
                if msg:find("[Pp]ermission") then
                    results[path] = "permission_denied"
                else
                    results[path] = "error: " .. msg
                end
            end
        end

        local out = "[SECURITY TEST] 檔案存取測試結果:\n"
        for path, result in pairs(results) do
            out = out .. "  " .. path .. " => " .. result:sub(1, 120) .. "\n"
        end
        io.stderr:write(out)
        event.log["file_access_test"] = results
    end
    emit(event)
end
