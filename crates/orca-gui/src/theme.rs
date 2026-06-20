//! Runtime theming for the application shell.
//!
//! The compiled-in [`struct@BASE_CSS`] carries layout only (paddings, radii,
//! sizes). *All colors* are generated at runtime by [`apply`] from the active
//! [`Scheme`] (a named palette), an optional accent override and an optional
//! background image, then loaded into one replaceable USER-priority provider.
//! Recoloring is therefore live: the appearance picker calls [`apply`] again.
//!
//! A user CSS override at `<themes_dir>/active.css` is still honored on top.

use std::cell::RefCell;
use std::path::Path;

use relm4::gtk;

/// The built-in structural stylesheet (layout only), compiled into the binary.
const BASE_CSS: &str = include_str!("../resources/style.css");

thread_local! {
    /// The single, reusable provider that carries the generated color CSS.
    /// Thread-local because GTK objects are neither `Send` nor `Sync`, and all
    /// theme calls happen on the GTK main thread.
    static COLOR_PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

/// A named color palette.
#[derive(Debug, Clone, Copy)]
pub struct Scheme {
    /// Stable id stored in config and used by the picker.
    pub id: &'static str,
    /// Human-facing display name.
    pub name: &'static str,
    /// Window background (solid base behind any image).
    bg: &'static str,
    /// Raised surface color (panels, header, toolbar).
    surface: &'static str,
    /// Primary text color.
    text: &'static str,
    /// Secondary / dim text color.
    subtext: &'static str,
    /// Default accent when the user has not chosen one.
    accent: &'static str,
}

/// All built-in schemes. The first is the default. Deliberately spans warm and
/// cool palettes so the default does not read as a generic blue/purple theme.
pub const SCHEMES: &[Scheme] = &[
    Scheme {
        id: "graphite",
        name: "Grafit",
        bg: "#1b1b1f",
        surface: "#26262c",
        text: "#ededf0",
        subtext: "#9b9ba4",
        accent: "#e0a96d",
    },
    Scheme {
        id: "mocha",
        name: "Mocha",
        bg: "#1e1e2e",
        surface: "#313244",
        text: "#cdd6f4",
        subtext: "#a6adc8",
        accent: "#f5c2e7",
    },
    Scheme {
        id: "nord",
        name: "Nord",
        bg: "#2e3440",
        surface: "#3b4252",
        text: "#eceff4",
        subtext: "#aeb6c2",
        accent: "#88c0d0",
    },
    Scheme {
        id: "midnight",
        name: "Gece Mavisi",
        bg: "#0e1320",
        surface: "#18203a",
        text: "#e6ebff",
        subtext: "#93a0c0",
        accent: "#5ec8ff",
    },
    Scheme {
        id: "forest",
        name: "Orman",
        bg: "#14201a",
        surface: "#1e2e25",
        text: "#e7f0ea",
        subtext: "#9bb3a4",
        accent: "#7fd1a0",
    },
    Scheme {
        id: "rose",
        name: "Gül",
        bg: "#20141a",
        surface: "#2e1f27",
        text: "#f3e6ec",
        subtext: "#c09bab",
        accent: "#ff8fab",
    },
    Scheme {
        id: "latte",
        name: "Latte",
        bg: "#eff1f5",
        surface: "#e6e9ef",
        text: "#4c4f69",
        subtext: "#6c6f85",
        accent: "#1e66f5",
    },
    Scheme {
        id: "gruvbox",
        name: "Gruvbox",
        bg: "#282828",
        surface: "#3c3836",
        text: "#ebdbb2",
        subtext: "#a89984",
        accent: "#fe8019",
    },
    Scheme {
        id: "dracula",
        name: "Dracula",
        bg: "#282a36",
        surface: "#44475a",
        text: "#f8f8f2",
        subtext: "#6272a4",
        accent: "#bd93f9",
    },
    Scheme {
        id: "high-contrast",
        name: "Yüksek Kontrast",
        bg: "#000000",
        surface: "#1a1a1a",
        text: "#ffffff",
        subtext: "#d6d6d6",
        accent: "#ffd60a",
    },
];

impl Scheme {
    /// This scheme's default accent color as a `#rrggbb` string.
    #[must_use]
    pub fn accent_hex(&self) -> String {
        self.accent.to_owned()
    }
}

/// Look up a scheme by id, falling back to the default (first) scheme.
#[must_use]
pub fn scheme(id: &str) -> &'static Scheme {
    SCHEMES.iter().find(|s| s.id == id).unwrap_or(&SCHEMES[0])
}

