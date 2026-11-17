//! Incremental, line-oriented Markdown lexer for syntax colouring (PLAN D3).
//!
//! Markdown colouring is almost entirely line-local. The exceptions — fenced code, front matter,
//! HTML comments and `$$` math blocks — change how *following* lines are read, so the lexer keeps
//! one [`BlockState`] per line: the state at the start of that line. After an edit, lines are
//! re-scanned from the edited row only until the state at the start of a row past the edit is the
//! same as before (it has converged); everything below is untouched. Tokens for a line are produced
//! on demand from its start state, so only visible lines are ever tokenized.
//!
//! Inline syntax (emphasis, links, code spans…) is delegated to `pulldown-cmark` on the single
//! line, so the highlighter and the structural index use one parser and one set of extensions.
//!
//! Known simplifications (colouring only, never affects the file): indented code blocks are not
//! coloured as code, and a table body row is only recognised when it starts with `|`.

use std::borrow::Cow;
use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Parser, Tag};
use ropey::{LineType, Rope};

/// The buffer always uses LF line breaks (PLAN §3.3), so rows are counted by LF.
const LINES: LineType = LineType::LF;

/// Lines longer than this are not inline-lexed (minified or generated content); they keep only
/// their block-level colour. Mirrors the editor's `max_layout_line_chars` default (PLAN §3.3).
pub const MAX_INLINE_LEX_BYTES: usize = 10_000;

/// State at the start of a line, for the constructs that span lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlockState {
    #[default]
    Normal,
    /// Inside a fenced code block opened by `len` repetitions of `fence` (`` ` `` or `~`).
    FencedCode { fence: u8, len: u8 },
    /// Inside front matter opened on the first line by `---` (YAML) or `+++` (TOML).
    FrontMatter { fence: u8 },
    /// Inside an HTML comment that has not been closed yet.
    HtmlComment,
    /// Inside a `$$` display-math block.
    MathBlock,
}

/// What a span of text is. The UI maps each kind to a theme style; nothing here knows colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Heading text of the given level (1–6).
    Heading(u8),
    /// `#` characters and setext underlines.
    HeadingMarker,
    Emphasis,
    Strong,
    Strikethrough,
    /// Inline code, including its backticks.
    CodeSpan,
    /// A code fence line (```` ``` ```` plus info string).
    CodeFence,
    /// A line inside a fenced code block.
    CodeBlock,
    /// Link text, image alt text, reference labels.
    LinkText,
    /// Link destinations, reference ids, autolinks.
    LinkUrl,
    /// Markup punctuation that should recede: brackets, parentheses, emphasis delimiters.
    Marker,
    ListMarker,
    TaskMarker,
    QuoteMarker,
    /// Thematic break (`---`, `***`).
    Rule,
    /// Table pipes and delimiter rows.
    TableMarker,
    FrontMatter,
    Html,
    Comment,
    Math,
    /// Footnote references and definitions (`[^1]`).
    FootnoteRef,
    /// The backslash of an escape (`\*`).
    Escape,
}

/// A coloured span. `range` is in bytes, relative to the start of its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// Per-document lexer state: the [`BlockState`] at the start of every line.
#[derive(Clone, Debug, Default)]
pub struct Highlighter {
    states: Vec<BlockState>,
}

impl Highlighter {
    pub fn new(text: &Rope) -> Self {
        let mut highlighter = Self::default();
        highlighter.rebuild(text);
        highlighter
    }

    /// Re-scan the whole document.
    pub fn rebuild(&mut self, text: &Rope) {
        let rows = text.len_lines(LINES);
        self.states.clear();
        self.states.reserve(rows);
        let mut state = BlockState::Normal;
        for (row, line) in text.lines(LINES).enumerate().take(rows) {
            self.states.push(state);
            state = next_state(state, &Cow::from(line), row);
        }
        self.states.resize(rows, state);
    }

