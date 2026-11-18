//! Opening the palette and carrying out what was picked in it (see `crate::palette`).

use crate::menus;
use crate::palette::{COMMANDS_PREFIX, Choice, Palette, PaletteEvent, Sources};

use super::*;

impl Workspace {
    fn open_palette(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.active_document().read(cx);
        let editor = doc.editor().focus_handle(cx);
        let headings = doc
            .index()
            .map(|index| {
                index
                    .headings
                    .iter()
                    .map(|h| (h.text.clone(), h.level, h.range.start))
                    .collect()
            })
            .unwrap_or_default();
        // Open tabs first (the active one last: it is already in front), then recent files, then
        // the folder's files — each once (a set: a folder can hold thousands).
        let active = doc.path().map(std::path::Path::to_path_buf);
        let mut files: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = active.iter().cloned().collect();
        let tabs = self
            .documents
            .iter()
            .filter_map(|d| d.read(cx).path().map(std::path::Path::to_path_buf));
        let folder_files = self.folder.iter().flat_map(|f| f.files());
        for path in tabs.chain(self.recent.iter().cloned()).chain(folder_files) {
            if seen.insert(path.clone()) {
                files.push(path);
            }
        }
        files.extend(active);
        let commands = menus::commands()
            .into_iter()
            .map(|c| (c.label.into(), c.place.into(), c.action))
            .collect();
        let sources = Sources {
            commands,
            files,
            root: self.folder.as_ref().map(|f| f.root.clone()),
            headings,
            editor,
        };
        let palette = cx.new(|cx| Palette::new(query, sources, window, cx));
        let subscription = cx.subscribe_in(&palette, window, |this, _, event, window, cx| {
            this.close_palette(window, cx);
            if let PaletteEvent::Chosen(choice) = event {
                this.run_choice(choice.clone(), window, cx);
            }
        });
        self.palette = Some((palette, subscription));
        cx.notify();
    }

    /// Close the palette and give the editor the keyboard back.
    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            let editor = self.active_document().read(cx).editor().clone();
            window.focus(&editor.focus_handle(cx), cx);
            cx.notify();
        }
    }

    pub(super) fn run_choice(
        &mut self,
        choice: Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match choice {
            // From the editor, which has the focus again: every action is handled from there.
            Choice::Command(name) => match cx.build_action(name, None) {
                Ok(action) => window.dispatch_action(action, cx),
                Err(error) => eprintln!("mdpp: palette: {name}: {error}"),
            },
            Choice::Open(path) => self.open_paths(vec![path], window, cx),
            Choice::Jump(offset) => self.jump_to(offset, window, cx),
            Choice::Line(line, column) => {
                let editor = self.active_document().read(cx).editor().clone();
                crate::goto::move_caret(&editor, line, column, window, cx);
            }
        }
    }

    pub(super) fn command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(COMMANDS_PREFIX, window, cx);
    }

    pub(super) fn quick_open(
        &mut self,
        _: &QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette("", window, cx);
    }

    pub(super) fn go_to_heading(
        &mut self,
        _: &GoToHeading,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette("@", window, cx);
    }

    /// The palette floats over the top of the window; a click elsewhere closes it.
    pub(super) fn render_palette(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (palette, _) = self.palette.as_ref()?;
        Some(deferred(
            div()
                .absolute()
                .top(px(crate::palette::TOP))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("palette")
                        .w(px(crate::palette::WIDTH))
                        .max_w_full()
                        .shadow_lg()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().popover)
                        .on_mouse_down_out(
                            cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
                        )
                        .child(palette.clone()),
                ),
        ))
    }
}
