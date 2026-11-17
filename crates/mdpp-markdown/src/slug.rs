//! GitHub-compatible heading anchors.
//!
//! GitHub lower-cases the heading text, drops every character that is not a letter, mark, number,
//! connector punctuation, space or hyphen, and turns spaces into hyphens. Repeated anchors get
//! `-1`, `-2`, … appended in document order. Matching it exactly matters: links written for GitHub
//! must resolve in MD++ and vice versa.

use std::collections::HashMap;

/// The anchor GitHub generates for `text`, before de-duplication.
pub fn github_slug(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() || is_combining_mark(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// Combining marks (Unicode category M) that GitHub keeps — e.g. the dot of `i̇` produced by
/// lower-casing the Turkish `İ`. These blocks cover the marks that occur in practice; `char` has
/// no general-category query in std.
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{0483}'..='\u{0489}'
            | '\u{0591}'..='\u{05BD}'
            | '\u{0610}'..='\u{061A}'
            | '\u{064B}'..='\u{065F}'
            | '\u{0900}'..='\u{0903}'
            | '\u{093A}'..='\u{094F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// Hands out unique anchors in document order: `intro`, `intro-1`, `intro-2`, …
#[derive(Debug, Default)]
pub struct SlugCounter {
    seen: HashMap<String, usize>,
}

impl SlugCounter {
    pub fn unique(&mut self, slug: String) -> String {
        match self.seen.get_mut(&slug) {
            Some(count) => {
                *count += 1;
                format!("{slug}-{count}")
            }
            None => {
                self.seen.insert(slug.clone(), 0);
                slug
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_github() {
        assert_eq!(github_slug("Hello, World!"), "hello-world");
        assert_eq!(github_slug("  Trim me  "), "trim-me");
        assert_eq!(github_slug("API v2.0 — notes"), "api-v20--notes");
        assert_eq!(github_slug("snake_case and-dash"), "snake_case-and-dash");
        assert_eq!(github_slug("Çalışma Şekli"), "çalışma-şekli");
        assert_eq!(github_slug("İstanbul"), "i\u{307}stanbul");
        assert_eq!(github_slug("`code` *emph*"), "code-emph");
    }

    #[test]
    fn duplicates_are_numbered() {
        let mut counter = SlugCounter::default();
        assert_eq!(counter.unique("a".into()), "a");
        assert_eq!(counter.unique("a".into()), "a-1");
        assert_eq!(counter.unique("b".into()), "b");
        assert_eq!(counter.unique("a".into()), "a-2");
    }
}
