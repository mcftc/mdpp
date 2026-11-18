//! Diagnostics (PLAN §4.3, MVP set for this document): links to headings or files that do not
//! exist, references without a definition, definitions nobody uses, footnotes defined twice.
//!
//! Each finding carries its rule id; how severe a rule is (or whether it is off) is a setting
//! (`[diagnostics]` in `assets/default-settings.toml`, the one place the defaults live).

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use mdpp_markdown::{DocIndex, LinkKind, normalize_label};

use crate::paths::{OtherHeading, PathSource, headings_of, is_relative_path, percent_decode};

/// Rule ids: the stable names users configure.
pub const MISSING_ANCHOR: &str = "missing-anchor";
pub const MISSING_FILE: &str = "missing-file";
pub const UNDEFINED_REFERENCE: &str = "undefined-reference";
pub const UNUSED_DEFINITION: &str = "unused-definition";
pub const DUPLICATE_FOOTNOTE: &str = "duplicate-footnote";

/// Every rule [`diagnose`] can report.
pub const RULES: &[&str] = &[
    MISSING_ANCHOR,
    MISSING_FILE,
    UNDEFINED_REFERENCE,
    UNUSED_DEFINITION,
    DUPLICATE_FOOTNOTE,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Bytes of the document the diagnostic covers.
    pub range: Range<usize>,
    /// One of [`RULES`].
    pub code: &'static str,
    pub message: String,
}

/// Diagnose one document. `doc_dir` is its folder (`None` while unsaved: relative paths are then
/// not checked).
pub fn diagnose(
    index: &DocIndex,
    doc_dir: Option<&Path>,
    files: &dyn PathSource,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // Each linked Markdown file is read once per run, however many links point into it.
    let mut other_headings: std::collections::HashMap<
        std::path::PathBuf,
        Option<Vec<OtherHeading>>,
    > = std::collections::HashMap::new();
    let mut report = |code: &'static str, range: Range<usize>, message: String| {
        out.push(Diagnostic {
            range,
            code,
            message,
        });
    };

    for link in &index.links {
        if !matches!(
            link.kind,
            LinkKind::Inline | LinkKind::Reference | LinkKind::Image
        ) {
            continue;
        }
        let dest = link.dest.trim();
        if let Some(anchor) = dest.strip_prefix('#') {
            let anchor = percent_decode(anchor);
            if !anchor.is_empty() && index.heading_with_anchor(&anchor).is_none() {
                report(
                    MISSING_ANCHOR,
                    link.range.clone(),
                    format!("No heading with the anchor `#{anchor}` in this document."),
                );
            }
        } else if let Some(dir) = doc_dir
            && is_relative_path(dest)
        {
            let path = percent_decode(dest.split(['#', '?']).next().unwrap_or_default());
            if path.is_empty() {
                continue;
            }
            let full = dir.join(&path);
            if !files.exists(&full) {
                report(
                    MISSING_FILE,
                    link.range.clone(),
                    format!("File `{path}` not found."),
                );
            } else if let Some((_, anchor)) = dest.split_once('#')
                && !anchor.is_empty()
            {
                let anchor = percent_decode(anchor);
                let headings = other_headings
                    .entry(full.clone())
                    .or_insert_with(|| headings_of(files, &full));
                if let Some(headings) = headings
                    && !headings.iter().any(|h| h.anchor == anchor)
                {
                    report(
                        MISSING_ANCHOR,
                        link.range.clone(),
                        format!("No heading with the anchor `#{anchor}` in `{path}`."),
                    );
                }
            }
        }
    }

    for broken in &index.broken_references {
        report(
            UNDEFINED_REFERENCE,
            broken.range.clone(),
            format!("Reference `[{}]` is not defined.", broken.label),
        );
    }

    let used: HashSet<String> = index
        .links
        .iter()
        .filter_map(|link| link.label.as_deref().map(normalize_label))
        .collect();
    for definition in &index.references {
        if !used.contains(&normalize_label(&definition.label)) {
            report(
                UNUSED_DEFINITION,
                definition.range.clone(),
                format!("Definition `[{}]` is never used.", definition.label),
            );
        }
    }

    let mut footnotes = HashSet::new();
    for footnote in &index.footnotes {
        if !footnotes.insert(normalize_label(&footnote.label)) {
            report(
                DUPLICATE_FOOTNOTE,
                footnote.range.clone(),
                format!(
                    "Footnote `[^{}]` is already defined above; this one is ignored.",
                    footnote.label
                ),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::fake::FakeFs;

    #[test]
    fn reports_missing_targets() {
        let doc = "\
# Intro
[ok](#intro) [bad](#nope) [file](notes.md#x) [gone](missing%20file.md)
[web](https://e.com) [mail](mailto:a@b.c) [abs](/root.md) ![img](img/a.png)
[ref][r] [broken][zz] [Short] and a note[^n]

[r]: #intro
[short]: #intro
[spare]: https://e.com
[^n]: One.
[^n]: Two.
";
        let index = DocIndex::build(doc);
        let fs = FakeFs::with(&["/d/notes.md", "/d/img/a.png"]);
        let found: Vec<_> = diagnose(&index, Some(Path::new("/d")), &fs)
            .into_iter()
            .map(|d| (d.code, doc[d.range].to_string()))
            .collect();
        assert_eq!(
            found,
            vec![
                ("missing-anchor", "[bad](#nope)".to_string()),
                ("missing-file", "[gone](missing%20file.md)".to_string()),
                ("undefined-reference", "[broken][zz]".to_string()),
                ("unused-definition", "[spare]: https://e.com".to_string()),
                ("duplicate-footnote", "[^n]: Two.\n".to_string()),
            ]
        );
    }

    #[test]
    fn anchors_in_other_files() {
        let doc = "[a](guide.md#setup) [b](guide.md#nope) [c](guide.md#nope) [d](img.png#x)";
        let index = DocIndex::build(doc);
        let fs = FakeFs::with(&["/d/guide.md", "/d/img.png"]).text("/d/guide.md", "# Setup\n");
        let found: Vec<_> = diagnose(&index, Some(Path::new("/d")), &fs)
            .into_iter()
            .map(|d| (d.code, doc[d.range].to_string(), d.message))
            .collect();
        let message = "No heading with the anchor `#nope` in `guide.md`.".to_string();
        assert_eq!(
            found,
            vec![
                (
                    "missing-anchor",
                    "[b](guide.md#nope)".to_string(),
                    message.clone()
                ),
                ("missing-anchor", "[c](guide.md#nope)".to_string(), message),
            ]
        );
    }

    #[test]
    fn unsaved_documents_skip_file_checks() {
        let index = DocIndex::build("[x](nowhere.md)");
        assert!(diagnose(&index, None, &FakeFs::default()).is_empty());
    }
}
