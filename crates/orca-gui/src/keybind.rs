//! Central keybind ActionMap: parse config strings → (modifier, key) pairs,
//! detect conflicts, and dispatch key events to application actions.

use std::collections::HashMap;

use relm4::gtk::gdk;

use crate::config::Keybinds;

/// Every application action that can be bound to a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    CloseTab,
    DuplicateTab,
    ToggleHidden,
    ToggleDualPane,
    OpenTerminal,
    CopyToPane,
    MoveToPane,
    Rename,
    Search,
    PathEntry,
    Back,
    Forward,
    Up,
    ViewList,
    ViewIcon,
    ViewDetail,
    VaultAdd,
}

impl Action {
    /// Human-readable action name (used in the settings keybind table).
    pub fn label(self) -> &'static str {
        match self {
            Action::NewTab => "New Tab",
            Action::CloseTab => "Close Tab",
            Action::DuplicateTab => "Duplicate Tab",
            Action::ToggleHidden => "Toggle Hidden Files",
            Action::ToggleDualPane => "Toggle Dual Pane",
            Action::OpenTerminal => "Toggle Terminal",
            Action::CopyToPane => "Copy to Other Pane",
            Action::MoveToPane => "Move to Other Pane",
            Action::Rename => "Rename",
            Action::Search => "Search",
            Action::PathEntry => "Edit Path",
            Action::Back => "Back",
            Action::Forward => "Forward",
            Action::Up => "Up",
            Action::ViewList => "List View",
            Action::ViewIcon => "Icon View",
            Action::ViewDetail => "Detail View",
            Action::VaultAdd => "Add to Vault",
        }
    }

    /// Stable config key for this action (matches `[keybinds]` field names).
    pub fn config_key(self) -> &'static str {
        match self {
            Action::NewTab => "new_tab",
            Action::CloseTab => "close_tab",
            Action::DuplicateTab => "duplicate_tab",
            Action::ToggleHidden => "toggle_hidden",
            Action::ToggleDualPane => "toggle_dual_pane",
            Action::OpenTerminal => "open_terminal",
            Action::CopyToPane => "copy_to_pane",
            Action::MoveToPane => "move_to_pane",
            Action::Rename => "rename",
            Action::Search => "search",
            Action::PathEntry => "path_entry",
            Action::Back => "back",
            Action::Forward => "forward",
            Action::Up => "up",
            Action::ViewList => "view_list",
            Action::ViewIcon => "view_icon",
            Action::ViewDetail => "view_detail",
            Action::VaultAdd => "vault_add",
        }
    }

    /// All actions in display order.
    pub const ALL: &'static [Action] = &[
        Action::Back,
        Action::Forward,
        Action::Up,
        Action::NewTab,
        Action::CloseTab,
        Action::DuplicateTab,
        Action::Rename,
        Action::Search,
        Action::PathEntry,
        Action::ToggleHidden,
        Action::ToggleDualPane,
        Action::OpenTerminal,
        Action::CopyToPane,
        Action::MoveToPane,
        Action::ViewList,
        Action::ViewIcon,
        Action::ViewDetail,
        Action::VaultAdd,
    ];
}

/// A parsed key combination: modifier mask + keyval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub mods: gdk::ModifierType,
    pub key: gdk::Key,
}

impl KeyCombo {
    /// Parse a human-readable keybind string into a `(modifier, key)` combo.
    ///
    /// Format: `"Ctrl+T"`, `"Alt+Left"`, `"F3"`, `"Ctrl+Shift+V"`.
    /// Returns `None` for unrecognised modifiers or key names.
    pub fn parse(s: &str) -> Option<Self> {
        let mut mods = gdk::ModifierType::empty();
        let parts: Vec<&str> = s.split('+').collect();
        if parts.is_empty() {
            return None;
        }
        let key_str = parts.last()?;
        for part in &parts[..parts.len() - 1] {
            match *part {
                "Ctrl" => mods |= gdk::ModifierType::CONTROL_MASK,
                "Alt" => mods |= gdk::ModifierType::ALT_MASK,
                "Shift" => mods |= gdk::ModifierType::SHIFT_MASK,
                "Super" => mods |= gdk::ModifierType::SUPER_MASK,
                _ => return None,
            }
        }
        let key = parse_key_name(key_str)?;
        Some(KeyCombo { mods, key })
    }

