//! Go to definition and hover (PLAN §4.3): what the link or footnote under the cursor points to.

use std::ops::Range;
use std::path::{Path, PathBuf};

use mdpp_markdown::{DocIndex, LinkKind};

use crate::paths::{PathSource, is_relative_path, percent_decode};

/// Hover text is cut after this many characters (a footnote can be long).
const HOVER_MAX_CHARS: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A place in this document (a heading or a footnote definition).
    InDocument(Range<usize>),
    /// Another file, optionally at a heading anchor.
    File {
        path: PathBuf,
        anchor: Option<String>,
    },
    /// A URL (`https:`, `mailto:` …).
    External(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The link or reference under the cursor.
    pub origin: Range<usize>,
    pub target: Target,
}

/// Where the link or footnote reference at `offset` leads. Unresolvable targets (a missing
/// heading, a relative link in an unsaved document) give `None`.
pub fn target_at(index: &DocIndex, offset: usize, doc_dir: Option<&Path>) -> Option<Found> {
    if let Some(reference) = index
        .footnote_references
        .iter()
        .find(|f| f.range.contains(&offset))
    {
        let definition = index
            .footnotes
            .iter()
            .find(|f| f.label == reference.label)?;
        return Some(Found {
            origin: reference.range.clone(),
            target: Target::InDocument(definition.range.clone()),
        });
    }
    // Innermost link (an image inside a link is more specific).
    let link = index
        .links
        .iter()
        .filter(|l| l.range.contains(&offset))
        .min_by_key(|l| l.range.len())?;
    let dest = link.dest.trim();
    let target = if let Some(anchor) = dest.strip_prefix('#') {
        let heading = index.heading_with_anchor(&percent_decode(anchor))?;
        Target::InDocument(heading.range.start..heading.range.start)
    } else if link.kind == LinkKind::Wiki {
        let name = if Path::new(dest).extension().is_some() {
            dest.to_string()
        } else {
            format!("{dest}.md")
        };
        Target::File {
            path: doc_dir?.join(name),
            anchor: None,
        }
    } else if is_relative_path(dest) {
        let (path, anchor) = match dest.split_once('#') {
            Some((path, anchor)) => (path, Some(percent_decode(anchor))),
            None => (dest, None),
        };
        let path = path.split('?').next().unwrap_or_default();
        Target::File {
            path: doc_dir?.join(percent_decode(path)),
            anchor,
        }
    } else if dest.contains(':') {
        Target::External(dest.to_string())
    } else {
        return None; // absolute paths: no reliable root for them
    };
    Some(Found {
        origin: link.range.clone(),
        target,
    })
}

/// Text describing what is under the cursor, and the range it describes.
pub fn hover_at(
    index: &DocIndex,
    source: &str,
    offset: usize,
    doc_dir: Option<&Path>,
    files: &dyn PathSource,
) -> Option<(Range<usize>, String)> {
    let found = target_at(index, offset, doc_dir);
    let Some(found) = found else {
        // Explain a link that does not resolve instead of staying silent.
        let link = index.links.iter().find(|l| l.range.contains(&offset))?;
        let anchor = link.dest.strip_prefix('#')?;
        return Some((
            link.range.clone(),
            format!("No heading `#{anchor}` in this document."),
        ));
    };
    let text = match &found.target {
        Target::InDocument(range) => {
            match index.headings.iter().find(|h| h.range.start == range.start) {
                Some(heading) => format!(
                    "{} {}",
                    "#".repeat(usize::from(heading.level)),
                    heading.text
                ),
                None => truncate(source.get(range.clone()).unwrap_or_default().trim()),
            }
        }
        Target::File { path, anchor } => {
            let missing = if files.exists(path) {
                ""
            } else {
                " — not found"
            };
            let anchor = anchor
                .as_deref()
                .map(|a| format!("#{a}"))
                .unwrap_or_default();
            format!("{}{anchor}{missing}", path.display())
        }
        Target::External(url) => url.clone(),
    };
    Some((found.origin, text))
}

fn truncate(text: &str) -> String {
    match text.char_indices().nth(HOVER_MAX_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::fake::FakeFs;

    const DOC: &str = "\
# Setup
See [setup](#setup), [guide](docs/guide.md#install), [web](https://e.com), [[Notes]],
[bad](#nope) and a note[^1].

[^1]: The footnote text.
";

    fn at(needle: &str) -> usize {
        DOC.find(needle).unwrap() + 1
    }

    #[test]
    fn targets() {
        let index = DocIndex::build(DOC);
        let dir = Some(Path::new("/d"));
        assert_eq!(
            target_at(&index, at("[setup]"), dir).unwrap().target,
            Target::InDocument(0..0)
        );
        assert_eq!(
            target_at(&index, at("[guide]"), dir).unwrap().target,
            Target::File {
                path: "/d/docs/guide.md".into(),
                anchor: Some("install".into())
            }
        );
        assert_eq!(
            target_at(&index, at("[web]"), dir).unwrap().target,
            Target::External("https://e.com".into())
        );
        assert_eq!(
            target_at(&index, at("[[Notes]]"), dir).unwrap().target,
            Target::File {
                path: "/d/Notes.md".into(),
                anchor: None
            }
        );
        let footnote = target_at(&index, at("[^1]"), dir).unwrap();
        let Target::InDocument(range) = footnote.target else {
            panic!()
        };
        assert!(DOC[range].starts_with("[^1]: The footnote"));
        assert_eq!(target_at(&index, at("[bad]"), dir), None);
        assert_eq!(target_at(&index, at("[guide]"), None), None); // unsaved: no folder
        assert_eq!(target_at(&index, 2, dir), None); // plain text
    }

    #[test]
    fn hovers() {
        let index = DocIndex::build(DOC);
        let fs = FakeFs::with(&["/d/docs/guide.md"]);
        let hover = |needle: &str| {
            hover_at(&index, DOC, at(needle), Some(Path::new("/d")), &fs).map(|(_, t)| t)
        };
        assert_eq!(hover("[setup]").as_deref(), Some("# Setup"));
        assert_eq!(
            hover("[guide]").as_deref(),
            Some("/d/docs/guide.md#install")
        );
        assert_eq!(
            hover("[[Notes]]").as_deref(),
            Some("/d/Notes.md — not found")
        );
        assert_eq!(hover("[^1]").as_deref(), Some("[^1]: The footnote text."));
        assert_eq!(
            hover("[bad]").as_deref(),
            Some("No heading `#nope` in this document.")
        );
    }
}
