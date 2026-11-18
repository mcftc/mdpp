//! The palette (PLAN §4.1): one search box for commands, files, headings and lines, like Zed's
//! and VS Code's. What it searches follows the first character typed:
//!
//! | typed     | lists                                                        |
//! |-----------|--------------------------------------------------------------|
//! | `>`       | every menu command, with its shortcut (Ctrl+Shift+P opens here) |
//! | `@`       | the headings of the document                                 |
//! | `:`       | go to `line` or `line:column`                                |
//! | otherwise | open tabs and recent files (Ctrl+P opens here)               |
//!
//! The commands are the menus' (`assets/menus.toml`), so a new menu entry is in the palette by
//! itself. Matching is fuzzy (`mdpp_core::fuzzy`). The palette only reports the choice; the
//! workspace carries it out from the editor, where every action is handled.

use std::path::PathBuf;

use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;
use mdpp_core::cli::parse_line_column;
use mdpp_core::fuzzy;

/// What the first character of the query selects.
pub const COMMANDS_PREFIX: &str = ">";
const HEADINGS_PREFIX: char = '@';
const LINE_PREFIX: char = ':';
/// Width of the palette and its distance from the top of the window, in logical pixels.
pub const WIDTH: f32 = 560.0;
pub const TOP: f32 = 48.0;
const MAX_HEIGHT: f32 = 380.0;

/// What the user picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// A menu command, by action name.
    Command(&'static str),
    Open(PathBuf),
    /// A byte offset in the active document (a heading).
    Jump(usize),
    Line(u32, Option<u32>),
}

pub enum PaletteEvent {
    Chosen(Choice),
    Dismissed,
}

/// One row the palette can show.
struct Row {
    label: SharedString,
    detail: SharedString,
    choice: Choice,
    /// For the shortcut shown at the right.
    action: Option<Box<dyn Action>>,
}

/// What the palette searches, gathered when it opens.
pub struct Sources {
    pub commands: Vec<(SharedString, SharedString, &'static str)>,
    pub files: Vec<PathBuf>,
    /// Text, level, byte offset.
    pub headings: Vec<(String, u8, usize)>,
    /// Shortcuts are looked up where the command will run: in the editor.
    pub editor: FocusHandle,
}

pub struct Palette {
    state: Entity<CommandState>,
    sources: Sources,
    rows: Vec<Row>,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn new(query: &str, sources: Sources, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| {
            state.set_query(query.to_string(), window, cx);
            state.focus(window, cx);
        });
        let mut this = Self {
            state,
            sources,
            rows: Vec::new(),
        };
        this.search(query, cx);
        this
    }

    /// The rows for `query`, best match first.
    fn search(&mut self, query: &str, cx: &App) {
        let sources = &self.sources;
        self.rows = if let Some(text) = query.strip_prefix(COMMANDS_PREFIX) {
            // Label first, then "Edit › Format" too, so a menu name finds its commands.
            let names: Vec<String> = sources
                .commands
                .iter()
                .map(|(label, place, _)| format!("{label} {place}"))
                .collect();
            fuzzy::rank(text, names.iter().map(String::as_str))
                .into_iter()
                .map(|i| {
                    let (label, place, action) = &sources.commands[i];
                    Row {
                        label: label.clone(),
                        detail: place.clone(),
                        choice: Choice::Command(action),
                        action: cx.build_action(action, None).ok(),
                    }
                })
                .collect()
        } else if let Some(text) = query.strip_prefix(HEADINGS_PREFIX) {
            fuzzy::rank(text, sources.headings.iter().map(|(t, _, _)| t.as_str()))
                .into_iter()
                .map(|i| {
                    let (text, level, offset) = &sources.headings[i];
                    Row {
                        label: text.clone().into(),
                        detail: "#".repeat(usize::from(*level)).into(),
                        choice: Choice::Jump(*offset),
                        action: None,
                    }
                })
                .collect()
        } else if let Some(text) = query.strip_prefix(LINE_PREFIX) {
            parse_line_column(text.trim())
                .map(|(line, column)| Row {
                    label: match column {
                        Some(column) => format!("Go to line {line}, column {column}"),
                        None => format!("Go to line {line}"),
                    }
                    .into(),
                    detail: SharedString::default(),
                    choice: Choice::Line(line, column),
                    action: None,
                })
                .into_iter()
                .collect()
        } else {
            let names: Vec<String> = sources
                .files
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            // The file name decides first; the folder only breaks ties and helps narrow down.
            let by_name: Vec<&str> = sources
                .files
                .iter()
                .zip(&names)
                .map(|(path, full)| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(full.as_str())
                })
                .collect();
            let mut ranked = fuzzy::rank(query, by_name.iter().copied());
            for i in fuzzy::rank(query, names.iter().map(String::as_str)) {
                if !ranked.contains(&i) {
                    ranked.push(i);
                }
            }
            ranked
                .into_iter()
                .map(|i| {
                    let path = &sources.files[i];
                    Row {
                        label: by_name[i].to_string().into(),
                        detail: path
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                            .into(),
                        choice: Choice::Open(path.clone()),
                        action: None,
                    }
                })
                .collect()
        };
    }

    fn placeholder(query: &str) -> &'static str {
        match query.chars().next() {
            Some('>') => "Run a command…",
            Some(HEADINGS_PREFIX) => "Go to a heading…",
            Some(LINE_PREFIX) => "Go to line[:column]…",
            _ => "Open a file — or type > for commands, @ for headings, : for a line",
        }
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let query = self.state.read(cx).query(cx);
        let muted = cx.theme().muted_foreground;
        let editor = self.sources.editor.clone();
        let items = self.rows.iter().map(|row| {
            let (label, detail) = (row.label.clone(), row.detail.clone());
            let action = row.action.as_ref().map(|a| a.boxed_clone());
            let editor = editor.clone();
            CommandItem::new()
                .label(label.clone())
                .child(move |window, _| {
                    let keys = action
                        .as_ref()
                        .and_then(|a| Kbd::binding_for_action_in(a.as_ref(), &editor, window));
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(div().flex_shrink_0().child(label.clone()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(muted)
                                .child(detail.clone()),
                        )
                        .children(keys)
                })
        });
        let (on_query, on_confirm, on_cancel) = (this.clone(), this.clone(), this);
        Command::new(&self.state)
            .filterable(false)
            .placeholder(Self::placeholder(&query))
            .max_h(px(MAX_HEIGHT))
            .items(items)
            .on_query(move |query, _, cx| {
                on_query
                    .update(cx, |this, cx| {
                        this.search(query, cx);
                        cx.notify();
                    })
                    .ok();
            })
            .on_confirm(move |path, _, cx| {
                on_confirm
                    .update(cx, |this, cx| {
                        if let Some(row) = this.rows.get(path.row) {
                            cx.emit(PaletteEvent::Chosen(row.choice.clone()));
                        }
                    })
                    .ok();
            })
            .on_cancel(move |_, cx| {
                on_cancel
                    .update(cx, |_, cx| cx.emit(PaletteEvent::Dismissed))
                    .ok();
            })
    }
}
