//! Network Locations UI (Phase 5.8).
//!
//! A dialog listing saved SFTP/FTP connections. Users can add/remove connections,
//! connect to them (password retrieved from the OS keyring or prompted), and
//! browse the remote directory tree. Files can be downloaded to `~/Downloads`
//! with a progress indicator.

use std::path::{Path, PathBuf};

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_core::{FtpClient, Protocol, RemoteEntry, SavedConnection, SftpAuth, SftpClient};

use crate::format;
use crate::i18n;

/// An established remote session.
#[derive(Clone)]
pub(crate) enum ActiveSession {
    Sftp(SftpClient),
    Ftp(FtpClient),
}

impl std::fmt::Debug for ActiveSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sftp(_) => f.debug_tuple("Sftp").finish(),
            Self::Ftp(_) => f.debug_tuple("Ftp").finish(),
        }
    }
}

impl ActiveSession {
    async fn list(&self, path: &Path) -> orca_core::Result<Vec<RemoteEntry>> {
        match self {
            Self::Sftp(c) => c.list(path).await,
            Self::Ftp(c) => c.list(path).await,
        }
    }

    async fn download(&self, remote: &Path, local: &Path) -> orca_core::Result<()> {
        match self {
            Self::Sftp(c) => c.download(remote, local).await,
            Self::Ftp(c) => c.download(remote, local).await,
        }
    }
}

/// Network browser model.
pub struct NetworkBrowser {
    connections: Vec<SavedConnection>,
    session: Option<ActiveSession>,
    connected_name: Option<String>,
    current_path: PathBuf,
    entries: Vec<RemoteEntry>,
    /// Connection list widget — rebuilt when connections change.
    conn_list: gtk::ListBox,
    /// Remote file list widget — rebuilt after each directory listing.
    file_list: gtk::ListBox,
    /// Label showing the current remote path.
    path_label: gtk::Label,
    /// Status / error label below the file list.
    status_label: gtk::Label,
    /// Revealer that shows/hides the progress bar.
    progress_rev: gtk::Revealer,
    /// Progress bar pulsed during async ops.
    // Pulsed via the revealer animation; direct field reads not needed.
    #[allow(dead_code)]
    progress_bar: gtk::ProgressBar,
    /// Disconnect button (sensitive only when connected).
    disconnect_btn: gtk::Button,
    /// Up-directory button (sensitive only when connected).
    up_btn: gtk::Button,
    /// Stack toggling placeholder vs. file list.
    file_stack: gtk::Stack,
}

#[derive(Debug)]
pub enum NetworkBrowserInput {
    /// Re-read saved connections from disk and refresh the list.
    Reload,
    /// Initiate a connection: look up stored password first.
    ConnectSaved(SavedConnection),
    /// Perform the actual connect with `password`.
    DoConnect(SavedConnection, String),
    /// Disconnect the active session.
    Disconnect,
    /// List a remote directory.
    ListDir(PathBuf),
    /// Navigate to the parent of the current remote path.
    NavigateUp,
    /// Download a remote file to `~/Downloads`.
    Download(RemoteEntry),
    /// Persist a new/updated connection; optionally store its password.
    SaveConnection(SavedConnection, String, bool),
    /// Remove a saved connection by name.
    RemoveConnection(String),
}

#[derive(Debug)]
pub enum NetworkBrowserCmd {
    /// Keyring result (password if found, `None` if not stored).
    GotPassword(SavedConnection, Option<String>),
    /// Connection attempt result.
    Connected(SavedConnection, Result<ActiveSession, String>),
    /// Directory listing result.
    Listed(PathBuf, Result<Vec<RemoteEntry>, String>),
    /// Download result: (filename, outcome).
    Downloaded(String, Result<(), String>),
}

#[relm4::component(pub)]
impl Component for NetworkBrowser {
    type Init = ();
    type Input = NetworkBrowserInput;
    type Output = ();
    type CommandOutput = NetworkBrowserCmd;

    view! {
        #[root]
        gtk::Window {
            set_title: Some(&i18n::t("net.title")),
            set_default_width: 780,
            set_default_height: 520,
            set_modal: true,
        }
    }

