//! The settings page (Phase 7, brought forward).
//!
//! A dedicated content page (not a popover) exposing appearance controls — color
//! scheme, accent color, background image, background dim — and the interface
//! language. The page owns no configuration: each change is emitted as a
//! [`SettingsOutput`] so the application applies and persists it.

use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent};

use crate::config::Keybinds;
use crate::i18n::{self, Lang};
use crate::keybind::{Action, KeyCombo};
use crate::theme;

/// Initial values to seed the controls from the current config.
#[derive(Debug, Clone)]
pub struct SettingsInit {
    /// Active color scheme id.
    pub scheme: String,
    /// Accent override hex, or empty for the scheme default.
    pub accent: String,
    /// Background scrim strength, 0.0–1.0.
    pub dim: f64,
    /// UI / list font (Pango description).
    pub font: String,
    /// Show dot-prefixed hidden files.
    pub show_hidden: bool,
    /// Open items on a single click.
    pub single_click: bool,
    /// Confirm before moving files to the trash.
    pub confirm_delete: bool,
    /// Default view mode id for new panes (`"list"`, `"icon"`, `"detail"`).
    pub default_view: String,
    /// Interface language.
    pub language: Lang,
    /// Current keybind config.
    pub keybinds: Keybinds,
    /// Shell path for the embedded terminal (empty = `$SHELL`).
    pub terminal_shell: String,
    /// Font for the embedded terminal (Pango description).
    pub terminal_font: String,
}

/// The settings page emits no internal messages.
#[derive(Debug)]
pub enum SettingsInput {}

/// Changes requested from the settings page.
// Every variant is a "set X" command, so the shared `Set` prefix is intentional
// and reads naturally; renaming would only obscure intent.
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum SettingsOutput {
    /// Change the color scheme.
    SetScheme(String),
    /// Change the accent color (hex `#rrggbb`).
    SetAccent(String),
    /// Set or clear the background image.
    SetBackground(Option<PathBuf>),
    /// Change the background scrim strength.
    SetDim(f64),
    /// Change the UI / list font (Pango description).
    SetFont(String),
    /// Toggle hidden-file visibility.
    SetShowHidden(bool),
    /// Toggle single-click activation.
    SetSingleClick(bool),
    /// Toggle trash confirmation.
    SetConfirmDelete(bool),
    /// Change the default view mode id for new panes.
    SetDefaultView(String),
    /// Change the interface language.
    SetLanguage(Lang),
    /// Update one keybind: (config_key, new_binding_string).
    SetKeybind(String, String),
    /// Update the terminal shell path.
    SetTerminalShell(String),
    /// Update the terminal font.
    SetTerminalFont(String),
    /// Reset all settings to defaults.
    ResetDefaults,
}

/// The settings page component.
pub struct SettingsPage;

impl SimpleComponent for SettingsPage {
    type Init = SettingsInit;
    type Input = SettingsInput;
    type Output = SettingsOutput;
    type Root = gtk::ScrolledWindow;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .css_classes(["orca-home-scroll"])
            .build()
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .css_classes(["orca-home"])
            .build();

        content.append(
            &gtk::Label::builder()
                .label(i18n::t("settings.title"))
                .halign(gtk::Align::Start)
                .css_classes(["orca-home-title"])
                .build(),
        );

        // Appearance.
        content.append(&section_label(&i18n::t("home.appearance")));
        content.append(&scheme_row(&init, &sender));
        content.append(&accent_row(&init, &sender));
        content.append(&font_row(&init, &sender));
        content.append(&background_row(&sender));
        content.append(&dim_row(&init, &sender));

        // Behaviour.
        content.append(&section_label(&i18n::t("home.behaviour")));
        content.append(&switch_row(
            &i18n::t("set.show_hidden"),
            init.show_hidden,
            &sender,
            SettingsOutput::SetShowHidden,
        ));
        content.append(&switch_row(
            &i18n::t("set.single_click"),
            init.single_click,
            &sender,
            SettingsOutput::SetSingleClick,
        ));
        content.append(&switch_row(
            &i18n::t("set.confirm_delete"),
            init.confirm_delete,
            &sender,
            SettingsOutput::SetConfirmDelete,
        ));
        content.append(&default_view_row(&init, &sender));

