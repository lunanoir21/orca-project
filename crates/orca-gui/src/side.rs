//! A tabbed pane container (Phase 4.2).
//!
//! [`TabbedPane`] wraps a `GtkNotebook` whose pages are independent
//! [`FilePane`] browsers. It manages tab lifecycle (new, close, switch, reorder,
//! duplicate), keeps each tab's label in sync with its directory, and bubbles up
//! only the *active* tab's [`PaneOutput`] so the application chrome tracks the
//! visible tab.

use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};

use orca_core::SortKey;

use crate::pane::{FilePane, PaneInit, PaneInput, PaneOutput, ViewMode};

/// One open tab: its stable id, the pane controller, its label widget, and the
/// directory it currently shows.
struct Tab {
    id: usize,
    controller: Controller<FilePane>,
    label: gtk::Label,
    dir: PathBuf,
}

/// Initialisation for a tabbed side. The pane defaults are applied to every tab
/// this container opens, so new tabs honour the user's configuration.
#[derive(Debug, Clone)]
pub struct SideInit {
    /// Directory the first tab opens on.
    pub dir: PathBuf,
    /// Whether new tabs show hidden files.
    pub show_hidden: bool,
    /// Whether new tabs activate entries on a single click.
    pub single_click: bool,
    /// Whether new tabs confirm before trashing.
    pub confirm_delete: bool,
    /// View mode new tabs open in.
    pub mode: ViewMode,
}

/// Messages the tabbed pane handles.
#[derive(Debug)]
pub enum SideInput {
    /// Route a pane message to the active tab.
    Forward(PaneInput),
    /// Apply hidden-file visibility to every tab (settings change).
    SetAllHidden(bool),
    /// Apply single-click activation to every tab (settings change).
    SetAllSingleClick(bool),
    /// Apply trash confirmation to every tab (settings change).
    SetAllConfirmDelete(bool),
    /// Open a new tab on the active tab's directory.
    NewTab,
    /// Close the active tab (no-op if it is the last one).
    CloseCurrent,
    /// Close a specific tab by id.
    CloseTab(usize),
    /// Switch to the next / previous tab.
    Next,
    /// Switch to the previous tab.
    Prev,
    /// Duplicate the active tab.
    Duplicate,
    /// The notebook switched to the page at the given index.
    Switched(u32),
    /// An event from the tab with the given id.
    TabEvent(usize, PaneOutput),
}

/// Messages the tabbed pane emits.
#[derive(Debug)]
pub enum SideOutput {
    /// An event from the active tab, to drive the application chrome.
    Event(PaneOutput),
}

/// A notebook of file-browser tabs.
pub struct TabbedPane {
    notebook: gtk::Notebook,
    tabs: Vec<Tab>,
    /// Id of the active tab.
    active: usize,
    /// Monotonic id source for tabs.
    next_id: usize,
    /// Hidden-file default applied to new tabs.
    show_hidden: bool,
    /// Single-click default applied to new tabs.
    single_click: bool,
    /// Trash-confirmation default applied to new tabs.
    confirm_delete: bool,
    /// View-mode default applied to new tabs.
    mode: ViewMode,
}

#[relm4::component(pub)]
impl Component for TabbedPane {
    type Init = SideInit;
    type Input = SideInput;
    type Output = SideOutput;
    type CommandOutput = ();

