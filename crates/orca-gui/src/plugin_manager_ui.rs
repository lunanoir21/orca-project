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

/// Dialog model.
pub struct PluginManagerDialog;

/// Messages the dialog handles.
#[derive(Debug)]
pub enum PluginManagerInput {
    /// User pressed "Open plugins folder".
    OpenFolder,
}

#[relm4::component(pub)]
impl Component for PluginManagerDialog {
    type Init = PluginManagerInit;
    type Input = PluginManagerInput;
    type Output = ();
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
            let list_box = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .build();
            list_box.add_css_class("boxed-list");

            for info in &plugins {
                list_box.append(&build_plugin_row(info));
            }

            scroll.set_child(Some(&list_box));
            outer.append(&scroll);
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

        let model = PluginManagerDialog;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            PluginManagerInput::OpenFolder => {
                let plugins_dir = dirs::data_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("orca/plugins");
                let _ = std::process::Command::new("xdg-open")
                    .arg(&plugins_dir)
                    .spawn();
            }
        }
    }
}

/// Build a single plugin row widget.
fn build_plugin_row(info: &PluginInfo) -> gtk::ListBoxRow {
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

    // Header: name + version + state badge
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