        // Language.
        content.append(&section_label(&i18n::t("home.language")));
        content.append(&language_row(&init, &sender));
        content.append(
            &gtk::Label::builder()
                .label(i18n::t("home.lang_restart"))
                .wrap(true)
                .xalign(0.0)
                .css_classes(["dim-label", "orca-note"])
                .build(),
        );

        // Keybinds.
        content.append(&section_label(&i18n::t("set.keybinds")));
        content.append(&keybinds_grid(&init, &sender));

        // Terminal.
        content.append(&section_label(&i18n::t("set.terminal")));
        content.append(&terminal_shell_row(&init, &sender));
        content.append(&terminal_font_row(&init, &sender));

        // Reset.
        let reset_btn = gtk::Button::builder()
            .label(i18n::t("set.reset_defaults"))
            .halign(gtk::Align::Start)
            .css_classes(["destructive-action"])
            .build();
        {
            let sender = sender.clone();
            reset_btn.connect_clicked(move |_| {
                sender.output(SettingsOutput::ResetDefaults).ok();
            });
        }
        content.append(&reset_btn);

        root.set_child(Some(&content));
        ComponentParts {
            model: SettingsPage,
            widgets: (),
        }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {}
    }
}

/// A section heading label.
fn section_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .css_classes(["orca-section-label"])
        .build()
}

/// A row: a leading label plus a trailing, end-aligned control.
fn labelled_row(label: &str) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .css_classes(["orca-settings-row"])
        .build();
    row.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build(),
    );
    row
}

/// Color-scheme dropdown.
fn scheme_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("home.scheme"));
    let names: Vec<&str> = theme::SCHEMES.iter().map(|s| s.name).collect();
    let dropdown = gtk::DropDown::from_strings(&names);
    if let Some(idx) = theme::SCHEMES.iter().position(|s| s.id == init.scheme) {
        dropdown.set_selected(idx as u32);
    }
    let sender = sender.clone();
    dropdown.connect_selected_notify(move |d| {
        if let Some(s) = theme::SCHEMES.get(d.selected() as usize) {
            sender
                .output(SettingsOutput::SetScheme(s.id.to_owned()))
                .ok();
        }
    });
    row.append(&dropdown);
    row
}

/// Accent color button.
fn accent_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("home.accent"));
    let dialog = gtk::ColorDialog::builder().with_alpha(false).build();
    let button = gtk::ColorDialogButton::builder().dialog(&dialog).build();

    let seed = if init.accent.is_empty() {
        theme::scheme(&init.scheme).accent_hex()
    } else {
        init.accent.clone()
    };
    if let Ok(rgba) = gtk::gdk::RGBA::parse(&seed) {
        button.set_rgba(&rgba);
    }
    let sender = sender.clone();
    button.connect_rgba_notify(move |b| {
        sender
            .output(SettingsOutput::SetAccent(rgba_to_hex(&b.rgba())))
            .ok();
    });
    row.append(&button);
    row
}

/// A labelled toggle row that emits `make(active)` whenever flipped.
fn switch_row(
    label: &str,
    active: bool,
    sender: &ComponentSender<SettingsPage>,
    make: fn(bool) -> SettingsOutput,
) -> gtk::Box {
    let row = labelled_row(label);
    let toggle = gtk::Switch::builder()
        .active(active)
        .valign(gtk::Align::Center)
        .build();
    let sender = sender.clone();
    toggle.connect_active_notify(move |s| {
        sender.output(make(s.is_active())).ok();
    });
    row.append(&toggle);
    row
}

/// Default-view dropdown (List / Icon / Detail).
fn default_view_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("set.default_view"));
    let ids = ["list", "icon", "detail"];
    let names: Vec<String> = ids
        .iter()
        .map(|id| i18n::t(&format!("set.view_{id}")))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let dropdown = gtk::DropDown::from_strings(&refs);
    if let Some(idx) = ids.iter().position(|id| *id == init.default_view) {
        dropdown.set_selected(idx as u32);
    }
    let sender = sender.clone();
    dropdown.connect_selected_notify(move |d| {
        if let Some(id) = ids.get(d.selected() as usize) {
            sender
                .output(SettingsOutput::SetDefaultView((*id).to_owned()))
                .ok();
        }
    });
    row.append(&dropdown);
    row
}

