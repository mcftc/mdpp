//! Find / replace bar (PLAN §4.1, D15): literal or regex, match case, whole word, in selection;
//! all matches highlighted, the current one selected; replace one or all (one undo step each).
//! The engine is `mdpp_core::search`; this file is only the UI and the editor plumbing.

use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    EditorState, Input, InputEvent, InputState, RangeDecoration, RangeDecorationCollection,
    RangeDecorationStyle, RopeExt,
};
use gpui_kit::component::{ActiveTheme, IconName, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mdpp_core::search::{MAX_MATCHES, Search, SearchOptions};

/// Key context of the find and go-to-line bars: Escape inside their inputs closes them.
pub const KEY_CONTEXT: &str = "Bar";
/// Opacity of the match highlight drawn over the theme's warning colour.
const MATCH_HIGHLIGHT_OPACITY: f32 = 0.35;

/// The bar asks the workspace to close it.
pub struct CloseRequested;

pub struct FindBar {
    query: Entity<InputState>,
    replacement: Entity<InputState>,
    options: SearchOptions,
    /// Search only inside this range (captured when "in selection" is switched on).
    scope: Option<Range<usize>>,
    show_replace: bool,
    target: Option<Entity<EditorState>>,
    matches: Vec<Range<usize>>,
    current: Option<usize>,
    error: Option<String>,
    highlights: Option<RangeDecorationCollection>,
    _subscriptions: Vec<Subscription>,
    target_subscription: Option<Subscription>,
}

impl EventEmitter<CloseRequested> for FindBar {}

impl FindBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace"));
        let subscriptions =
            vec![
                cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => this.research(true, window, cx),
                        InputEvent::PressEnter { shift, .. } => this.step(!*shift, window, cx),
                        _ => {}
                    }
                }),
                cx.subscribe_in(
                    &replacement,
                    window,
                    |this, _, event: &InputEvent, window, cx| {
                        if let InputEvent::PressEnter { .. } = event {
                            this.replace_current(window, cx);
                        }
                    },
                ),
            ];
        Self {
            query,
            replacement,
            options: SearchOptions::default(),
            scope: None,
            show_replace: false,
            target: None,
            matches: Vec::new(),
            current: None,
            error: None,
            highlights: None,
            _subscriptions: subscriptions,
            target_subscription: None,
        }
    }

    /// Show the bar for `editor`, prefilled with its selection when that is a single line.
    pub fn open(
        &mut self,
        editor: Entity<EditorState>,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_replace = replace;
        let selected = {
            let state = editor.read(cx);
            let range = state.selected_range();
            let text = state.text().slice(range).to_string();
            (!text.is_empty() && !text.contains('\n')).then_some(text)
        };
        self.retarget(editor, window, cx);
        if let Some(text) = selected {
            self.query
                .update(cx, |query, cx| query.set_value(text, window, cx));
        }
        self.query.update(cx, |query, cx| query.focus(window, cx));
        self.research(false, window, cx);
    }

    /// Follow another editor (tab switch) without moving the focus.
    pub fn retarget(
        &mut self,
        editor: Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.target.as_ref() == Some(&editor) {
            return;
        }
        self.clear_highlights(cx);
        self.scope = None;
        self.target_subscription = Some(cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.research(false, window, cx);
                }
            },
        ));
        self.target = Some(editor);
        self.research(false, window, cx);
    }

    /// The bar is hidden: drop the highlights.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.clear_highlights(cx);
        self.target = None;
        self.target_subscription = None;
    }

    fn clear_highlights(&mut self, cx: &mut Context<Self>) {
        if let Some(highlights) = self.highlights.take() {
            highlights.dispose(cx);
        }
    }

    /// Recompute matches. With `reveal`, select the first match at or after the cursor.
    fn research(&mut self, reveal: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let pattern = self.query.read(cx).value().to_string();
        self.error = None;
        self.matches.clear();
        self.current = None;
        if !pattern.is_empty() {
            match Search::new(&pattern, self.options) {
                Ok(search) => {
                    let text = target.read(cx).text().to_string();
                    self.matches = search.find_all(&text, self.scope.clone(), MAX_MATCHES + 1);
                    let cursor = target.read(cx).cursor();
                    self.current = (!self.matches.is_empty()).then(|| {
                        self.matches
                            .partition_point(|m| m.end <= cursor)
                            .min(self.matches.len() - 1)
                    });
                }
                Err(message) => self.error = Some(message),
            }
        }
        self.update_highlights(cx);
        if reveal {
            self.select_current(window, cx);
        }
        cx.notify();
    }

    fn update_highlights(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let color = cx.theme().warning.opacity(MATCH_HIGHLIGHT_OPACITY);
        let decorations: Vec<RangeDecoration> = self
            .matches
            .iter()
            .take(MAX_MATCHES)
            .map(|range| {
                RangeDecoration::new(range.clone())
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color)
            })
            .collect();
        match &self.highlights {
            Some(highlights) => highlights.set(decorations, cx),
            None => {
                self.highlights = Some(target.update(cx, |state, cx| {
                    state.create_range_decorations_collection(decorations, cx)
                }))
            }
        }
    }

    /// Select the current match and scroll it into view.
    fn select_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(target), Some(index)) = (self.target.clone(), self.current) else {
            return;
        };
        let Some(range) = self.matches.get(index).cloned() else {
            return;
        };
        target.update(cx, |state, cx| {
            // Moving the cursor scrolls the editor to it; then the match is selected.
            let position = state.text().offset_to_position(range.end);
            state.set_cursor_position(position, window, cx);
            state.set_selected_range(range, cx);
        });
    }

    /// Next (or previous) match, wrapping around.
    pub fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.matches.len().min(MAX_MATCHES);
        if count == 0 {
            return;
        }
        self.current = Some(match (self.current, forward) {
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) => (i + count - 1) % count,
            (None, _) => 0,
        });
        self.select_current(window, cx);
        cx.notify();
    }

    fn replace_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(target), Some(index)) = (self.target.clone(), self.current) else {
            return;
        };
        let (Some(range), Ok(search)) = (
            self.matches.get(index).cloned(),
            Search::new(&self.query.read(cx).value(), self.options),
        ) else {
            return;
        };
        let replacement = self.replacement.read(cx).value().to_string();
        target.update(cx, |state, cx| {
            let matched = state.text().slice(range.clone()).to_string();
            state.set_selected_range(range, cx);
            state.replace(search.replacement_for(&matched, &replacement), window, cx);
        });
        // The edit triggers `research` through the editor subscription; then go to the next match.
        self.research(true, window, cx);
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let Ok(search) = Search::new(&self.query.read(cx).value(), self.options) else {
            return;
        };
        let replacement = self.replacement.read(cx).value().to_string();
        let text = target.read(cx).text().to_string();
        let (replaced, count) = search.replace_all(&text, self.scope.clone(), &replacement);
        if count > 0 {
            // One undo step for the whole replacement.
            target.update(cx, |state, cx| state.replace_all(replaced, window, cx));
        }
    }

    fn toggle(
        &mut self,
        option: fn(&mut SearchOptions) -> &mut bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flag = option(&mut self.options);
        *flag = !*flag;
        self.research(true, window, cx);
    }

    fn toggle_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = match (&self.scope, &self.target) {
            (None, Some(target)) => {
                Some(target.read(cx).selected_range()).filter(|r| !r.is_empty())
            }
            _ => None,
        };
        self.research(true, window, cx);
    }

    /// Set the query as if typed (used by the end-to-end check).
    pub fn set_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |query, cx| {
            query.set_value(text.to_string(), window, cx)
        });
        self.research(true, window, cx);
    }

    pub fn set_replacement(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replacement.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn replace_every_match(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_all(window, cx);
    }

    fn status(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        match (self.current, self.matches.len()) {
            (_, 0) => "No results".to_string(),
            (_, n) if n > MAX_MATCHES => format!("{MAX_MATCHES}+ matches"),
            (Some(i), n) => format!("{} of {n}", i + 1),
            (None, n) => format!("{n} matches"),
        }
    }
}