    fn init(
        _: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .build();

        // Paned: left = saved connections, right = remote file tree
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .position(230)
            .vexpand(true)
            .build();

        // ── Left panel ────────────────────────────────────────────────────────
        let left = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(8)
            .margin_end(4)
            .build();

        left.append(
            &gtk::Label::builder()
                .label(i18n::t("net.connections"))
                .halign(gtk::Align::Start)
                .css_classes(["orca-section-label"])
                .build(),
        );

        let conn_list = gtk::ListBox::new();
        conn_list.set_selection_mode(gtk::SelectionMode::None);
        conn_list.add_css_class("boxed-list");
        let conn_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&conn_list)
            .build();
        left.append(&conn_scroll);

        let add_btn = gtk::Button::with_label(&i18n::t("net.add"));
        add_btn.add_css_class("suggested-action");
        {
            let s = sender.clone();
            let r = root.clone();
            add_btn.connect_clicked(move |_| show_add_dialog(&r, &s));
        }
        left.append(&add_btn);

        paned.set_start_child(Some(&left));

        // ── Right panel ───────────────────────────────────────────────────────
        let right = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(4)
            .margin_end(8)
            .build();

        // Path bar
        let path_bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .build();

        let up_btn = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text(i18n::t("net.up"))
            .sensitive(false)
            .build();
        {
            let s = sender.clone();
            up_btn.connect_clicked(move |_| s.input(NetworkBrowserInput::NavigateUp));
        }

        let path_label = gtk::Label::builder()
            .label("—")
            .halign(gtk::Align::Start)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .build();

        let disconnect_btn = gtk::Button::with_label(&i18n::t("net.disconnect"));
        disconnect_btn.set_sensitive(false);
        {
            let s = sender.clone();
            disconnect_btn.connect_clicked(move |_| s.input(NetworkBrowserInput::Disconnect));
        }

        path_bar.append(&up_btn);
        path_bar.append(&path_label);
        path_bar.append(&disconnect_btn);
        right.append(&path_bar);

        // File stack: placeholder or scrolled file list
        let file_stack = gtk::Stack::new();

        let placeholder = gtk::Label::builder()
            .label(i18n::t("net.not_connected"))
            .valign(gtk::Align::Center)
            .css_classes(["dim-label"])
            .build();
        file_stack.add_named(&placeholder, Some("placeholder"));

