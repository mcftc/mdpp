//! `mdpp-markdown`'s incremental lexer behind gpui-kit's `InputHighlighter` seam (PLAN D3).
//!
//! The lexer is shared with the completion provider (it knows whether the cursor is inside a
//! code block), so both see the same per-line block state.
//!
//! Fenced code is coloured from [`CodeColors`], which the document computes in the background
//! after a typing pause (syntect is too slow for a frame). Lines are found by content, so colours
//! stay put while text moves; only a line typed since then is coloured here, on its own.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::language_config::LanguageConfig;
use gpui_kit::component::input::{
    AutoClosingPair, BracketPair, EditorState, FoldRange, HighlightStyleResolver, InputEdit,
    InputHighlighter, InputHighlighterFactory, Rope, set_language_config,
};
use gpui_kit::{App, Context, HighlightStyle, SharedString, StrikethroughStyle, Window, px};
use mdpp_markdown::code::{self, CodeColors, LineHighlights};
use mdpp_markdown::{FencedBlock, Highlighter, TokenKind};
use ropey::LineType;

/// Language id the editor is created with; the factory answers only for it.
pub const LANGUAGE: &str = "markdown";

/// One lexer per document, shared by the highlighter and the completion provider.
pub type SharedLexer = Rc<RefCell<Highlighter>>;

/// The document's latest code colours, replaced by each background run.
pub type SharedCodeColors = Rc<RefCell<Arc<CodeColors>>>;

/// A line typed since the last background run is coloured on the UI thread, at ~0.1–0.5 ms a
/// line; a frame does at most this many (a paste waits for the background run instead).
const TYPED_LINES_PER_FRAME: usize = 4;

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

pub fn factory(
    lexer: SharedLexer,
    code_colors: SharedCodeColors,
    markup_fade: f32,
) -> InputHighlighterFactory {
    Rc::new(move |language: &str| {
        (language == LANGUAGE).then(|| {
            Box::new(MdHighlighter {
                lexer: lexer.clone(),
                code_colors: code_colors.clone(),
                text: Rope::new(),
                markup_fade,
                blocks: RefCell::default(),
                typed: RefCell::default(),
                typed_budget: Cell::new(0),
            }) as Box<dyn InputHighlighter>
        })
    })
}

struct MdHighlighter {
    lexer: SharedLexer,
    code_colors: SharedCodeColors,
    /// Snapshot of the text the lexer state describes (O(1) clone); `styles` reads from it.
    text: Rope,
    markup_fade: f32,
    /// Code blocks seen since the last edit, with their content key (finding a block and hashing
    /// it is per block, not per frame).
    blocks: RefCell<Vec<(FencedBlock, u64)>>,
    /// Lines coloured here since the last edit, by language and content.
    typed: RefCell<HashMap<(String, String), LineHighlights>>,
    typed_budget: Cell<usize>,
}

impl MdHighlighter {
    /// Colours of the code on `row` (inside a fenced block), and where the code starts in it.
    fn code_colors(&self, lexer: &Highlighter, row: usize) -> Option<(usize, LineHighlights)> {
        let mut blocks = self.blocks.borrow_mut();
        let index = match blocks.iter().position(|(b, _)| b.rows.contains(&row)) {
            Some(index) => index,
            None => {
                let block = lexer.fenced_block_at(&self.text, row)?;
                let lines: Vec<_> = block
                    .rows
                    .clone()
                    .map(|r| block.code_line(&self.text, r))
                    .collect();
                let key = code::block_key(&block.language, lines.iter().map(AsRef::as_ref));
                blocks.push((block, key));
                blocks.len() - 1
            }
        };
        let (block, key) = &blocks[index];
        let line = std::borrow::Cow::from(self.text.line(row, ropey::LineType::LF));
        let start = block.code_start(&line);
        let colors = self.code_colors.borrow();
        if let Some(lines) = colors.block(*key) {
            return Some((start, lines.get(row - block.rows.start)?.clone()));
        }
        let code = block.code_line(&self.text, row);
        if let Some(line) = colors.line(&block.language, &code) {
            return Some((start, line.clone()));
        }
        let typed_key = (block.language.clone(), code.into_owned());
        if let Some(line) = self.typed.borrow().get(&typed_key) {
            return Some((start, line.clone()));
        }
        let syntaxes = code::loaded()?;
        let budget = self.typed_budget.get();
        if budget == 0 {
            return None;
        }
        self.typed_budget.set(budget - 1);
        let line = code::highlight_lines(syntaxes, &typed_key.0, [typed_key.1.as_str()])?
            .pop()
            .unwrap_or_default();
        self.typed.borrow_mut().insert(typed_key, line.clone());
        Some((start, line))
    }

    /// Style runs of one token, relative to its line: one run, or for a line of code its colours
    /// on top of the plain code style.
    fn token_runs(
        &self,
        lexer: &Highlighter,
        row: usize,
        token: &mdpp_markdown::Token,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let base = style_for(token.kind, resolver, self.markup_fade);
        let colors = (token.kind == TokenKind::CodeBlock)
            .then(|| self.code_colors(lexer, row))
            .flatten();
        let Some((start, spans)) = colors else {
            return vec![(token.range.clone(), base)];
        };
        // Coloured code: names the theme does not style read as plain text, like in a code editor.
        let mut runs = Vec::with_capacity(spans.len() * 2 + 1);
        let mut pos = token.range.start;
        for (span, name) in spans {
            let span = (start + span.start).max(pos)..(start + span.end).min(token.range.end);
            if span.is_empty() {
                continue;
            }
            if span.start > pos {
                runs.push((pos..span.start, HighlightStyle::default()));
            }
            runs.push((span.clone(), resolver.style(name).unwrap_or_default()));
            pos = span.end;
        }
        if pos < token.range.end {
            runs.push((pos..token.range.end, HighlightStyle::default()));
        }
        runs
    }
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
        self.blocks.get_mut().clear();
        self.typed.get_mut().clear();
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
        self.typed_budget.set(TYPED_LINES_PER_FRAME);
        if range.start < end && lexer.line_count() == text.len_lines(LineType::LF) {
            let first = text.byte_to_line_idx(range.start, LineType::LF);
            let last = text.byte_to_line_idx(end, LineType::LF);
            for row in first..=last.min(lexer.line_count().saturating_sub(1)) {
                let line_start = text.line_to_byte_idx(row, LineType::LF);
                for token in lexer.line_tokens(text, row) {
                    for (run, style) in self.token_runs(&lexer, row, &token, resolver) {
                        let from = (line_start + run.start).max(pos);
                        let to = (line_start + run.end).min(end);
                        if from >= to {
                            continue;
                        }
                        if from > pos {
                            runs.push((pos..from, HighlightStyle::default()));
                        }
                        runs.push((from..to, style));
                        pos = to;
                    }
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