/// Font picker button.
fn font_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("set.font"));
    let dialog = gtk::FontDialog::new();
    let button = gtk::FontDialogButton::builder().dialog(&dialog).build();
    if !init.font.is_empty() {
        button.set_font_desc(&gtk::pango::FontDescription::from_string(&init.font));
    }
    let sender = sender.clone();
    button.connect_font_desc_notify(move |b| {
        if let Some(desc) = b.font_desc() {
            sender
                .output(SettingsOutput::SetFont(desc.to_str().to_string()))
                .ok();
        }
    });
    row.append(&button);
    row
}

/// Background image pick / clear buttons.
fn background_row(sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("home.appearance"));

    let pick = gtk::Button::builder()
        .label(i18n::t("home.pick_background"))
        .build();
    {
        let sender = sender.clone();
        pick.connect_clicked(move |button| pick_background(button, &sender));
    }
    row.append(&pick);

    let clear = gtk::Button::builder()
        .icon_name("edit-delete-symbolic")
        .tooltip_text(i18n::t("home.clear_background"))
        .build();
    {
        let sender = sender.clone();
        clear.connect_clicked(move |_| {
            sender.output(SettingsOutput::SetBackground(None)).ok();
        });
    }
    row.append(&clear);
    row
}

/// Background dim slider.
fn dim_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("settings.dim"));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
    scale.set_value(init.dim);
    scale.set_hexpand(true);
    scale.set_size_request(180, -1);
    let sender = sender.clone();
    scale.connect_value_changed(move |s| {
        sender.output(SettingsOutput::SetDim(s.value())).ok();
    });
    row.append(&scale);
    row
}

/// Language radio buttons.
fn language_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .css_classes(["orca-settings-row"])
        .build();

    let tr = gtk::CheckButton::with_label("Türkçe");
    let en = gtk::CheckButton::with_label("English");
    en.set_group(Some(&tr));
    match init.language {
        Lang::Turkish => tr.set_active(true),
        Lang::English => en.set_active(true),
    }
    {
        let sender = sender.clone();
        tr.connect_toggled(move |b| {
            if b.is_active() {
                sender
                    .output(SettingsOutput::SetLanguage(Lang::Turkish))
                    .ok();
            }
        });
    }
    {
        let sender = sender.clone();
        en.connect_toggled(move |b| {
            if b.is_active() {
                sender
                    .output(SettingsOutput::SetLanguage(Lang::English))
                    .ok();
            }
        });
    }
    row.append(&tr);
    row.append(&en);
    row
}

/// Open an image file chooser and emit the chosen path.
fn pick_background(anchor: &gtk::Button, sender: &ComponentSender<SettingsPage>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    filter.add_mime_type("image/*");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let dialog = gtk::FileDialog::builder()
        .title(i18n::t("home.pick_background"))
        .filters(&filters)
        .modal(true)
        .build();

    let window = anchor.root().and_then(|r| r.downcast::<gtk::Window>().ok());
    let sender = sender.clone();
    dialog.open(
        window.as_ref(),
        gtk::gio::Cancellable::NONE,
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    sender
                        .output(SettingsOutput::SetBackground(Some(path)))
                        .ok();
                }
            }
        },
    );
}

/// Grid of Action | Current binding | "Click to rebind" button.
fn keybinds_grid(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(12)
        .build();

    for (row, action) in Action::ALL.iter().enumerate() {
        let action = *action;
        let label = gtk::Label::builder()
            .label(action.label())
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        grid.attach(&label, 0, row as i32, 1, 1);

        let binding_str = keybind_str_for(action, &init.keybinds);
        let binding_lbl = gtk::Label::builder()
            .label(&binding_str)
            .halign(gtk::Align::Start)
            .css_classes(["monospace"])
            .width_chars(18)
            .build();
        grid.attach(&binding_lbl, 1, row as i32, 1, 1);

        let btn = gtk::Button::builder().label(i18n::t("set.rebind")).build();
        {
            let sender = sender.clone();
            let lbl_ref = binding_lbl.clone();
            let config_key = action.config_key().to_owned();
            btn.connect_clicked(move |button| {
                capture_key(button, config_key.clone(), lbl_ref.clone(), &sender);
            });
        }
        grid.attach(&btn, 2, row as i32, 1, 1);
    }
    grid
}

