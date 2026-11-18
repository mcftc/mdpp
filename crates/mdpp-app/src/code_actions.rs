//! `mdpp_lang::actions` behind gpui-kit's code action menu (Ctrl+.): create a missing linked
//! file, insert or update a table of contents, convert a link to a reference link.
//!
//! The menu shows titles; the actions computed with them are kept here until one is chosen, so
//! applying needs no second pass over the text.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{CodeActionProvider, EditorState};
use gpui_kit::{App, AppContext as _, Entity, Result, SharedString, Task, Window};
use lsp_types::CodeAction as LspCodeAction;
use mdpp_lang::FileSystem;
use mdpp_lang::actions::{ActionKind, CodeAction, code_actions};
use mdpp_markdown::DocIndex;

use crate::document::SharedPath;

pub struct MdCodeActions {
    path: SharedPath,
    offered: Rc<RefCell<Vec<CodeAction>>>,
}

impl MdCodeActions {
    pub fn new(path: SharedPath) -> Self {
        Self {
            path,
            offered: Rc::default(),
        }
    }
}

impl CodeActionProvider for MdCodeActions {
    fn id(&self) -> SharedString {
        "mdpp".into()
    }

    fn code_actions(
        &self,
        state: Entity<EditorState>,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<LspCodeAction>>> {
        let text = state.read(cx).text().to_string();
        let doc_dir = self
            .path
            .borrow()
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf);
        let offered = self.offered.clone();
        let found = cx.background_spawn(async move {
            let index = DocIndex::build(&text);
            code_actions(&text, &index, range.start, doc_dir.as_deref(), &FileSystem)
        });
        cx.spawn(async move |_| {
            let found = found.await;
            let titles = found
                .iter()
                .map(|action| LspCodeAction {
                    title: action.title.clone(),
                    ..Default::default()
                })
                .collect();
            *offered.borrow_mut() = found;
            Ok(titles)
        })
    }

    fn perform_code_action(
        &self,
        state: Entity<EditorState>,
        action: LspCodeAction,
        _push_to_history: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let chosen = self
            .offered
            .borrow()
            .iter()
            .find(|offered| offered.title == action.title)
            .cloned();
        match chosen.map(|c| c.kind) {
            // Called while the editor is being updated: edit it right after.
            Some(ActionKind::Edit(edit)) => window.defer(cx, move |window, cx| {
                state.update(cx, |state, cx| {
                    state.set_selected_range(edit.range, cx);
                    state.replace(edit.text, window, cx);
                    state.set_selected_range(edit.select, cx);
                })
            }),
            Some(ActionKind::CreateFile { path, text }) => {
                let created = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| {
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                    })
                    .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()));
                if let Err(error) = created {
                    return Task::ready(Err(anyhow::anyhow!(
                        "Could not create {}: {error}",
                        path.display()
                    )));
                }
                // The workspace opens it, as from the recent-files menu.
                window.dispatch_action(Box::new(crate::workspace::OpenRecent { path }), cx);
            }
            None => {}
        }
        Task::ready(Ok(()))
    }
}
