//! `mdpp-markdown`'s incremental lexer behind gpui-kit's `InputHighlighter` seam (PLAN D3).
//!
//! The lexer is shared with the completion provider (it knows whether the cursor is inside a
//! code block), so both see the same per-line block state.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::language_config::LanguageConfig;
use gpui_kit::component::input::{
    AutoClosingPair, BracketPair, EditorState, FoldRange, HighlightStyleResolver, InputEdit,
    InputHighlighter, InputHighlighterFactory, Rope, set_language_config,
};
use gpui_kit::{App, Context, HighlightStyle, SharedString, StrikethroughStyle, Window, px};
use mdpp_markdown::{Highlighter, TokenKind};
use ropey::LineType;

/// Language id the editor is created with; the factory answers only for it.
pub const LANGUAGE: &str = "markdown";

/// One lexer per document, shared by the highlighter and the completion provider.
pub type SharedLexer = Rc<RefCell<Highlighter>>;

/// Characters before which auto-closing may insert the closing character (besides whitespace and
/// the end of the text): closing brackets and sentence punctuation.
const AUTO_CLOSE_BEFORE: &str = ")]}.,;:!?";

/// Register the editing configuration of Markdown: auto-closing pairs from settings.
pub fn configure_language(pairs: &[String], cx: &mut App) {
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .filter_map(|pair| {
            let mut chars = pair.chars();
            let (open, close) = (chars.next()?, chars.next()?);
            chars
                .next()
                .is_none()
                .then(|| (open.to_string(), close.to_string()))
        })
        .collect();
    // `LanguageConfig` is non-exhaustive: start from the default and set what Markdown needs.
    let mut config = LanguageConfig::default();
    config.brackets = pairs
        .iter()
        .map(|(o, c)| BracketPair::new(o.clone(), c.clone()))
        .collect();
    config.auto_closing_pairs = Some(
        pairs
            .iter()
            .map(|(o, c)| AutoClosingPair::new(o.clone(), c.clone()))
            .collect(),
    );
    config.auto_close_before = AUTO_CLOSE_BEFORE.into();
    config.indentation_rules = None;
    set_language_config(LANGUAGE, config, cx);
}

pub fn factory(lexer: SharedLexer, markup_fade: f32) -> InputHighlighterFactory {
    Rc::new(move |language: &str| {
        (language == LANGUAGE).then(|| {
            Box::new(MdHighlighter {
                lexer: lexer.clone(),
                text: Rope::new(),
                markup_fade,
            }) as Box<dyn InputHighlighter>
        })
    })
}

struct MdHighlighter {
    lexer: SharedLexer,
    /// Snapshot of the text the lexer state describes (O(1) clone); `styles` reads from it.
    text: Rope,
    markup_fade: f32,
}

impl InputHighlighter for MdHighlighter {
    fn language(&self) -> SharedString {
        LANGUAGE.into()
    }

    fn update(
        &mut self,
        edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        self.text = text.clone();
        let mut lexer = self.lexer.borrow_mut();
        match edit {
            Some(edit) => lexer.edit(
                text,
                edit.start_position.row,
                edit.old_end_position.row,
                edit.new_end_position.row,
            ),
            None => lexer.rebuild(text),
        }
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let text = &self.text;
        let lexer = self.lexer.borrow();
        let end = range.end.min(text.len());
        let mut runs = Vec::new();
        let mut pos = range.start;
        if range.start < end && lexer.line_count() == text.len_lines(LineType::LF) {
            let first = text.byte_to_line_idx(range.start, LineType::LF);
            let last = text.byte_to_line_idx(end, LineType::LF);
            for row in first..=last.min(lexer.line_count().saturating_sub(1)) {
                let line_start = text.line_to_byte_idx(row, LineType::LF);
                for token in lexer.line_tokens(text, row) {
                    let from = (line_start + token.range.start).max(pos);
                    let to = (line_start + token.range.end).min(end);
                    if from >= to {
                        continue;
                    }
                    if from > pos {
                        runs.push((pos..from, HighlightStyle::default()));
                    }
                    runs.push((from..to, style_for(token.kind, resolver, self.markup_fade)));
                    pos = to;
                }
            }
        }
        if pos < range.end {
            runs.push((pos..range.end, HighlightStyle::default()));
        }
        runs
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// Theme style for a token. Names are gpui-kit theme syntax keys (the theme is the metadata);
/// the first one the active theme defines wins. Markup is dimmed by opacity, so it recedes in
/// every theme without needing its own colour.
fn style_for(kind: TokenKind, resolver: &dyn HighlightStyleResolver, fade: f32) -> HighlightStyle {
    use TokenKind as K;
    let (names, dim): (&[&str], bool) = match kind {
        K::Heading(_) => (&["title"], false),
        K::HeadingMarker => (&["title"], true),
        K::Emphasis => (&["emphasis"], false),
        K::Strong => (&["emphasis.strong"], false),
        K::Strikethrough => (&[], true),
        K::CodeSpan => (&["text.code.span", "text.literal"], false),
        K::CodeBlock => (&["text.literal"], false),
        K::CodeFence => (&["comment"], false),
        K::LinkText => (&["link_text"], false),
        K::LinkUrl | K::FootnoteRef => (&["link_uri"], false),
        K::Marker | K::TableMarker => (&[], true),
        K::ListMarker | K::TaskMarker => (&["punctuation.list_marker", "keyword"], false),
        K::QuoteMarker | K::Rule => (&["comment"], true),
        K::FrontMatter => (&["comment.doc", "comment"], false),
        K::Html => (&["tag"], false),
        K::Comment => (&["comment"], false),
        K::Math => (&["string.special", "string"], false),
        K::Escape => (&["string.escape"], true),
    };
    let mut style = names
        .iter()
        .find_map(|name| resolver.style(name))
        .unwrap_or_default();
    if dim {
        style.fade_out = Some(fade);
    }
    if kind == K::Strikethrough {
        style.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            color: None,
        });
    }
    style
}
