//! Editing commands on the active document (PLAN §4.3, §6.3): Markdown assists, whole-line
//! commands, go to line, zoom and word wrap.
//!
//! Every assist is a pure function in `mdpp_lang` from the editor's text and selection to one
//! `Edit`; here it is applied as a single undo step.

use std::ops::{Range, RangeInclusive};
use std::path::PathBuf;

use gpui_kit::component::input::{self, Rope};
use gpui_kit::*;
use mdpp_core::assets::{PASTED_IMAGE_STEM, free_name, image_folder, link_from};
use mdpp_lang::assist::{self, Edit};
use mdpp_lang::{lines, percent_encode_path, rename, table};

/// An image to put into the document: pasted bytes or a dropped file.
pub(super) enum NewImage {
    Bytes {
        extension: &'static str,
        bytes: Vec<u8>,
    },
    File(PathBuf),
}

/// Store `image` for document `doc`; returns its link (relative to the document, unencoded) and
/// a name for the alt text.
fn store_image(
    doc: &std::path::Path,
    template: &str,
    image: NewImage,
) -> Result<(String, String), String> {
    if let NewImage::File(path) = &image
        && let Some(link) = link_from(doc, path)
    {
        return Ok((link, stem_of(path)));
    }
    let Some((folder, folder_link)) = image_folder(doc, template) else {
        return Err(format!(
            "assets_dir_template `{template}` must stay inside the document's folder."
        ));
    };
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
    let (name, alt) = match &image {
        NewImage::Bytes { extension, .. } => (
            free_name(&folder, PASTED_IMAGE_STEM, extension),
            String::new(),
        ),
        NewImage::File(path) => {
            let extension = path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            (
                free_name(&folder, &stem_of(path), &extension),
                stem_of(path),
            )
        }
    };
    let target = folder.join(&name);
    let written = match image {
        NewImage::Bytes { bytes, .. } => std::fs::write(&target, bytes),
        NewImage::File(path) => std::fs::copy(&path, &target).map(drop),
    };
    written.map_err(|e| format!("Could not store the image in {}: {e}", target.display()))?;
    let link = if folder_link.is_empty() {
        name
    } else {
        format!("{folder_link}/{name}")
    };
    Ok((link, alt))
}