/// Return the current binding string for `action` from the Keybinds config.
fn keybind_str_for(action: Action, kb: &Keybinds) -> String {
    match action {
        Action::NewTab => kb.new_tab.clone(),
        Action::CloseTab => kb.close_tab.clone(),
        Action::DuplicateTab => kb.duplicate_tab.clone(),
        Action::ToggleHidden => kb.toggle_hidden.clone(),
        Action::ToggleDualPane => kb.toggle_dual_pane.clone(),
        Action::OpenTerminal => kb.open_terminal.clone(),
        Action::CopyToPane => kb.copy_to_pane.clone(),
        Action::MoveToPane => kb.move_to_pane.clone(),
        Action::Rename => kb.rename.clone(),
        Action::Search => kb.search.clone(),
        Action::PathEntry => kb.path_entry.clone(),
        Action::Back => kb.back.clone(),
        Action::Forward => kb.forward.clone(),
        Action::Up => kb.up.clone(),
        Action::ViewList => kb.view_list.clone(),
        Action::ViewIcon => kb.view_icon.clone(),
        Action::ViewDetail => kb.view_detail.clone(),
        Action::VaultAdd => kb.vault_add.clone(),
    }
}

/// Show a "press a key" dialog, then emit SetKeybind with the result.
fn capture_key(
    anchor: &gtk::Button,
    config_key: String,
    binding_lbl: gtk::Label,
    sender: &ComponentSender<SettingsPage>,
) {
    let window = anchor.root().and_then(|r| r.downcast::<gtk::Window>().ok());

    let dialog = gtk::Window::builder()
        .title(i18n::t("set.press_key"))
        .modal(true)
        .default_width(280)
        .default_height(100)
        .build();
    if let Some(ref win) = window {
        dialog.set_transient_for(Some(win));
    }

    let lbl = gtk::Label::builder()
        .label(i18n::t("set.press_key"))
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    dialog.set_child(Some(&lbl));

    let sender = sender.clone();
    let dialog_ref = dialog.clone();
    let key_ctrl = gtk::EventControllerKey::new();
    key_ctrl.connect_key_pressed(move |_, keyval, _, state| {
        // Build a combo string from the event.
        let mut parts: Vec<&str> = Vec::new();
        if state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            parts.push("Ctrl");
        }
        if state.contains(gtk::gdk::ModifierType::ALT_MASK) {
            parts.push("Alt");
        }
        if state.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
            parts.push("Shift");
        }
        let combo_obj = KeyCombo {
            mods: state
                & (gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK),
            key: keyval,
        };
        let combo_str = combo_obj.to_display_string();
        if combo_str.is_empty() {
            return gtk::glib::Propagation::Proceed;
        }
        binding_lbl.set_label(&combo_str);
        sender
            .output(SettingsOutput::SetKeybind(config_key.clone(), combo_str))
            .ok();
        dialog_ref.close();
        gtk::glib::Propagation::Stop
    });
    dialog.add_controller(key_ctrl);
    dialog.present();
}

/// Terminal shell path entry row.
fn terminal_shell_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("set.terminal_shell"));
    let entry = gtk::Entry::builder()
        .text(&init.terminal_shell)
        .placeholder_text("$SHELL")
        .hexpand(true)
        .build();
    let sender = sender.clone();
    entry.connect_changed(move |e| {
        sender
            .output(SettingsOutput::SetTerminalShell(e.text().to_string()))
            .ok();
    });
    row.append(&entry);
    row
}

/// Terminal font picker row.
fn terminal_font_row(init: &SettingsInit, sender: &ComponentSender<SettingsPage>) -> gtk::Box {
    let row = labelled_row(&i18n::t("set.terminal_font"));
    let dialog = gtk::FontDialog::new();
    let button = gtk::FontDialogButton::builder().dialog(&dialog).build();
    if !init.terminal_font.is_empty() {
        button.set_font_desc(&gtk::pango::FontDescription::from_string(
            &init.terminal_font,
        ));
    }
    let sender = sender.clone();
    button.connect_font_desc_notify(move |b| {
        if let Some(desc) = b.font_desc() {
            sender
                .output(SettingsOutput::SetTerminalFont(desc.to_str().to_string()))
                .ok();
        }
    });
    row.append(&button);
    row
}

/// Format an opaque `RGBA` as `#rrggbb`.
fn rgba_to_hex(c: &gtk::gdk::RGBA) -> String {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        to_u8(c.red()),
        to_u8(c.green()),
        to_u8(c.blue())
    )
}
