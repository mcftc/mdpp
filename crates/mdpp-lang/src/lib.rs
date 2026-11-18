//! MD++ intellisense (PLAN D7, §4.3).
//!
//! Pure functions over a rope snapshot, its [`DocIndex`](mdpp_markdown::DocIndex) and the file
//! system (behind [`PathSource`], so tests use a fake). Results carry byte ranges; the UI adapter
//! converts them to its own coordinates. Nothing here depends on a UI toolkit.
//!
//! Completion is two-step so the caller only pays for what it needs: [`context_at`] looks at the
//! current line synchronously (always up to date, PLAN §3.2), and [`candidates`] fills the list,
//! using a `DocIndex` only when [`Context::needs_index`] says so.

pub mod assist;
mod complete;
mod data;
mod diagnostics;
pub mod lines;
pub mod navigate;
mod paths;
pub mod table;

pub use complete::{
    Completion, CompletionKind, Context, ContextKind, candidates, context_at, is_image,
};
pub use diagnostics::{Diagnostic, RULES, diagnose};
pub use paths::{
    DirEntry, FileSystem, OtherHeading, PathSource, file_uri, headings_of, is_markdown,
    path_from_file_uri, percent_decode, percent_encode_path,
};
