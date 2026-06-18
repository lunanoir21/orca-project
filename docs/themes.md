# Orca Theme Authoring Guide

Orca themes are CSS files loaded by GTK4's CSS provider. Built-in themes ship
in the `themes/` directory; user themes go in `$XDG_DATA_HOME/orca/themes/`
(typically `~/.local/share/orca/themes/`).

Switch themes live from Settings → Appearance → Theme without restarting.

---

## Theme File Structure

A theme file is a plain `.css` file. The filename (without `.css`) becomes the
theme name shown in the Settings UI.

```
themes/
├── catppuccin-mocha.css    # built-in (dark)
├── catppuccin-latte.css    # built-in (light)
├── gruvbox-dark.css        # built-in
├── nord.css                # built-in
└── dracula.css             # built-in
```

---

## CSS Variables

Orca defines the following custom properties (CSS variables) in `base.css` that
themes can override:

```css
:root {
    /* Background layers */
    --orca-bg:          #1e1e2e;   /* main window background */
    --orca-bg2:         #313244;   /* sidebar, panels */
    --orca-bg3:         #45475a;   /* hover, selected rows */

    /* Text */
    --orca-fg:          #cdd6f4;   /* primary text */
    --orca-fg2:         #a6adc8;   /* secondary / dim text */
    --orca-fg3:         #7f849c;   /* placeholder, disabled */

    /* Accent */
    --orca-accent:      #89b4fa;   /* links, focus rings, progress */
    --orca-accent-fg:   #1e1e2e;   /* text on accent-colored surfaces */

    /* Status */
    --orca-success:     #a6e3a1;
    --orca-warning:     #f9e2af;
    --orca-error:       #f38ba8;

    /* Borders */
    --orca-border:      #45475a;
    --orca-border-focus: #89b4fa;

    /* Spacing / shape */
    --orca-radius:      6px;
    --orca-padding:     8px;
}
```

---

## Minimal Theme Example

```css
/* my-theme.css — A minimal warm dark theme */
:root {
    --orca-bg:          #1c1917;
    --orca-bg2:         #292524;
    --orca-bg3:         #44403c;
    --orca-fg:          #e7e5e4;
    --orca-fg2:         #a8a29e;
    --orca-fg3:         #78716c;
    --orca-accent:      #fb923c;
    --orca-accent-fg:   #1c1917;
    --orca-success:     #86efac;
    --orca-warning:     #fde68a;
    --orca-error:       #fca5a5;
    --orca-border:      #44403c;
    --orca-border-focus: #fb923c;
}
```

Save this as `~/.local/share/orca/themes/my-theme.css` and select it in
Settings → Appearance → Theme.

---

## GTK4 Selector Reference

Orca uses standard GTK4 selectors alongside custom CSS classes. Useful ones:

| Selector | Description |
|----------|-------------|
| `.orca-pane` | The file browser pane container |
| `.orca-places` | The sidebar rail |
| `.orca-bookmark-row` | A bookmark row in the sidebar |
| `.orca-statusbar` | The status bar at the bottom |
| `columnview` | The list/detail view |
| `gridview` | The icon view |
| `searchbar` | The inline search bar |
| `.dim-label` | Secondary / muted text labels |
| `.success` | Green-tinted text |
| `.warning` | Yellow-tinted text |
| `.error` | Red-tinted text |
| `.monospace` | Monospace font (checksum display, octal) |