    /// Update after an edit. Rows are those of a tree-sitter style edit: the text between
    /// `first_row` and `old_last_row` (inclusive) was replaced by text spanning `first_row` to
    /// `new_last_row`. `text` is the document after the edit.
    pub fn edit(
        &mut self,
        text: &Rope,
        first_row: usize,
        old_last_row: usize,
        new_last_row: usize,
    ) {
        let rows = text.len_lines(LINES);
        if first_row >= self.states.len() || old_last_row < first_row || new_last_row < first_row {
            return self.rebuild(text);
        }
        // States at the start of the replaced rows are stale; give the new rows placeholders.
        let old_end = (old_last_row + 1).min(self.states.len());
        let added = new_last_row - first_row;
        self.states.splice(
            first_row + 1..old_end,
            std::iter::repeat_n(BlockState::Normal, added),
        );
        if self.states.len() != rows {
            return self.rebuild(text);
        }
        // The state at the start of `first_row` depends only on rows above it, so it is still valid.
        let mut state = self.states[first_row];
        for row in first_row..rows.saturating_sub(1) {
            state = next_state(state, &Cow::from(text.line(row, LINES)), row);
            let next = row + 1;
            if next > new_last_row && self.states[next] == state {
                break; // converged: everything below is unchanged
            }
            self.states[next] = state;
        }
    }

    pub fn line_count(&self) -> usize {
        self.states.len()
    }

    pub fn state_at(&self, row: usize) -> BlockState {
        self.states.get(row).copied().unwrap_or_default()
    }

    /// Tokens of one line, in order and non-overlapping; ranges are relative to the line start.
    pub fn line_tokens(&self, text: &Rope, row: usize) -> Vec<Token> {
        let Some(&state) = self.states.get(row) else {
            return Vec::new();
        };
        let line = Cow::from(text.line(row, LINES));
        let next = (row + 1 < self.states.len()).then(|| Cow::from(text.line(row + 1, LINES)));
        tokens_for_line(state, &line, next.as_deref(), row)
    }

    /// The fenced code block whose content includes `row` (not its fences), if any.
    pub fn fenced_block_at(&self, text: &Rope, row: usize) -> Option<FencedBlock> {
        let in_code = |r: usize| matches!(self.state_at(r), BlockState::FencedCode { .. });
        if !in_code(row) || self.states.len() != text.len_lines(LINES) {
            return None;
        }
        let BlockState::FencedCode { fence, len } = self.state_at(row) else {
            return None;
        };
        let line = |r: usize| Cow::from(text.line(r, LINES));
        let is_close = |r: usize| {
            is_fence_close(
                strip_containers(line(r).trim_end_matches(['\n', '\r'])),
                fence,
                len,
            )
        };
        if is_close(row) {
            return None;
        }
        // The opening fence is the nearest row above that does not start inside the block.
        let open = (0..row).rev().find(|&r| !in_code(r))?;
        let (quotes, indent, language) = open_fence(line(open).trim_end_matches(['\n', '\r']))?;
        let mut end = row + 1;
        while end < self.states.len() && in_code(end) && !is_close(end) {
            end += 1;
        }
        Some(FencedBlock {
            language,
            rows: open + 1..end,
            quotes,
            indent,
        })
    }

    /// Every fenced code block with at least one content line, in document order.
    pub fn fenced_blocks(&self, text: &Rope) -> Vec<FencedBlock> {
        let mut blocks = Vec::new();
        let mut row = 0;
        while row < self.states.len() {
            match self.fenced_block_at(text, row) {
                Some(block) => {
                    row = block.rows.end;
                    blocks.push(block);
                }
                None => row += 1,
            }
        }
        blocks
    }
}

/// A fenced code block as the lexer sees it: its language, its content rows, and how much of each
/// content line is container (blockquote markers, the fence's indentation) rather than code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FencedBlock {
    /// First word of the info string; empty when there is none.
    pub language: String,
    /// Rows between the fences (to the end of the document when the block is never closed).
    pub rows: Range<usize>,
    /// Blockquote markers around the opening fence.
    quotes: usize,
    /// Indentation of the opening fence inside its quotes (list items included); content lines
    /// lose up to this many spaces, as CommonMark specifies.
    indent: usize,
}

impl FencedBlock {
    /// Byte offset where the code starts in one of this block's lines.
    pub fn code_start(&self, line: &str) -> usize {
        let (pos, _) = skip_quotes(line, self.quotes);
        let spaces = line[pos..]
            .bytes()
            .take(self.indent)
            .take_while(|&b| b == b' ')
            .count();
        pos + spaces
    }

    /// The code on `row` (one of [`Self::rows`]), without its line break.
    pub fn code_line<'a>(&self, text: &'a Rope, row: usize) -> Cow<'a, str> {
        let line = Cow::from(text.line(row, LINES));
        let end = line.trim_end_matches(['\n', '\r']).len();
        match line {
            Cow::Borrowed(line) => Cow::Borrowed(&line[self.code_start(line)..end]),
            Cow::Owned(line) => Cow::Owned(line[self.code_start(&line)..end].to_string()),
        }
    }
}