    /// Format back to the `"Ctrl+T"` style string.
    pub fn to_display_string(self) -> String {
        let mut parts = Vec::new();
        if self.mods.contains(gdk::ModifierType::CONTROL_MASK) {
            parts.push("Ctrl");
        }
        if self.mods.contains(gdk::ModifierType::ALT_MASK) {
            parts.push("Alt");
        }
        if self.mods.contains(gdk::ModifierType::SHIFT_MASK) {
            parts.push("Shift");
        }
        if self.mods.contains(gdk::ModifierType::SUPER_MASK) {
            parts.push("Super");
        }
        let key_part = key_to_name(self.key).unwrap_or_default();
        if !key_part.is_empty() {
            parts.push(key_part);
        }
        parts.join("+")
    }
}

/// Map from `Action` → `KeyCombo` with O(1) reverse lookup for conflict detection.
pub struct ActionMap {
    /// Forward map: action → combo (used for the settings keybind table).
    forward: HashMap<Action, KeyCombo>,
    /// Reverse map: combo → action (used for key event dispatch).
    reverse: HashMap<KeyCombo, Action>,
}

impl ActionMap {
    /// Build from a `Keybinds` config section. Logs a warning for each conflict.
    #[must_use]
    pub fn from_keybinds(kb: &Keybinds) -> Self {
        let pairs: &[(&str, Action)] = &[
            (&kb.new_tab, Action::NewTab),
            (&kb.close_tab, Action::CloseTab),
            (&kb.duplicate_tab, Action::DuplicateTab),
            (&kb.toggle_hidden, Action::ToggleHidden),
            (&kb.toggle_dual_pane, Action::ToggleDualPane),
            (&kb.open_terminal, Action::OpenTerminal),
            (&kb.copy_to_pane, Action::CopyToPane),
            (&kb.move_to_pane, Action::MoveToPane),
            (&kb.rename, Action::Rename),
            (&kb.search, Action::Search),
            (&kb.path_entry, Action::PathEntry),
            (&kb.back, Action::Back),
            (&kb.forward, Action::Forward),
            (&kb.up, Action::Up),
            (&kb.view_list, Action::ViewList),
            (&kb.view_icon, Action::ViewIcon),
            (&kb.view_detail, Action::ViewDetail),
            (&kb.vault_add, Action::VaultAdd),
        ];

        let mut forward: HashMap<Action, KeyCombo> = HashMap::new();
        let mut reverse: HashMap<KeyCombo, Action> = HashMap::new();

        for (binding_str, action) in pairs {
            match KeyCombo::parse(binding_str) {
                Some(combo) => {
                    if let Some(existing) = reverse.get(&combo) {
                        tracing::warn!(
                            binding = %binding_str,
                            action_a = ?existing,
                            action_b = ?action,
                            "keybind conflict: same combo assigned to two actions"
                        );
                    } else {
                        forward.insert(*action, combo);
                        reverse.insert(combo, *action);
                    }
                }
                None => {
                    tracing::warn!(
                        binding = %binding_str,
                        action = ?action,
                        "keybind: could not parse binding string"
                    );
                }
            }
        }

        Self { forward, reverse }
    }

    /// Look up which action corresponds to a key event, if any.
    #[must_use]
    pub fn action_for(&self, key: gdk::Key, mods: gdk::ModifierType) -> Option<Action> {
        // Normalise: strip Lock/Mod2 (NumLock) masks that GTK sometimes includes.
        let clean_mods = mods
            & (gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::SUPER_MASK);
        // Try both the keyval as-is and its lowercase equivalent, so Ctrl+T and
        // Ctrl+t from the config both match regardless of Shift state.
        let combo = KeyCombo { mods: clean_mods, key };
        if let Some(a) = self.reverse.get(&combo) {
            return Some(*a);
        }
        // Try lowercase variant (for letter keys where Shift may send uppercase).
        let lower = key.to_lower();
        if lower != key {
            let combo_lower = KeyCombo { mods: clean_mods, key: lower };
            if let Some(a) = self.reverse.get(&combo_lower) {
                return Some(*a);
            }
        }
        None
    }