fn stem_of(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Encode each part of a `/`-separated link.
fn percent_encode_link(link: &str) -> String {
    link.split('/')
        .map(percent_encode_path)
        .collect::<Vec<_>>()
        .join("/")
}

use super::*;
use mdpp_core::i18n::{t, tf};

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

    /// Insert a snippet over the selection; one that does not start a line gets a line break
    /// before it, since snippets are blocks.
    pub(super) fn insert_snippet(
        &mut self,
        action: &InsertSnippet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snippet) = crate::snippets::find(&action.name) else {
            return;
        };
        let (body, caret) = snippet.expand();
        let state = self.active_document().read(cx).editor().read(cx);
        let range = state.selected_range();
        let text = state.text();
        let line_start = text.line_to_byte_idx(
            text.byte_to_line_idx(range.start, ropey::LineType::LF),
            ropey::LineType::LF,
        );
        let lead = if range.start == line_start { "" } else { "\n" };
        let at = range.start + lead.len() + caret;
        self.apply_edit(
            Edit {
                range,
                text: format!("{lead}{body}"),
                select: at..at,
            },
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

    /// Paste: an image is stored next to the document and linked; a URL pasted over selected
    /// text becomes a link; anything else pastes normally.
    pub(super) fn smart_paste(
        &mut self,
        _: &SmartPaste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let item = cx.read_from_clipboard();
        // Text wins when the clipboard also has it (a browser copy carries both).
        let image = item.as_ref().and_then(|item| {
            let text = item
                .entries()
                .iter()
                .any(|e| matches!(e, ClipboardEntry::String(_)));
            item.entries().iter().find_map(|entry| match entry {
                ClipboardEntry::Image(image) if !text => Some(NewImage::Bytes {
                    extension: image.format.extension(),
                    bytes: image.bytes.clone(),
                }),
                _ => None,
            })
        });
        if let Some(image) = image {
            return self.insert_images(vec![image], window, cx);
        }
        let clipboard = item.and_then(|item| item.text());
        let state = self.active_document().read(cx).editor().read(cx);
        let edit = clipboard
            .and_then(|text| assist::paste_link(state.text(), state.selected_range(), &text));
        match edit {
            Some(edit) => self.apply_edit(edit, window, cx),
            None => window.dispatch_action(Box::new(input::Paste), cx),
        }
    }

    /// Store images next to the active document (`assets_dir_template`) and insert their links
    /// at the caret. An image already inside the document's folder is linked where it is.
    pub(super) fn insert_images(
        &mut self,
        images: Vec<NewImage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let doc = self.active_document().read(cx);
        let Some(doc_path) = doc.path().map(std::path::Path::to_path_buf) else {
            let message = t("Save the document first: images are stored next to it.");
            return window.push_notification(Notification::warning(message), cx);
        };
        let state = doc.editor().read(cx);
        let selection = state.selected_range();
        let selected = state.text().slice(selection.clone()).to_string();
        let mut links = Vec::new();
        for image in images {
            match store_image(&doc_path, &self.settings.assets_dir_template, image) {
                Ok((link, name)) => {
                    let alt = if selected.is_empty() {
                        name
                    } else {
                        selected.clone()
                    };
                    links.push(format!("![{alt}]({})", percent_encode_link(&link)));
                }
                Err(problem) => window.push_notification(Notification::error(problem), cx),
            }
        }
        if links.is_empty() {
            return;
        }
        let text = links.join("\n");
        let end = selection.start + text.len();
        self.apply_edit(
            Edit {
                range: selection,
                text,
                select: end..end,
            },
            window,
            cx,
        );
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

    /// Show the prompt bar with `ask`.
    pub(super) fn prompt(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        self.show_prompt = true;
        self.prompt_bar
            .update(cx, |bar, cx| bar.open(ask, window, cx));
        cx.notify();
    }

    /// F2 on a heading (or a link to one): rename it, with every link in this document to a
    /// heading whose anchor changes.
    pub(super) fn rename_heading(
        &mut self,
        _: &RenameHeading,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.active_document().read(cx).editor().clone();
        let state = editor.read(cx);
        let text = state.text().to_string();
        let index = mdpp_markdown::DocIndex::build(&text);
        let Some((which, current)) = rename::heading_at(&text, &index, state.cursor()) else {
            let message = t("Put the caret on a heading, or on a link to one, to rename it.");
            return window.push_notification(Notification::info(message), cx);
        };
        let submit: crate::prompt::Submit = Box::new(move |typed, window, cx| {
            let text = editor.read(cx).text().to_string();
            let edit = rename::rename_heading(&text, which, typed).ok_or(t(
                "Type a new name (one line, different from the current one)",
            ))?;
            editor.update(cx, |state, cx| {
                state.set_selected_range(edit.range, cx);
                state.replace(edit.text, window, cx);
                state.set_selected_range(edit.select, cx);
            });
            Ok(())
        });
        let ask = Ask {
            label: t("Rename heading").to_string().into(),
            placeholder: t("New heading text").to_string().into(),
            value: current,
            submit,
        };
        self.prompt(ask, window, cx);
    }

    /// Shift+F12 on a heading: find in files for links to its anchor (`#anchor)`), in the
    /// folder's files or the open tabs' files as saved.
    pub(super) fn find_references(
        &mut self,
        _: &FindReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.active_document().read(cx).editor().read(cx);
        let text = state.text().to_string();
        let index = mdpp_markdown::DocIndex::build(&text);
        let Some((which, _)) = rename::heading_at(&text, &index, state.cursor()) else {
            let message = t("Put the caret on a heading to find the links to it.");
            return window.push_notification(Notification::info(message), cx);
        };
        let query = format!("#{})", index.headings[which].anchor);
        self.show_side = true;
        self.side = folder::Side::Search;
        self.search_panel
            .update(cx, |panel, cx| panel.focus(Some(query), window, cx));
        cx.notify();
    }

    /// Ctrl+G: `line` or `line:column`, counted from 1.
    pub(super) fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.active_document().read(cx).editor().clone();
        let state = editor.read(cx);
        let lines = state.text().len_lines(ropey::LineType::LF);
        let current = state.cursor_position().line + 1;
        let submit: crate::prompt::Submit = Box::new(move |typed, window, cx| {
            let (line, column) = mdpp_core::cli::parse_line_column(typed.trim())
                .ok_or(t("Type a line number, or line:column"))?;
            crate::prompt::move_caret(&editor, line, column, window, cx);
            Ok(())
        });
        let ask = Ask {
            label: t("Go to line").to_string().into(),
            placeholder: tf(
                "Line 1–{lines}, or line:column (now {current})",
                &[("lines", &lines), ("current", &current)],
            )
            .into(),
            value: String::new(),
            submit,
        };
        self.prompt(ask, window, cx);
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