        let file_list = gtk::ListBox::new();
        file_list.set_selection_mode(gtk::SelectionMode::None);
        file_list.add_css_class("boxed-list");
        let file_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&file_list)
            .build();
        file_stack.add_named(&file_scroll, Some("files"));
        file_stack.set_visible_child_name("placeholder");

        right.append(&file_stack);

        // Progress bar (hidden until an async op is running)
        let progress_bar = gtk::ProgressBar::new();
        progress_bar.set_pulse_step(0.1);
        let progress_rev = gtk::Revealer::builder()
            .child(&progress_bar)
            .reveal_child(false)
            .transition_duration(150)
            .build();
        right.append(&progress_rev);

        // Status / error label
        let status_label = gtk::Label::builder()
            .label("")
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build();
        right.append(&status_label);

        paned.set_end_child(Some(&right));
        outer.append(&paned);

        // Bottom close button
        let close_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .halign(gtk::Align::End)
            .margin_bottom(8)
            .margin_end(8)
            .spacing(0)
            .build();
        let close_btn = gtk::Button::with_label(&i18n::t("common.close"));
        {
            let r = root.clone();
            close_btn.connect_clicked(move |_| r.close());
        }
        close_row.append(&close_btn);
        outer.append(&close_row);

        root.set_child(Some(&outer));

        let connections = orca_core::load_connections().unwrap_or_default();

        let mut model = NetworkBrowser {
            connections,
            session: None,
            connected_name: None,
            current_path: PathBuf::from("/"),
            entries: Vec::new(),
            conn_list,
            file_list,
            path_label,
            status_label,
            progress_rev,
            progress_bar,
            disconnect_btn,
            up_btn,
            file_stack,
        };
        model.rebuild_conn_list(&sender);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            NetworkBrowserInput::Reload => {
                self.connections = orca_core::load_connections().unwrap_or_default();
                self.rebuild_conn_list(&sender);
            }
            NetworkBrowserInput::ConnectSaved(conn) => {
                let name = conn.name.clone();
                let user = conn.username.clone();
                sender.oneshot_command(async move {
                    let pw = orca_core::net::credentials::get_password(&name, &user)
                        .await
                        .unwrap_or(None);
                    NetworkBrowserCmd::GotPassword(conn, pw)
                });
            }
            NetworkBrowserInput::DoConnect(conn, password) => {
                self.status_label.set_label(&i18n::t("net.connecting"));
                self.progress_rev.set_reveal_child(true);
                let host = conn.host.clone();
                let port = conn.port;
                let username = conn.username.clone();
                let protocol = conn.protocol;
                sender.oneshot_command(async move {
                    let result = match protocol {
                        Protocol::Sftp => SftpClient::connect(
                            &host,
                            port,
                            &username,
                            SftpAuth::Password(password),
                        )
                        .await
                        .map(ActiveSession::Sftp)
                        .map_err(|e| e.to_string()),

                        Protocol::Ftp => FtpClient::connect(&host, port, &username, password)
                            .await
                            .map(ActiveSession::Ftp)
                            .map_err(|e| e.to_string()),
                    };
                    NetworkBrowserCmd::Connected(conn, result)
                });
            }
            NetworkBrowserInput::Disconnect => {
                self.session = None;
                self.connected_name = None;
                self.entries.clear();
                self.current_path = PathBuf::from("/");
                self.path_label.set_label("—");
                self.disconnect_btn.set_sensitive(false);
                self.up_btn.set_sensitive(false);
                self.file_stack.set_visible_child_name("placeholder");
                self.status_label.set_label("");
            }
            NetworkBrowserInput::ListDir(path) => {
                if let Some(session) = self.session.clone() {
                    self.progress_rev.set_reveal_child(true);
                    let p = path.clone();
                    sender.oneshot_command(async move {
                        let result = session.list(&p).await.map_err(|e| e.to_string());
                        NetworkBrowserCmd::Listed(path, result)
                    });
                }
            }
            NetworkBrowserInput::NavigateUp => {
                if let Some(parent) = self.current_path.parent() {
                    sender.input(NetworkBrowserInput::ListDir(parent.to_path_buf()));
                }
            }
            NetworkBrowserInput::Download(entry) => {
                if let Some(session) = self.session.clone() {
                    let remote = entry.path.clone();
                    let name = entry.name.clone();
                    let local_dir =
                        dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
                    let local = local_dir.join(&name);
                    self.progress_rev.set_reveal_child(true);
                    self.status_label.set_label(&i18n::t("net.downloading"));
                    sender.oneshot_command(async move {
                        let result =
                            session.download(&remote, &local).await.map_err(|e| e.to_string());
                        NetworkBrowserCmd::Downloaded(name, result)
                    });
                }
            }
            NetworkBrowserInput::SaveConnection(conn, password, save_pw) => {
                if let Err(e) = orca_core::upsert_connection(conn.clone()) {
                    tracing::warn!(error = %e, "save connection failed");
                }
                if save_pw && !password.is_empty() {
                    let name = conn.name.clone();
                    let user = conn.username.clone();
                    relm4::spawn(async move {
                        if let Err(e) =
                            orca_core::net::credentials::store_password(&name, &user, &password)
                                .await
                        {
                            tracing::warn!(error = %e, "store password failed");
                        }
                    });
                }
                sender.input(NetworkBrowserInput::Reload);
            }
            NetworkBrowserInput::RemoveConnection(name) => {
                if let Err(e) = orca_core::remove_connection(&name) {
                    tracing::warn!(error = %e, "remove connection failed");
                }
                sender.input(NetworkBrowserInput::Reload);
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match msg {
            NetworkBrowserCmd::GotPassword(conn, Some(pw)) => {
                sender.input(NetworkBrowserInput::DoConnect(conn, pw));
            }
            NetworkBrowserCmd::GotPassword(conn, None) => {
                show_password_dialog(root, conn, &sender);
            }
            NetworkBrowserCmd::Connected(conn, Ok(session)) => {
                self.progress_rev.set_reveal_child(false);
                self.connected_name = Some(conn.name.clone());
                self.session = Some(session);
                self.disconnect_btn.set_sensitive(true);
                self.up_btn.set_sensitive(true);
                let start = if conn.remote_path.is_empty() {
                    PathBuf::from("/")
                } else {
                    PathBuf::from(&conn.remote_path)
                };
                self.status_label
                    .set_label(&i18n::tf("net.connected_to", &[("name", &conn.name)]));
                sender.input(NetworkBrowserInput::ListDir(start));
            }
            NetworkBrowserCmd::Connected(_, Err(e)) => {
                self.progress_rev.set_reveal_child(false);
                self.status_label
                    .set_label(&i18n::tf("net.error", &[("msg", &e)]));
            }
            NetworkBrowserCmd::Listed(path, Ok(entries)) => {
                self.progress_rev.set_reveal_child(false);
                self.current_path = path.clone();
                self.path_label.set_label(&path.to_string_lossy());
                self.entries = entries;
                self.file_stack.set_visible_child_name("files");
                self.rebuild_file_list(&sender);
            }
            NetworkBrowserCmd::Listed(_, Err(e)) => {
                self.progress_rev.set_reveal_child(false);
                self.status_label
                    .set_label(&i18n::tf("net.error", &[("msg", &e)]));
            }
            NetworkBrowserCmd::Downloaded(name, Ok(())) => {
                self.progress_rev.set_reveal_child(false);
                let dir = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
                self.status_label.set_label(
                    &i18n::tf("net.downloaded", &[("path", &dir.join(&name).to_string_lossy())]),
                );
            }
            NetworkBrowserCmd::Downloaded(name, Err(e)) => {
                self.progress_rev.set_reveal_child(false);
                self.status_label
                    .set_label(&i18n::tf("net.dl_error", &[("name", &name), ("msg", &e)]));
            }
        }
    }
}

