//! Lightweight UI localization (Phase 7.x, brought forward).
//!
//! Translations are embedded as flat TOML tables (one per language) and loaded
//! once at startup into a process-global string map. UI code calls [`t`] with a
//! dotted key; an unknown key returns the key itself, so a missing translation
//! is visible but never panics. Simple `{placeholder}` substitution is provided
//! by [`tf`].
//!
//! The active language is chosen from the config `[general] language` value and
//! cannot change at runtime without rebuilding widgets; switching it in the UI
//! rewrites the config and takes effect on the next launch.

use std::collections::HashMap;
use std::sync::OnceLock;

/// A supported interface language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// Turkish (default).
    #[default]
    Turkish,
    /// English.
    English,
}

impl Lang {
    /// Parse a language from a config code (`"tr"`, `"en"`, …); unknown codes
    /// fall back to the default (Turkish).
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en_us" | "english" => Lang::English,
            _ => Lang::Turkish,
        }
    }

    /// The config code for this language.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Lang::Turkish => "tr",
            Lang::English => "en",
        }
    }

    /// The embedded translation table for this language.
    fn table(self) -> &'static str {
        match self {
            Lang::Turkish => include_str!("../resources/i18n/tr.toml"),
            Lang::English => include_str!("../resources/i18n/en.toml"),
        }
    }
}

/// Process-global string table, populated once by [`init`].
static STRINGS: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Load the translation table for `lang`. Safe to call once at startup; a second
/// call is ignored (the first language wins).
pub fn init(lang: Lang) {
    let map: HashMap<String, String> = toml::from_str(lang.table()).unwrap_or_default();
    let _ = STRINGS.set(map);
}

/// Translate a dotted key. Unknown keys (or use before [`init`]) return the key.
#[must_use]
pub fn t(key: &str) -> String {
    STRINGS
        .get()
        .and_then(|m| m.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

/// Translate `key`, then replace each `{name}` placeholder with its argument.
#[must_use]
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut out = t(key);
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_code_roundtrip() {
        assert_eq!(Lang::from_code("en"), Lang::English);
        assert_eq!(Lang::from_code("TR"), Lang::Turkish);
        assert_eq!(Lang::from_code("zzz"), Lang::Turkish);
        assert_eq!(Lang::English.code(), "en");
    }

    #[test]
    fn embedded_tables_parse_and_align() {
        let tr: HashMap<String, String> = toml::from_str(Lang::Turkish.table()).unwrap();
        let en: HashMap<String, String> = toml::from_str(Lang::English.table()).unwrap();
        assert!(tr.contains_key("home.title"));
        // Both languages must define the same key set, or the UI shows raw keys.
        let mut tr_keys: Vec<&String> = tr.keys().collect();
        let mut en_keys: Vec<&String> = en.keys().collect();
        tr_keys.sort();
        en_keys.sort();
        assert_eq!(tr_keys, en_keys, "tr.toml and en.toml key sets differ");
    }

    #[test]
    fn tf_substitutes_placeholders() {
        // `init` is process-global and idempotent; this is the only test that
        // depends on a loaded table, so seeding Turkish here is safe.
        init(Lang::Turkish);
        let out = tf("status.items", &[("n", "5")]);
        assert!(out.contains('5'), "got: {out}");
    }
}