    view! {
        #[root]
        #[name = "notebook"]
        gtk::Notebook {
            set_scrollable: true,
            set_show_border: false,
            connect_switch_page[sender] => move |_, _, page| {
                sender.input(SideInput::Switched(page));
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();
        let mut model = TabbedPane {
            notebook: widgets.notebook.clone(),
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            show_hidden: init.show_hidden,
            single_click: init.single_click,
            confirm_delete: init.confirm_delete,
            mode: init.mode,
        };
        model.open_tab(init.dir, &sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SideInput::Forward(msg) => {
                if let Some(tab) = self.active_tab() {
                    tab.controller.emit(msg);
                }
            }
            SideInput::SetAllHidden(v) => {
                self.show_hidden = v;
                for tab in &self.tabs {
                    tab.controller.emit(PaneInput::SetHidden(v));
                }
            }
            SideInput::SetAllSingleClick(v) => {
                self.single_click = v;
                for tab in &self.tabs {
                    tab.controller.emit(PaneInput::SetSingleClick(v));
                }
            }
            SideInput::SetAllConfirmDelete(v) => {
                self.confirm_delete = v;
                for tab in &self.tabs {
                    tab.controller.emit(PaneInput::SetConfirmDelete(v));
                }
            }
            SideInput::NewTab => {
                let dir = self.active_dir();
                self.open_tab(dir, &sender);
            }
            SideInput::Duplicate => {
                let dir = self.active_dir();
                self.open_tab(dir, &sender);
            }
            SideInput::CloseCurrent => {
                let id = self.active;
                self.close_tab(id);
            }
            SideInput::CloseTab(id) => self.close_tab(id),
            SideInput::Next => {
                self.notebook.next_page();
            }
            SideInput::Prev => {
                self.notebook.prev_page();
            }
            SideInput::Switched(page) => {
                if let Some(tab) = self.tabs.get(page as usize) {
                    self.active = tab.id;
                    // Resync the chrome to the newly visible tab.
                    tab.controller.emit(PaneInput::Announce);
                }
            }
            SideInput::TabEvent(id, out) => {
                // Keep the originating tab's label and cached dir current.
                if let PaneOutput::DirChanged(path) = &out {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
                        tab.dir = path.clone();
                        tab.label.set_text(&tab_title(path));
                    }
                }
                // Only the active tab drives the application chrome.
                if id == self.active {
                    sender.output(SideOutput::Event(out)).ok();
                }
            }
        }
    }
}

impl TabbedPane {
    /// The active tab, if any.
    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.active)
    }

    /// The active tab's directory (or `/` as a fallback).
    fn active_dir(&self) -> PathBuf {
        self.active_tab()
            .map(|t| t.dir.clone())
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Open a new tab on `dir` and switch to it.
    fn open_tab(&mut self, dir: PathBuf, sender: &ComponentSender<Self>) {
        let id = self.next_id;
        self.next_id += 1;

        let controller = FilePane::builder()
            .launch(PaneInit {
                dir: dir.clone(),
                show_hidden: self.show_hidden,
                single_click: self.single_click,
                confirm_delete: self.confirm_delete,
                sort: SortKey::Name,
                mode: self.mode,
            })
            .forward(sender.input_sender(), move |out| {
                SideInput::TabEvent(id, out)
            });

        let label = gtk::Label::new(Some(&tab_title(&dir)));
        let tab_header = build_tab_header(id, &label, sender);

        let page = self
            .notebook
            .append_page(controller.widget(), Some(&tab_header));
        self.notebook.set_tab_reorderable(controller.widget(), true);

        self.tabs.push(Tab {
            id,
            controller,
            label,
            dir,
        });
        self.active = id;
        self.notebook.set_current_page(Some(page));
    }

    /// Close the tab with the given id, unless it is the last remaining tab.
    fn close_tab(&mut self, id: usize) {
        if self.tabs.len() <= 1 {
            return;
        }
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.notebook.remove_page(Some(pos as u32));
        self.tabs.remove(pos);
        // The notebook will emit switch-page, updating `active`; ensure it is
        // valid in the meantime.
        if self.active == id {
            let fallback = pos.min(self.tabs.len().saturating_sub(1));
            if let Some(tab) = self.tabs.get(fallback) {
                self.active = tab.id;
            }
        }
    }
}

/// Build a tab header widget: a directory label plus a close button.
fn build_tab_header(
    id: usize,
    label: &gtk::Label,
    sender: &ComponentSender<TabbedPane>,
) -> gtk::Box {
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(18);
    header.append(label);

    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .has_frame(false)
        .build();
    let sender = sender.clone();
    close.connect_clicked(move |_| sender.input(SideInput::CloseTab(id)));
    header.append(&close);
    header
}

/// The tab label for a directory: its final component, or the whole path.
fn tab_title(path: &std::path::Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}
