//! The settings window (PLAN §6.1a), like Zed's: search, a page per category, one row per setting
//! with its description and a control. gpui-component's settings view over
//! `mdpp_core::settings_ui`, which supplies every page, title, description, default and control
//! from metadata — nothing here names a setting.
//!
//! A change is written into the user's settings.toml (comments kept; a default value removes the
//! line). The settings watcher then applies it to the editor exactly like a hand edit, so there is
//! one way settings change.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mdpp_core::settings::merged;
use mdpp_core::settings_ui::{Control, Entry, PAGES, lookup, write_value};

/// Something the main window does for this one: open the user's settings.toml ("Edit in
/// settings.toml"), or apply a change at once (the config folder may not be watched yet).
pub type MainWindowCall = Rc<dyn Fn(&mut App)>;

pub struct SettingsWindow {
    /// The user's settings.toml; `None` when the platform has no config folder.
    path: Option<PathBuf>,
    /// Every setting's current value (user file over the defaults), shared with the fields.
    values: Rc<RefCell<toml::Table>>,
    /// Problems with the user's file, or with the last write.
    problems: Rc<RefCell<Vec<String>>>,
    edit_file: MainWindowCall,
    on_change: MainWindowCall,
}

impl SettingsWindow {
    pub fn new(
        path: Option<PathBuf>,
        edit_file: MainWindowCall,
        on_change: MainWindowCall,
    ) -> Self {
        let this = Self {
            path,
            values: Rc::default(),
            problems: Rc::default(),
            edit_file,
            on_change,
        };
        this.reload();
        this
    }

    /// Read the user's file again (after it changed on disk).
    pub fn reload(&self) {
        let (values, problems) = merged(&self.user_text());
        *self.values.borrow_mut() = values;
        *self.problems.borrow_mut() = problems;
    }

    fn user_text(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default()
    }

    /// Write one setting into the user's file and show it at once.
    fn writer(&self) -> impl Fn(&str, toml::Value) + use<> {
        let (path, values, problems) = (
            self.path.clone(),
            self.values.clone(),
            self.problems.clone(),
        );
        move |key, value| {
            let Some(path) = &path else {
                problems
                    .borrow_mut()
                    .push("No settings folder on this system: changes cannot be kept.".into());
                return;
            };
            let user = std::fs::read_to_string(path).unwrap_or_default();
            let written = write_value(&user, key, &value).and_then(|text| {
                if let Some(folder) = path.parent() {
                    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
                }
                mdpp_core::file::save(path, text.as_bytes())
                    .map(|_| text)
                    .map_err(|e| format!("Could not write {}: {e}", path.display()))
            });
            match written {
                Ok(text) => {
                    let (merged, found) = merged(&text);
                    *values.borrow_mut() = merged;
                    *problems.borrow_mut() = found;
                }
                Err(problem) => problems.borrow_mut().push(problem),
            }
        }
    }

    /// Changes one setting as its control would (used by the end-to-end check).
    pub fn setter(&self) -> impl Fn(&str, toml::Value, &mut App) + use<> {
        let (write, on_change) = (self.writer(), self.on_change.clone());
        move |key, value, cx| {
            write(key, value);
            on_change(cx);
        }
    }

    fn item(&self, entry: &Entry) -> SettingItem {
        let write = Rc::new(self.writer());
        let on_change = self.on_change.clone();
        let values = self.values.clone();
        let key: Rc<str> = entry.key.as_str().into();
        let current = {
            let (values, key) = (values.clone(), key.clone());
            move || {
                lookup(&values.borrow(), &key)
                    .cloned()
                    .unwrap_or(toml::Value::Boolean(false))
            }
        };
        let set = {
            let key = key.clone();
            move |value: toml::Value| write(&key, value)
        };
        let item = match &entry.control {
            Control::Switch => {
                let on_change = on_change.clone();
                let default = entry.default.as_bool().unwrap_or_default();
                SettingItem::new(
                    entry.title.clone(),
                    SettingField::switch(
                        move |_| current().as_bool().unwrap_or_default(),
                        move |on, cx| {
                            set(toml::Value::Boolean(on));
                            on_change(cx);
                        },
                    )
                    .default_value(default),
                )
            }
            Control::Number { min, max, step } => {
                let on_change = on_change.clone();
                let integer = entry.default.is_integer();
                let default = number(&entry.default);
                SettingItem::new(
                    entry.title.clone(),
                    SettingField::number_input(
                        NumberFieldOptions {
                            min: *min,
                            max: *max,
                            step: *step,
                        },
                        move |_| number(&current()),
                        move |value, cx| {
                            set(if integer {
                                toml::Value::Integer(value.round() as i64)
                            } else {
                                toml::Value::Float(value)
                            });
                            on_change(cx);
                        },
                    )
                    .default_value(default),
                )
            }
            Control::Choice(choices) => {
                let on_change = on_change.clone();
                let options = choices
                    .iter()
                    .map(|c| (SharedString::from(c.clone()), SharedString::from(c.clone())))
                    .collect();
                let default = text(&entry.default);
                SettingItem::new(
                    entry.title.clone(),
                    SettingField::dropdown(
                        options,
                        move |_| text(&current()).into(),
                        move |choice: SharedString, cx| {
                            set(toml::Value::String(choice.to_string()));
                            on_change(cx);
                        },
                    )
                    .default_value(default),
                )
            }
            Control::Text => {
                let on_change = on_change.clone();
                let list = entry.default.is_array();
                let default = text(&entry.default);
                SettingItem::new(
                    entry.title.clone(),
                    SettingField::input(
                        move |_| text(&current()).into(),
                        move |typed: SharedString, cx| {
                            set(if list {
                                toml::Value::Array(
                                    typed
                                        .split_whitespace()
                                        .map(|word| toml::Value::String(word.into()))
                                        .collect(),
                                )
                            } else {
                                toml::Value::String(typed.to_string())
                            });
                            on_change(cx);
                        },
                    )
                    .default_value(default),
                )
            }
        };
        item.description(entry.description.clone())
            .keywords([entry.key.clone()])
    }
}

/// A number setting as the field shows it.
fn number(value: &toml::Value) -> f64 {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|i| i as f64))
        .unwrap_or_default()
}

/// A text setting as the field shows it: a string, or a list as space-separated words.
fn text(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(items) => items
            .iter()
            .filter_map(toml::Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let problems = self.problems.borrow().clone();
        let edit_file = self.edit_file.clone();
        let pages = PAGES.iter().map(|page| {
            let edit_file = edit_file.clone();
            SettingPage::new(page.title.clone())
                .title_suffix(move |_, _| {
                    let edit_file = edit_file.clone();
                    Button::new("edit-settings-file")
                        .label("Edit in settings.toml")
                        .outline()
                        .small()
                        .on_click(move |_, _, cx| edit_file(cx))
                })
                .groups(page.groups.iter().map(|group| {
                    SettingGroup::new()
                        .title(group.title.clone())
                        .items(group.entries.iter().map(|entry| self.item(entry)))
                }))
        });
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .when(!problems.is_empty(), |root| {
                root.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .bg(theme.danger.opacity(0.15))
                        .child(problems.join("\n")),
                )
            })
            .child(Settings::new("mdpp-settings").pages(pages))
    }
}
