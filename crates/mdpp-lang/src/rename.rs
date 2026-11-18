//! Rename a heading (PLAN §4.3, F2): the heading's text changes, and so does every link in this
//! document to a heading whose anchor changes — including the duplicates numbered after it
//! (`#a-1` becoming `#a` when the first `a` is renamed). One edit, one undo step.

use std::ops::Range;

use mdpp_markdown::{DocIndex, LinkKind};

use crate::assist::Edit;
use crate::paths::percent_decode;

/// The heading at byte `offset`: the heading on that line, or the one a `#anchor` link there
/// points to. Its index in [`DocIndex::headings`] and its text as written.
pub fn heading_at(text: &str, index: &DocIndex, offset: usize) -> Option<(usize, String)> {
    let on_line = index
        .headings
        .iter()
        .position(|h| h.range.start <= offset && offset <= h.range.end);
    let by_link = || {
        let link = index
            .links
            .iter()
            .find(|l| l.range.start <= offset && offset <= l.range.end)?;
        let anchor = percent_decode(link.dest.strip_prefix('#')?);
        index.headings.iter().position(|h| h.anchor == anchor)
    };
    let which = on_line.or_else(by_link)?;
    let content = content_range(text, &index.headings[which].range)?;
    Some((which, text[content].to_string()))
}

/// Rename heading `which` to `new_text`. `None` when the name is empty, would not stay one
/// heading, or nothing changes.
pub fn rename_heading(text: &str, which: usize, new_text: &str) -> Option<Edit> {
    let new_text = new_text.trim();
    if new_text.is_empty() || new_text.contains('\n') {
        return None;
    }
    let old = DocIndex::build(text);
    let content = content_range(text, &old.headings.get(which)?.range)?;
    let mut renamed = text.to_string();
    renamed.replace_range(content.clone(), new_text);
    let new = DocIndex::build(&renamed);
    if new.headings.len() != old.headings.len() {
        return None; // e.g. the new text ended the heading early
    }
    // Old anchor → new anchor, for every heading whose anchor moved.
    let moved: Vec<(&str, &str)> = old
        .headings
        .iter()
        .zip(&new.headings)
        .filter(|(o, n)| o.anchor != n.anchor)
        .map(|(o, n)| (o.anchor.as_str(), n.anchor.as_str()))
        .collect();
    // Destinations as written: inline links and images, and reference definitions.
    let destinations = new
        .links
        .iter()
        .filter(|l| matches!(l.kind, LinkKind::Inline | LinkKind::Image))
        .map(|l| (&l.dest, &l.range))
        .chain(new.references.iter().map(|r| (&r.dest, &r.range)));
    let mut changes: Vec<(Range<usize>, String)> = Vec::new();
    for (dest, range) in destinations {
        let Some(anchor) = dest.strip_prefix('#') else {
            continue;
        };
        let decoded = percent_decode(anchor);
        let Some((_, to)) = moved.iter().find(|(from, _)| *from == decoded) else {
            continue;
        };
        let source = &renamed[range.clone()];
        if let Some(at) = source.rfind(dest.as_str()) {
            let start = range.start + at + 1; // after `#`
            changes.push((start..start + anchor.len(), to.to_string()));
        }
    }
    changes.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, to) in changes {
        renamed.replace_range(range, &to);
    }
    if renamed == text {
        return None;
    }
    // One edit over the part that changed.
    let prefix = common_prefix(text, &renamed);
    let suffix = common_suffix(&text[prefix..], &renamed[prefix..]);
    let caret = DocIndex::build(&renamed).headings[which].range.start;
    Some(Edit {
        range: prefix..text.len() - suffix,
        text: renamed[prefix..renamed.len() - suffix].to_string(),
        select: caret..caret,
    })
}

/// The text of a heading as written: after `#`s (ATX) or the first line (Setext), without
/// closing `#`s or an explicit `{#id}`.
fn content_range(text: &str, heading: &Range<usize>) -> Option<Range<usize>> {
    let line_end = text[heading.start..]
        .find('\n')
        .map_or(text.len(), |i| heading.start + i);
    let line = &text[heading.start..line_end];
    let indent = line.len() - line.trim_start().len();
    let after_indent = &line[indent..];
    let hashes = after_indent.chars().take_while(|&c| c == '#').count();
    let mut start = indent + hashes;
    start += line[start..].len() - line[start..].trim_start().len();
    let mut end = line.trim_end().len();
    if let Some(open) = line[..end].rfind("{#")
        && line[..end].ends_with('}')
        && open >= start
    {
        end = line[..open].trim_end().len();
    }
    if hashes > 0 {
        let closing = line[..end].trim_end_matches('#');
        if closing.len() < end && (closing.ends_with([' ', '\t']) || closing.len() == start) {
            end = closing.trim_end().len();
        }
    }
    (start < end).then(|| heading.start + start..heading.start + end)
}

fn common_prefix(a: &str, b: &str) -> usize {
    let mut n = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    while !a.is_char_boundary(n) {
        n -= 1;
    }
    n
}

fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = a
        .bytes()
        .rev()
        .zip(b.bytes().rev())
        .take_while(|(x, y)| x == y)
        .count();
    while !a.is_char_boundary(a.len() - n) || !b.is_char_boundary(b.len() - n) {
        n -= 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, edit: &Edit) -> String {
        let mut out = text.to_string();
        out.replace_range(edit.range.clone(), &edit.text);
        out
    }

    #[test]
    fn renames_the_heading_and_its_links() {
        let doc =
            "# Install guide ##\n\nSee [it](#install-guide) and [ref].\n\n[ref]: #install-guide\n";
        let index = DocIndex::build(doc);
        let (which, text) = heading_at(doc, &index, 3).unwrap();
        assert_eq!((which, text.as_str()), (0, "Install guide"));
        let edit = rename_heading(doc, which, "Setup").unwrap();
        assert_eq!(
            apply(doc, &edit),
            "# Setup ##\n\nSee [it](#setup) and [ref].\n\n[ref]: #setup\n"
        );
        // From a link to the heading.
        let at_link = doc.find("[it]").unwrap() + 2;
        assert_eq!(heading_at(doc, &index, at_link).unwrap().0, 0);
    }

    #[test]
    fn duplicate_numbers_shift_with_the_rename() {
        let doc = "## Notes\n## Notes\n\n[a](#notes) [b](#notes-1) [c](other.md#notes)\n";
        let edit = rename_heading(doc, 0, "Intro").unwrap();
        assert_eq!(
            apply(doc, &edit),
            "## Intro\n## Notes\n\n[a](#intro) [b](#notes) [c](other.md#notes)\n"
        );
    }

    #[test]
    fn explicit_ids_setext_and_refusals() {
        let doc = "### Custom {#my-id}\n\nTitle\n=====\n[x](#title)\n";
        assert_eq!(
            heading_at(doc, &DocIndex::build(doc), 0).unwrap().1,
            "Custom"
        );
        let edit = rename_heading(doc, 0, "Other").unwrap();
        assert_eq!(
            apply(doc, &edit),
            "### Other {#my-id}\n\nTitle\n=====\n[x](#title)\n"
        );
        let edit = rename_heading(doc, 1, "Name").unwrap();
        assert_eq!(
            apply(doc, &edit),
            "### Custom {#my-id}\n\nName\n=====\n[x](#name)\n"
        );
        assert_eq!(rename_heading(doc, 1, "  "), None);
        assert_eq!(rename_heading(doc, 1, "Title"), None, "unchanged");
        assert_eq!(rename_heading(doc, 9, "x"), None);
    }
}
