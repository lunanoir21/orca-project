//! Vault UI integration — Phase 6.
//!
//! [`VaultPanel`] is a relm4 [`Component`] that renders the "Kasalar" (Vaults)
//! section of the Places sidebar and drives all vault lifecycle flows: unlock,
//! lock, create, import/export backup, integrity check, and file ingestion.
//!
//! The component owns a shared [`VaultManager`] behind an `Arc<StdMutex>` so
//! that blocking vault I/O can be dispatched via [`relm4::spawn_blocking`]
//! without holding the GTK main-thread lock.
//!
//! # Security
//! This module never stores, logs, or displays raw passphrases. Passphrase
//! strings are consumed by `open_vault` / `Vault::create` and immediately
//! dropped. Key material lives only inside the [`Vault`]'s `DerivedKey`
//! field and is zeroized on lock / drop.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use orca_vault::{IntegrityReport, VaultConfig, VaultManager};

use crate::dialogs;
use crate::i18n;

// ---------------------------------------------------------------------------
// Public init payload
// ---------------------------------------------------------------------------

/// Initialisation payload: a shared vault manager.
pub struct VaultPanelInit {
    pub mgr: Arc<Mutex<VaultManager>>,
}

// ---------------------------------------------------------------------------
// Input / output / command
// ---------------------------------------------------------------------------

/// Messages the vault panel handles.
#[derive(Debug, Clone)]
pub enum VaultPanelInput {
    /// Rebuild the vault list from current manager state (public for external callers).
    #[allow(dead_code)]
    Refresh,
    /// 1-second auto-lock countdown tick (fired by glib::timeout).
    Tick,
    /// Show the passphrase dialog then unlock the named vault.
    TryUnlock(String),
    /// Called by the passphrase dialog callback; runs the actual unlock.
    DoUnlock { name: String, passphrase: String },
    /// Lock the named vault immediately.
    LockVault(String),
    /// Navigate the active pane into the vault's on-disk directory.
    NavigateVault(String),
    /// Add `paths` to the best available unlocked vault (or show error).
    AddFiles(Vec<PathBuf>),
    /// Add `paths` to a specific vault.
    AddFilesToVault { vault: String, paths: Vec<PathBuf> },
    /// Show the "Create New Vault" dialog.
    ShowCreateDialog,
    /// Actually create the vault after the dialog is confirmed.
    DoCreate {
        name: String,
        path: PathBuf,
        passphrase: String,
    },
    /// Show the "Import Vault" dialog.
    ShowImportDialog,
    /// Actually import the vault backup.
    DoImport {
        name: String,
        backup: PathBuf,
        dest: PathBuf,
        passphrase: String,
    },
    /// Export a backup of the named vault to a user-chosen path.
    ExportBackup(String),
    /// Verify the integrity of the named vault.
    VerifyIntegrity(String),
    /// Unregister the named vault (no disk delete).
    RemoveVault(String),
}

/// What the vault panel reports to the application shell.
#[derive(Debug, Clone)]
pub enum VaultPanelOutput {
    /// Navigate the active pane to this path.
    Navigate(PathBuf),
    /// A vault auto-locked; shell should update the status bar.
    AutoLocked(String),
    /// Concise vault status for the shell's status bar (updated every tick).
    StatusText(String),
    /// Vault configs changed; shell should persist them.
    ConfigsChanged(Vec<VaultConfig>),
}

/// Background task results.
#[derive(Debug)]
pub enum VaultPanelCmd {
    Unlocked(String, Result<(), String>),
    Created(String, Result<(), String>),
    FilesAdded(usize, Result<(), String>),
    Exported(Result<(), String>),
    Imported(String, Result<(), String>),
    IntegrityDone(String, Result<IntegrityReport, String>),
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

pub struct VaultPanel {
    mgr: Arc<Mutex<VaultManager>>,
    list_box: gtk::ListBox,
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[relm4::component(pub)]
impl Component for VaultPanel {
    type Init = VaultPanelInit;
    type Input = VaultPanelInput;
    type Output = VaultPanelOutput;
    type CommandOutput = VaultPanelCmd;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 2,
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let VaultPanelInit { mgr } = init;
        let widgets = view_output!();

        // Section header
        root.append(&section_label(&i18n::t("vault.section")));

        // Action buttons row
        let btn_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .margin_start(8)
            .margin_end(8)
            .build();

