//! Go to line bar (Ctrl+G, PLAN §6.3): `line` or `line:column`, counted from 1.

use gpui_kit::component::input::{EditorState, Input, InputEvent, InputState, Position};
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mdpp_core::cli::parse_line_column;

use crate::find::{CloseRequested, KEY_CONTEXT};

pub struct GoToLineBar {
    input: Entity<InputState>,
    target: Option<Entity<EditorState>>,
    invalid: bool,
    _subscription: Subscription,
}

impl EventEmitter<CloseRequested> for GoToLineBar {}

impl GoToLineBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.go(window, cx),
                InputEvent::Change => {
                    this.invalid = false;
                    cx.notify();
                }
                _ => {}
            },
        );
        Self {
            input,
            target: None,
            invalid: false,
            _subscription: subscription,
        }
    }

    /// Show the bar for `editor`, its placeholder naming the valid range.
    pub fn open(
        &mut self,
        editor: Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = editor.read(cx);
        let lines = state.text().len_lines(ropey::LineType::LF);
        let current = state.cursor_position().line + 1;
        let placeholder = format!("Line 1–{lines}, or line:column (now {current})");
        self.target = Some(editor);
        self.invalid = false;
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
    }

    fn go(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value();
        let (Some((line, column)), Some(editor)) = (parse_line_column(&typed), &self.target) else {
            self.invalid = true;
            cx.notify();
            return;
        };
        move_caret(editor, line, column, window, cx);
        cx.emit(CloseRequested);
    }
}

/// Put the caret on `line` (from 1; past the end: the last line), at `column` (from 1) or the
/// start of the line. Shared by this bar and the palette's `:line`.
pub fn move_caret(
    editor: &Entity<EditorState>,
    line: u32,
    column: Option<u32>,
    window: &mut Window,
    cx: &mut App,
) {
    editor.update(cx, |state, cx| {
        let last = state
            .text()
            .len_lines(ropey::LineType::LF)
            .saturating_sub(1) as u32;
        let position = Position::new((line - 1).min(last), column.map_or(0, |c| c - 1));
        state.set_cursor_position(position, window, cx);
    });
}

impl Render for GoToLineBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .key_context(KEY_CONTEXT)
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(div().text_sm().child("Go to line"))
            .child(div().flex_1().child(Input::new(&self.input).small()))
            .when(self.invalid, |bar| {
                bar.child(
                    div()
                        .text_xs()
                        .text_color(theme.danger)
                        .child("Type a line number, or line:column"),
                )
            })
    }
}