    /// Return the combo bound to `action`, or `None` if unbound.
    ///
    /// Used by the settings dialog keybind table to display current bindings.
    #[must_use]
    #[allow(dead_code)]
    pub fn combo_for(&self, action: Action) -> Option<KeyCombo> {
        self.forward.get(&action).copied()
    }
}

/// Translate a key name string to a `gdk::Key`.
fn parse_key_name(name: &str) -> Option<gdk::Key> {
    let key = match name {
        "A" | "a" => gdk::Key::a,
        "B" | "b" => gdk::Key::b,
        "C" | "c" => gdk::Key::c,
        "D" | "d" => gdk::Key::d,
        "E" | "e" => gdk::Key::e,
        "F" | "f" => gdk::Key::f,
        "G" | "g" => gdk::Key::g,
        "H" | "h" => gdk::Key::h,
        "I" | "i" => gdk::Key::i,
        "J" | "j" => gdk::Key::j,
        "K" | "k" => gdk::Key::k,
        "L" | "l" => gdk::Key::l,
        "M" | "m" => gdk::Key::m,
        "N" | "n" => gdk::Key::n,
        "O" | "o" => gdk::Key::o,
        "P" | "p" => gdk::Key::p,
        "Q" | "q" => gdk::Key::q,
        "R" | "r" => gdk::Key::r,
        "S" | "s" => gdk::Key::s,
        "T" | "t" => gdk::Key::t,
        "U" | "u" => gdk::Key::u,
        "V" | "v" => gdk::Key::v,
        "W" | "w" => gdk::Key::w,
        "X" | "x" => gdk::Key::x,
        "Y" | "y" => gdk::Key::y,
        "Z" | "z" => gdk::Key::z,
        "0" => gdk::Key::_0,
        "1" => gdk::Key::_1,
        "2" => gdk::Key::_2,
        "3" => gdk::Key::_3,
        "4" => gdk::Key::_4,
        "5" => gdk::Key::_5,
        "6" => gdk::Key::_6,
        "7" => gdk::Key::_7,
        "8" => gdk::Key::_8,
        "9" => gdk::Key::_9,
        "Left" => gdk::Key::Left,
        "Right" => gdk::Key::Right,
        "Up" => gdk::Key::Up,
        "Down" => gdk::Key::Down,
        "Tab" => gdk::Key::Tab,
        "Return" | "Enter" => gdk::Key::Return,
        "Escape" | "Esc" => gdk::Key::Escape,
        "Space" => gdk::Key::space,
        "BackSpace" | "Backspace" => gdk::Key::BackSpace,
        "Delete" | "Del" => gdk::Key::Delete,
        "Insert" => gdk::Key::Insert,
        "Home" => gdk::Key::Home,
        "End" => gdk::Key::End,
        "Page_Up" | "PageUp" => gdk::Key::Page_Up,
        "Page_Down" | "PageDown" => gdk::Key::Page_Down,
        "F1" => gdk::Key::F1,
        "F2" => gdk::Key::F2,
        "F3" => gdk::Key::F3,
        "F4" => gdk::Key::F4,
        "F5" => gdk::Key::F5,
        "F6" => gdk::Key::F6,
        "F7" => gdk::Key::F7,
        "F8" => gdk::Key::F8,
        "F9" => gdk::Key::F9,
        "F10" => gdk::Key::F10,
        "F11" => gdk::Key::F11,
        "F12" => gdk::Key::F12,
        _ => return None,
    };
    Some(key)
}