impl NetworkBrowser {
    /// Rebuild the saved connections list in the left panel.
    fn rebuild_conn_list(&mut self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.conn_list.first_child() {
            self.conn_list.remove(&child);
        }

        if self.connections.is_empty() {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(
                &gtk::Label::builder()
                    .label(i18n::t("net.no_connections"))
                    .css_classes(["dim-label"])
                    .margin_top(8)
                    .margin_bottom(8)
                    .build(),
            ));
            self.conn_list.append(&row);
            return;
        }

        for conn in &self.connections {
            let row = gtk::ListBoxRow::new();
            let bx = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(6)
                .margin_end(6)
                .build();

            let icon = match conn.protocol {
                Protocol::Sftp => "network-server-symbolic",
                Protocol::Ftp => "folder-remote-symbolic",
            };
            bx.append(&gtk::Image::builder().icon_name(icon).pixel_size(18).build());

            let info = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .hexpand(true)
                .build();
            info.append(
                &gtk::Label::builder()
                    .label(&conn.name)
                    .halign(gtk::Align::Start)
                    .build(),
            );
            info.append(
                &gtk::Label::builder()
                    .label(format!("{}@{}:{}", conn.username, conn.host, conn.port))
                    .halign(gtk::Align::Start)
                    .css_classes(["dim-label"])
                    .build(),
            );
            bx.append(&info);

            let connect_btn = gtk::Button::with_label(&i18n::t("net.connect"));
            connect_btn.add_css_class("flat");
            {
                let s = sender.clone();
                let c = conn.clone();
                connect_btn.connect_clicked(move |_| {
                    s.input(NetworkBrowserInput::ConnectSaved(c.clone()));
                });
            }

            let remove_btn = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .has_frame(false)
                .tooltip_text(i18n::t("net.remove"))
                .build();
            {
                let s = sender.clone();
                let name = conn.name.clone();
                remove_btn.connect_clicked(move |_| {
                    s.input(NetworkBrowserInput::RemoveConnection(name.clone()));
                });
            }

            bx.append(&connect_btn);
            bx.append(&remove_btn);
            row.set_child(Some(&bx));
            self.conn_list.append(&row);
        }
    }

    /// Rebuild the remote file list in the right panel.
    fn rebuild_file_list(&mut self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.file_list.first_child() {
            self.file_list.remove(&child);
        }

        if self.entries.is_empty() {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(
                &gtk::Label::builder()
                    .label(i18n::t("net.empty"))
                    .css_classes(["dim-label"])
                    .margin_top(8)
                    .margin_bottom(8)
                    .build(),
            ));
            self.file_list.append(&row);
            return;
        }

        for entry in &self.entries {
            let row = gtk::ListBoxRow::new();
            let bx = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(6)
                .margin_end(6)
                .build();

            let icon = if entry.is_dir {
                "folder-symbolic"
            } else {
                "text-x-generic-symbolic"
            };
            bx.append(&gtk::Image::builder().icon_name(icon).pixel_size(16).build());

            let name_lbl = gtk::Label::builder()
                .label(&entry.name)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            bx.append(&name_lbl);

            if entry.is_dir {
                let nav_btn = gtk::Button::builder()
                    .icon_name("go-next-symbolic")
                    .has_frame(false)
                    .tooltip_text(i18n::t("net.open_dir"))
                    .build();
                let s = sender.clone();
                let path = entry.path.clone();
                nav_btn.connect_clicked(move |_| {
                    s.input(NetworkBrowserInput::ListDir(path.clone()));
                });
                bx.append(&nav_btn);
            } else {
                let size_lbl = gtk::Label::builder()
                    .label(format::size(entry.size))
                    .halign(gtk::Align::End)
                    .css_classes(["dim-label"])
                    .build();
                bx.append(&size_lbl);

                let dl_btn = gtk::Button::builder()
                    .icon_name("folder-download-symbolic")
                    .has_frame(false)
                    .tooltip_text(i18n::t("net.download"))
                    .build();
                let s = sender.clone();
                let e = entry.clone();
                dl_btn.connect_clicked(move |_| {
                    s.input(NetworkBrowserInput::Download(e.clone()));
                });
                bx.append(&dl_btn);
            }

            row.set_child(Some(&bx));
            self.file_list.append(&row);
        }
    }
}

