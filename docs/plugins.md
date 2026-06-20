# Orca Plugin Authoring Guide

Orca plugins are `.lua` files loaded at startup from:

1. `$XDG_DATA_HOME/orca/plugins/` (user plugins, take priority)
2. The built-in `plugins/` directory shipped with Orca

---

## Metadata Header

Every plugin must start with a metadata comment block:

```lua
-- @name    My Plugin
-- @version 1.0.0
-- @author  Your Name
-- @description One-line description of what this plugin does
```

`@name` is required. `@version` defaults to `0.0.0`, `@author` defaults to `unknown`.

---

## Sandbox

Plugins run in a restricted Lua 5.4 environment:

| Allowed | Blocked |
|---------|---------|
| `string` | `io` |
| `table` | `os` |
| `math` | `package` |
| `orca.*` API | `debug` |
| | `require` |

Any attempt to call `require()` or access `io`, `os`, `package`, or `debug`
raises a runtime error. Vault internals are inaccessible.

Hook execution is limited to **500 ms**. A plugin that exceeds this limit is
moved to an error state and disabled for the session.

---

## API Reference

### Event Hooks

```lua
-- Called when the user selects a file.
orca.on_file_select(function(path)
    -- path: absolute string path of the selected file
end)

-- Called when the active directory changes.
orca.on_dir_change(function(path)
    -- path: absolute string path of the new directory
end)

-- Called when a vault locks (manually or via auto-lock timer).
orca.on_vault_lock(function(vault_name)
    -- vault_name: the name string of the vault that locked
end)
```

### UI Extensions

```lua
-- Register a named action (appears in toolbar / command palette).
-- fn receives a Lua array of selected file paths.
orca.register_action("action_id", "Display Label", function(files)
    for _, path in ipairs(files) do
        orca.log("acting on " .. path)
    end
end)

-- Add an item to the right-click context menu.
orca.add_context_item("Menu Label", function(files)
    -- files: array of selected paths at activation time
end)

-- Set a badge on a specific file path.
-- text: short string shown on the item (e.g. "M", "?", "✓")
-- color: CSS color string (e.g. "#e5c890", "red")
orca.set_badge(path, text, color)
```

### Utilities

```lua
-- Run a shell command (max 5 s timeout). Returns stdout as a string.
-- Only use this when every part of the command is a fixed literal — it runs
-- through `sh -c`, so interpolating a file path or other variable data into
-- the string is a shell-injection risk.
local output = orca.exec("git status --porcelain")

-- Run a program directly with an argv array — no shell involved, so this is
-- the safe choice whenever any argument is a file path or other variable
-- data (it can contain spaces, `;`, backticks, etc. with no special effect).
local output = orca.exec_argv("git", {"-C", dir, "status", "--porcelain"})

-- Show a desktop notification.
orca.notify("Title", "Body text")

-- Open a path with the default application.
orca.open("/home/user/document.pdf")

-- Write a line to the plugin log (visible in the Plugin Manager dialog).
orca.log("something happened")
```

---

## Example: Hello World

```lua
-- @name    Hello World
-- @version 1.0.0
-- @author  example
-- @description Greets the user when they navigate to their home directory

local home = orca.exec("echo $HOME"):gsub("%s+$", "")

orca.on_dir_change(function(path)
    if path == home then
        orca.notify("Welcome home!", path)
    end
end)
```

---

## Managing Plugins

Open the **Plugin Manager** from the sidebar (Plugins section). From there you
can see each plugin's state (enabled / disabled / error), view its log output,
and open the user plugins folder.

Plugins can be hot-reloaded without restarting Orca via the Plugin Manager.