impl Render for FindBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status_color = if self.error.is_some() {
            theme.danger
        } else {
            theme.muted_foreground
        };
        let toggle = |id: &'static str, label: &'static str, tooltip: &'static str, on: bool| {
            Button::new(id)
                .label(label)
                .ghost()
                .small()
                .selected(on)
                .tooltip(tooltip)
        };
        let find_row = h_flex()
            .gap_1()
            .child(div().flex_1().child(Input::new(&self.query).small()))
            .child(
                toggle(
                    "match-case",
                    "Aa",
                    "Match case",
                    self.options.case_sensitive,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle(|o| &mut o.case_sensitive, window, cx)
                })),
            )
            .child(
                toggle("whole-word", "W", "Whole word", self.options.whole_word).on_click(
                    cx.listener(|this, _, window, cx| {
                        this.toggle(|o| &mut o.whole_word, window, cx)
                    }),
                ),
            )
            .child(
                toggle("regex", ".*", "Regular expression", self.options.regex).on_click(
                    cx.listener(|this, _, window, cx| this.toggle(|o| &mut o.regex, window, cx)),
                ),
            )
            .child(
                toggle(
                    "in-selection",
                    "Sel",
                    "Find in selection",
                    self.scope.is_some(),
                )
                .on_click(cx.listener(|this, _, window, cx| this.toggle_scope(window, cx))),
            )
            .child(
                div()
                    .min_w(px(96.))
                    .text_xs()
                    .text_color(status_color)
                    .child(self.status()),
            )
            .child(
                Button::new("previous")
                    .label("↑")
                    .ghost()
                    .small()
                    .tooltip("Previous match (Shift+Enter)")
                    .on_click(cx.listener(|this, _, window, cx| this.step(false, window, cx))),
            )
            .child(
                Button::new("next")
                    .label("↓")
                    .ghost()
                    .small()
                    .tooltip("Next match (Enter)")
                    .on_click(cx.listener(|this, _, window, cx| this.step(true, window, cx))),
            )
            .child(
                Button::new("toggle-replace")
                    .label("⇄")
                    .ghost()
                    .small()
                    .selected(self.show_replace)
                    .tooltip("Replace")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_replace = !this.show_replace;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("close-find")
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .tooltip("Close (Escape)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(CloseRequested))),
            );
        v_flex()
            .key_context(KEY_CONTEXT)
            .w_full()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(find_row)
            .when(self.show_replace, |bar| {
                bar.child(
                    h_flex()
                        .gap_1()
                        .child(div().flex_1().child(Input::new(&self.replacement).small()))
                        .child(
                            Button::new("replace-one")
                                .label("Replace")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.replace_current(window, cx)
                                })),
                        )
                        .child(
                            Button::new("replace-all")
                                .label("All")
                                .small()
                                .tooltip("Replace all")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.replace_all(window, cx)),
                                ),
                        ),
                )
            })
    }
}
