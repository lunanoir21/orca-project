# Orca Theme Authoring Guide

Orca's look comes from two layers, in priority order (highest wins):

1. **A user override file** at `$XDG_DATA_HOME/orca/themes/active.css`
   (typically `~/.local/share/orca/themes/active.css`) — if present, it is
   loaded on top of everything else. This is the only file Orca reads from
   the themes directory; there is no per-file theme picker.
2. **A generated color stylesheet**, rebuilt at runtime from the active
   *scheme* + optional accent override + optional background image, every
   time you change anything in Settings → Appearance. This is where almost
   all color comes from.

Structural CSS (paddings, radii, transition durations — deliberately no
color) is compiled into the binary from `crates/orca-gui/resources/style.css`
and loaded at the lowest priority, so the generated colors and any user
override always win.

---

## Built-in Schemes

The picker in Settings → Appearance → Theme lists the schemes defined in
`crate::theme::SCHEMES` (`crates/orca-gui/src/theme.rs`) — there is no
separate `.css` file per scheme. Each one is five colors:

```rust
Scheme {
    id: "graphite",   // stable id, stored in config.toml
    name: "Grafit",   // shown in the picker
    bg: "#1b1b1f",      // window background
    surface: "#26262c", // raised panels (toolbar, headerbar, cards)
    text: "#ededf0",    // primary text
    subtext: "#9b9ba4", // secondary / dim text
    accent: "#e0a96d",  // default accent (overridable per-user)
}
```

Built-in ids: `graphite`, `mocha`, `nord`, `midnight`, `forest`, `rose`,
`latte`, `gruvbox`, `dracula`, `high-contrast`. To add a built-in scheme,
append a `Scheme` entry to `SCHEMES` — it appears in the picker automatically,
no other wiring needed.

Three more things are configurable alongside the scheme, all from the same
Settings page:

- **Accent override** — a hex color that replaces the scheme's default accent.
- **Background image + dim** — an image drawn behind the whole window, darkened by a `linear-gradient` scrim (0.0 = none, 1.0 = black).
- **Panel opacity** — a multiplier (0.3–1.0) on every panel/chrome background alpha (sidebar, toolbar, preview, status bar, ...), for a more see-through shell.

The `themes/` directory at the repository root (`catppuccin-mocha.css`,
`catppuccin-latte.css`, `gruvbox-dark.css`, `nord.css`, `dracula.css`) holds
**reference starting points for `active.css`**, not auto-loaded built-in
themes — copy one to `~/.local/share/orca/themes/active.css` (and edit it) to
use it.

---

## Writing an `active.css` Override

GTK4 CSS, not web CSS: colors are GTK *named colors* defined with
`@define-color`, referenced as `@name`, not CSS custom properties
(`--foo: ...; var(--foo)` does not exist in GTK4 CSS). The generated
stylesheet defines these names every time it runs:

```css
@define-color orca_bg #1b1b1f;
@define-color orca_surface #26262c;
@define-color orca_text #ededf0;
@define-color orca_subtext #9b9ba4;
@define-color orca_accent #e0a96d;
```

`active.css` is loaded *after* that, so it can either reference those names
or just set flat colors directly. A minimal override that recolors the
sidebar without touching anything else:

```css
.orca-places {
    background: #14141a;
}
.orca-place:hover {
    background: alpha(@orca_accent, 0.15);
}
```

Save this as `~/.local/share/orca/themes/active.css`. Unlike the scheme
picker, this file does not need an app restart or a settings change to take
effect on next launch, but it also has no live-toggle — remove or empty the
file to go back to the generated colors only.

---

## Useful Selectors

A non-exhaustive map of the custom classes Orca's own CSS targets (see
`crates/orca-gui/resources/style.css` and the `generate_css` function in
`theme.rs` for the full set):

| Selector | Description |
|----------|-------------|
| `window.orca-root` | The main window — background image/color lives here |
| `.orca-places` | The Places sidebar |
| `.orca-place` | A single sidebar row (button) |
| `.orca-toolbar` / `.orca-navbar` | Toolbar / breadcrumb bar |
| `.orca-card`, `.orca-card-grid` | Home page cards (quick actions, user dirs, recents) |
| `.orca-preview`, `.orca-preview-text` | The side preview panel |
| `.orca-quicklook-scrim`, `.orca-quicklook-card` | The Quick Look fullscreen overlay |
| `.orca-statusbar` | The status bar at the bottom |
| `columnview`, `gridview`, `listview` | The list/detail/icon file views |
| `.dim-label` | Secondary / muted text (GTK built-in) |
| `.success` / `.warning` / `.error` | Semantic status colors (GTK built-in) |
| `.monospace` | Monospace font (checksum display, octal, terminal-adjacent text) |