/// Load the structural stylesheet and register the color provider, then a user
/// override at `<themes_dir>/active.css` if present.
///
/// Must be called after GTK is initialised (a default [`gtk::gdk::Display`]
/// must exist); otherwise it logs a warning and does nothing.
pub fn load(themes_dir: &Path) {
    let Some(display) = gtk::gdk::Display::default() else {
        tracing::warn!("no default display; skipping CSS load");
        return;
    };

    add_provider_from_data(&display, BASE_CSS, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let color = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &display,
        &color,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );
    COLOR_PROVIDER.with(|p| *p.borrow_mut() = Some(color));

    let user_css = themes_dir.join("active.css");
    match std::fs::read_to_string(&user_css) {
        Ok(css) => {
            tracing::info!(path = %user_css.display(), "loading user theme");
            // One step above USER so a hand-written override beats generated colors.
            add_provider_from_data(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            tracing::warn!(path = %user_css.display(), error = %e, "failed to read user theme")
        }
    }
}

/// Regenerate and apply the color CSS from the active scheme, an optional accent
/// override (hex like `#rrggbb`), an optional background image, the scrim
/// `dim` (0.0–1.0) drawn over that image, and a `panel_opacity` (0.0–1.0)
/// multiplier applied to every panel/chrome background alpha (sidebar,
/// toolbar, preview, status bar, ...) so the user can make the whole shell
/// more see-through without touching per-widget CSS.
pub fn apply(
    scheme_id: &str,
    accent: Option<&str>,
    image: Option<&Path>,
    dim: f64,
    panel_opacity: f64,
    font: &str,
) {
    let s = scheme(scheme_id);
    let accent = accent.filter(|a| !a.is_empty()).unwrap_or(s.accent);
    let css = generate_css(s, accent, image, dim, panel_opacity, font);
    COLOR_PROVIDER.with(|p| {
        if let Some(provider) = p.borrow().as_ref() {
            provider.load_from_string(&css);
        }
    });
}

/// Build the full color stylesheet for the given palette.
fn generate_css(
    s: &Scheme,
    accent: &str,
    image: Option<&Path>,
    dim: f64,
    panel_opacity: f64,
    font: &str,
) -> String {
    // Panel/chrome background alphas all scale by the same user-controlled
    // multiplier; interaction-feedback colors (hover/active/selected/borders)
    // are left alone so they stay legible at any opacity setting.
    let p = |base: f64| (base * panel_opacity).clamp(0.05, 1.0);
    let chrome_bg = p(0.72);
    let panel_bg = p(0.86);
    let preview_text_bg = p(0.55);
    let list_bg = p(0.82);
    let popover_bg = p(0.97);
    let statusbar_bg = p(0.50);
    // The window rule carries either a flat background or the dimmed image.
    let window_bg = match image {
        Some(path) => {
            let dim = dim.clamp(0.0, 1.0);
            let uri = gtk::glib::filename_to_uri(path, None)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| format!("file://{}", path.display()));
            format!(
                "background-color: {bg};\
                 background-image: linear-gradient(rgba(10,10,14,{dim}), rgba(10,10,14,{dim})), url(\"{uri}\");\
                 background-size: cover, cover;\
                 background-position: center, center;\
                 background-repeat: no-repeat, no-repeat;",
                bg = s.bg,
            )
        }
        None => format!("background-color: {};", s.bg),
    };

    // Translate the Pango font description (e.g. "JetBrains Mono 11") into CSS
    // family + size declarations, since GTK CSS does not accept the Pango form.
    let font_css = font_to_css(font);

    format!(
        "@define-color orca_bg {bg};\n\
         @define-color orca_surface {surface};\n\
         @define-color orca_text {text};\n\
         @define-color orca_subtext {subtext};\n\
         @define-color orca_accent {accent};\n\
         \n\
         window.orca-root {{ {window_bg} color: @orca_text; {font_css} }}\n\
         window.orca-root > box, window.orca-root > box > box,\n\
         .orca-home, .orca-home-scroll, .orca-home-scroll > viewport {{ background: transparent; }}\n\
         window.orca-root headerbar {{ background: alpha(@orca_surface, {chrome_bg}); color: @orca_text; border: none; box-shadow: none; min-height: 40px; }}\n\
         window.orca-root headerbar label {{ font-weight: 600; }}\n\
         .orca-places {{ background: alpha(@orca_bg, {panel_bg}); border-right: 1px solid alpha(@orca_text, 0.06); }}\n\
         .orca-preview {{ background: alpha(@orca_bg, {panel_bg}); border-left: 1px solid alpha(@orca_text, 0.06); }}\n\
         .orca-terminal {{ border-top: 1px solid alpha(@orca_text, 0.10); }}\n\
         .orca-preview-text {{ background: alpha(@orca_bg, {preview_text_bg}); }}\n\
         .orca-preview-text text {{ background: transparent; color: @orca_text; }}\n\
         .orca-place {{ color: alpha(@orca_text, 0.82); background: transparent; box-shadow: none; border: none; }}\n\
         .orca-place:hover {{ background: alpha(@orca_text, 0.08); color: @orca_text; }}\n\
         .orca-place:active {{ background: alpha(@orca_accent, 0.22); }}\n\
         .orca-place image {{ color: alpha(@orca_text, 0.70); }}\n\
         .orca-settings-nav {{ border-right: 1px solid alpha(@orca_text, 0.06); }}\n\
         .orca-settings-cat {{ color: alpha(@orca_text, 0.78); background: transparent; }}\n\
         .orca-settings-cat:hover {{ background: alpha(@orca_text, 0.08); color: @orca_text; }}\n\
         .orca-settings-cat:checked {{ background: alpha(@orca_accent, 0.18); color: @orca_text; }}\n\
         .orca-toolbar {{ background: alpha(@orca_surface, {chrome_bg}); }}\n\
         .orca-toolbar separator {{ background: alpha(@orca_text, 0.08); }}\n\
         .orca-toolbar button:hover {{ background: alpha(@orca_text, 0.08); }}\n\
         .orca-navbar {{ background: alpha(@orca_surface, {chrome_bg}); border-bottom: 1px solid alpha(@orca_text, 0.08); }}\n\
         .orca-crumb {{ color: alpha(@orca_text, 0.72); }}\n\
         .orca-crumb:hover {{ background: alpha(@orca_text, 0.08); color: @orca_text; }}\n\
         .orca-crumb-current {{ color: @orca_accent; font-weight: 600; }}\n\
         .orca-notebook > header.top {{ background: alpha(@orca_surface, {chrome_bg}); border-bottom: 1px solid alpha(@orca_text, 0.08); }}\n\
         .orca-notebook > header.top tab {{ background: transparent; color: alpha(@orca_text, 0.65); border: none; box-shadow: none; }}\n\
         .orca-notebook > header.top tab:hover {{ background: alpha(@orca_text, 0.06); color: @orca_text; }}\n\
         .orca-notebook > header.top tab:checked {{ background: alpha(@orca_text, 0.08); color: @orca_text; box-shadow: inset 0 -2px @orca_accent; }}\n\
         window.orca-root popover.background {{ background: alpha(@orca_surface, {popover_bg}); color: @orca_text; }}\n\
         .orca-statusbar {{ background: alpha(@orca_surface, {statusbar_bg}); border-top: 1px solid alpha(@orca_text, 0.06); }}\n\
         .orca-statusbar label {{ opacity: 0.85; }}\n\
         window.orca-root columnview, window.orca-root gridview, window.orca-root listview {{ background: alpha(@orca_bg, {list_bg}); color: @orca_text; }}\n\
         window.orca-root columnview > header button {{ background: alpha(@orca_surface, 0.60); }}\n\
         window.orca-root row:selected, window.orca-root :selected {{ background: alpha(@orca_accent, 0.30); color: @orca_text; }}\n\
         .orca-home-title {{ color: @orca_text; }}\n\
         .orca-section-label {{ color: alpha(@orca_text, 0.50); }}\n\
         .orca-card-grid > flowboxchild {{ background: transparent; }}\n\
         .orca-card {{ background: alpha(@orca_text, 0.05); border: 1px solid alpha(@orca_text, 0.08); box-shadow: none; }}\n\
         .orca-card:hover {{ background: alpha(@orca_text, 0.10); border-color: alpha(@orca_accent, 0.55); }}\n\
         .orca-card:active {{ background: alpha(@orca_text, 0.07); }}\n\
         .orca-card label {{ color: @orca_text; }}\n\
         .orca-card-subtitle {{ color: @orca_subtext; }}\n\
         .orca-card-icon {{ color: @orca_accent; }}\n\
         .orca-drive-bar trough {{ background: alpha(@orca_text, 0.10); }}\n\
         .orca-drive-bar progress {{ background: linear-gradient(90deg, @orca_accent, lighter(@orca_accent)); }}\n\
         .orca-note {{ color: @orca_subtext; }}\n\
         .orca-quicklook-scrim {{ background: alpha(black, 0.55); }}\n\
         .orca-quicklook-card {{ background: @orca_surface; color: @orca_text; box-shadow: 0 8px 32px alpha(black, 0.35); }}\n",
        bg = s.bg,
        surface = s.surface,
        text = s.text,
        subtext = s.subtext,
        accent = accent,
        font_css = font_css,
    )
}

