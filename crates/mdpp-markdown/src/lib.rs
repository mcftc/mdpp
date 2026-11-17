//! Markdown lexing and structure for MD++.
//!
//! * [`highlight`] — an incremental, line-oriented lexer for syntax colouring. Block state that
//!   spans lines (fenced code, front matter, HTML comments, math blocks) is tracked per line, so an
//!   edit re-lexes only from the edited line until that state converges (PLAN D3).
//! * [`index`] — the document structure (headings, anchors, links, reference definitions), built
//!   from a full `pulldown-cmark` parse on a snapshot (PLAN D4).
//! * [`slug`] — GitHub-compatible heading anchors.
//! * [`blocks`] — the document as blocks with source ranges, for the preview (PLAN D6).
//! * [`export`] — self-contained HTML.
//! * [`code`] — colours inside fenced code blocks (syntect, PLAN D5).
//!
//! This crate never depends on a UI toolkit; it works on [`ropey::Rope`] snapshots and `&str`.

pub mod blocks;
pub mod code;
pub mod export;
pub mod highlight;
pub mod index;
pub mod slug;

pub use highlight::{BlockState, FencedBlock, Highlighter, Token, TokenKind};
pub use index::{
    BrokenReference, DocIndex, Footnote, Heading, Link, LinkKind, ReferenceDefinition,
    normalize_label,
};

/// Words in `text`: whitespace-separated runs containing at least one letter or digit, so Markdown
/// markup (`-`, `#`, `|`, `---`) does not count.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

/// The one place that decides which Markdown extensions MD++ understands. The highlighter and the
/// structural index both use it, so the two never disagree about what a line means.
pub fn parser_options() -> pulldown_cmark::Options {
    use pulldown_cmark::Options;
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_WIKILINKS
}

#[cfg(test)]
mod tests {
    #[test]
    fn words_ignore_markup() {
        assert_eq!(
            super::word_count("# Title\n\n- one two\n- 3\n\n| a | b |\n---\nğüş"),
            7
        );
    }
}
