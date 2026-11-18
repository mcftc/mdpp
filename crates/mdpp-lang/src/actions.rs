//! Code actions (PLAN §4.3, Ctrl+.): create a missing linked file, insert or update a table of
//! contents, turn an inline link into a reference link. Each one is computed from the text, its
//! index and the caret; applying it is one [`Edit`] (or creating the file).

use std::path::{Path, PathBuf};

use mdpp_markdown::slug::github_slug;
use mdpp_markdown::{DocIndex, LinkKind, normalize_label};

use crate::assist::Edit;
use crate::paths::{PathSource, is_markdown, is_relative_path, percent_decode};

/// Marks around a generated table of contents, so it can be found and updated.
pub const TOC_START: &str = "<!-- toc -->";
pub const TOC_END: &str = "<!-- /toc -->";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Create this Markdown file (with a first heading) and open it.
    CreateFile {
        path: PathBuf,
        text: String,
    },
    Edit(Edit),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeAction {
    pub title: String,
    pub kind: ActionKind,
}

/// The actions available with the caret at byte `offset`.
pub fn code_actions(
    text: &str,
    index: &DocIndex,
    offset: usize,
    doc_dir: Option<&Path>,
    files: &dyn PathSource,
) -> Vec<CodeAction> {
    let mut out = Vec::new();
    let link = index
        .links
        .iter()
        .find(|l| l.range.start <= offset && offset <= l.range.end);
    if let Some(link) = link {
        out.extend(create_file(text, link, doc_dir, files));
        out.extend(to_reference(text, index, link));
    }
    out.extend(table_of_contents(text, index, offset));
    out
}

fn create_file(
    text: &str,
    link: &mdpp_markdown::Link,
    doc_dir: Option<&Path>,
    files: &dyn PathSource,
) -> Option<CodeAction> {
    let dest = link.dest.trim();
    if link.kind != LinkKind::Inline || !is_relative_path(dest) {
        return None;
    }
    let name = percent_decode(dest.split(['#', '?']).next()?);
    let path = doc_dir?.join(&name);
    if name.is_empty() || !is_markdown(&path) || files.exists(&path) {
        return None;
    }
    let title = link_text(&text[link.range.clone()]).unwrap_or(name.clone());
    Some(CodeAction {
        title: format!("Create {name}"),
        kind: ActionKind::CreateFile {
            path,
            text: format!("# {title}\n"),
        },
    })
}

/// `[text](dest)` → `[text][label]`, with `[label]: dest` at the end of the document.
fn to_reference(text: &str, index: &DocIndex, link: &mdpp_markdown::Link) -> Option<CodeAction> {
    if link.kind != LinkKind::Inline {
        return None;
    }
    let source = &text[link.range.clone()];
    let shown = link_text(source)?;
    let destination = &source[source.rfind("](")? + 2..source.len() - 1];
    let base = match github_slug(&shown) {
        slug if slug.is_empty() => "link".to_string(),
        slug => slug,
    };
    let taken = |label: &str| {
        index
            .references
            .iter()
            .any(|r| normalize_label(&r.label) == normalize_label(label))
    };
    let label = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}-{n}")))
        .find(|label| !taken(label))?;
    let mut replaced = format!("[{shown}][{label}]");
    replaced.push_str(&text[link.range.end..]);
    if !replaced.ends_with('\n') {
        replaced.push('\n');
    }
    if !replaced.ends_with("\n\n") && index.references.is_empty() {
        replaced.push('\n');
    }
    replaced.push_str(&format!("[{label}]: {destination}\n"));
    let caret = link.range.start + shown.len() + label.len() + 4;
    Some(CodeAction {
        title: "Convert to reference link".into(),
        kind: ActionKind::Edit(Edit {
            range: link.range.start..text.len(),
            text: replaced,
            select: caret..caret,
        }),
    })
}

/// The text between the first `[` and its `](` of a link's source.
fn link_text(source: &str) -> Option<String> {
    let close = source.rfind("](")?;
    Some(source.get(1..close)?.to_string())
}

