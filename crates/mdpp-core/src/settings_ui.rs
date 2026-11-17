//! What the settings window shows, and how it writes a change (PLAN §6.1a). Metadata-driven:
//! every setting's name, default and description come from `default-settings.toml` (its comments),
//! the pages, groups and control limits from `settings-ui.toml`. Nothing here lists a setting.
//!
//! A change is written into the user's `settings.toml` with `toml_edit`, keeping their comments
//! and layout; setting a value back to its default removes the line, so the file keeps naming
//! only what differs. The settings watcher then applies it like a hand edit.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::settings::DEFAULT_SETTINGS;

pub const SETTINGS_UI: &str = include_str!("../../../assets/settings-ui.toml");

/// One page of the settings window.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub title: String,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub title: String,
    pub entries: Vec<Entry>,
}

/// One setting as the window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Dotted name: `font_size`, `diagnostics.missing-anchor`.
    pub key: String,
    pub title: String,
    pub description: String,
    pub default: toml::Value,
    pub control: Control,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Switch,
    Number {
        min: f64,
        max: f64,
        step: f64,
    },
    Choice(Vec<String>),
    /// A string, or a list of strings edited as space-separated words.
    Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiFile {
    page: Vec<PageSpec>,
    #[serde(default)]
    controls: HashMap<String, ControlSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageSpec {
    title: String,
    group: Vec<GroupSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupSpec {
    title: String,
    settings: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlSpec {
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    choices: Option<Vec<String>>,
}

/// The settings window's pages, every setting placed (checked by a test).
pub static PAGES: LazyLock<Vec<Page>> = LazyLock::new(build_pages);

fn build_pages() -> Vec<Page> {
    let ui: UiFile =
        toml::from_str(SETTINGS_UI).expect("assets/settings-ui.toml is valid (tested)");
    let defaults: toml::Table = toml::from_str(DEFAULT_SETTINGS).expect("defaults parse");
    let descriptions = descriptions(DEFAULT_SETTINGS);
    let entry = |key: String, control_key: &str| {
        let default = lookup(&defaults, &key)?.clone();
        let spec = ui.controls.get(control_key);
        let control = control_for(&default, spec);
        Some(Entry {
            title: title(&key),
            description: descriptions.get(&key).cloned().unwrap_or_default(),
            key,
            default,
            control,
        })
    };
    ui.page
        .iter()
        .map(|page| Page {
            title: page.title.clone(),
            groups: page
                .group
                .iter()
                .map(|group| Group {
                    title: group.title.clone(),
                    entries: group
                        .settings
                        .iter()
                        .flat_map(|name| match name.strip_suffix(".*") {
                            // Every entry of a table, in the defaults file's order.
                            Some(table) => match defaults.get(table) {
                                Some(toml::Value::Table(entries)) => entries
                                    .keys()
                                    .filter_map(|key| entry(format!("{table}.{key}"), name))
                                    .collect(),
                                _ => Vec::new(),
                            },
                            None => entry(name.clone(), name).into_iter().collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

fn control_for(default: &toml::Value, spec: Option<&ControlSpec>) -> Control {
    let spec_or_default = ControlSpec::default();
    let spec = spec.unwrap_or(&spec_or_default);
    if let Some(choices) = &spec.choices {
        return Control::Choice(choices.clone());
    }
    match default {
        toml::Value::Boolean(_) => Control::Switch,
        toml::Value::Integer(_) | toml::Value::Float(_) => Control::Number {
            min: spec.min.unwrap_or(0.0),
            max: spec.max.unwrap_or(f64::from(u32::MAX)),
            step: spec.step.unwrap_or(1.0),
        },
        _ => Control::Text,
    }
}

/// `cursor_blink_interval_ms` → "Cursor blink interval (ms)"; `missing-anchor` → "Missing anchor".
fn title(key: &str) -> String {
    let name = key.rsplit('.').next().unwrap_or(key);
    let (name, unit) = match name.strip_suffix("_ms") {
        Some(name) => (name, " (ms)"),
        None => (name, ""),
    };
    let words = name.replace(['_', '-'], " ");
    let mut chars = words.chars();
    let first = chars.next().map(|c| c.to_uppercase().to_string());
    format!("{}{}{unit}", first.unwrap_or_default(), chars.as_str())
}

/// The comment above each setting of the defaults file, by dotted name. Settings written right
/// below another one, with no comment of their own, share its comment (window width and height).
fn descriptions(defaults: &str) -> HashMap<String, String> {
    let body = defaults
        .split_once("\n\n")
        .map_or(defaults, |(_, body)| body);
    let mut out = HashMap::new();
    let mut table = String::new();
    let mut comment: Vec<&str> = Vec::new();
    let mut after_key = false;
    for line in body.lines().map(str::trim) {
        if line.is_empty() {
            comment.clear();
            after_key = false;
        } else if let Some(text) = line.strip_prefix('#') {
            if after_key {
                comment.clear();
                after_key = false;
            }
            comment.push(text.trim());
        } else if let Some(name) = line.strip_prefix('[') {
            table = format!("{}.", name.trim_end_matches(']').trim());
            comment.clear();
        } else if let Some((key, _)) = line.split_once('=') {
            out.insert(format!("{table}{}", key.trim()), comment.join(" "));
            after_key = true;
        }
    }
    out
}

/// The value of a dotted name in a settings table.
pub fn lookup<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let mut parts = key.split('.');
    let mut value = table.get(parts.next()?)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

/// `user` (a settings.toml) with `key` set to `value`; the line is removed when `value` is the
/// default. Comments and the order of everything else are kept.
pub fn write_value(user: &str, key: &str, value: &toml::Value) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = user
        .parse()
        .map_err(|error| format!("settings.toml is not valid TOML: {error}"))?;
    let defaults: toml::Table = toml::from_str(DEFAULT_SETTINGS).expect("defaults parse");
    let is_default = lookup(&defaults, key) == Some(value);
    let parts: Vec<&str> = key.split('.').collect();
    let (last, tables) = parts.split_last().ok_or("empty setting name")?;
    let mut table = doc.as_table_mut();
    for part in tables {
        if is_default && !table.contains_key(part) {
            return Ok(doc.to_string()); // nothing to remove
        }
        table = table
            .entry(part)
            .or_insert_with(toml_edit::table)
            .as_table_mut()
            .ok_or_else(|| format!("`{part}` in settings.toml is not a table"))?;
    }
    if is_default {
        table.remove(last);
    } else {
        table.insert(last, toml_edit::value(edit_value(value)?));
    }
    Ok(doc.to_string())
}

fn edit_value(value: &toml::Value) -> Result<toml_edit::Value, String> {
    Ok(match value {
        toml::Value::String(s) => s.as_str().into(),
        toml::Value::Integer(i) => (*i).into(),
        toml::Value::Float(f) => (*f).into(),
        toml::Value::Boolean(b) => (*b).into(),
        toml::Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(edit_value(item)?);
            }
            array.into()
        }
        other => return Err(format!("cannot write {other} into settings.toml")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Level, Settings};

    #[test]
    fn every_setting_is_placed_once_with_a_description() {
        let defaults: toml::Table = toml::from_str(DEFAULT_SETTINGS).unwrap();
        let mut expected = Vec::new();
        for (key, value) in &defaults {
            match value {
                toml::Value::Table(table) => {
                    expected.extend(table.keys().map(|inner| format!("{key}.{inner}")))
                }
                _ => expected.push(key.clone()),
            }
        }
        let mut placed: Vec<&Entry> = PAGES
            .iter()
            .flat_map(|page| page.groups.iter().flat_map(|group| &group.entries))
            .collect();
        placed.sort_by(|a, b| a.key.cmp(&b.key));
        expected.sort();
        let keys: Vec<&str> = placed.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(
            keys, expected,
            "settings-ui.toml must place every setting once"
        );
        for entry in placed {
            assert!(
                !entry.description.is_empty(),
                "{} has no description",
                entry.key
            );
        }
    }

    #[test]
    fn controls_follow_the_default_values() {
        let find = |key: &str| {
            PAGES
                .iter()
                .flat_map(|p| p.groups.iter().flat_map(|g| &g.entries))
                .find(|e| e.key == key)
                .unwrap()
                .clone()
        };
        assert_eq!(find("soft_wrap").control, Control::Switch);
        assert_eq!(find("auto_close_pairs").control, Control::Text);
        assert!(
            matches!(find("markup_fade").control, Control::Number { step, .. } if step == 0.05)
        );
        let level = find("diagnostics.missing-anchor");
        assert_eq!(level.title, "Missing anchor");
        assert_eq!(find("hot_exit_interval_ms").title, "Hot exit interval (ms)");
        assert_eq!(
            find("window_height").description,
            find("window_width").description
        );
        // Every choice offered for a diagnostic is a level the settings accept.
        let Control::Choice(choices) = level.control else {
            panic!("diagnostics are choices")
        };
        for choice in choices {
            toml::Value::String(choice)
                .try_into::<Level>()
                .expect("a valid level");
        }
    }

    #[test]
    fn writing_keeps_comments_and_removes_defaults() {
        let user = "# my settings\nsoft_wrap = false # I like it long\n\n[diagnostics]\nmissing-file = \"off\"\n";
        let out = write_value(user, "font_size", &toml::Value::Integer(15)).unwrap();
        assert!(out.contains("# my settings") && out.contains("# I like it long"));
        assert!(Settings::load(&out).settings.font_size == 15.0);
        let out = write_value(&out, "diagnostics.missing-anchor", &"error".into()).unwrap();
        let loaded = Settings::load(&out);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.settings.diagnostics["missing-anchor"], Level::Error);
        assert_eq!(loaded.settings.diagnostics["missing-file"], Level::Off);
        // Back to the default: the line goes away.
        let out = write_value(&out, "soft_wrap", &toml::Value::Boolean(true)).unwrap();
        assert!(!out.contains("soft_wrap"));
        let out = write_value("", "diagnostics.missing-file", &"warning".into()).unwrap();
        assert_eq!(out, "");
        assert!(write_value("x = [", "font_size", &toml::Value::Integer(1)).is_err());
    }
}
