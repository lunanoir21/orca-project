//! Plugin manager dialog (Phase 8.5).
//!
//! Lists all discovered plugins with their state (enabled/disabled/error),
//! provides per-plugin enable/disable toggles and a reload button, and shows
//! per-plugin log output in an expander.

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_plugin::{PluginInfo, PluginState};

use crate::i18n;

/// Input: snapshot list from the live [`orca_plugin::PluginManager`].
pub type PluginManagerInit = Vec<PluginInfo>;

/// Dialog model. Keeps the list box around so [`PluginManagerInput::Refresh`]
/// can rebuild rows in place after a toggle/reload changes backend state.
pub struct PluginManagerDialog {
    list_box: Option<gtk::ListBox>,
}

/// Messages the dialog handles.
#[derive(Debug)]
pub enum PluginManagerInput {
    /// User pressed "Open plugins folder".
    OpenFolder,
    /// User flipped a plugin's enable switch.
    Toggle(String, bool),
    /// User pressed the reload button on a plugin row.
    Reload(String),
    /// The shell pushed a fresh snapshot after handling a `Toggle`/`Reload`
    /// output (the dialog has no direct access to the live `PluginManager`).
    Refresh(Vec<PluginInfo>),
}

/// What the dialog reports to the application shell, which owns the live
/// `PluginManager` and is the only thing allowed to mutate it.
#[derive(Debug)]
pub enum PluginManagerOutput {
    /// Enable (`true`) or disable (`false`) the named plugin.
    Toggle(String, bool),
    /// Hot-reload the named plugin.
    Reload(String),
}

#[relm4::component(pub)]
impl Component for PluginManagerDialog {
    type Init = PluginManagerInit;
    type Input = PluginManagerInput;
    type Output = PluginManagerOutput;
    type CommandOutput = ();

    view! {
        #[root]
        gtk::Window {
            set_title: Some(&i18n::t("plugin.title")),
            set_modal: true,
            set_default_width: 560,
            set_default_height: 420,
        }
    }

    fn init(
        plugins: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();

        let mut list_box = None;
        if plugins.is_empty() {
            let lbl = gtk::Label::builder()
                .label(i18n::t("plugin.no_plugins"))
                .vexpand(true)
                .valign(gtk::Align::Center)
                .build();
            lbl.add_css_class("dim-label");
            outer.append(&lbl);
        } else {
            let scroll = gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();
            let lb = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .build();
            lb.add_css_class("boxed-list");

            for info in &plugins {
                lb.append(&build_plugin_row(info, &sender));
            }

            scroll.set_child(Some(&lb));
            outer.append(&scroll);
            list_box = Some(lb);
        }

        // Bottom button row
        let btn_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .build();

        let open_btn = gtk::Button::with_label(&i18n::t("plugin.open_folder"));
        {
            let s = sender.clone();
            open_btn.connect_clicked(move |_| s.input(PluginManagerInput::OpenFolder));
        }

        let close_btn = gtk::Button::with_label(&i18n::t("common.close"));
        {
            let root = root.clone();
            close_btn.connect_clicked(move |_| root.close());
        }
        close_btn.add_css_class("suggested-action");

        btn_row.append(&open_btn);
        btn_row.append(&close_btn);
        outer.append(&btn_row);

        root.set_child(Some(&outer));

        let model = PluginManagerDialog { list_box };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            PluginManagerInput::OpenFolder => {
                let plugins_dir = dirs::data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("orca/plugins");
                let _ = std::process::Command::new("xdg-open")
                    .arg(&plugins_dir)
                    .spawn();
            }
            PluginManagerInput::Toggle(id, on) => {
                sender.output(PluginManagerOutput::Toggle(id, on)).ok();
            }
            PluginManagerInput::Reload(id) => {
                sender.output(PluginManagerOutput::Reload(id)).ok();
            }
            PluginManagerInput::Refresh(plugins) => {
                let Some(list_box) = &self.list_box else {
                    return;
                };
                while let Some(child) = list_box.first_child() {
                    list_box.remove(&child);
                }
                for info in &plugins {
                    list_box.append(&build_plugin_row(info, &sender));
                }
            }
        }
    }
}

/// Build a single plugin row widget.
fn build_plugin_row(
    info: &PluginInfo,
    sender: &ComponentSender<PluginManagerDialog>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);

    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();

    // Header: name + version + reload button + enable switch + state badge
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let name_lbl = gtk::Label::builder()
        .label(format!("{} v{}", info.name, info.version))
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    name_lbl.add_css_class("heading");
    header.append(&name_lbl);

    let reload_btn = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .has_frame(false)
        .tooltip_text(i18n::t("plugin.reload"))
        .build();
    {
        let s = sender.clone();
        let id = info.id.clone();
        reload_btn.connect_clicked(move |_| s.input(PluginManagerInput::Reload(id.clone())));
    }
    header.append(&reload_btn);

    let enabled_switch = gtk::Switch::builder()
        .active(info.state == PluginState::Enabled)
        .valign(gtk::Align::Center)
        .tooltip_text(i18n::t("plugin.enabled"))
        .build();
    // A plugin stuck in an error state can't be flipped back to enabled from
    // here — it needs a successful reload first.
    enabled_switch.set_sensitive(!matches!(info.state, PluginState::Error(_)));
    {
        let s = sender.clone();
        let id = info.id.clone();
        enabled_switch.connect_state_set(move |_, on| {
            s.input(PluginManagerInput::Toggle(id.clone(), on));
            gtk::glib::Propagation::Proceed
        });
    }
    header.append(&enabled_switch);

    let state_lbl = gtk::Label::builder()
        .label(state_label(&info.state))
        .halign(gtk::Align::End)
        .build();
    state_lbl.add_css_class(state_css_class(&info.state));
    header.append(&state_lbl);

    vbox.append(&header);

    // Description + author
    let desc = gtk::Label::builder()
        .label(format!("{} — {}", info.description, info.author))
        .halign(gtk::Align::Start)
        .wrap(true)
        .build();
    desc.add_css_class("dim-label");
    vbox.append(&desc);

    // Log expander (only shown if there's something to show)
    if !info.log.is_empty() {
        let exp = gtk::Expander::builder()
            .label(i18n::t("plugin.log"))
            .build();
        let log_view = gtk::TextView::builder()
            .editable(false)
            .monospace(true)
            .build();
        log_view.buffer().set_text(&info.log.join("\n"));
        let log_scroll = gtk::ScrolledWindow::builder()
            .height_request(80)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        log_scroll.set_child(Some(&log_view));
        exp.set_child(Some(&log_scroll));
        vbox.append(&exp);
    }

    // Error message (if in error state)
    if let PluginState::Error(msg) = &info.state {
        let err_lbl = gtk::Label::builder()
            .label(msg.as_str())
            .halign(gtk::Align::Start)
            .wrap(true)
            .build();
        err_lbl.add_css_class("error");
        vbox.append(&err_lbl);
    }

    row.set_child(Some(&vbox));
    row
}

fn state_label(state: &PluginState) -> &'static str {
    match state {
        PluginState::Enabled => "●",
        PluginState::Disabled => "○",
        PluginState::Error(_) => "✗",
    }
}

fn state_css_class(state: &PluginState) -> &'static str {
    match state {
        PluginState::Enabled => "success",
        PluginState::Disabled => "dim-label",
        PluginState::Error(_) => "error",
    }
}
