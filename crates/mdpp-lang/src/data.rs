//! Completion vocabularies shipped as data files (PLAN principle 6): adding a language, an alert
//! type or an image format is an edit to `data/*.txt`, not to code.

use std::sync::LazyLock;

fn lines(text: &'static str) -> Vec<&'static str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

/// Info-string names offered after a code fence.
pub static FENCE_LANGUAGES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| lines(include_str!("../data/fence-languages.txt")));

/// GitHub alert types offered after `> [!`.
pub static ALERTS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| lines(include_str!("../data/alerts.txt")));

/// File extensions offered inside image links (`![](…)`), lower case.
pub static IMAGE_EXTENSIONS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| lines(include_str!("../data/image-extensions.txt")));