/// Skip up to `max` blockquote markers (each with the indentation before it and one space after
/// it); returns the offset after them and how many there were.
fn skip_quotes(line: &str, max: usize) -> (usize, usize) {
    let mut pos = 0;
    let mut quotes = 0;
    while quotes < max {
        let at = pos + leading_whitespace(&line[pos..]);
        if line.as_bytes().get(at) != Some(&b'>') {
            break;
        }
        pos = at + 1;
        if line.as_bytes().get(pos) == Some(&b' ') {
            pos += 1;
        }
        quotes += 1;
    }
    (pos, quotes)
}

/// Quote depth, fence indentation and language of an opening fence line.
fn open_fence(line: &str) -> Option<(usize, usize, String)> {
    let (pos, quotes) = skip_quotes(line, usize::MAX);
    let rest = &line[pos..];
    let mut start = leading_whitespace(rest);
    if let Some(marker) = list_marker(&rest[start..]) {
        start += marker;
        start += leading_whitespace(&rest[start..]);
    }
    let body = &rest[start..];
    let (fence, len) = fence_open(body)?;
    let info = &body[run_of(body, fence).max(usize::from(len))..];
    let language = info.split_whitespace().next().unwrap_or_default();
    Some((quotes, start, language.to_string()))
}

/// The state at the start of the line after `line`, given the state at its start.
pub fn next_state(state: BlockState, line: &str, row: usize) -> BlockState {
    let content = line.trim_end_matches(['\n', '\r']);
    match state {
        BlockState::Normal => {
            if row == 0
                && let Some(fence) = front_matter_fence(content)
            {
                return BlockState::FrontMatter { fence };
            }
            let body = strip_containers(content);
            if let Some((fence, len)) = fence_open(body) {
                BlockState::FencedCode { fence, len }
            } else if opens_math_block(body) {
                BlockState::MathBlock
            } else if unclosed_comment_start(content).is_some() {
                BlockState::HtmlComment
            } else {
                BlockState::Normal
            }
        }
        BlockState::FencedCode { fence, len } => {
            if is_fence_close(strip_containers(content), fence, len) {
                BlockState::Normal
            } else {
                state
            }
        }
        BlockState::FrontMatter { fence } => {
            if is_front_matter_close(content, fence) {
                BlockState::Normal
            } else {
                state
            }
        }
        BlockState::HtmlComment => {
            if content.contains("-->") {
                BlockState::Normal
            } else {
                state
            }
        }
        BlockState::MathBlock => {
            if strip_containers(content).contains("$$") {
                BlockState::Normal
            } else {
                state
            }
        }
    }
}

/// Tokens of a line that starts in `state`. `next_line` is used to recognise setext headings
/// and table header rows.
pub fn tokens_for_line(
    state: BlockState,
    line: &str,
    next_line: Option<&str>,
    row: usize,
) -> Vec<Token> {
    let content = line.trim_end_matches(['\n', '\r']);
    let end = content.len();
    let mut out = Vec::new();
    match state {
        BlockState::FencedCode { fence, len } => {
            let kind = if is_fence_close(strip_containers(content), fence, len) {
                TokenKind::CodeFence
            } else {
                TokenKind::CodeBlock
            };
            push(&mut out, 0..end, kind);
        }
        BlockState::FrontMatter { fence } => {
            let kind = if is_front_matter_close(content, fence) {
                TokenKind::Marker
            } else {
                TokenKind::FrontMatter
            };
            push(&mut out, 0..end, kind);
        }
        BlockState::MathBlock => push(&mut out, 0..end, TokenKind::Math),
        BlockState::HtmlComment => match content.find("-->") {
            Some(close) => {
                push(&mut out, 0..close + 3, TokenKind::Comment);
                lex_inline(&content[close + 3..], close + 3, None, false, &mut out);
            }
            None => push(&mut out, 0..end, TokenKind::Comment),
        },
        BlockState::Normal => lex_normal(content, next_line, row, &mut out),
    }
    out
}

