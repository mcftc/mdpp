//! Find in files (PLAN §4.1, Ctrl+Shift+F): the left panel's search view. Searches the opened
//! folder's files — or the open tabs' files when no folder is open — with the find bar's engine
//! and options (`mdpp_core::search`), in the background after a typing pause. A click on a
//! result opens the file at the match.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mdpp_core::cli::OpenRequest;
use mdpp_core::search::{FileResults, MAX_MATCHES, Search, SearchOptions, search_files};

use crate::find::{MATCH_HIGHLIGHT_OPACITY, option_buttons};

/// Asks the workspace to open a result.
pub struct OpenMatch(pub OpenRequest);

/// The files to search, asked for at each search (the folder may have been listed again).
pub type FileSource = Rc<dyn Fn(&App) -> Vec<PathBuf>>;

/// One row of the results: a file, or a match under it.
#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Match(usize),
}

pub struct SearchPanel {
    query: Entity<InputState>,
    options: SearchOptions,
    files: FileSource,
    /// Quiet time after typing before searching, and the largest file searched.
    debounce: Duration,
    max_file_bytes: u64,
    results: Arc<FileResults>,
    rows: Arc<Vec<Row>>,
    error: Option<String>,
    searching: bool,
    _search: Task<()>,
    _subscription: Subscription,
}

impl EventEmitter<OpenMatch> for SearchPanel {}

impl SearchPanel {
    pub fn new(
        files: FileSource,
        debounce: Duration,
        max_file_bytes: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find in files"));
        let subscription = cx.subscribe_in(&query, window, |this, _, event, _, cx| match event {
            InputEvent::Change => this.search(this.debounce, cx),
            InputEvent::PressEnter { .. } => this.search(Duration::ZERO, cx),
            _ => {}
        });
        Self {
            query,
            options: SearchOptions::default(),
            files,
            debounce,
            max_file_bytes,
            results: Arc::default(),
            rows: Arc::default(),
            error: None,
            searching: false,
            _search: Task::ready(()),
            _subscription: subscription,
        }
    }

    /// Put the keyboard in the query (with `text` when given, e.g. the editor's selection).
    pub fn focus(&mut self, text: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |query, cx| {
            if let Some(text) = text {
                query.set_value(text, window, cx);
            }
            query.focus(window, cx);
        });
    }

    pub fn set_limits(&mut self, debounce: Duration, max_file_bytes: u64) {
        (self.debounce, self.max_file_bytes) = (debounce, max_file_bytes);
    }

    fn toggle(
        &mut self,
        option: fn(&mut SearchOptions) -> &mut bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flag = option(&mut self.options);
        *flag = !*flag;
        self.search(Duration::ZERO, cx);
    }

    /// Search again after `delay` (a newer search replaces a pending one).
    fn search(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let pattern = self.query.read(cx).value().to_string();
        if pattern.is_empty() {
            self.results = Arc::default();
            self.rows = Arc::default();
            self.error = None;
            self.searching = false;
            self._search = Task::ready(());
            return cx.notify();
        }
        let search = match Search::new(&pattern, self.options) {
            Ok(search) => search,
            Err(error) => {
                self.error = Some(error);
                return cx.notify();
            }
        };
        self.error = None;
        self.searching = true;
        let files = (self.files)(cx);
        let max_bytes = self.max_file_bytes;
        self._search = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let results = cx
                .background_spawn(
                    async move { search_files(&search, &files, max_bytes, MAX_MATCHES) },
                )
                .await;
            this.update(cx, |this, cx| {
                this.rows = Arc::new(rows_of(&results));
                this.results = Arc::new(results);
                this.searching = false;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// How many matches the last search found.
    pub fn match_count(&self) -> usize {
        self.results.matches.len()
    }

    /// Ask the workspace to open match `index` at its line and column.
    pub fn open_match(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(found) = self.results.matches.get(index) {
            cx.emit(OpenMatch(OpenRequest {
                path: found.path.clone(),
                line: Some(found.line + 1),
                column: Some(found.column + 1),
            }));
        }
    }

    fn status(&self, cx: &App) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if self.searching {
            return "Searching…".into();
        }
        let results = &self.results;
        match results.matches.len() {
            0 if self.query.read(cx).value().is_empty() => String::new(),
            0 => "No matches".into(),
            count => format!(
                "{count}{} matches in {} files",
                if results.truncated { "+" } else { "" },
                results.files_with_matches
            ),
        }
    }
}

/// Each file with matches, followed by its matches.
fn rows_of(results: &FileResults) -> Vec<Row> {
    let mut rows = Vec::new();
    for (index, found) in results.matches.iter().enumerate() {
        if index == 0 || results.matches[index - 1].path != found.path {
            rows.push(Row::File(index));
        }
        rows.push(Row::Match(index));
    }
    rows
}

impl Render for SearchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status_color = if self.error.is_some() {
            theme.danger
        } else {
            theme.muted_foreground
        };
        let [case, word, regex] = option_buttons(self.options, cx, Self::toggle);
        let (results, rows) = (self.results.clone(), self.rows.clone());
        let list = uniform_list(
            "search-results",
            rows.len(),
            cx.processor(move |_, range: std::ops::Range<usize>, _window, cx| {
                let theme = cx.theme();
                let (muted, hover_bg) = (theme.muted_foreground, theme.secondary);
                let highlight = HighlightStyle {
                    background_color: Some(theme.warning.opacity(MATCH_HIGHLIGHT_OPACITY)),
                    ..Default::default()
                };
                range
                    .map(|row| match rows[row] {
                        Row::File(index) => {
                            let path = &results.matches[index].path;
                            let name = path.file_name().map_or_else(
                                || path.display().to_string(),
                                |n| n.to_string_lossy().into_owned(),
                            );
                            div()
                                .id(row)
                                .px_2()
                                .pt_1()
                                .text_sm()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(name)
                        }
                        Row::Match(index) => {
                            let found = &results.matches[index];
                            div()
                                .id(row)
                                .flex()
                                .gap_2()
                                .pl_4()
                                .pr_2()
                                .text_sm()
                                .cursor_pointer()
                                .hover(move |row| row.bg(hover_bg))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.open_match(index, cx)),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_color(muted)
                                        .child(format!("{}", found.line + 1)),
                                )
                                .child(
                                    div().truncate().child(
                                        StyledText::new(found.preview.clone()).with_highlights([(
                                            found.highlight.clone(),
                                            highlight,
                                        )]),
                                    ),
                                )
                        }
                    })
                    .collect()
            }),
        )
        .flex_1()
        .pb_1();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .gap_1()
                    .p_1()
                    .child(div().flex_1().child(Input::new(&self.query).small()))
                    .child(case)
                    .child(word)
                    .child(regex),
            )
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .text_color(status_color)
                    .child(self.status(cx)),
            )
            .when(!self.rows.is_empty(), |panel| panel.child(list))
    }
}
