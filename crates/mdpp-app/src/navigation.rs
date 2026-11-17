//! `mdpp-lang` navigation behind gpui-kit's definition and hover providers (PLAN §4.3).
//!
//! Both build a fresh structure index on a background thread, so the offsets they report always
//! match the text the editor is showing (the shared index may trail the last edit).

use std::path::Path;
use std::str::FromStr;

use gpui_kit::component::input::{DefinitionProvider, HoverProvider, Rope, RopeExt};
use gpui_kit::{App, AppContext as _, Result, Task, Window};
use lsp_types::{Hover, HoverContents, LocationLink, MarkedString, Uri};
use mdpp_lang::navigate::{self, Target};
use mdpp_lang::{FileSystem, file_uri};
use mdpp_markdown::DocIndex;

use crate::document::SharedPath;

/// Target URI meaning "this document": not http(s), so gpui-kit jumps within the editor.
pub const THIS_DOCUMENT_URI: &str = "mdpp:this-document";

pub struct MdNavigation {
    path: SharedPath,
}

impl MdNavigation {
    pub fn new(path: SharedPath) -> Self {
        Self { path }
    }

    fn doc_dir(&self) -> Option<std::path::PathBuf> {
        self.path
            .borrow()
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }
}

fn lsp_range(text: &Rope, range: std::ops::Range<usize>) -> lsp_types::Range {
    let end = range.end.min(text.len());
    lsp_types::Range::new(
        text.offset_to_position(range.start.min(end)),
        text.offset_to_position(end),
    )
}

impl DefinitionProvider for MdNavigation {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<LocationLink>>> {
        let text = text.clone();
        let doc_dir = self.doc_dir();
        cx.background_spawn(async move {
            let index = DocIndex::build(&text.to_string());
            let Some(found) = navigate::target_at(&index, offset, doc_dir.as_deref()) else {
                return Ok(Vec::new());
            };
            let (uri, range) = match found.target {
                Target::InDocument(range) => {
                    (THIS_DOCUMENT_URI.to_string(), lsp_range(&text, range))
                }
                Target::File { path, anchor } => (
                    file_uri(&path, anchor.as_deref()),
                    lsp_types::Range::default(),
                ),
                Target::External(url) => (url, lsp_types::Range::default()),
            };
            let Ok(target_uri) = Uri::from_str(&uri) else {
                return Ok(Vec::new()); // a destination that is not a valid URI: nothing to follow
            };
            Ok(vec![LocationLink {
                origin_selection_range: Some(lsp_range(&text, found.origin)),
                target_uri,
                target_range: range,
                target_selection_range: range,
            }])
        })
    }
}

impl HoverProvider for MdNavigation {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let text = text.clone();
        let doc_dir = self.doc_dir();
        cx.background_spawn(async move {
            let source = text.to_string();
            let index = DocIndex::build(&source);
            Ok(
                navigate::hover_at(&index, &source, offset, doc_dir.as_deref(), &FileSystem).map(
                    |(range, message)| Hover {
                        contents: HoverContents::Scalar(MarkedString::String(message)),
                        range: Some(lsp_range(&text, range)),
                    },
                ),
            )
        })
    }
}