/// Insert a table of contents at the caret, or update the one between the marks (`None`
/// without headings).
fn table_of_contents(text: &str, index: &DocIndex, offset: usize) -> Option<CodeAction> {
    let existing = text.find(TOC_START).and_then(|start| {
        let end = text[start..].find(TOC_END)? + start + TOC_END.len();
        Some(start..end)
    });
    let headings: Vec<_> = index
        .headings
        .iter()
        .filter(|h| {
            existing
                .as_ref()
                .is_none_or(|toc| !toc.contains(&h.range.start))
        })
        .collect();
    let top = headings.iter().map(|h| h.level).min()?;
    let mut toc = format!("{TOC_START}\n");
    for heading in &headings {
        let indent = "  ".repeat(usize::from(heading.level - top));
        toc.push_str(&format!(
            "{indent}- [{}](#{})\n",
            heading.text, heading.anchor
        ));
    }
    toc.push_str(TOC_END);
    Some(match existing {
        Some(range) => CodeAction {
            title: "Update table of contents".into(),
            kind: ActionKind::Edit(Edit {
                select: range.start..range.start,
                range,
                text: toc,
            }),
        },
        None => {
            // On a line of its own, where the caret's line starts.
            let start = text[..offset.min(text.len())]
                .rfind('\n')
                .map_or(0, |i| i + 1);
            toc.push_str("\n\n");
            CodeAction {
                title: "Insert table of contents".into(),
                kind: ActionKind::Edit(Edit {
                    range: start..start,
                    select: start + toc.len()..start + toc.len(),
                    text: toc,
                }),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::fake::FakeFs;

    fn apply(text: &str, action: &CodeAction) -> String {
        let ActionKind::Edit(edit) = &action.kind else {
            panic!("an edit")
        };
        let mut out = text.to_string();
        out.replace_range(edit.range.clone(), &edit.text);
        out
    }

    fn actions(text: &str, at: &str) -> Vec<CodeAction> {
        let fs = FakeFs::with(&["/d/here.md"]);
        let offset = text.find(at).unwrap() + 1;
        code_actions(
            text,
            &DocIndex::build(text),
            offset,
            Some(Path::new("/d")),
            &fs,
        )
    }

    #[test]
    fn create_a_missing_linked_file() {
        let text = "See [Next steps](plans/next%20steps.md#a) and [here](here.md).\n";
        let found = actions(text, "[Next");
        assert_eq!(found[0].title, "Create plans/next steps.md");
        assert_eq!(
            found[0].kind,
            ActionKind::CreateFile {
                path: PathBuf::from("/d/plans/next steps.md"),
                text: "# Next steps\n".into()
            }
        );
        assert!(
            actions(text, "[here")
                .iter()
                .all(|a| !a.title.starts_with("Create"))
        );
    }

    #[test]
    fn inline_link_to_reference() {
        let text = "Read [the Guide](https://e.com/g \"t\") now.\n\n[other]: x\n";
        let found = actions(text, "[the");
        let convert = found
            .iter()
            .find(|a| a.title.starts_with("Convert"))
            .unwrap();
        assert_eq!(
            apply(text, convert),
            "Read [the Guide][the-guide] now.\n\n[other]: x\n[the-guide]: https://e.com/g \"t\"\n"
        );
        let text = "[a](x) and [a](y)\n\n[a]: z\n";
        let found = actions(text, "[a](x");
        let convert = found
            .iter()
            .find(|a| a.title.starts_with("Convert"))
            .unwrap();
        assert!(apply(text, convert).ends_with("[a]: z\n[a-2]: x\n"));
    }

    #[test]
    fn table_of_contents_inserted_then_updated() {
        let text = "Intro\n\n## One\n### One a\n## Two\n";
        let toc = actions(text, "Intro").pop().unwrap();
        assert_eq!(toc.title, "Insert table of contents");
        let with_toc = apply(text, &toc);
        assert_eq!(
            with_toc,
            "<!-- toc -->\n- [One](#one)\n  - [One a](#one-a)\n- [Two](#two)\n<!-- /toc -->\n\nIntro\n\n## One\n### One a\n## Two\n"
        );
        let renamed = with_toc.replace("## Two", "## Three");
        let update = actions(&renamed, "Intro").pop().unwrap();
        assert_eq!(update.title, "Update table of contents");
        assert!(apply(&renamed, &update).contains("- [Three](#three)\n<!-- /toc -->"));
        assert!(actions("no headings", "no").is_empty());
    }
}