        let add_btn = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .has_frame(false)
            .tooltip_text(i18n::t("vault.add_new"))
            .build();
        {
            let s = sender.clone();
            add_btn.connect_clicked(move |_| s.input(VaultPanelInput::ShowCreateDialog));
        }

        let import_btn = gtk::Button::builder()
            .icon_name("document-import-symbolic")
            .has_frame(false)
            .tooltip_text(i18n::t("vault.import_vault"))
            .build();
        {
            let s = sender.clone();
            import_btn.connect_clicked(move |_| s.input(VaultPanelInput::ShowImportDialog));
        }

        btn_row.append(&add_btn);
        btn_row.append(&import_btn);
        root.append(&btn_row);

        // Vault list
        let list_box = gtk::ListBox::new();
        list_box.set_selection_mode(gtk::SelectionMode::None);
        root.append(&list_box);

        let model = VaultPanel { mgr, list_box };

        // Populate immediately
        model.rebuild_list(&sender);

        // 1-second glib ticker for auto-lock countdown display
        {
            let s = sender.clone();
            gtk::glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
                s.input(VaultPanelInput::Tick);
                gtk::glib::ControlFlow::Continue
            });
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        let parent = parent_window(root);

        match msg {
            VaultPanelInput::Refresh => {
                self.rebuild_list(&sender);
            }

            VaultPanelInput::Tick => {
                // Check for expired timers and fire auto-lock before rebuilding.
                let expired: Vec<String> = {
                    let guard = self.mgr.lock().unwrap();
                    guard
                        .configs()
                        .iter()
                        .filter(|c| {
                            guard.is_unlocked(&c.name)
                                && guard
                                    .timer(&c.name)
                                    .is_some_and(|t| t.is_enabled() && t.expired())
                        })
                        .map(|c| c.name.clone())
                        .collect()
                };
                for name in &expired {
                    let mut guard = self.mgr.lock().unwrap();
                    let _ = guard.lock_vault(name);
                    tracing::info!(vault = %name, "auto-lock fired");
                    drop(guard);
                    sender
                        .output(VaultPanelOutput::AutoLocked(name.clone()))
                        .ok();
                }
                self.rebuild_list(&sender);
                self.emit_status(&sender);
            }

            VaultPanelInput::TryUnlock(name) => {
                let s = sender.clone();
                let n = name.clone();
                dialogs::passphrase(parent.as_ref(), &i18n::t("vault.unlock"), move |pass| {
                    s.input(VaultPanelInput::DoUnlock {
                        name: n.clone(),
                        passphrase: pass,
                    });
                });
            }

            VaultPanelInput::DoUnlock { name, passphrase } => {
                let mgr = self.mgr.clone();
                let n = name.clone();
                sender.oneshot_command(async move {
                    let result = relm4::spawn_blocking(move || {
                        mgr.lock()
                            .unwrap()
                            .open_vault(&n, passphrase.as_bytes())
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    VaultPanelCmd::Unlocked(name, result)
                });
            }

            VaultPanelInput::LockVault(name) => {
                {
                    let mut guard = self.mgr.lock().unwrap();
                    let _ = guard.lock_vault(&name);
                }
                self.rebuild_list(&sender);
                self.emit_status(&sender);
            }

            VaultPanelInput::NavigateVault(name) => {
                let path = self
                    .mgr
                    .lock()
                    .unwrap()
                    .configs()
                    .iter()
                    .find(|c| c.name == name)
                    .map(|c| c.path.clone());
                if let Some(p) = path {
                    sender.output(VaultPanelOutput::Navigate(p)).ok();
                }
            }

            VaultPanelInput::AddFiles(paths) => {
                let vault = {
                    let guard = self.mgr.lock().unwrap();
                    let active = guard.active().map(str::to_owned);
                    if let Some(ref n) = active {
                        if guard.is_unlocked(n) {
                            active
                        } else {
                            guard
                                .configs()
                                .iter()
                                .find(|c| guard.is_unlocked(&c.name))
                                .map(|c| c.name.clone())
                        }
                    } else {
                        guard
                            .configs()
                            .iter()
                            .find(|c| guard.is_unlocked(&c.name))
                            .map(|c| c.name.clone())
                    }
                };
                match vault {
                    None => dialogs::info(
                        parent.as_ref(),
                        &i18n::t("vault.add_files"),
                        &i18n::t("vault.no_unlocked"),
                    ),
                    Some(v) => sender.input(VaultPanelInput::AddFilesToVault { vault: v, paths }),
                }
            }

            VaultPanelInput::AddFilesToVault { vault, paths } => {
                let mgr = self.mgr.clone();
                let n = vault;
                let count = paths.len();
                sender.oneshot_command(async move {
                    let result = relm4::spawn_blocking(move || {
                        let mut guard = mgr.lock().unwrap();
                        let v = guard
                            .vault_mut(&n)
                            .ok_or_else(|| "vault not unlocked".to_owned())?;
                        for p in &paths {
                            v.add_file(p).map_err(|e| e.to_string())?;
                        }
                        Ok(())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    VaultPanelCmd::FilesAdded(count, result)
                });
            }

            VaultPanelInput::ShowCreateDialog => {
                show_create_dialog(parent.as_ref(), sender.clone());
            }

            VaultPanelInput::DoCreate {
                name,
                path,
                passphrase,
            } => {
                let mgr = self.mgr.clone();
                let n = name.clone();
                let p = path.clone();
                sender.oneshot_command(async move {
                    let result = relm4::spawn_blocking(move || {
                        let config = VaultConfig {
                            name: n.clone(),
                            path: p,
                            auto_lock_minutes: Some(5),
                            scheme: Default::default(),
                            integrity_hash: Default::default(),
                        };
                        mgr.lock()
                            .unwrap()
                            .add_vault(config, passphrase.as_bytes())
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    VaultPanelCmd::Created(name, result)
                });
            }

            VaultPanelInput::ShowImportDialog => {
                show_import_dialog(parent.as_ref(), sender.clone());
            }

            VaultPanelInput::DoImport {
                name,
                backup,
                dest,
                passphrase,
            } => {
                let mgr = self.mgr.clone();
                let n = name.clone();
                sender.oneshot_command(async move {
                    let result = relm4::spawn_blocking(move || {
                        // Import the backup to disk, then register with the manager.
                        orca_vault::Vault::import_backup(&backup, passphrase.as_bytes(), &dest, &n)
                            .map_err(|e| e.to_string())?;
                        let config = VaultConfig {
                            name: n.clone(),
                            path: dest.join(&n),
                            auto_lock_minutes: Some(5),
                            scheme: Default::default(),
                            integrity_hash: Default::default(),
                        };
                        mgr.lock()
                            .unwrap()
                            .register_existing(config, passphrase.as_bytes())
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    VaultPanelCmd::Imported(name, result)
                });
            }

            VaultPanelInput::ExportBackup(vault) => {
                let mgr = self.mgr.clone();
                let n = vault.clone();
                let initial = format!("{n}-backup.tar.age");
                let dialog = gtk::FileDialog::builder()
                    .title(i18n::t("vault.export"))
                    .initial_name(initial)
                    .modal(true)
                    .build();
                let s = sender.clone();
                dialog.save(
                    parent.as_ref(),
                    gtk::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(dest) = file.path() {
                                let mgr2 = mgr.clone();
                                let n2 = n.clone();
                                let s2 = s.clone();
                                relm4::spawn(async move {
                                    let result = relm4::spawn_blocking(move || {
                                        let guard = mgr2.lock().unwrap();
                                        let v = guard
                                            .vault(&n2)
                                            .ok_or_else(|| "vault not unlocked".to_owned())?;
                                        v.export_backup(&dest).map_err(|e| e.to_string())
                                    })
                                    .await
                                    .unwrap_or_else(|e| Err(e.to_string()));
                                    s2.oneshot_command(
                                        async move { VaultPanelCmd::Exported(result) },
                                    );
                                });
                            }
                        }
                    },
                );
            }

            VaultPanelInput::VerifyIntegrity(vault) => {
                let mgr = self.mgr.clone();
                let n = vault.clone();
                sender.oneshot_command(async move {
                    let result = relm4::spawn_blocking(move || {
                        let guard = mgr.lock().unwrap();
                        let v = guard
                            .vault(&n)
                            .ok_or_else(|| "vault not unlocked".to_owned())?;
                        v.verify_integrity().map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    VaultPanelCmd::IntegrityDone(vault, result)
                });
            }

            VaultPanelInput::RemoveVault(name) => {
                {
                    let mut guard = self.mgr.lock().unwrap();
                    let _ = guard.remove_vault(&name);
                }
                self.rebuild_list(&sender);
                let configs = self.mgr.lock().unwrap().configs().to_vec();
                sender
                    .output(VaultPanelOutput::ConfigsChanged(configs))
                    .ok();
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        let parent = parent_window(root);

        match msg {
            VaultPanelCmd::Unlocked(_name, Ok(())) => {
                self.rebuild_list(&sender);
                self.emit_status(&sender);
                let configs = self.mgr.lock().unwrap().configs().to_vec();
                sender
                    .output(VaultPanelOutput::ConfigsChanged(configs))
                    .ok();
            }

            VaultPanelCmd::Unlocked(_name, Err(e)) => {
                // Wrong passphrase produces a MAC error from age/chacha20poly.
                let msg = if e.to_lowercase().contains("mac")
                    || e.to_lowercase().contains("decrypt")
                    || e.to_lowercase().contains("passphrase")
                {
                    i18n::t("vault.wrong_pass")
                } else {
                    i18n::tf("vault.unlock_fail", &[("err", &e)])
                };
                dialogs::info(parent.as_ref(), &i18n::t("vault.unlock"), &msg);
            }

            VaultPanelCmd::Created(_name, Ok(())) => {
                self.rebuild_list(&sender);
                self.emit_status(&sender);
                let configs = self.mgr.lock().unwrap().configs().to_vec();
                sender
                    .output(VaultPanelOutput::ConfigsChanged(configs))
                    .ok();
            }

            VaultPanelCmd::Created(_name, Err(e)) => {
                dialogs::info(
                    parent.as_ref(),
                    &i18n::t("vault.new_title"),
                    &i18n::tf("vault.unlock_fail", &[("err", &e)]),
                );
            }

            VaultPanelCmd::FilesAdded(n, Ok(())) => {
                dialogs::info(
                    parent.as_ref(),
                    &i18n::t("vault.add_files"),
                    &i18n::tf("vault.files_added", &[("n", &n.to_string())]),
                );
            }

            VaultPanelCmd::FilesAdded(_n, Err(e)) => {
                dialogs::info(
                    parent.as_ref(),
                    &i18n::t("vault.add_files"),
                    &i18n::tf("vault.files_fail", &[("err", &e)]),
                );
            }

            VaultPanelCmd::Exported(Ok(())) => {
                dialogs::info(
                    parent.as_ref(),
                    &i18n::t("vault.export"),
                    &i18n::t("vault.export_done"),
                );
            }

            VaultPanelCmd::Exported(Err(e)) => {
                dialogs::info(
                    parent.as_ref(),
                    &i18n::t("vault.export"),
                    &i18n::tf("vault.export_fail", &[("err", &e)]),
                );
            }

            VaultPanelCmd::Imported(_name, Ok(())) => {
                self.rebuild_list(&sender);
                let configs = self.mgr.lock().unwrap().configs().to_vec();
                sender
                    .output(VaultPanelOutput::ConfigsChanged(configs))
                    .ok();
            }

            VaultPanelCmd::Imported(_name, Err(e)) => {
                dialogs::info(
                    parent.as_ref(),
                    &i18n::t("vault.import_title"),
                    &i18n::tf("vault.unlock_fail", &[("err", &e)]),
                );
            }

            VaultPanelCmd::IntegrityDone(name, Ok(report)) => {
                let text = format_integrity_report(&report);
                let title = i18n::tf("vault.integrity_title", &[("name", &name)]);
                dialogs::info(parent.as_ref(), &title, &text);
            }

            VaultPanelCmd::IntegrityDone(_name, Err(e)) => {
                dialogs::info(
                    parent.as_ref(),
                    &i18n::t("vault.integrity"),
                    &i18n::tf("vault.unlock_fail", &[("err", &e)]),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Model helpers
// ---------------------------------------------------------------------------

impl VaultPanel {
    /// Rebuild the vault list box from a snapshot of manager state.
    fn rebuild_list(&self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }

        let (configs, unlocked_map, remaining_map) = {
            let guard = self.mgr.lock().unwrap();
            let configs: Vec<VaultConfig> = guard.configs().to_vec();
            let mut unlocked_map = std::collections::HashMap::new();
            let mut remaining_map = std::collections::HashMap::new();
            for c in &configs {
                let u = guard.is_unlocked(&c.name);
                unlocked_map.insert(c.name.clone(), u);
                let rem = if u {
                    guard.timer(&c.name).and_then(|t| t.remaining())
                } else {
                    None
                };
                remaining_map.insert(c.name.clone(), rem);
            }
            (configs, unlocked_map, remaining_map)
        };

        if configs.is_empty() {
            let lbl = gtk::Label::builder()
                .label(i18n::t("vault.empty"))
                .halign(gtk::Align::Start)
                .margin_start(12)
                .build();
            lbl.add_css_class("dim-label");
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&lbl));
            self.list_box.append(&row);
            return;
        }

        for cfg in &configs {
            let unlocked = *unlocked_map.get(&cfg.name).unwrap_or(&false);
            let remaining = remaining_map.get(&cfg.name).copied().flatten();
            let row = build_vault_row(cfg, unlocked, remaining, sender);
            self.list_box.append(&row);
        }
    }

    /// Emit a concise status-bar string for the active or first unlocked vault.
    fn emit_status(&self, sender: &ComponentSender<Self>) {
        let guard = self.mgr.lock().unwrap();
        let name = guard.active().map(str::to_owned).or_else(|| {
            guard
                .configs()
                .iter()
                .find(|c| guard.is_unlocked(&c.name))
                .map(|c| c.name.clone())
        });
        let text = match name {
            None => i18n::t("vault.none"),
            Some(ref n) if guard.is_unlocked(n) => {
                match guard.timer(n).and_then(|t| t.remaining()) {
                    None => i18n::tf("vault.status_open", &[("name", n)]),
                    Some(r) => i18n::tf(
                        "vault.status_countdown",
                        &[
                            ("name", n),
                            ("mins", &format!("{:02}", r.as_secs() / 60)),
                            ("secs", &format!("{:02}", r.as_secs() % 60)),
                        ],
                    ),
                }
            }
            Some(ref n) => i18n::tf("vault.status_locked_sb", &[("name", n)]),
        };
        sender.output(VaultPanelOutput::StatusText(text)).ok();
    }
}

// ---------------------------------------------------------------------------
// Widget builders
// ---------------------------------------------------------------------------

fn build_vault_row(
    cfg: &VaultConfig,
    unlocked: bool,
    remaining: Option<std::time::Duration>,
    sender: &ComponentSender<VaultPanel>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(8)
        .margin_end(4)
        .build();

    // Top line: icon + name + countdown + buttons
    let top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();

    top.append(
        &gtk::Image::builder()
            .icon_name(if unlocked {
                "channel-secure-symbolic"
            } else {
                "channel-insecure-symbolic"
            })
            .pixel_size(16)
            .build(),
    );

    let name_lbl = gtk::Label::builder()
        .label(&cfg.name)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    top.append(&name_lbl);

    // Countdown (only when unlocked and auto-lock enabled)
    if let Some(rem) = remaining {
        let mins = rem.as_secs() / 60;
        let secs = rem.as_secs() % 60;
        let cd = gtk::Label::builder()
            .label(
                i18n::tf(
                    "vault.countdown",
                    &[
                        ("mins", &format!("{mins:02}")),
                        ("secs", &format!("{secs:02}")),
                    ],
                )
                .as_str(),
            )
            .halign(gtk::Align::End)
            .build();
        cd.add_css_class("dim-label");
        top.append(&cd);
    }

    // Lock / unlock toggle button
    let toggle = gtk::Button::builder()
        .icon_name(if unlocked {
            "system-lock-screen-symbolic"
        } else {
            "dialog-password-symbolic"
        })
        .has_frame(false)
        .tooltip_text(if unlocked {
            i18n::t("vault.lock")
        } else {
            i18n::t("vault.unlock")
        })
        .build();
    {
        let s = sender.clone();
        let n = cfg.name.clone();
        toggle.connect_clicked(move |_| {
            if unlocked {
                s.input(VaultPanelInput::LockVault(n.clone()));
            } else {
                s.input(VaultPanelInput::TryUnlock(n.clone()));
            }
        });
    }
    top.append(&toggle);

    // Context menu button
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .has_frame(false)
        .build();
    let popover = build_popover(cfg, unlocked, sender);
    menu_btn.set_popover(Some(&popover));
    top.append(&menu_btn);

    outer.append(&top);

    // "Open in vault" quick link (visible only when unlocked)
    if unlocked {
        let nav_btn = gtk::Button::builder()
            .label(i18n::t("vault.navigate"))
            .has_frame(false)
            .halign(gtk::Align::Start)
            .build();
        nav_btn.add_css_class("dim-label");
        {
            let s = sender.clone();
            let n = cfg.name.clone();
            nav_btn.connect_clicked(move |_| s.input(VaultPanelInput::NavigateVault(n.clone())));
        }
        outer.append(&nav_btn);
    }

    row.set_child(Some(&outer));
    row
}

/// Build the context-menu popover for a vault row.
fn build_popover(
    cfg: &VaultConfig,
    unlocked: bool,
    sender: &ComponentSender<VaultPanel>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();

    let mk = |label: &str,
              msg: VaultPanelInput,
              popover: &gtk::Popover,
              sender: &ComponentSender<VaultPanel>| {
        let btn = gtk::Button::builder()
            .label(label)
            .has_frame(false)
            .halign(gtk::Align::Start)
            .build();
        let s = sender.clone();
        let p = popover.clone();
        btn.connect_clicked(move |_| {
            p.popdown();
            s.input(msg.clone());
        });
        btn
    };

    if unlocked {
        vbox.append(&mk(
            &i18n::t("vault.export"),
            VaultPanelInput::ExportBackup(cfg.name.clone()),
            &popover,
            sender,
        ));
        vbox.append(&mk(
            &i18n::t("vault.integrity"),
            VaultPanelInput::VerifyIntegrity(cfg.name.clone()),
            &popover,
            sender,
        ));
    }
    vbox.append(&mk(
        &i18n::t("vault.remove"),
        VaultPanelInput::RemoveVault(cfg.name.clone()),
        &popover,
        sender,
    ));

    popover.set_child(Some(&vbox));
    popover
}

// ---------------------------------------------------------------------------
// Dialog helpers
// ---------------------------------------------------------------------------

fn show_create_dialog(parent: Option<&gtk::Window>, sender: ComponentSender<VaultPanel>) {
    let window = gtk::Window::builder()
        .title(i18n::t("vault.new_title"))
        .modal(true)
        .default_width(420)
        .build();
    if let Some(p) = parent {
        window.set_transient_for(Some(p));
    }

    let vbox = form_box();

    append_label(&vbox, &i18n::t("vault.new_name"));
    let name_entry = gtk::Entry::builder().placeholder_text("Personal").build();
    vbox.append(&name_entry);

    append_label(&vbox, &i18n::t("vault.new_path"));
    let (path_row, path_entry) = path_picker_row(window.clone(), true);
    vbox.append(&path_row);

    append_label(&vbox, &i18n::t("vault.passphrase"));
    let pass_entry = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    vbox.append(&pass_entry);

    let btn_row = dialog_buttons(&window, &i18n::t("vault.new_create"), {
        let ne = name_entry.clone();
        let pe = path_entry.clone();
        let pse = pass_entry.clone();
        let w = window.clone();
        move || {
            let name = ne.text().to_string();
            let path_str = pe.text().to_string();
            let pass = pse.text().to_string();
            if name.is_empty() || path_str.is_empty() || pass.is_empty() {
                return;
            }
            sender.input(VaultPanelInput::DoCreate {
                name,
                path: PathBuf::from(path_str),
                passphrase: pass,
            });
            w.close();
        }
    });
    vbox.append(&btn_row);

    window.set_child(Some(&vbox));
    window.present();
    name_entry.grab_focus();
}

fn show_import_dialog(parent: Option<&gtk::Window>, sender: ComponentSender<VaultPanel>) {
    let window = gtk::Window::builder()
        .title(i18n::t("vault.import_title"))
        .modal(true)
        .default_width(440)
        .build();
    if let Some(p) = parent {
        window.set_transient_for(Some(p));
    }

    let vbox = form_box();

    append_label(&vbox, &i18n::t("vault.import_name"));
    let name_entry = gtk::Entry::new();
    vbox.append(&name_entry);

    append_label(&vbox, &i18n::t("vault.import_backup"));
    let (backup_row, backup_entry) = file_picker_row(window.clone());
    vbox.append(&backup_row);

    append_label(&vbox, &i18n::t("vault.import_dest"));
    let (dest_row, dest_entry) = path_picker_row(window.clone(), false);
    vbox.append(&dest_row);

    append_label(&vbox, &i18n::t("vault.passphrase"));
    let pass_entry = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    vbox.append(&pass_entry);

    let btn_row = dialog_buttons(&window, &i18n::t("vault.import_do"), {
        let ne = name_entry.clone();
        let be = backup_entry.clone();
        let de = dest_entry.clone();
        let pse = pass_entry.clone();
        let w = window.clone();
        move || {
            let name = ne.text().to_string();
            let backup = PathBuf::from(be.text().to_string());
            let dest = PathBuf::from(de.text().to_string());
            let pass = pse.text().to_string();
            if name.is_empty()
                || backup.as_os_str().is_empty()
                || dest.as_os_str().is_empty()
                || pass.is_empty()
            {
                return;
            }
            sender.input(VaultPanelInput::DoImport {
                name,
                backup,
                dest,
                passphrase: pass,
            });
            w.close();
        }
    });
    vbox.append(&btn_row);

    window.set_child(Some(&vbox));
    window.present();
}

// ---------------------------------------------------------------------------
// Small UI helpers shared between dialogs
// ---------------------------------------------------------------------------

fn form_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build()
}

fn append_label(parent: &gtk::Box, text: &str) {
    parent.append(
        &gtk::Label::builder()
            .label(text)
            .halign(gtk::Align::Start)
            .build(),
    );
}

/// A text-entry + "Choose…" button row for a directory path.
/// `create_dir` controls whether the picker is `select_folder` (true) or `open` (false).
fn path_picker_row(win: gtk::Window, _create_dir: bool) -> (gtk::Box, gtk::Entry) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let entry = gtk::Entry::builder().hexpand(true).build();
    let btn = gtk::Button::with_label(&i18n::t("vault.choose_dir"));
    {
        let e = entry.clone();
        btn.connect_clicked(move |_| {
            let e2 = e.clone();
            let dialog = gtk::FileDialog::builder()
                .title(i18n::t("vault.choose_dir"))
                .modal(true)
                .build();
            dialog.select_folder(Some(&win), gtk::gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        e2.set_text(&path.to_string_lossy());
                    }
                }
            });
        });
    }
    row.append(&entry);
    row.append(&btn);
    (row, entry)
}

/// A text-entry + "…" button row for picking a file to open.
fn file_picker_row(win: gtk::Window) -> (gtk::Box, gtk::Entry) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let entry = gtk::Entry::builder().hexpand(true).build();
    let btn = gtk::Button::with_label("…");
    {
        let e = entry.clone();
        btn.connect_clicked(move |_| {
            let e2 = e.clone();
            let dialog = gtk::FileDialog::builder().modal(true).build();
            dialog.open(Some(&win), gtk::gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        e2.set_text(&path.to_string_lossy());
                    }
                }
            });
        });
    }
    row.append(&entry);
    row.append(&btn);
    (row, entry)
}

/// Cancel + confirm button row.
fn dialog_buttons(
    window: &gtk::Window,
    confirm_label: &str,
    on_confirm: impl Fn() + 'static,
) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    let cancel = gtk::Button::with_label(&i18n::t("common.cancel"));
    {
        let w = window.clone();
        cancel.connect_clicked(move |_| w.close());
    }
    let confirm = gtk::Button::builder()
        .label(confirm_label)
        .css_classes(["suggested-action"])
        .build();
    confirm.connect_clicked(move |_| on_confirm());
    row.append(&cancel);
    row.append(&confirm);
    row
}

// ---------------------------------------------------------------------------
// Integrity report formatter
// ---------------------------------------------------------------------------

fn format_integrity_report(report: &IntegrityReport) -> String {
    if report.is_clean() {
        return i18n::t("vault.integrity_ok");
    }
    let mut issues: Vec<String> = Vec::new();
    for name in &report.tampered {
        issues.push(i18n::tf("vault.integrity_tampered", &[("name", name)]));
    }
    for name in &report.missing {
        issues.push(i18n::tf("vault.integrity_missing", &[("name", name)]));
    }
    for stored in &report.orphaned {
        issues.push(i18n::tf("vault.integrity_orphaned", &[("name", stored)]));
    }
    let header = i18n::tf(
        "vault.integrity_issues",
        &[("n", &issues.len().to_string())],
    );
    format!("{header}\n\n{}", issues.join("\n"))
}

// ---------------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------------

fn parent_window(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.root().and_then(|r| r.downcast::<gtk::Window>().ok())
}

fn section_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .css_classes(["orca-section-label"])
        .build()
}
