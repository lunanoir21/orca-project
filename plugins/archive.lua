-- @name Archive
-- @version 1.0.0
-- @author orca
-- @description Adds context-menu items to compress/extract archive files

-- Supported archive extensions and their extract commands, as argv arrays
-- (program + args) rather than shell strings: file/dir names are real
-- filesystem paths that may contain spaces or shell metacharacters, and
-- orca.exec_argv passes each token straight to execve with no shell in
-- between, so they can't be used for command injection.
local extractors = {
    zip    = {"unzip", "-d", "{dir}", "{file}"},
    tar    = {"tar", "-xf", "{file}", "-C", "{dir}"},
    gz     = {"tar", "-xzf", "{file}", "-C", "{dir}"},
    bz2    = {"tar", "-xjf", "{file}", "-C", "{dir}"},
    xz     = {"tar", "-xJf", "{file}", "-C", "{dir}"},
    ["7z"] = {"7z", "x", "{file}", "-o{dir}"},
    zst    = {"tar", "-I", "zstd", "-xf", "{file}", "-C", "{dir}"},
}

local function ext(path)
    -- Handle double extensions like .tar.gz / .tar.bz2 / .tar.xz / .tar.zst
    local double = path:match("%.tar%.(gz|bz2|xz|zst)$")
    if double then return double end
    return path:match("%.([^.]+)$")
end

local function dir_of(path)
    return path:match("^(.*)/[^/]+$") or "."
end

-- Substitute {file}/{dir} into every token of an extractor template,
-- returning (program, args) ready for orca.exec_argv.
local function build_argv(tmpl, file, dir)
    local args = {}
    for i = 2, #tmpl do
        args[i - 1] = tmpl[i]:gsub("{file}", file):gsub("{dir}", dir)
    end
    return tmpl[1], args
end

-- Context menu: "Extract Here"
orca.add_context_item("Extract Here", function(files)
    for _, file in ipairs(files) do
        local e = ext(file)
        local tmpl = extractors[e]
        if tmpl then
            local out_dir = dir_of(file)
            local program, args = build_argv(tmpl, file, out_dir)
            local result = orca.exec_argv(program, args)
            if result then
                orca.log("extracted " .. file)
            end
        else
            orca.log("unsupported archive format: " .. (e or "unknown"))
        end
    end
end)

-- Context menu: "Extract to Subfolder"
orca.add_context_item("Extract to Subfolder", function(files)
    for _, file in ipairs(files) do
        local e = ext(file)
        local tmpl = extractors[e]
        if tmpl then
            -- Strip extension(s) to get subfolder name.
            local name = file:match("([^/]+)$") or "archive"
            name = name:gsub("%.tar%.[a-z]+$", ""):gsub("%.[a-z0-9]+$", "")
            local out_dir = dir_of(file) .. "/" .. name
            orca.exec_argv("mkdir", {"-p", out_dir})
            local program, args = build_argv(tmpl, file, out_dir)
            orca.exec_argv(program, args)
            orca.log("extracted " .. file .. " → " .. out_dir)
        end
    end
end)

-- Action: "Compress to zip" (toolbar/menu action on selected files)
orca.register_action("compress_zip", "Compress to .zip", function(files)
    if #files == 0 then return end
    -- Name the archive after the first selected item.
    local first = files[1]:match("([^/]+)$") or "archive"
    local out_dir = dir_of(files[1])
    local zip_name = out_dir .. "/" .. first .. ".zip"

    local args = {"-r", zip_name}
    for _, f in ipairs(files) do
        args[#args + 1] = f
    end
    local result = orca.exec_argv("zip", args)
    if result then
        orca.log("created " .. zip_name)
        orca.notify("Archive created", zip_name)
    end
end)