/// Show a modal dialog for adding a new saved connection.
fn show_add_dialog(parent: &gtk::Window, sender: &ComponentSender<NetworkBrowser>) {
    let dialog = gtk::Window::builder()
        .title(i18n::t("net.add_title"))
        .modal(true)
        .transient_for(parent)
        .default_width(360)
        .resizable(false)
        .build();

    let form = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let name_entry = gtk::Entry::new();
    form.append(&form_field(&i18n::t("net.name"), &name_entry));

    let proto_model = gtk::StringList::new(&["SFTP", "FTP"]);
    let proto_combo = gtk::DropDown::new(Some(proto_model), gtk::Expression::NONE);
    form.append(&form_field(&i18n::t("net.protocol"), &proto_combo));

    let host_entry = gtk::Entry::builder().placeholder_text("example.com").build();
    form.append(&form_field(&i18n::t("net.host"), &host_entry));

    let port_adj = gtk::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let port_spin = gtk::SpinButton::builder()
        .adjustment(&port_adj)
        .numeric(true)
        .build();
    form.append(&form_field(&i18n::t("net.port"), &port_spin));

    let user_entry = gtk::Entry::new();
    form.append(&form_field(&i18n::t("net.username"), &user_entry));

    let path_entry = gtk::Entry::builder().placeholder_text("/home/user").build();
    form.append(&form_field(&i18n::t("net.remote_path"), &path_entry));

    let pw_entry = gtk::Entry::builder().visibility(false).build();
    form.append(&form_field(&i18n::t("net.password"), &pw_entry));

    let save_pw_check = gtk::CheckButton::builder()
        .label(i18n::t("net.save_password"))
        .active(true)
        .build();
    form.append(&save_pw_check);

    // Auto-update port when protocol changes (22↔21)
    {
        let port_adj2 = port_adj.clone();
        proto_combo.connect_selected_notify(move |combo| {
            port_adj2.set_value(if combo.selected() == 0 { 22.0 } else { 21.0 });
        });
    }

    // Buttons
    let btn_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .margin_top(8)
        .build();
    let cancel_btn = gtk::Button::with_label(&i18n::t("common.cancel"));
    let save_btn = gtk::Button::with_label(&i18n::t("net.save"));
    save_btn.add_css_class("suggested-action");

    {
        let d = dialog.clone();
        cancel_btn.connect_clicked(move |_| d.close());
    }
    {
        let d = dialog.clone();
        let s = sender.clone();
        let n = name_entry.clone();
        let h = host_entry.clone();
        let sp = port_spin.clone();
        let u = user_entry.clone();
        let pa = path_entry.clone();
        let pw = pw_entry.clone();
        let sv = save_pw_check.clone();
        let pr = proto_combo.clone();
        save_btn.connect_clicked(move |_| {
            let protocol = if pr.selected() == 0 {
                Protocol::Sftp
            } else {
                Protocol::Ftp
            };
            let conn = SavedConnection {
                name: n.text().to_string(),
                protocol,
                host: h.text().to_string(),
                port: sp.value() as u16,
                username: u.text().to_string(),
                remote_path: pa.text().to_string(),
            };
            s.input(NetworkBrowserInput::SaveConnection(
                conn,
                pw.text().to_string(),
                sv.is_active(),
            ));
            d.close();
        });
    }

    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    form.append(&btn_row);

    dialog.set_child(Some(&form));
    dialog.present();
}

