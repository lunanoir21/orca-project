-- @name Git Status
-- @version 1.0.0
-- @author orca
-- @description Sets git status badges on files in tracked repositories

-- Badge colors for each git status letter.
local colors = {
    M = "#e5c890", -- modified  → yellow
    A = "#a6d189", -- added     → green
    D = "#e78284", -- deleted   → red
    R = "#ca9ee6", -- renamed   → purple
    C = "#85c1dc", -- copied    → blue
    U = "#ef9f76", -- unmerged  → orange
    ["?"] = "#a5adce", -- untracked → muted blue
}

local function scan_repo(dir)
    local out = orca.exec("git -C " .. dir .. " status --porcelain 2>/dev/null")
    if not out or out == "" then return end

    for line in out:gmatch("[^\n]+") do
        -- porcelain format: XY path (or XY old -> new for renames)
        local xy, path = line:match("^(..)%s+(.+)$")
        if xy and path then
            -- rename: "old -> new" — show badge on new path
            local renamed = path:match("^.+%->%s+(.+)$")
            if renamed then path = renamed end

            -- index status (first char) takes priority; fall back to work-tree
            local ch = xy:sub(1, 1)
            if ch == " " then ch = xy:sub(2, 2) end
            if ch == " " then ch = nil end

            if ch then
                local color = colors[ch] or "#a5adce"
                local full = dir .. "/" .. path
                orca.set_badge(full, ch, color)
            end
        end
    end
end

orca.on_dir_change(function(path)
    -- Walk up to find the git repo root so we badge all dirty files at once.
    local root = orca.exec(
        "git -C " .. path .. " rev-parse --show-toplevel 2>/dev/null"
    ):gsub("%s+$", "")
    if root and root ~= "" then
        scan_repo(root)
    end
end)
