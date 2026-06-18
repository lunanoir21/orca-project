//! Navigation bar (Phase 3.4): a clickable breadcrumb of the current path that
//! can flip to an editable path entry (Ctrl+L), navigating on Enter.
//!
//! The breadcrumb is rebuilt imperatively on every path change because its
//! segment count is dynamic; the bar swaps between the breadcrumb and the entry
//! using an internal `GtkStack`.

use std::path::{Path, PathBuf};

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent};

/// The navigation bar component.
pub struct NavBar {
    /// The path currently displayed.
    path: PathBuf,
    /// Whether the editable entry (rather than the breadcrumb) is shown.
    editing: bool,
    /// The breadcrumb container, rebuilt on each path change.
    crumbs: gtk::Box,
    /// The editable path entry.
    entry: gtk::Entry,
    /// The stack switching between breadcrumb and entry.
    stack: gtk::Stack,
}

/// Messages the navigation bar handles.
#[derive(Debug)]
pub enum NavInput {
    /// Display a new current path (from the pane).
    SetPath(PathBuf),
    /// Toggle between breadcrumb and editable entry (Ctrl+L).
    ToggleEntry,
    /// Show the breadcrumb (e.g. Escape in the entry).
    ShowBreadcrumb,
    /// A breadcrumb segment was clicked.
    Crumb(PathBuf),
    /// The path entry was activated (Enter).
    EntryActivated,
}

/// Messages the navigation bar emits.
#[derive(Debug)]
pub enum NavOutput {
    /// The user chose to navigate to the given path.
    Navigate(PathBuf),
}

#[relm4::component(pub)]
impl SimpleComponent for NavBar {
    type Init = PathBuf;
    type Input = NavInput;
    type Output = NavOutput;

    view! {
        #[root]
        #[name = "stack"]
        gtk::Stack {
            add_css_class: "orca-navbar",
        }
    }

    fn init(
        path: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let crumbs = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(0)
            .build();
        let crumb_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&crumbs)
            .build();

        let entry = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text("Type a path and press Enter")
            .build();
        {
            let sender = sender.clone();
            entry.connect_activate(move |_| sender.input(NavInput::EntryActivated));
        }
        {
            let sender = sender.clone();
            let key = gtk::EventControllerKey::new();
            key.connect_key_pressed(move |_, keyval, _, _| {
                if keyval == gtk::gdk::Key::Escape {
                    sender.input(NavInput::ShowBreadcrumb);
                    gtk::glib::Propagation::Stop
                } else {
                    gtk::glib::Propagation::Proceed
                }
            });
            entry.add_controller(key);
        }

        let widgets = view_output!();
        widgets.stack.add_named(&crumb_scroll, Some("crumbs"));
        widgets.stack.add_named(&entry, Some("entry"));
        widgets.stack.set_visible_child_name("crumbs");

        let model = NavBar {
            path: path.clone(),
            editing: false,
            crumbs,
            entry,
            stack: widgets.stack.clone(),
        };
        model.rebuild_crumbs(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            NavInput::SetPath(path) => {
                self.path = path;
                self.editing = false;
                self.rebuild_crumbs(&sender);
                self.stack.set_visible_child_name("crumbs");
            }
            NavInput::ToggleEntry => {
                if self.editing {
                    self.editing = false;
                    self.stack.set_visible_child_name("crumbs");
                } else {
                    self.editing = true;
                    self.entry.set_text(&self.path.to_string_lossy());
                    self.stack.set_visible_child_name("entry");
                    self.entry.grab_focus();
                    self.entry.set_position(-1);
                }
            }
            NavInput::ShowBreadcrumb => {
                self.editing = false;
                self.stack.set_visible_child_name("crumbs");
            }
            NavInput::Crumb(path) => {
                sender.output(NavOutput::Navigate(path)).ok();
            }
            NavInput::EntryActivated => {
                let text = self.entry.text();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    sender
                        .output(NavOutput::Navigate(expand_path(trimmed)))
                        .ok();
                }
                self.editing = false;
                self.stack.set_visible_child_name("crumbs");
            }
        }
    }
}

impl NavBar {
    /// Tear down and rebuild the breadcrumb buttons for the current path.
    fn rebuild_crumbs(&self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.crumbs.first_child() {
            self.crumbs.remove(&child);
        }
        for (label, target) in crumb_segments(&self.path) {
            let button = gtk::Button::builder()
                .label(&label)
                .has_frame(false)
                .css_classes(["orca-crumb"])
                .build();
            let sender = sender.clone();
            button.connect_clicked(move |_| {
                sender.input(NavInput::Crumb(target.clone()));
            });
            self.crumbs.append(&button);
        }
    }
}

/// Expand a leading `~` to the user's home directory.
fn expand_path(input: &str) -> PathBuf {
    if let Some(rest) = input.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if input == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(input)
}

/// Build `(label, target_path)` pairs from filesystem root down to `path`.
fn crumb_segments(path: &Path) -> Vec<(String, PathBuf)> {
    let mut acc: Vec<(String, PathBuf)> = Vec::new();
    let mut current = PathBuf::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::RootDir => {
                current.push("/");
                acc.push(("/".to_owned(), current.clone()));
            }
            Component::Normal(name) => {
                current.push(name);
                acc.push((name.to_string_lossy().into_owned(), current.clone()));
            }
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                acc.push((
                    prefix.as_os_str().to_string_lossy().into_owned(),
                    current.clone(),
                ));
            }
            // `.` and `..` are not expected in a canonical current directory.
            Component::CurDir | Component::ParentDir => {}
        }
    }
    if acc.is_empty() {
        acc.push((path.to_string_lossy().into_owned(), path.to_path_buf()));
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_build_cumulative_paths() {
        let seg = crumb_segments(Path::new("/home/user/docs"));
        assert_eq!(seg.len(), 4);
        assert_eq!(seg[0], ("/".to_owned(), PathBuf::from("/")));
        assert_eq!(seg[1].1, PathBuf::from("/home"));
        assert_eq!(seg[2].1, PathBuf::from("/home/user"));
        assert_eq!(
            seg[3],
            ("docs".to_owned(), PathBuf::from("/home/user/docs"))
        );
    }

    #[test]
    fn expand_tilde() {
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand_path("~/x"), home.join("x"));
            assert_eq!(expand_path("~"), home);
        }
        assert_eq!(expand_path("/etc/hosts"), PathBuf::from("/etc/hosts"));
    }
}
