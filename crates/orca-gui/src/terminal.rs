//! The embedded terminal panel (Phase 4.7).
//!
//! A VTE terminal hosted in a collapsible bottom panel. The terminal runs the
//! user's login shell; when the panel is shown and the active pane changes
//! directory, the shell is sent a `cd` so the terminal tracks the browser. If
//! the shell exits it is respawned, so the panel always holds a live shell.
//!
//! This panel shells out to an interactive process by design. It does not, and
//! must not, touch vault state — it is an ordinary terminal scoped to whatever
//! the user types, exactly like a standalone terminal emulator.

use std::path::{Path, PathBuf};

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent};
use vte4::{PtyFlags, Terminal, TerminalExt, TerminalExtManual};

/// Lines of scrollback kept in the terminal buffer.
const SCROLLBACK_LINES: i64 = 10_000;

/// Messages the terminal panel handles.
#[derive(Debug)]
pub enum TerminalInput {
    /// Change the shell's working directory to `path` (sent as a `cd`).
    Cd(PathBuf),
    /// Respawn the shell (used when the previous one exits).
    Respawn,
}

/// The embedded terminal panel component.
pub struct TerminalPanel {
    /// The VTE terminal widget.
    term: Terminal,
    /// The directory the shell was last told to switch to, reused on respawn.
    dir: PathBuf,
}

impl SimpleComponent for TerminalPanel {
    type Init = PathBuf;
    type Input = TerminalInput;
    type Output = ();
    type Root = gtk::ScrolledWindow;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::ScrolledWindow::builder()
            .height_request(220)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["orca-terminal"])
            .build()
    }

    fn init(
        dir: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let term = Terminal::new();
        term.set_vexpand(true);
        term.set_hexpand(true);
        term.set_scrollback_lines(SCROLLBACK_LINES);
        root.set_child(Some(&term));

        // A shell that exits (Ctrl-D, `exit`) should not leave a dead panel.
        let respawn = sender.clone();
        term.connect_child_exited(move |_, _| respawn.input(TerminalInput::Respawn));

        spawn_shell(&term, &dir);
        ComponentParts {
            model: TerminalPanel { term, dir },
            widgets: (),
        }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            TerminalInput::Cd(path) => {
                self.dir = path;
                let line = format!("cd {}\n", shell_quote(&self.dir));
                self.term.feed_child(line.as_bytes());
            }
            TerminalInput::Respawn => spawn_shell(&self.term, &self.dir),
        }
    }
}

/// Spawn the user's login shell in `dir`. Failures are logged, not surfaced:
/// an empty terminal is a clear-enough signal and there is nothing actionable.
fn spawn_shell(term: &Terminal, dir: &Path) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
    let cwd = dir.to_str();
    term.spawn_async(
        PtyFlags::DEFAULT,
        cwd,
        &[&shell],
        &[],
        gtk::glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        gtk::gio::Cancellable::NONE,
        |result| {
            if let Err(e) = result {
                tracing::warn!(error = %e, "failed to spawn terminal shell");
            }
        },
    );
}

/// Quote a path for safe use as a single shell argument (single-quote wrapping
/// with `'\''` escaping), so directories with spaces or quotes `cd` correctly.
fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_plain_path() {
        assert_eq!(shell_quote(Path::new("/home/u/docs")), "'/home/u/docs'");
    }

    #[test]
    fn escapes_embedded_quote() {
        assert_eq!(shell_quote(Path::new("/a'b")), "'/a'\\''b'");
    }

    #[test]
    fn quotes_path_with_spaces() {
        assert_eq!(shell_quote(Path::new("/my dir")), "'/my dir'");
    }
}
