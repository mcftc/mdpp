//! Settings (PLAN D9). The defaults live in one place only — `assets/default-settings.toml`,
//! embedded in the binary and documented line by line. A user's `settings.toml` contains just the
//! lines they change: its keys are laid over the defaults, unknown keys are reported, and the
//! complete result is deserialized. No default value is repeated in Rust code.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Deserializer};

/// The shipped defaults, also what "show all settings" displays.
pub const DEFAULT_SETTINGS: &str = include_str!("../../../assets/default-settings.toml");

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(rename = "cursor_blink_interval_ms", deserialize_with = "millis")]
    pub cursor_blink_interval: Duration,
    #[serde(rename = "cursor_blink_idle_timeout_ms", deserialize_with = "millis")]
    pub cursor_blink_idle_timeout: Duration,
    #[serde(rename = "structure_debounce_ms", deserialize_with = "millis")]
    pub structure_debounce: Duration,
    pub soft_wrap: bool,
    pub soft_wrap_max_bytes: usize,
    pub line_numbers: bool,
    pub font_size: f32,
    pub markup_fade: f32,
    pub code_highlight_max_lines: usize,
    /// Two-character strings: opening and closing character.
    pub auto_close_pairs: Vec<String>,
    pub hot_exit: bool,
    #[serde(rename = "hot_exit_interval_ms", deserialize_with = "millis")]
    pub hot_exit_interval: Duration,
    #[serde(rename = "file_watch_debounce_ms", deserialize_with = "millis")]
    pub file_watch_debounce: Duration,
    pub recent_files_max: usize,
    pub workspace_max_files: usize,
    pub search_max_file_bytes: u64,
    pub assets_dir_template: String,
    pub window_width: f32,
    pub window_height: f32,
    pub menu_bar: bool,
    pub focus_mode_width: f32,
    pub language: String,
    pub outline_on_start: bool,
    pub outline_width: f32,
    pub reading_words_per_minute: u32,
    pub preview_on_start: bool,
    pub preview_max_width: f32,
    pub preview_remote_images: bool,
    /// Rule id → how it is shown.
    pub diagnostics: BTreeMap<String, Level>,
}

/// How a diagnostic rule is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Off,
    Hint,
    Info,
    Warning,
    Error,
}

impl Default for Settings {
    fn default() -> Self {
        toml::from_str(DEFAULT_SETTINGS).expect("assets/default-settings.toml is valid (tested)")
    }
}

/// Result of reading a user's settings: always usable, plus what was wrong with the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub settings: Settings,
    pub warnings: Vec<String>,
}

impl Settings {
    /// Lay `user` (the contents of a settings.toml) over the defaults. A broken file never stops
    /// the editor: unparsable files and bad values fall back to the defaults with a warning.
    pub fn load(user: &str) -> Loaded {
        let (merged, mut warnings) = merged(user);
        let settings =
            toml::Value::Table(merged)
                .try_into()
                .unwrap_or_else(|error: toml::de::Error| {
                    warnings.push(format!(
                        "settings.toml has a bad value, using defaults: {error}"
                    ));
                    Settings::default()
                });
        Loaded { settings, warnings }
    }
}

/// Every setting's current value: `user` laid over the defaults, as a table (what the settings
/// window reads), with what was wrong with the user's file.
pub fn merged(user: &str) -> (toml::Table, Vec<String>) {
    let mut warnings = Vec::new();
    let mut merged: toml::Table = toml::from_str(DEFAULT_SETTINGS).expect("defaults parse");
    match toml::from_str::<toml::Table>(user) {
        Ok(table) => merge(&mut merged, table, "", &mut warnings),
        Err(error) => warnings.push(format!("settings.toml is not valid TOML: {error}")),
    }
    (merged, warnings)
}

/// A first `settings.toml` for the user: every default, commented out, under a short guide.
pub fn user_template() -> String {
    format!(
        "# MD++ settings. Remove the # in front of a setting and change its value; MD++ applies\n\
         # the change when you save this file. These are the defaults.\n\n{}",
        comment_out(DEFAULT_SETTINGS)
    )
}

/// The body of a shipped defaults file (its header comment dropped) with every value line
/// commented out, so a user file can show the defaults without setting any of them.
pub(crate) fn comment_out(defaults: &str) -> String {
    let body = defaults
        .split_once("\n\n")
        .map_or(defaults, |(_, body)| body);
    body.lines()
        .map(|line| {
            if line.is_empty() || line.starts_with('#') {
                format!("{line}\n")
            } else {
                format!("#{line}\n")
            }
        })
        .collect()
}

/// Lay `user` over `base`, key by key; tables (like `[diagnostics]`) merge entry by entry so a
/// user file names only what it changes. Keys the defaults do not have are reported.
fn merge(base: &mut toml::Table, user: toml::Table, path: &str, warnings: &mut Vec<String>) {
    for (key, value) in user {
        let name = format!("{path}{key}");
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(inner)), toml::Value::Table(value)) => {
                merge(inner, value, &format!("{name}."), warnings)
            }
            (Some(slot), value) => *slot = value,
            (None, _) => warnings.push(format!("unknown setting `{name}` ignored")),
        }
    }
}

fn millis<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    u64::deserialize(deserializer).map(Duration::from_millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_complete_and_valid() {
        // `deny_unknown_fields` + no field defaults: every field must be in the file, and only those.
        let settings = Settings::default();
        assert_eq!(settings.structure_debounce, Duration::from_millis(120));
        assert!(settings.soft_wrap);
    }

    #[test]
    fn user_values_override_defaults() {
        let loaded = Settings::load("soft_wrap = false\nstructure_debounce_ms = 300\n");
        assert!(loaded.warnings.is_empty());
        assert!(!loaded.settings.soft_wrap);
        assert_eq!(
            loaded.settings.structure_debounce,
            Duration::from_millis(300)
        );
        assert_eq!(
            loaded.settings.line_numbers,
            Settings::default().line_numbers
        );
    }

    #[test]
    fn the_user_template_sets_nothing() {
        let template = user_template();
        assert!(template.contains("#soft_wrap = true"));
        let loaded = Settings::load(&template);
        assert!(loaded.warnings.is_empty());
        assert_eq!(loaded.settings, Settings::default());
    }

    #[test]
    fn tables_merge_entry_by_entry() {
        let loaded = Settings::load("[diagnostics]\nmissing-file = \"error\"\n");
        assert!(loaded.warnings.is_empty());
        let rules = &loaded.settings.diagnostics;
        assert_eq!(rules["missing-file"], Level::Error);
        assert_eq!(
            rules["missing-anchor"],
            Settings::default().diagnostics["missing-anchor"]
        );
    }

    #[test]
    fn problems_are_reported_not_fatal() {
        let loaded = Settings::load("soft_wraps = true\n");
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(
            loaded.warnings,
            vec!["unknown setting `soft_wraps` ignored".to_string()]
        );

        let loaded = Settings::load("soft_wrap = \"yes\"\n");
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.warnings.len(), 1);

        let loaded = Settings::load("[diagnostics]\nno-such-rule = \"error\"\n");
        assert_eq!(
            loaded.warnings,
            vec!["unknown setting `diagnostics.no-such-rule` ignored".to_string()]
        );

        let loaded = Settings::load("this is not toml");
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.warnings.len(), 1);
    }
}
