//! A one-line prompt under the tabs: Go to line (Ctrl+G, PLAN §6.3) and Rename heading (F2).
//! What Enter does is the caller's: the bar shows the label, the input and the caller's error,
//! and closes when the caller accepts.

use gpui_kit::component::input::{EditorState, Input, InputEvent, InputState, Position};
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::find::{CloseRequested, KEY_CONTEXT};

/// Runs on Enter with the typed text; `Err` keeps the bar open with the message.
pub type Submit = Box<dyn Fn(&str, &mut Window, &mut App) -> Result<(), String>>;

/// What a prompt asks.
pub struct Ask {
    pub label: SharedString,
    pub placeholder: SharedString,
    /// Text the input starts with (selected, so typing replaces it).
    pub value: String,
    pub submit: Submit,
}

pub struct PromptBar {
    input: Entity<InputState>,
    label: SharedString,
    submit: Option<Submit>,
    error: Option<String>,
    _subscription: Subscription,
}

impl EventEmitter<CloseRequested> for PromptBar {}

impl PromptBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                InputEvent::Change => {
                    this.error = None;
                    cx.notify();
                }
                _ => {}
            },
        );
        Self {
            input,
            label: SharedString::default(),
            submit: None,
            error: None,
            _subscription: subscription,
        }
    }

    pub fn open(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        self.label = ask.label;
        self.submit = Some(ask.submit);
        self.error = None;
        self.input.update(cx, |input, cx| {
            input.set_placeholder(ask.placeholder, window, cx);
            input.set_value(ask.value, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        let Some(submit) = &self.submit else {
            return;
        };
        match submit(&typed, window, cx) {
            Ok(()) => cx.emit(CloseRequested),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
}

/// Put the caret on `line` (from 1; past the end: the last line), at `column` (from 1) or the
/// start of the line. Shared by Go to line and the palette's `:line`.
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

impl Render for PromptBar {
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
            .child(div().text_sm().child(self.label.clone()))
            .child(div().flex_1().child(Input::new(&self.input).small()))
            .when_some(self.error.clone(), |bar, error| {
                bar.child(div().text_xs().text_color(theme.danger).child(error))
            })
    }
}
