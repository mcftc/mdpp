//! UI translations (PLAN §4.4). English is written in the code and the data files; a locale file
//! (`assets/locales/<code>.toml`) maps each English text to its translation. A missing entry
//! shows the English text, so a partial translation is never broken. Placeholders are `{name}`.
//!
//! The language is the `language` setting: `auto` follows the system, or a locale code. It is
//! chosen once at startup.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Locales shipped with MD++: code → file. A new language is a file here and a row here.
const LOCALES: &[(&str, &str)] = &[("tr", include_str!("../../../assets/locales/tr.toml"))];
/// The language of the code and data files.
pub const SOURCE_LANGUAGE: &str = "en";
/// The `language` setting's value for "follow the system".
pub const AUTO: &str = "auto";

static ACTIVE: OnceLock<HashMap<String, String>> = OnceLock::new();

/// Every language the `language` setting accepts.
pub fn languages() -> impl Iterator<Item = &'static str> {
    [AUTO, SOURCE_LANGUAGE]
        .into_iter()
        .chain(LOCALES.iter().map(|(code, _)| *code))
}

/// Choose the language for this run (later calls change nothing). Returns the locale code used.
pub fn init(setting: &str) -> &'static str {
    let wanted = if setting == AUTO {
        sys_locale::get_locale().unwrap_or_default()
    } else {
        setting.to_string()
    };
    // `tr-TR`, `tr_TR.UTF-8` → `tr`.
    let base = wanted
        .split(['-', '_', '.'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let found = LOCALES.iter().find(|(code, _)| *code == base);
    let table = found.map(|(_, text)| parse(text)).unwrap_or_default();
    let _ = ACTIVE.set(table);
    found.map_or(SOURCE_LANGUAGE, |(code, _)| code)
}

fn parse(text: &str) -> HashMap<String, String> {
    match toml::from_str::<toml::Table>(text) {
        Ok(table) => table
            .into_iter()
            .filter_map(|(english, value)| Some((english, value.as_str()?.to_string())))
            .collect(),
        Err(error) => {
            eprintln!("mdpp: a locale file is not valid TOML: {error}");
            HashMap::new()
        }
    }
}

/// `english` in the chosen language.
pub fn t(english: &str) -> &str {
    ACTIVE
        .get()
        .and_then(|table| table.get(english))
        .map_or(english, String::as_str)
}

/// [`t`], then each `{name}` replaced by its value.
pub fn tf(english: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = t(english).to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_files_parse_and_keep_placeholders() {
        for (code, text) in LOCALES {
            let table: toml::Table = toml::from_str(text).unwrap_or_else(|e| panic!("{code}: {e}"));
            for (english, translated) in &table {
                let translated = translated.as_str().expect("translations are strings");
                let names = |s: &str| {
                    let mut found: Vec<String> = s
                        .split('{')
                        .skip(1)
                        .filter_map(|part| part.split_once('}').map(|(n, _)| n.to_string()))
                        .collect();
                    found.sort();
                    found
                };
                assert_eq!(names(english), names(translated), "{code}: {english}");
                assert!(!translated.trim().is_empty(), "{code}: {english}");
            }
        }
        assert!(languages().any(|l| l == "tr"));
    }

    #[test]
    fn untranslated_text_stays_english() {
        // Without init, nothing is translated.
        assert_eq!(t("Something new"), "Something new");
        assert_eq!(tf("{n} files", &[("n", &3)]), "3 files");
    }
}
