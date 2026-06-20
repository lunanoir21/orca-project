-- @name Secure Open
-- @version 1.0.0
-- @author orca
-- @description Opens any selected file in a bubblewrap sandbox: read-only
--              filesystem, no network, no view of the rest of your home
--              directory. For PDFs and other files you don't fully trust.
--
-- This is namespace-based isolation (bubblewrap / Linux user namespaces),
-- not a hardened security boundary like seccomp filtering or a portal model:
-- it stops the opened app from reading/writing other files or reaching the
-- network over a routed connection, but it cannot stop every possible local
-- side channel (e.g. an app talking to a host service over a D-Bus socket
-- that's still reachable). Treat it as a strong reduction in blast radius,
-- not a guarantee.
--
-- Requires `bubblewrap` (the `bwrap` binary) to be installed. If it isn't,
-- this plugin notifies once and does nothing, rather than silently falling
-- back to an unsandboxed open.

local bwrap_checked = false
local bwrap_available = false

local function has_bwrap()
    if bwrap_checked then
        return bwrap_available
    end
    bwrap_checked = true
    -- `exec_argv` raises a Lua error if the program can't be found/spawned;
    -- "command -v" via the real shell isn't available to us (exec_argv has no
    -- shell), so probe with bwrap's own --version instead.
    local ok = pcall(function()
        orca.exec_argv("bwrap", {"--version"})
    end)
    bwrap_available = ok
    return ok
end

orca.add_context_item("Secure Open", function(files)
    if not has_bwrap() then
        orca.notify(
            "Secure Open",
            "bubblewrap (bwrap) is not installed — this feature is unavailable."
        )
        return
    end

    for _, file in ipairs(files) do
        local args = {
            "--ro-bind", "/", "/",
            "--dev", "/dev",
            "--proc", "/proc",
            "--tmpfs", "/tmp",
            "--tmpfs", "/home",
            "--tmpfs", "/root",
            -- Re-expose just the one file being viewed, read-only, after the
            -- /home (or /root) tmpfs above has hidden everything else there.
            "--ro-bind", file, file,
            "--unshare-all",
            "--die-with-parent",
            "--",
            "xdg-open", file,
        }
        -- spawn_argv, not exec_argv: the viewer is a long-running GUI
        -- process, not a short command we wait on and capture output from.
        orca.spawn_argv("bwrap", args)
        orca.log("secure-open: sandboxed " .. file)
    end
end)
