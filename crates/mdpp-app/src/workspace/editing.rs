//! Editing commands on the active document (PLAN §4.3, §6.3): Markdown assists, whole-line
//! commands, go to line, zoom and word wrap.
//!
//! Every assist is a pure function in `mdpp_lang` from the editor's text and selection to one
//! `Edit`; here it is applied as a single undo step.

use std::ops::{Range, RangeInclusive};

use gpui_kit::component::input::{self, Rope};
use gpui_kit::*;
use mdpp_lang::assist::{self, Edit};
use mdpp_lang::{lines, table};

use super::*;

/// Zoom changes the editor text by this many pixels per step, within this range (smaller is
/// unreadable; larger no longer fits a useful line on a laptop screen).
const ZOOM_STEP: i32 = 1;
const FONT_SIZE_RANGE: RangeInclusive<f32> = 6.0..=72.0;

impl Workspace {
    /// Apply an assist to the active editor as one undo step.
    pub(super) fn apply_edit(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.active_document().read(cx).editor().clone();
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.text, window, cx);
            state.set_selected_range(edit.select, cx);
        });
    }

    /// Run a command that always applies (formatting, lines).
    fn edit_with(
        &mut self,
        command: impl FnOnce(&Rope, Range<usize>) -> Edit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.active_document().read(cx).editor().read(cx);
        let edit = command(state.text(), state.selected_range());
        self.apply_edit(edit, window, cx);
    }

    /// Run an assist bound to a key the editor also uses (Enter, Tab): when a completion or
    /// code-action menu is open, the caret is in code, or the assist has nothing to do, the
    /// editor's own `fallback` action runs instead.
    fn assist_or(
        &mut self,
        assist: impl FnOnce(&Rope, Range<usize>) -> Option<Edit>,
        fallback: Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let doc = self.active_document().read(cx);
        let state = doc.editor().read(cx);
        let menu_open = state.completion_menu_state().open || state.code_action_menu_state().open;
        let edit = (!menu_open && doc.cursor_in_prose(cx))
            .then(|| assist(state.text(), state.selected_range()))
            .flatten();
        match edit {
            Some(edit) => self.apply_edit(edit, window, cx),
            None => window.dispatch_action(fallback, cx),
        }
    }

    /// Enter: continue lists, tasks and quotes (PLAN §4.3); with text selected, the editor's.
    pub(super) fn smart_enter(
        &mut self,
        _: &SmartEnter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enter = input::Enter {
            secondary: false,
            shift: false,
        };
        self.assist_or(
            |text, selection| {
                selection
                    .is_empty()
                    .then(|| {
                        table::on_enter(text, selection.start)
                            .or_else(|| assist::on_enter(text, selection.start))
                    })
                    .flatten()
            },
            Box::new(enter),
            window,
            cx,
        );
    }

    /// Tab in a table moves to the next cell; on a list item it nests the item; anywhere else
    /// the editor indents as usual.
    pub(super) fn smart_tab(&mut self, _: &SmartTab, window: &mut Window, cx: &mut Context<Self>) {
        self.assist_or(
            |text, selection| {
                table::on_tab(text, selection.clone(), false)
                    .or_else(|| assist::indent_list(text, selection, false))
            },
            Box::new(input::IndentInline),
            window,
            cx,
        );
    }

    pub(super) fn smart_shift_tab(
        &mut self,
        _: &SmartShiftTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.assist_or(
            |text, selection| {
                table::on_tab(text, selection.clone(), true)
                    .or_else(|| assist::indent_list(text, selection, true))
            },
            Box::new(input::OutdentInline),
            window,
            cx,
        );
    }

    /// Align the columns of the table at the caret (Edit → Format → Format Table).
    pub(super) fn format_table(
        &mut self,
        _: &FormatTable,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.active_document().read(cx).editor().read(cx);
        if let Some(edit) = table::format(state.text(), state.cursor()) {
            self.apply_edit(edit, window, cx);
        }
    }

    /// Paste: a URL pasted over selected text becomes a link; anything else pastes normally.
    pub(super) fn smart_paste(
        &mut self,
        _: &SmartPaste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
        let state = self.active_document().read(cx).editor().read(cx);
        let edit = clipboard
            .and_then(|text| assist::paste_link(state.text(), state.selected_range(), &text));
        match edit {
            Some(edit) => self.apply_edit(edit, window, cx),
            None => window.dispatch_action(Box::new(input::Paste), cx),
        }
    }

    fn toggle_marker(&mut self, marker: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_with(
            |text, selection| assist::toggle_wrap(text, selection, marker),
            window,
            cx,
        );
    }

    pub(super) fn toggle_bold(
        &mut self,
        _: &ToggleBold,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_marker("**", window, cx);
    }

    pub(super) fn toggle_italic(
        &mut self,
        _: &ToggleItalic,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_marker("*", window, cx);
    }

    pub(super) fn toggle_code(
        &mut self,
        _: &ToggleCode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_marker("`", window, cx);
    }

    pub(super) fn toggle_strikethrough(
        &mut self,
        _: &ToggleStrikethrough,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_marker("~~", window, cx);
    }

    pub(super) fn insert_link(
        &mut self,
        _: &InsertLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_with(assist::insert_link, window, cx);
    }

    pub(super) fn toggle_task(
        &mut self,
        _: &ToggleTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_with(
            |text, selection| assist::toggle_task(text, selection.start),
            window,
            cx,
        );
    }

    pub(super) fn add_heading_level(
        &mut self,
        _: &AddHeadingLevel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_with(
            |text, selection| assist::shift_heading(text, selection.start, 1),
            window,
            cx,
        );
    }

    pub(super) fn remove_heading_level(
        &mut self,
        _: &RemoveHeadingLevel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_with(
            |text, selection| assist::shift_heading(text, selection.start, -1),
            window,
            cx,
        );
    }

    pub(super) fn duplicate_lines(
        &mut self,
        _: &DuplicateLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_with(lines::duplicate_lines, window, cx);
    }

    pub(super) fn delete_lines(
        &mut self,
        _: &DeleteLines,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_with(lines::delete_lines, window, cx);
    }

    fn move_lines(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.active_document().read(cx).editor().read(cx);
        if let Some(edit) = lines::move_lines(state.text(), state.selected_range(), up) {
            self.apply_edit(edit, window, cx);
        }
    }

    pub(super) fn move_lines_up(
        &mut self,
        _: &MoveLinesUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_lines(true, window, cx);
    }

    pub(super) fn move_lines_down(
        &mut self,
        _: &MoveLinesDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_lines(false, window, cx);
    }

    pub(super) fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        self.show_goto = true;
        let editor = self.active_document().read(cx).editor().clone();
        self.goto_bar
            .update(cx, |bar, cx| bar.open(editor, window, cx));
        cx.notify();
    }

    /// Editor text size: the `font_size` setting moved by the zoom steps.
    pub(super) fn font_size(&self) -> f32 {
        (self.settings.font_size + (self.zoom * ZOOM_STEP) as f32)
            .clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end())
    }

    fn zoom_by(&mut self, steps: i32, cx: &mut Context<Self>) {
        let before = self.font_size();
        self.zoom += steps;
        if self.font_size() == before {
            self.zoom -= steps; // at a bound: do not build up steps that change nothing
        }
        cx.notify();
    }

    pub(super) fn zoom_in(&mut self, _: &ZoomIn, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(1, cx);
    }

    pub(super) fn zoom_out(&mut self, _: &ZoomOut, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(-1, cx);
    }

    pub(super) fn zoom_reset(
        &mut self,
        _: &ZoomReset,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.zoom = 0;
        cx.notify();
    }

    /// Alt+Z: wrap long lines or not, for every tab (also large documents, which open unwrapped).
    pub(super) fn toggle_word_wrap(
        &mut self,
        _: &ToggleWordWrap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let wrap = !self.active_document().read(cx).soft_wrap();
        self.wrap = Some(wrap);
        for doc in &self.documents {
            doc.update(cx, |doc, cx| doc.set_soft_wrap(wrap, window, cx));
        }
        cx.notify();
    }
}
