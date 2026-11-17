//! Colours inside fenced code blocks (PLAN D5): syntect with bat's syntax collection (~200
//! languages). What syntect matched is mapped onto the app theme's highlight names (`keyword`,
//! `string`, …) by `data/code-scopes.txt`, so code follows the light/dark theme rather than a
//! syntect theme. The editor and the preview both use it.
//!
//! Loading the syntax set takes tens of milliseconds, so the UI thread only ever asks
//! [`loaded`] and never waits; [`load`] runs on a background thread.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, LazyLock, OnceLock};

use ropey::Rope;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::highlight::{Highlighter, MAX_INLINE_LEX_BYTES};

/// Highlight names of one line: ordered, non-overlapping, ranges relative to the line start.
pub type LineHighlights = Vec<(Range<usize>, &'static str)>;

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();

/// The syntax set, loading it on first use (blocks for tens of milliseconds the first time).
pub fn load() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

/// The syntax set if [`load`] has finished, without waiting.
pub fn loaded() -> Option<&'static SyntaxSet> {
    SYNTAXES.get()
}

/// `key value` rows of a data file; `-` as the value means none. `#` starts a comment line.
fn rows(text: &'static str) -> impl Iterator<Item = (&'static str, Option<&'static str>)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (key, value) = line.split_once(char::is_whitespace)?;
            let value = value.trim();
            Some((key, (value != "-").then_some(value)))
        })
}

/// Scope prefix → theme highlight name, in priority order.
static SCOPE_NAMES: LazyLock<Vec<(&str, Option<&str>)>> =
    LazyLock::new(|| rows(include_str!("../data/code-scopes.txt")).collect());

/// Info-string name (lower case) → the token syntect knows it by, or `None` for plain text.
static ALIASES: LazyLock<HashMap<&str, Option<&str>>> =
    LazyLock::new(|| rows(include_str!("../data/code-aliases.txt")).collect());

/// The syntax for the language word of a fence's info string; `None` for plain text or a
/// language syntect does not know.
pub fn find_syntax<'s>(syntaxes: &'s SyntaxSet, language: &str) -> Option<&'s SyntaxReference> {
    let lower = language.to_lowercase();
    let token = match ALIASES.get(lower.as_str()) {
        Some(None) => return None,
        Some(Some(alias)) => alias,
        None => language,
    };
    // Syntect matches file extensions exactly and syntax names in any case.
    [token, lower.as_str()]
        .into_iter()
        .find_map(|token| syntaxes.find_syntax_by_token(token))
        .filter(|syntax| syntax.name != "Plain Text")
}

/// The theme highlight name of one scope, or `None` to let the enclosing scope decide.
fn scope_name(scope: Scope) -> Option<&'static str> {
    let scope = scope.build_string();
    SCOPE_NAMES
        .iter()
        .find(|(prefix, _)| {
            scope
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
        .and_then(|&(_, name)| name)
}

/// Highlight each line of a code block (`lines` without their line breaks). `None` when the
/// language is plain text or unknown. Lines longer than the inline-lexing limit stay plain and
/// are not fed to the parser (minified content would stall it).
pub fn highlight_lines<'a>(
    syntaxes: &SyntaxSet,
    language: &str,
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<Vec<LineHighlights>> {
    let syntax = find_syntax(syntaxes, language)?;
    let mut parser = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut names = HashMap::<Scope, Option<&'static str>>::new();
    let mut line_with_break = String::new();
    let mut out = Vec::new();
    for line in lines {
        let mut spans = LineHighlights::new();
        line_with_break.clear();
        line_with_break.push_str(line);
        line_with_break.push('\n'); // the syntaxes are the "newlines" variant
        if line.len() <= MAX_INLINE_LEX_BYTES
            && let Ok(operations) = parser.parse_line(&line_with_break, syntaxes)
        {
            let mut cursor = 0;
            for (index, operation) in operations {
                let index = index.min(line.len());
                push(&mut spans, cursor..index, &stack, &mut names);
                // A scope operation syntect cannot apply leaves the stack as it was.
                let _ = stack.apply(&operation);
                cursor = index;
            }
            push(&mut spans, cursor..line.len(), &stack, &mut names);
        }
        out.push(spans);
    }
    Some(out)
}

/// Add `range` with the name of the innermost scope that has one (so the quotes of a string
/// colour as the string), merging with the previous span when it continues it.
fn push(
    spans: &mut LineHighlights,
    range: Range<usize>,
    stack: &ScopeStack,
    names: &mut HashMap<Scope, Option<&'static str>>,
) {
    if range.is_empty() {
        return;
    }
    let Some(name) = stack
        .scopes
        .iter()
        .rev()
        .find_map(|scope| *names.entry(*scope).or_insert_with(|| scope_name(*scope)))
    else {
        return;
    };
    match spans.last_mut() {
        Some((last, last_name)) if last.end == range.start && *last_name == name => {
            last.end = range.end;
        }
        _ => spans.push((range, name)),
    }
}

/// Colours of a document's fenced code blocks, computed in the background on a snapshot (syntect
/// costs 0.1–0.5 ms per line, too slow for the UI thread) and looked up by content, so they stay
/// right while the text moves under them:
/// * by block (language and every line): exact;
/// * by line (language and that line): for a block being edited, whose other lines keep their
///   colours until the next run. Right unless a line's colour depends on the lines above it
///   (inside a multi-line comment or string); the next run corrects it.
#[derive(Debug, Default)]
pub struct CodeColors {
    blocks: HashMap<u64, Arc<[LineHighlights]>>,
    lines: HashMap<u64, (Arc<[LineHighlights]>, usize)>,
}

impl CodeColors {
    /// Colour the fenced blocks of `text`, at most `max_lines` code lines in all (larger blocks
    /// stay plain). Blocks unchanged since `previous` are reused, not parsed again. Loads the
    /// syntax set on first use: call it on a background thread.
    pub fn build(text: &Rope, previous: &CodeColors, max_lines: usize) -> CodeColors {
        let syntaxes = load();
        let mut out = CodeColors::default();
        let mut budget = max_lines;
        for block in Highlighter::new(text).fenced_blocks(text) {
            if block.rows.len() > budget {
                continue;
            }
            let lines: Vec<Cow<str>> = block
                .rows
                .clone()
                .map(|row| block.code_line(text, row))
                .collect();
            let key = block_key(&block.language, lines.iter().map(AsRef::as_ref));
            let colors = match previous.blocks.get(&key) {
                Some(colors) => colors.clone(),
                None => {
                    let Some(colors) =
                        highlight_lines(syntaxes, &block.language, lines.iter().map(AsRef::as_ref))
                    else {
                        continue; // plain text or an unknown language: costs nothing
                    };
                    colors.into()
                }
            };
            budget -= lines.len();
            for (index, line) in lines.iter().enumerate() {
                out.lines
                    .entry(line_key(&block.language, line))
                    .or_insert_with(|| (colors.clone(), index));
            }
            out.blocks.insert(key, colors);
        }
        out
    }

    /// Colours of every line of the block with this [`block_key`].
    pub fn block(&self, key: u64) -> Option<&[LineHighlights]> {
        self.blocks.get(&key).map(AsRef::as_ref)
    }

    /// Colours this line had somewhere in a block of `language`.
    pub fn line(&self, language: &str, line: &str) -> Option<&LineHighlights> {
        let (colors, index) = self.lines.get(&line_key(language, line))?;
        colors.get(*index)
    }
}

/// Identity of a code block's content, for [`CodeColors::block`].
pub fn block_key<'a>(language: &str, lines: impl IntoIterator<Item = &'a str>) -> u64 {
    let mut hasher = DefaultHasher::new();
    language.hash(&mut hasher);
    for line in lines {
        line.hash(&mut hasher);
    }
    hasher.finish()
}