/// Translate a Pango font description string into CSS `font-family` and
/// `font-size` declarations. An empty or unparsable description yields an empty
/// string, leaving the platform default font in place.
fn font_to_css(font: &str) -> String {
    let font = font.trim();
    if font.is_empty() {
        return String::new();
    }
    let desc = gtk::pango::FontDescription::from_string(font);
    let mut out = String::new();
    if let Some(family) = desc.family() {
        if !family.is_empty() {
            out.push_str(&format!("font-family: \"{family}\";"));
        }
    }
    let size = desc.size();
    if size > 0 {
        if desc.is_size_absolute() {
            out.push_str(&format!(" font-size: {}px;", size / gtk::pango::SCALE));
        } else {
            out.push_str(&format!(" font-size: {}pt;", size / gtk::pango::SCALE));
        }
    }
    out
}

fn add_provider_from_data(display: &gtk::gdk::Display, css: &str, priority: u32) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    gtk::style_context_add_provider_for_display(display, &provider, priority);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_lookup_falls_back_to_default() {
        assert_eq!(scheme("graphite").id, "graphite");
        assert_eq!(scheme("does-not-exist").id, SCHEMES[0].id);
    }

    #[test]
    fn generated_css_uses_accent_override() {
        let css = generate_css(&SCHEMES[0], "#ff0000", None, 0.4, 1.0, "JetBrains Mono 11");
        assert!(css.contains("@define-color orca_accent #ff0000;"));
        assert!(css.contains("window.orca-root"));
        assert!(css.contains("font-family: \"JetBrains Mono\";"));
        assert!(css.contains("font-size: 11pt;"));
    }

    #[test]
    fn generated_css_embeds_background_uri() {
        let css = generate_css(
            &SCHEMES[0],
            "#ff0000",
            Some(Path::new("/tmp/x.jpg")),
            0.5,
            1.0,
            "",
        );
        assert!(css.contains("background-image:"));
        assert!(css.contains("x.jpg"));
    }

    #[test]
    fn panel_opacity_scales_chrome_alpha_not_interaction_colors() {
        let full = generate_css(&SCHEMES[0], "#ff0000", None, 0.4, 1.0, "");
        let half = generate_css(&SCHEMES[0], "#ff0000", None, 0.4, 0.5, "");
        assert!(full.contains("alpha(@orca_surface, 0.72)"));
        assert!(half.contains("alpha(@orca_surface, 0.36)"));
        // Hover/selection feedback must not move with the opacity slider.
        assert!(full.contains("alpha(@orca_accent, 0.30)"));
        assert!(half.contains("alpha(@orca_accent, 0.30)"));
    }
}
