//! Find and replace (PLAN D15): literal or regular-expression search with case and whole-word
//! options, optionally inside a range, and replacement with `$1`-style groups in regex mode.
//!
//! Searches run on a `&str` snapshot of the document. Copying a rope out costs ≈ 0.3 ms per MB,
//! which is simpler than a rope cursor adapter (`regex-cursor` targets ropey 1, the editor uses
//! ropey 2) and fast enough for interactive search.

use std::ops::Range;
use std::path::{Path, PathBuf};

use regex::{Regex, RegexBuilder};

use crate::text::{Eol, decode};

/// More matches than this are not collected: the count shows "10000+" and the editor stays fast.
pub const MAX_MATCHES: usize = 10_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}

#[derive(Clone, Debug)]
pub struct Search {
    regex: Regex,
    options: SearchOptions,
}

impl Search {
    /// `Err` carries a message for an invalid regular expression.
    pub fn new(pattern: &str, options: SearchOptions) -> Result<Self, String> {
        let source = if options.regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        let regex = RegexBuilder::new(&source)
            .case_insensitive(!options.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|error| match error {
                regex::Error::Syntax(message) => {
                    message.lines().last().unwrap_or_default().to_string()
                }
                other => other.to_string(),
            })?;
        Ok(Search { regex, options })
    }

    /// Non-empty matches in `text` (inside `within` when given), at most `limit`.
    pub fn find_all(
        &self,
        text: &str,
        within: Option<Range<usize>>,
        limit: usize,
    ) -> Vec<Range<usize>> {
        let scope = within.unwrap_or(0..text.len());
        let scope = scope.start.min(text.len())..scope.end.min(text.len());
        self.regex
            .find_iter(&text[..scope.end])
            .map(|m| m.range())
            .filter(|r| r.start >= scope.start && !r.is_empty())
            .filter(|r| !self.options.whole_word || is_whole_word(text, r))
            .take(limit)
            .collect()
    }

    /// What `matched` (a match of this search) is replaced with.
    pub fn replacement_for(&self, matched: &str, replacement: &str) -> String {
        if !self.options.regex {
            return replacement.to_string();
        }
        let mut out = String::new();
        if let Some(captures) = self.regex.captures(matched) {
            captures.expand(replacement, &mut out);
        }
        out
    }

    /// Replace every match (inside `within` when given). Returns the new text and the count.
    pub fn replace_all(
        &self,
        text: &str,
        within: Option<Range<usize>>,
        replacement: &str,
    ) -> (String, usize) {
        let matches = self.find_all(text, within, usize::MAX);
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for range in &matches {
            out.push_str(&text[last..range.start]);
            out.push_str(&self.replacement_for(&text[range.clone()], replacement));
            last = range.end;
        }
        out.push_str(&text[last..]);
        (out, matches.len())
    }
}

/// One match of find in files, as its result row shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatch {
    pub path: PathBuf,
    /// Line and column (in characters), from 0.
    pub line: u32,
    pub column: u32,
    /// The line around the match, and the match within it.
    pub preview: String,
    pub highlight: Range<usize>,
}

/// What find in files found, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileResults {
    pub matches: Vec<FileMatch>,
    pub files_with_matches: usize,
    /// Stopped at the match limit.
    pub truncated: bool,
}

/// Characters of a result line shown before the match.
const PREVIEW_BEFORE: usize = 40;
/// Characters of a result line shown in all.
const PREVIEW_CHARS: usize = 160;

/// Search `files` (decoded like opened files; binary files and files over `max_bytes` are
/// skipped) for at most `limit` matches.
pub fn search_files(
    search: &Search,
    files: &[PathBuf],
    max_bytes: u64,
    limit: usize,
) -> FileResults {
    let mut results = FileResults::default();
    for path in files {
        let Some(text) = read_text(path, max_bytes) else {
            continue;
        };
        let found = search.find_all(&text, None, limit - results.matches.len());
        if found.is_empty() {
            continue;
        }
        results.files_with_matches += 1;
        let mut line = 0;
        let mut line_start = 0;
        let mut counted = 0; // bytes up to which newlines were counted
        for range in found {
            line += text[counted..range.start].matches('\n').count() as u32;
            counted = range.start;
            line_start = text[..range.start]
                .rfind('\n')
                .map_or(0, |i| i + 1)
                .max(line_start);
            let line_end = text[range.start..]
                .find('\n')
                .map_or(text.len(), |i| range.start + i);
            let column = text[line_start..range.start].chars().count();
            let skip = column.saturating_sub(PREVIEW_BEFORE);
            let shown_start = text[line_start..]
                .char_indices()
                .nth(skip)
                .map_or(line_start, |(i, _)| line_start + i);
            let shown: String = text[shown_start..line_end]
                .chars()
                .take(PREVIEW_CHARS)
                .collect();
            let trimmed = shown.trim_start();
            let cut = (range.start - shown_start).saturating_sub(shown.len() - trimmed.len());
            let preview = trimmed.trim_end().to_string();
            let highlight = cut.min(preview.len())..(cut + range.len()).min(preview.len());
            results.matches.push(FileMatch {
                path: path.clone(),
                line,
                column: column as u32,
                preview,
                highlight,
            });
        }
        if results.matches.len() >= limit {
            results.truncated = true;
            break;
        }
    }
    results
}