/// Show a modal password prompt when the keyring has no stored credential.
fn show_password_dialog(
    parent: &gtk::Window,
    conn: SavedConnection,
    sender: &ComponentSender<NetworkBrowser>,
) {
    let title = i18n::tf(
        "net.password_prompt",
        &[("user", &conn.username), ("host", &conn.host)],
    );
    let dialog = gtk::Window::builder()
        .title(&title)
        .modal(true)
        .transient_for(parent)
        .default_width(300)
        .resizable(false)
        .build();

    let form = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    form.append(
        &gtk::Label::builder()
            .label(&title)
            .halign(gtk::Align::Start)
            .build(),
    );

    let pw_entry = gtk::Entry::builder().visibility(false).build();
    form.append(&pw_entry);

    let save_check = gtk::CheckButton::builder()
        .label(i18n::t("net.save_password"))
        .active(false)
        .build();
    form.append(&save_check);

    let btn_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let cancel_btn = gtk::Button::with_label(&i18n::t("common.cancel"));
    let connect_btn = gtk::Button::with_label(&i18n::t("net.connect"));
    connect_btn.add_css_class("suggested-action");

    {
        let d = dialog.clone();
        cancel_btn.connect_clicked(move |_| d.close());
    }
    {
        let d = dialog.clone();
        let s = sender.clone();
        let pw = pw_entry.clone();
        let sc = save_check.clone();
        let c = conn.clone();
        connect_btn.connect_clicked(move |_| {
            let password = pw.text().to_string();
            if sc.is_active() && !password.is_empty() {
                let name = c.name.clone();
                let user = c.username.clone();
                let pw2 = password.clone();
                relm4::spawn(async move {
                    if let Err(e) =
                        orca_core::net::credentials::store_password(&name, &user, &pw2).await
                    {
                        tracing::warn!(error = %e, "store password failed");
                    }
                });
            }
            s.input(NetworkBrowserInput::DoConnect(c.clone(), password));
            d.close();
        });
    }

    btn_row.append(&cancel_btn);
    btn_row.append(&connect_btn);
    form.append(&btn_row);
    dialog.set_child(Some(&form));
    dialog.present();
}

/// Build a vertical label + widget pair for dialog forms.
fn form_field<W: IsA<gtk::Widget>>(label: &str, widget: &W) -> gtk::Box {
    let bx = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    bx.append(
        &gtk::Label::builder()
            .label(label)
            .halign(gtk::Align::Start)
            .css_classes(["dim-label"])
            .build(),
    );
    bx.append(widget);
    bx
}