fn lex_normal(content: &str, next_line: Option<&str>, row: usize, out: &mut Vec<Token>) {
    let end = content.len();
    if row == 0 && front_matter_fence(content).is_some() {
        push(out, 0..end, TokenKind::Marker);
        return;
    }

    // Blockquote markers, possibly nested (`> > text`).
    let mut pos = 0;
    loop {
        let indent = leading_whitespace(&content[pos..]);
        if content.as_bytes().get(pos + indent) == Some(&b'>') {
            push(out, pos + indent..pos + indent + 1, TokenKind::QuoteMarker);
            pos += indent + 1;
        } else {
            break;
        }
    }
    let start = pos + leading_whitespace(&content[pos..]);
    let body = &content[start..];
    if body.is_empty() {
        return;
    }

    // Lines that open a multi-line block.
    if fence_open(body).is_some() {
        push(out, start..end, TokenKind::CodeFence);
        return;
    }
    if opens_math_block(body) {
        push(out, start..end, TokenKind::Math);
        return;
    }
    if let Some(open) = unclosed_comment_start(body) {
        lex_inline(&body[..open], start, None, false, out);
        push(out, start + open..end, TokenKind::Comment);
        return;
    }

    // Whole-line block constructs.
    if is_thematic_break(body) {
        push(out, start..end, TokenKind::Rule);
        return;
    }
    if is_setext_underline(body, b'=') {
        push(out, start..end, TokenKind::HeadingMarker);
        return;
    }
    if let Some((level, hashes)) = atx_heading(body) {
        push(out, start..start + hashes, TokenKind::HeadingMarker);
        let text_end = start + atx_text_end(body);
        lex_inline(
            &content[start + hashes..text_end],
            start + hashes,
            Some(TokenKind::Heading(level)),
            false,
            out,
        );
        push(out, text_end..end, TokenKind::HeadingMarker);
        return;
    }
    if is_table_delimiter_row(body) {
        push(out, start..end, TokenKind::TableMarker);
        return;
    }
    if lex_definition(body, start, out) {
        return;
    }

    // Leading list marker and task box.
    let mut cursor = start;
    if let Some(marker) = list_marker(body) {
        push(out, start..start + marker, TokenKind::ListMarker);
        cursor = start + marker;
        cursor += leading_whitespace(&content[cursor..]);
        if let Some(task) = task_marker(&content[cursor..]) {
            push(out, cursor..cursor + task, TokenKind::TaskMarker);
            cursor += task;
        }
    }

    if fence_open(strip_indent(&content[cursor..])).is_some() {
        push(out, cursor..end, TokenKind::CodeFence);
        return;
    }

    let next = next_line.map(|line| line.trim_end_matches(['\n', '\r']));
    let base = match next {
        Some(next) if cursor == start && is_setext_underline(strip_indent(next), b'=') => {
            Some(TokenKind::Heading(1))
        }
        Some(next) if cursor == start && is_setext_underline(strip_indent(next), b'-') => {
            Some(TokenKind::Heading(2))
        }
        _ => None,
    };
    let table = body.starts_with('|') || next.is_some_and(is_table_delimiter_row);
    lex_inline(&content[cursor..], cursor, base, table, out);
}

/// Reference definitions (`[id]: url "title"`) and footnote definitions (`[^1]: text`).
fn lex_definition(body: &str, start: usize, out: &mut Vec<Token>) -> bool {
    if !body.starts_with('[') {
        return false;
    }
    let Some(close) = body.find("]:") else {
        return false;
    };
    let label = &body[1..close];
    if label.trim().is_empty() || label.contains(['[', ']']) {
        return false;
    }
    let after = close + 2;
    if label.starts_with('^') {
        push(out, start..start + after, TokenKind::FootnoteRef);
        lex_inline(&body[after..], start + after, None, false, out);
        return true;
    }
    push(out, start..start + 1, TokenKind::Marker);
    push(out, start + 1..start + close, TokenKind::LinkText);
    push(out, start + close..start + after, TokenKind::Marker);
    let dest_start = after + leading_whitespace(&body[after..]);
    let dest_len = body[dest_start..]
        .find(char::is_whitespace)
        .unwrap_or(body.len() - dest_start);
    push(
        out,
        start + dest_start..start + dest_start + dest_len,
        TokenKind::LinkUrl,
    );
    push(
        out,
        start + dest_start + dest_len..start + body.len(),
        TokenKind::Marker,
    );
    true
}