/// Translate a `gdk::Key` back to its display name.
fn key_to_name(key: gdk::Key) -> Option<&'static str> {
    let name = match key {
        gdk::Key::a => "A",
        gdk::Key::b => "B",
        gdk::Key::c => "C",
        gdk::Key::d => "D",
        gdk::Key::e => "E",
        gdk::Key::f => "F",
        gdk::Key::g => "G",
        gdk::Key::h => "H",
        gdk::Key::i => "I",
        gdk::Key::j => "J",
        gdk::Key::k => "K",
        gdk::Key::l => "L",
        gdk::Key::m => "M",
        gdk::Key::n => "N",
        gdk::Key::o => "O",
        gdk::Key::p => "P",
        gdk::Key::q => "Q",
        gdk::Key::r => "R",
        gdk::Key::s => "S",
        gdk::Key::t => "T",
        gdk::Key::u => "U",
        gdk::Key::v => "V",
        gdk::Key::w => "W",
        gdk::Key::x => "X",
        gdk::Key::y => "Y",
        gdk::Key::z => "Z",
        gdk::Key::_0 => "0",
        gdk::Key::_1 => "1",
        gdk::Key::_2 => "2",
        gdk::Key::_3 => "3",
        gdk::Key::_4 => "4",
        gdk::Key::_5 => "5",
        gdk::Key::_6 => "6",
        gdk::Key::_7 => "7",
        gdk::Key::_8 => "8",
        gdk::Key::_9 => "9",
        gdk::Key::Left => "Left",
        gdk::Key::Right => "Right",
        gdk::Key::Up => "Up",
        gdk::Key::Down => "Down",
        gdk::Key::Tab => "Tab",
        gdk::Key::Return => "Return",
        gdk::Key::Escape => "Escape",
        gdk::Key::space => "Space",
        gdk::Key::BackSpace => "BackSpace",
        gdk::Key::Delete => "Delete",
        gdk::Key::Insert => "Insert",
        gdk::Key::Home => "Home",
        gdk::Key::End => "End",
        gdk::Key::Page_Up => "Page_Up",
        gdk::Key::Page_Down => "Page_Down",
        gdk::Key::F1 => "F1",
        gdk::Key::F2 => "F2",
        gdk::Key::F3 => "F3",
        gdk::Key::F4 => "F4",
        gdk::Key::F5 => "F5",
        gdk::Key::F6 => "F6",
        gdk::Key::F7 => "F7",
        gdk::Key::F8 => "F8",
        gdk::Key::F9 => "F9",
        gdk::Key::F10 => "F10",
        gdk::Key::F11 => "F11",
        gdk::Key::F12 => "F12",
        _ => return None,
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ctrl_t() {
        let c = KeyCombo::parse("Ctrl+T").unwrap();
        assert_eq!(c.key, gdk::Key::t);
        assert!(c.mods.contains(gdk::ModifierType::CONTROL_MASK));
        assert!(!c.mods.contains(gdk::ModifierType::ALT_MASK));
    }

    #[test]
    fn parse_alt_left() {
        let c = KeyCombo::parse("Alt+Left").unwrap();
        assert_eq!(c.key, gdk::Key::Left);
        assert!(c.mods.contains(gdk::ModifierType::ALT_MASK));
    }

    #[test]
    fn parse_bare_f3() {
        let c = KeyCombo::parse("F3").unwrap();
        assert_eq!(c.key, gdk::Key::F3);
        assert!(c.mods.is_empty());
    }

    #[test]
    fn parse_ctrl_shift_v() {
        let c = KeyCombo::parse("Ctrl+Shift+V").unwrap();
        assert_eq!(c.key, gdk::Key::v);
        assert!(c.mods.contains(gdk::ModifierType::CONTROL_MASK));
        assert!(c.mods.contains(gdk::ModifierType::SHIFT_MASK));
    }

    #[test]
    fn display_round_trips() {
        let c = KeyCombo::parse("Ctrl+T").unwrap();
        assert_eq!(c.to_display_string(), "Ctrl+T");
        let c2 = KeyCombo::parse("Alt+Left").unwrap();
        assert_eq!(c2.to_display_string(), "Alt+Left");
    }

    #[test]
    fn unknown_modifier_returns_none() {
        assert!(KeyCombo::parse("Win+T").is_none());
    }

    #[test]
    fn action_map_from_defaults() {
        let kb = Keybinds::default();
        let map = ActionMap::from_keybinds(&kb);
        let combo = map.combo_for(Action::NewTab).unwrap();
        assert_eq!(combo.key, gdk::Key::t);
        assert!(combo.mods.contains(gdk::ModifierType::CONTROL_MASK));
    }
}
