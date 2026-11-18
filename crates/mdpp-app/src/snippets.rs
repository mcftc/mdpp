//! Snippets (PLAN §4.3): pieces of Markdown from `assets/snippets.toml`, inserted at the caret
//! from Edit → Insert Snippet or the command palette. `$0` marks where the caret goes.

use std::sync::LazyLock;

use serde::Deserialize;

const SNIPPETS: &str = include_str!("../../../assets/snippets.toml");
/// Where the caret goes after inserting.
const CARET: &str = "$0";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnippetFile {
    snippet: Vec<Snippet>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snippet {
    pub name: String,
    body: String,
}

impl Snippet {
    /// The text to insert and the caret's byte offset in it.
    pub fn expand(&self) -> (String, usize) {
        match self.body.find(CARET) {
            Some(at) => (self.body.replacen(CARET, "", 1), at),
            None => (self.body.clone(), self.body.len()),
        }
    }
}

static ALL: LazyLock<Vec<Snippet>> = LazyLock::new(|| {
    toml::from_str::<SnippetFile>(SNIPPETS)
        .expect("assets/snippets.toml is valid (tested)")
        .snippet
});

pub fn all() -> &'static [Snippet] {
    &ALL
}

pub fn find(name: &str) -> Option<&'static Snippet> {
    ALL.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_parse_with_one_caret_at_most() {
        assert!(all().len() >= 5);
        for snippet in all() {
            assert!(snippet.body.matches(CARET).count() <= 1, "{}", snippet.name);
            let (text, caret) = snippet.expand();
            assert!(caret <= text.len() && !text.contains(CARET));
        }
        let (text, caret) = find("Table").unwrap().expand();
        assert_eq!(&text[..caret], "| ");
    }
}