/// Inline tokens of `text`, which starts at byte `offset` of its line. `base` colours text that
/// no inline construct claims (heading text). `table` turns unescaped pipes into table markers.
fn lex_inline(
    text: &str,
    offset: usize,
    base: Option<TokenKind>,
    table: bool,
    out: &mut Vec<Token>,
) {
    let len = text.len();
    if len == 0 {
        return;
    }
    if len > MAX_INLINE_LEX_BYTES {
        if let Some(kind) = base {
            push(out, offset..offset + len, kind);
        }
        return;
    }

    // The single line is parsed on its own, so leading indentation must not turn it into an
    // indented code block; it is skipped here and kept as plain text.
    let indent = leading_whitespace(text);
    let parsed = &text[indent..];

    let mut kinds: Vec<Option<TokenKind>> = vec![base; len];
    let mut is_content = vec![false; len];
    let mut containers: Vec<(Range<usize>, TokenKind)> = Vec::new();

    for (event, range) in Parser::new_ext(parsed, crate::parser_options()).into_offset_iter() {
        let range = range.start + indent..range.end + indent;
        match event {
            Event::Start(tag) => {
                let kind = match tag {
                    Tag::Emphasis => TokenKind::Emphasis,
                    Tag::Strong => TokenKind::Strong,
                    Tag::Strikethrough => TokenKind::Strikethrough,
                    Tag::Link {
                        link_type: LinkType::Autolink | LinkType::Email,
                        ..
                    } => TokenKind::LinkUrl,
                    Tag::Link { .. } | Tag::Image { .. } => TokenKind::LinkText,
                    _ => continue,
                };
                fill(&mut kinds, range.clone(), Some(kind));
                containers.push((range, kind));
            }
            Event::Text(_) => fill(&mut is_content, range, true),
            Event::Code(_) => claim(&mut kinds, &mut is_content, range, TokenKind::CodeSpan),
            Event::InlineMath(_) | Event::DisplayMath(_) => {
                claim(&mut kinds, &mut is_content, range, TokenKind::Math)
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                claim(&mut kinds, &mut is_content, range, TokenKind::Html)
            }
            Event::FootnoteReference(_) => {
                claim(&mut kinds, &mut is_content, range, TokenKind::FootnoteRef)
            }
            _ => {}
        }
    }

    // Inside a container, whatever is not content is markup: delimiters, brackets, and — for
    // links — the destination after the text's closing bracket.
    for (range, kind) in &containers {
        let target_from = (*kind == TokenKind::LinkText)
            .then(|| link_target_start(text, range.clone(), &is_content))
            .flatten();
        for i in range.clone() {
            if is_content[i] {
                continue;
            }
            kinds[i] = Some(match target_from {
                Some(from) if i >= from && i + 1 < range.end => TokenKind::LinkUrl,
                _ => TokenKind::Marker,
            });
        }
    }

    let bytes = text.as_bytes();
    let literal = |kind: Option<TokenKind>| {
        matches!(
            kind,
            Some(TokenKind::CodeSpan | TokenKind::Math | TokenKind::Html)
        )
    };
    let mut i = 0;
    while i < len {
        match bytes[i] {
            b'\\' if i + 1 < len && bytes[i + 1].is_ascii_punctuation() && !literal(kinds[i]) => {
                kinds[i] = Some(TokenKind::Escape);
                i += 2;
                continue;
            }
            b'|' if table && !literal(kinds[i]) => kinds[i] = Some(TokenKind::TableMarker),
            // `[^label]` without a definition is plain text to the parser; still colour it.
            b'[' if bytes.get(i + 1) == Some(&b'^') && kinds[i].is_none() => {
                if let Some(close) = text[i..].find(']') {
                    let label = &text[i + 2..i + close];
                    if !label.is_empty() && !label.contains(char::is_whitespace) {
                        fill(&mut kinds, i..i + close + 1, Some(TokenKind::FootnoteRef));
                        i += close + 1;
                        continue;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    // Emit runs.
    let mut run_start = 0;
    for i in 1..=len {
        if i == len || kinds[i] != kinds[run_start] {
            if let Some(kind) = kinds[run_start] {
                push(out, offset + run_start..offset + i, kind);
            }
            run_start = i;
        }
    }
}

/// Byte where a link's destination part begins: after `](` or `][` that follows the link text.
fn link_target_start(text: &str, range: Range<usize>, is_content: &[bool]) -> Option<usize> {
    let after_text = (range.start..range.end)
        .rev()
        .find(|&i| is_content[i])
        .map_or(range.start, |i| i + 1);
    let tail = &text[after_text..range.end];
    tail.find("](")
        .or_else(|| tail.find("]["))
        .map(|p| after_text + p + 2)
}

fn fill<T: Clone>(slots: &mut [T], range: Range<usize>, value: T) {
    let end = range.end.min(slots.len());
    for slot in &mut slots[range.start.min(end)..end] {
        *slot = value.clone();
    }
}

/// A literal span (code, math, HTML): coloured as `kind` and counted as content.
fn claim(
    kinds: &mut [Option<TokenKind>],
    is_content: &mut [bool],
    range: Range<usize>,
    kind: TokenKind,
) {
    fill(kinds, range.clone(), Some(kind));
    fill(is_content, range, true);
}

fn push(out: &mut Vec<Token>, range: Range<usize>, kind: TokenKind) {
    if range.start < range.end {
        out.push(Token { range, kind });
    }
}

fn leading_whitespace(s: &str) -> usize {
    s.len() - s.trim_start_matches([' ', '\t']).len()
}

fn strip_indent(s: &str) -> &str {
    s.trim_start_matches([' ', '\t'])
}

/// Drop indentation, blockquote markers and a list marker, so fences inside quotes and list items
/// are recognised.
fn strip_containers(line: &str) -> &str {
    let mut s = strip_indent(line);
    while let Some(rest) = s.strip_prefix('>') {
        s = strip_indent(rest);
    }
    if let Some(marker) = list_marker(s) {
        s = strip_indent(&s[marker..]);
    }
    s
}

fn run_of(s: &str, byte: u8) -> usize {
    s.bytes().take_while(|&b| b == byte).count()
}

fn fence_open(body: &str) -> Option<(u8, u8)> {
    let fence = *body.as_bytes().first()?;
    if fence != b'`' && fence != b'~' {
        return None;
    }
    let len = run_of(body, fence);
    if len < 3 || (fence == b'`' && body[len..].contains('`')) {
        return None;
    }
    Some((fence, u8::try_from(len).unwrap_or(u8::MAX)))
}

fn is_fence_close(body: &str, fence: u8, len: u8) -> bool {
    let run = run_of(body, fence);
    run >= usize::from(len) && body[run..].trim().is_empty()
}

fn front_matter_fence(content: &str) -> Option<u8> {
    match content.trim_end() {
        "---" => Some(b'-'),
        "+++" => Some(b'+'),
        _ => None,
    }
}

fn is_front_matter_close(content: &str, fence: u8) -> bool {
    let line = content.trim_end();
    match fence {
        b'-' => line == "---" || line == "...",
        _ => line == "+++",
    }
}

/// `$$` that starts a display-math block spanning lines (not a one-line `$$x$$`).
fn opens_math_block(body: &str) -> bool {
    body.strip_prefix("$$")
        .is_some_and(|rest| !rest.contains("$$"))
}

/// Start of an HTML comment that is still open at the end of the line.
fn unclosed_comment_start(line: &str) -> Option<usize> {
    let open = line.rfind("<!--")?;
    (!line[open + 4..].contains("-->")).then_some(open)
}

fn is_thematic_break(body: &str) -> bool {
    let mut marks = body.bytes().filter(|b| !matches!(b, b' ' | b'\t'));
    let Some(first) = marks.next() else {
        return false;
    };
    matches!(first, b'-' | b'*' | b'_') && marks.clone().all(|b| b == first) && marks.count() >= 2
}

fn is_setext_underline(body: &str, mark: u8) -> bool {
    let run = run_of(body, mark);
    let min = if mark == b'-' { 2 } else { 1 };
    run >= min && body[run..].trim().is_empty()
}

/// Level and number of `#` characters of an ATX heading.
fn atx_heading(body: &str) -> Option<(u8, usize)> {
    let hashes = run_of(body, b'#');
    if !(1..=6).contains(&hashes) {
        return None;
    }
    match body.as_bytes().get(hashes) {
        None | Some(b' ' | b'\t') => Some((u8::try_from(hashes).ok()?, hashes)),
        _ => None,
    }
}

/// End of an ATX heading's text, before an optional closing `#` sequence.
fn atx_text_end(body: &str) -> usize {
    let trimmed = body.trim_end();
    let without = trimmed.trim_end_matches('#');
    if without.len() < trimmed.len() && (without.ends_with([' ', '\t']) || without.is_empty()) {
        without.trim_end().len().max(run_of(body, b'#'))
    } else {
        body.len()
    }
}

fn list_marker(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let marker = match bytes.first()? {
        b'-' | b'*' | b'+' => 1,
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 || !matches!(bytes.get(digits), Some(b'.' | b')')) {
                return None;
            }
            digits + 1
        }
        _ => return None,
    };
    matches!(bytes.get(marker), None | Some(b' ' | b'\t')).then_some(marker)
}

fn task_marker(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let boxed = bytes.len() >= 3
        && bytes[0] == b'['
        && matches!(bytes[1], b' ' | b'x' | b'X')
        && bytes[2] == b']';
    (boxed && matches!(bytes.get(3), None | Some(b' ' | b'\t'))).then_some(3)
}

fn is_table_delimiter_row(line: &str) -> bool {
    let line = line.trim();
    line.contains('|')
        && line.contains('-')
        && line
            .bytes()
            .all(|b| matches!(b, b'|' | b':' | b'-' | b' ' | b'\t'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind as K;

    fn kinds(line: &str) -> Vec<(String, TokenKind)> {
        tokens_for_line(BlockState::Normal, line, None, 1)
            .into_iter()
            .map(|t| (line[t.range].to_string(), t.kind))
            .collect()
    }

    fn states(text: &str) -> Vec<BlockState> {
        let h = Highlighter::new(&Rope::from_str(text));
        (0..h.line_count()).map(|r| h.state_at(r)).collect()
    }

    #[test]
    fn heading_marker_and_text() {
        assert_eq!(
            kinds("## Hello *you*"),
            vec![
                ("##".into(), K::HeadingMarker),
                (" Hello ".into(), K::Heading(2)),
                ("*".into(), K::Marker),
                ("you".into(), K::Emphasis),
                ("*".into(), K::Marker),
            ]
        );
    }

    #[test]
    fn closing_hashes_are_markers() {
        let toks = kinds("# Title ##");
        assert_eq!(toks.last().unwrap(), &(" ##".to_string(), K::HeadingMarker));
    }

    #[test]
    fn link_text_url_and_brackets() {
        assert_eq!(
            kinds("see [docs](./a.md#x)"),
            vec![
                ("[".into(), K::Marker),
                ("docs".into(), K::LinkText),
                ("](".into(), K::Marker),
                ("./a.md#x".into(), K::LinkUrl),
                (")".into(), K::Marker),
            ]
        );
    }

    #[test]
    fn image_and_autolink() {
        let toks = kinds("![alt](p.png) <https://x.y>");
        assert_eq!(toks[0], ("![".into(), K::Marker));
        assert!(toks.contains(&("p.png".into(), K::LinkUrl)));
        assert!(toks.contains(&("https://x.y".into(), K::LinkUrl)));
    }

    #[test]
    fn code_span_strong_strike_math_escape() {
        let toks = kinds("`a*b` **s** ~~d~~ $x$ \\*");
        assert!(toks.contains(&("`a*b`".into(), K::CodeSpan)));
        assert!(toks.contains(&("s".into(), K::Strong)));
        assert!(toks.contains(&("d".into(), K::Strikethrough)));
        assert!(toks.contains(&("$x$".into(), K::Math)));
        assert!(toks.contains(&("\\".into(), K::Escape)));
    }

    #[test]
    fn list_task_quote() {
        assert_eq!(
            kinds("> - [x] done"),
            vec![
                (">".into(), K::QuoteMarker),
                ("-".into(), K::ListMarker),
                ("[x]".into(), K::TaskMarker),
            ]
        );
        assert_eq!(kinds("12. item")[0], ("12.".into(), K::ListMarker));
    }

    #[test]
    fn indented_continuation_is_not_code() {
        assert_eq!(
            kinds("    plain **b**"),
            vec![
                ("**".into(), K::Marker),
                ("b".into(), K::Strong),
                ("**".into(), K::Marker),
            ]
        );
    }

    #[test]
    fn table_rows() {
        assert_eq!(
            kinds("|---|:--:|"),
            vec![("|---|:--:|".into(), K::TableMarker)]
        );
        let toks = kinds("| a | `b|c` |");
        assert_eq!(toks.iter().filter(|t| t.1 == K::TableMarker).count(), 3);
        assert!(toks.contains(&("`b|c`".into(), K::CodeSpan)));
        let header = tokens_for_line(BlockState::Normal, "a | b", Some("--|--"), 1);
        assert!(header.iter().any(|t| t.kind == K::TableMarker));
    }

    #[test]
    fn definitions_and_footnotes() {
        assert_eq!(
            kinds("[id]: https://e.com \"T\""),
            vec![
                ("[".into(), K::Marker),
                ("id".into(), K::LinkText),
                ("]:".into(), K::Marker),
                ("https://e.com".into(), K::LinkUrl),
                (" \"T\"".into(), K::Marker),
            ]
        );
        assert_eq!(kinds("[^1]: note")[0], ("[^1]:".into(), K::FootnoteRef));
        assert!(kinds("text[^n] more").contains(&("[^n]".into(), K::FootnoteRef)));
    }

    #[test]
    fn rules_and_setext() {
        assert_eq!(kinds("* * *"), vec![("* * *".into(), K::Rule)]);
        assert_eq!(kinds("==="), vec![("===".into(), K::HeadingMarker)]);
        let toks = tokens_for_line(BlockState::Normal, "Title", Some("---"), 1);
        assert_eq!(
            toks,
            vec![Token {
                range: 0..5,
                kind: K::Heading(2)
            }]
        );
    }

    #[test]
    fn block_states() {
        use BlockState as S;
        let code = S::FencedCode {
            fence: b'`',
            len: 3,
        };
        assert_eq!(
            states("a\n```rust\nlet x;\n```\nb"),
            vec![S::Normal, S::Normal, code, code, S::Normal]
        );
        // A shorter fence does not close; a longer one does.
        let four = S::FencedCode {
            fence: b'~',
            len: 4,
        };
        assert_eq!(
            states("~~~~\n~~~\n~~~~~\nx"),
            vec![S::Normal, four, four, S::Normal]
        );
        // Front matter only on the first line.
        let fm = S::FrontMatter { fence: b'-' };
        assert_eq!(
            states("---\nt: 1\n---\n---\nx"),
            vec![S::Normal, fm, fm, S::Normal, S::Normal]
        );
        assert_eq!(
            states("<!-- a\nb\n-->\nc"),
            vec![S::Normal, S::HtmlComment, S::HtmlComment, S::Normal]
        );
        assert_eq!(
            states("$$\nx\n$$\ny"),
            vec![S::Normal, S::MathBlock, S::MathBlock, S::Normal]
        );
        assert_eq!(states("$$x$$\ny"), vec![S::Normal, S::Normal]);
        // Fences inside quotes and list items.
        assert_eq!(states("> ```\n> a\n> ```\nb")[1], code);
        assert_eq!(
            states("- ```\n  a\n  ```\nb"),
            vec![S::Normal, code, code, S::Normal]
        );
        assert_eq!(kinds("- ```js")[1], ("```js".into(), K::CodeFence));
    }

    #[test]
    fn tokens_inside_blocks() {
        let text = Rope::from_str("```rust\nlet *x*;\n```\n");
        let h = Highlighter::new(&text);
        assert_eq!(h.line_tokens(&text, 0)[0].kind, K::CodeFence);
        assert_eq!(
            h.line_tokens(&text, 1),
            vec![Token {
                range: 0..8,
                kind: K::CodeBlock
            }]
        );
        assert_eq!(h.line_tokens(&text, 2)[0].kind, K::CodeFence);
        assert!(h.line_tokens(&text, 3).is_empty());
    }

    /// Incremental updates must always equal a full rebuild.
    #[test]
    fn incremental_edits_match_rebuild() {
        let pieces = [
            "```\n", "~~~~\n", "text\n", "# h\n", "<!-- x\n", "-->\n", "$$\n", "---\n", "> ```\n",
            "", "a", "\n", "``", "`",
        ];
        let mut text = Rope::from_str("intro\n```js\ncode\n```\n<!--\nc\n-->\n$$\nm\n$$\nend\n");
        let mut h = Highlighter::new(&text);
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound.max(1) as u64) as usize
        };
        for _ in 0..2_000 {
            let len = text.len();
            let mut start = next(len + 1);
            while !text.is_char_boundary(start) {
                start -= 1;
            }
            let mut end = (start + next(12)).min(len);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let insert = pieces[next(pieces.len())];
            let first_row = text.byte_to_line_idx(start, LINES);
            let old_last_row = text.byte_to_line_idx(end, LINES);
            text.remove(start..end);
            text.insert(start, insert);
            let new_last_row = text.byte_to_line_idx(start + insert.len(), LINES);
            h.edit(&text, first_row, old_last_row, new_last_row);
            let fresh = Highlighter::new(&text);
            assert_eq!(
                h.states, fresh.states,
                "diverged after edit at {start}..{end} -> {insert:?}"
            );
        }
    }
}
