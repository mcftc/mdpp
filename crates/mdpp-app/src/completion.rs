//! `mdpp-lang` completions behind gpui-kit's `CompletionProvider` (PLAN D7).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{CompletionProvider, Rope, RopeExt};
use gpui_kit::{App, AppContext as _, Result, Task, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    TextEdit,
};
use mdpp_lang::{Completion, CompletionKind, FileSystem};
use mdpp_markdown::DocIndex;
use ropey::LineType;

use crate::document::SharedPath;
use crate::highlight::SharedLexer;

/// The latest structure index of the document, published by the background analysis. It may be
/// one revision old: completion only takes names from it (PLAN §3.2).
pub type SharedIndex = Rc<RefCell<Option<Arc<DocIndex>>>>;

pub struct MdCompletions {
    lexer: SharedLexer,
    index: SharedIndex,
    /// Folder of the document; `None` while it is unsaved (no path completion then).
    doc_path: SharedPath,
}

impl MdCompletions {
    pub fn new(lexer: SharedLexer, index: SharedIndex, doc_path: SharedPath) -> Self {
        Self {
            lexer,
            index,
            doc_path,
        }
    }
}

impl CompletionProvider for MdCompletions {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let row = rope.byte_to_line_idx(offset.min(rope.len()), LineType::LF);
        let line_state = self.lexer.borrow().state_at(row);
        // The context comes from the current line of the current text — never stale (PLAN §3.2).
        let Some(context) = mdpp_lang::context_at(rope, offset, line_state) else {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        };
        let rope = rope.clone();
        let doc_dir = self
            .doc_path
            .borrow()
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf);
        let cached = self.index.borrow().clone();
        cx.background_spawn(async move {
            // Before the first analysis has finished there is no index yet: build one.
            let index = context
                .needs_index()
                .then(|| cached.unwrap_or_else(|| Arc::new(DocIndex::build(&rope.to_string()))));
            let items =
                mdpp_lang::candidates(&context, index.as_deref(), &FileSystem, doc_dir.as_deref())
                    .into_iter()
                    .map(|completion| to_lsp(completion, &rope))
                    .collect();
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, _new_text: &str, _cx: &mut App) -> bool {
        // Context detection is a few string checks on one line, so every keystroke may ask.
        true
    }
}

fn to_lsp(completion: Completion, rope: &Rope) -> CompletionItem {
    let range = lsp_types::Range::new(
        rope.offset_to_position(completion.replace.start),
        rope.offset_to_position(completion.replace.end),
    );
    CompletionItem {
        label: completion.label,
        kind: Some(match completion.kind {
            CompletionKind::Heading => CompletionItemKind::REFERENCE,
            CompletionKind::File => CompletionItemKind::FILE,
            CompletionKind::Folder => CompletionItemKind::FOLDER,
            CompletionKind::Reference | CompletionKind::Footnote => CompletionItemKind::REFERENCE,
            CompletionKind::Emoji => CompletionItemKind::TEXT,
            CompletionKind::Language => CompletionItemKind::KEYWORD,
            CompletionKind::Alert => CompletionItemKind::ENUM_MEMBER,
        }),
        detail: completion.detail,
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: completion.insert,
        })),
        ..Default::default()
    }
}