fn read_text(path: &Path, max_bytes: u64) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > max_bytes {
        return None;
    }
    decode(&std::fs::read(path).ok()?, Eol::Lf)
        .ok()
        .map(|loaded| loaded.text)
}

/// Not preceded or followed by a word character (works for patterns like `c++` too, unlike `\b`).
fn is_whole_word(text: &str, range: &Range<usize>) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let before = text[..range.start].chars().next_back().is_some_and(is_word);
    let after = text[range.end..].chars().next().is_some_and(is_word);
    !before && !after
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(pattern: &str, options: SearchOptions, text: &'a str) -> Vec<&'a str> {
        let search = Search::new(pattern, options).unwrap();
        search
            .find_all(text, None, MAX_MATCHES)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    const PLAIN: SearchOptions = SearchOptions {
        regex: false,
        case_sensitive: false,
        whole_word: false,
    };

    #[test]
    fn find_in_files_rows() {
        let dir = std::env::temp_dir().join(format!("mdpp-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, binary) = (dir.join("a.md"), dir.join("b.md"), dir.join("c.bin"));
        let long = format!("{}needle end", "x".repeat(100));
        std::fs::write(
            &a,
            format!("one\r\n  two needle\r\n{long}\nneedle needle\n"),
        )
        .unwrap();
        std::fs::write(&b, "nothing here").unwrap();
        std::fs::write(&binary, b"needle\0\0").unwrap();
        let search = Search::new("needle", PLAIN).unwrap();
        let files = [a.clone(), b, binary];
        let found = search_files(&search, &files, 1 << 20, 100);
        assert_eq!(found.files_with_matches, 1);
        let rows: Vec<_> = found
            .matches
            .iter()
            .map(|m| (m.line, m.column, &m.preview[m.highlight.clone()]))
            .collect();
        assert_eq!(
            rows,
            [
                (1, 6, "needle"),
                (2, 100, "needle"),
                (3, 0, "needle"),
                (3, 7, "needle")
            ]
        );
        assert_eq!(found.matches[0].preview, "two needle");
        assert!(
            found.matches[1].preview.starts_with(&"x".repeat(40))
                && found.matches[1].preview.len() < 100
        );
        let capped = search_files(&search, &files, 1 << 20, 2);
        assert!(capped.truncated && capped.matches.len() == 2);
        assert!(
            search_files(&search, &files, 4, 100).matches.is_empty(),
            "size cap"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn literal_and_case() {
        assert_eq!(find("a.b", PLAIN, "a.b axb A.B"), vec!["a.b", "A.B"]);
        let cased = SearchOptions {
            case_sensitive: true,
            ..PLAIN
        };
        assert_eq!(find("a.b", cased, "a.b axb A.B"), vec!["a.b"]);
        assert_eq!(find("ğüş", PLAIN, "ĞÜŞ ğüş"), vec!["ĞÜŞ", "ğüş"]);
    }

    #[test]
    fn whole_words_with_punctuation() {
        let whole = SearchOptions {
            whole_word: true,
            ..PLAIN
        };
        assert_eq!(
            find("cat", whole, "cat concat cat_ (cat)"),
            vec!["cat", "cat"]
        );
        // Only letters, digits and `_` count as word characters (as in Notepad++), so `c++` is
        // found in `c+++` but not in `xc++`.
        assert_eq!(find("c++", whole, "c++ c+++ xc++"), vec!["c++", "c++"]);
    }

    #[test]
    fn regex_lines_and_groups() {
        let regex = SearchOptions {
            regex: true,
            ..PLAIN
        };
        assert_eq!(
            find("^#+ ", regex, "# a\ntext # b\n## c"),
            vec!["# ", "## "]
        );
        assert!(find("x*", regex, "abc").is_empty()); // empty matches are skipped
        let search = Search::new(r"(\w+)@(\w+)", regex).unwrap();
        assert_eq!(
            search.replace_all("a@b, c@d", None, "$2@$1"),
            ("b@a, d@c".to_string(), 2)
        );
        assert!(Search::new("(unclosed", regex).is_err());
    }

    #[test]
    fn literal_replacement_is_not_expanded() {
        let search = Search::new("cost", PLAIN).unwrap();
        assert_eq!(
            search.replace_all("cost cost", None, "$5"),
            ("$5 $5".to_string(), 2)
        );
    }

    #[test]
    fn within_a_range_and_limits() {
        let search = Search::new("a", PLAIN).unwrap();
        let text = "a a a a";
        assert_eq!(
            search.find_all(text, Some(2..5), MAX_MATCHES),
            vec![2..3, 4..5]
        );
        assert_eq!(search.find_all(text, None, 3).len(), 3);
        assert_eq!(
            search.replace_all(text, Some(2..5), "b"),
            ("a b b a".to_string(), 2)
        );
    }
}