fn line_key(language: &str, line: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    (language, line).hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named<'a>(line: &'a str, spans: &LineHighlights) -> Vec<(&'a str, &'static str)> {
        spans
            .iter()
            .map(|(range, name)| (&line[range.clone()], *name))
            .collect()
    }

    #[test]
    fn rust_keywords_strings_and_comments() {
        let lines = ["fn main() {", "    let s = \"hi\"; // greet", "}"];
        let out = highlight_lines(load(), "rust", lines).expect("rust is known");
        assert_eq!(out.len(), 3);
        assert!(named(lines[0], &out[0]).contains(&("fn", "keyword")));
        let second = named(lines[1], &out[1]);
        assert!(second.contains(&("\"hi\"", "string")), "{second:?}");
        assert!(second.contains(&("// greet", "comment")), "{second:?}");
    }

    #[test]
    fn state_carries_across_lines() {
        let lines = ["/* a", "b */ x"];
        let out = highlight_lines(load(), "c", lines).expect("c is known");
        assert_eq!(named(lines[1], &out[1])[0], ("b */", "comment"));
    }

    #[test]
    fn aliases_plain_text_and_unknown_languages() {
        let syntaxes = load();
        assert!(find_syntax(syntaxes, "c#").is_some());
        assert!(find_syntax(syntaxes, "Python").is_some());
        assert!(find_syntax(syntaxes, "py").is_some());
        assert!(find_syntax(syntaxes, "text").is_none());
        assert!(find_syntax(syntaxes, "no-such-language").is_none());
    }

    #[test]
    fn every_data_row_names_something() {
        assert!(SCOPE_NAMES.len() > 10 && ALIASES.len() > 5);
        // A row like `comment` without a value would silently drop that scope's colour.
        assert_eq!(
            rows(include_str!("../data/code-scopes.txt")).count(),
            include_str!("../data/code-scopes.txt")
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .count()
        );
    }

    #[test]
    fn colours_found_by_block_and_by_line() {
        let text = Rope::from_str(
            "# Doc\n\n> ```rust\n> let a = 1;\n> // done\n> ```\n\n- item\n\n  ```py\n  x = 'q'\n  ```\n\n```\nplain\n```\n",
        );
        let colors = CodeColors::build(&text, &CodeColors::default(), 100);
        let rust = colors
            .block(block_key("rust", ["let a = 1;", "// done"]))
            .expect("quote markers are not part of the code");
        assert!(named("let a = 1;", &rust[0]).contains(&("let", "keyword")));
        assert_eq!(named("// done", &rust[1]), [("// done", "comment")]);
        // The list item's indentation is not code either; the plain block costs nothing.
        assert!(colors.block(block_key("py", ["x = 'q'"])).is_some());
        assert_eq!(colors.blocks.len(), 2);
        // Lines are found on their own while their block is being edited.
        assert!(colors.line("rust", "// done").is_some());
        assert!(colors.line("py", "// done").is_none());
        // Reused when unchanged; over the line budget, a block stays plain.
        let again = CodeColors::build(&text, &colors, 2);
        assert!(Arc::ptr_eq(
            &again.blocks[&block_key("rust", ["let a = 1;", "// done"])],
            &colors.blocks[&block_key("rust", ["let a = 1;", "// done"])]
        ));
        assert_eq!(again.blocks.len(), 1);
    }
}
