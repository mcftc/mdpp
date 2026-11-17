//! Document structure from one full `pulldown-cmark` parse (PLAN D4): headings with their anchors,
//! links, reference definitions, footnote definitions and references that point nowhere.
//!
//! Built on a snapshot in the background (≈ 8 ms per MB); every range is in bytes of the text it
//! was built from.

use std::ops::Range;

use pulldown_cmark::{BrokenLink, Event, HeadingLevel, LinkType, Parser, Tag, TagEnd};

use crate::slug::{SlugCounter, github_slug};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    /// 1–6.
    pub level: u8,
    /// Plain text of the heading (markup removed).
    pub text: String,
    /// Unique anchor: the explicit `{#id}` if given, else the GitHub slug, de-duplicated.
    pub anchor: String,
    pub range: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// `[text](dest)`
    Inline,
    /// `[text][id]`, `[text][]`, `[id]`
    Reference,
    /// `<https://…>`, `<a@b.c>`
    Autolink,
    /// `![alt](src)`
    Image,
    /// `[[target]]`
    Wiki,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// Destination as written (references are resolved to their definition's destination).
    pub dest: String,
    /// The definition label a reference link or image uses (`[text][label]`), as written.
    pub label: Option<String>,
    /// Range of the whole link in the source.
    pub range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceDefinition {
    /// Label as written in the definition.
    pub label: String,
    pub dest: String,
    pub range: Range<usize>,
}

/// `[^label]: …` or `[^label]`, and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Footnote {
    pub label: String,
    pub range: Range<usize>,
}

/// An explicit reference (`[text][id]` or `[id][]`) whose label has no definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokenReference {
    pub label: String,
    pub range: Range<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocIndex {
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    pub references: Vec<ReferenceDefinition>,
    pub broken_references: Vec<BrokenReference>,
    /// Footnote definitions (`[^label]: …`).
    pub footnotes: Vec<Footnote>,
    /// Footnote references (`[^label]` in text).
    pub footnote_references: Vec<Footnote>,
}

impl DocIndex {
    pub fn build(text: &str) -> Self {
        let mut broken_references = Vec::new();
        let mut on_broken = |link: BrokenLink<'_>| {
            // Shortcut references (`[word]`) are ordinary text far more often than intended links;
            // only explicit references are reported, as VS Code's Markdown language server does.
            if matches!(link.link_type, LinkType::Reference | LinkType::Collapsed) {
                broken_references.push(BrokenReference {
                    label: link.reference.to_string(),
                    range: link.span.clone(),
                });
            }
            None
        };
        let parser = Parser::new_with_broken_link_callback(
            text,
            crate::parser_options(),
            Some(&mut on_broken),
        );
        let mut events = parser.into_offset_iter();

        let mut index = DocIndex::default();
        let mut slugs = SlugCounter::default();
        let mut open_heading: Option<(u8, Option<String>, Range<usize>, String)> = None;

        for (event, range) in events.by_ref() {
            match event {
                Event::Start(Tag::Heading { level, id, .. }) => {
                    open_heading = Some((
                        level_number(level),
                        id.map(|i| i.to_string()),
                        range,
                        String::new(),
                    ));
                }
                Event::Text(text) | Event::Code(text) | Event::InlineMath(text) => {
                    if let Some((_, _, _, heading_text)) = open_heading.as_mut() {
                        heading_text.push_str(&text);
                    }
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some((level, id, range, text)) = open_heading.take() {
                        let anchor = slugs.unique(id.unwrap_or_else(|| github_slug(&text)));
                        index.headings.push(Heading {
                            level,
                            text,
                            anchor,
                            range,
                        });
                    }
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    id,
                    ..
                }) => {
                    let kind = match link_type {
                        LinkType::Inline => LinkKind::Inline,
                        LinkType::Autolink | LinkType::Email => LinkKind::Autolink,
                        LinkType::WikiLink { .. } => LinkKind::Wiki,
                        _ => LinkKind::Reference,
                    };
                    index.links.push(Link {
                        kind,
                        dest: dest_url.to_string(),
                        label: (!id.is_empty()).then(|| id.to_string()),
                        range,
                    });
                }
                Event::Start(Tag::Image { dest_url, id, .. }) => {
                    index.links.push(Link {
                        kind: LinkKind::Image,
                        dest: dest_url.to_string(),
                        label: (!id.is_empty()).then(|| id.to_string()),
                        range,
                    });
                }
                Event::Start(Tag::FootnoteDefinition(label)) => index.footnotes.push(Footnote {
                    label: label.to_string(),
                    range,
                }),
                Event::FootnoteReference(label) => index.footnote_references.push(Footnote {
                    label: label.to_string(),
                    range,
                }),
                _ => {}
            }
        }

        index.references = events
            .reference_definitions()
            .iter()
            .map(|(label, def)| ReferenceDefinition {
                label: label.to_string(),
                dest: def.dest.to_string(),
                range: def.span.clone(),
            })
            .collect();
        index.references.sort_by_key(|r| r.range.start);
        drop(events);
        index.broken_references = broken_references;
        index
    }

    pub fn heading_with_anchor(&self, anchor: &str) -> Option<&Heading> {
        self.headings.iter().find(|h| h.anchor == anchor)
    }
}

/// A link label as CommonMark matches it: case-insensitive, runs of whitespace as one space.
pub fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\
# Intro
Text with [inline](./a.md#top), [ref][r1], [missing][nope], [shortcut] and <https://x.y>.

## Intro
![logo](img/logo.png) and [[Wiki Page]].

### Custom {#my-id}
Footnote[^n].

[r1]: https://example.com
[^n]: The note.
";

    #[test]
    fn headings_and_anchors() {
        let index = DocIndex::build(DOC);
        let anchors: Vec<_> = index
            .headings
            .iter()
            .map(|h| (h.level, h.anchor.as_str()))
            .collect();
        assert_eq!(anchors, vec![(1, "intro"), (2, "intro-1"), (3, "my-id")]);
        assert_eq!(index.headings[2].text, "Custom");
        assert_eq!(&DOC[index.headings[0].range.clone()], "# Intro\n");
        assert!(index.heading_with_anchor("intro-1").is_some());
    }

    #[test]
    fn links_and_references() {
        let index = DocIndex::build(DOC);
        let links: Vec<_> = index
            .links
            .iter()
            .map(|l| (l.kind, l.dest.as_str()))
            .collect();
        assert_eq!(
            links,
            vec![
                (LinkKind::Inline, "./a.md#top"),
                (LinkKind::Reference, "https://example.com"),
                (LinkKind::Autolink, "https://x.y"),
                (LinkKind::Image, "img/logo.png"),
                (LinkKind::Wiki, "Wiki Page"),
            ]
        );
        assert_eq!(index.links[1].label.as_deref(), Some("r1"));
        assert_eq!(index.links[0].label, None);
        assert_eq!(index.references.len(), 1);
        assert_eq!(index.references[0].dest, "https://example.com");
        // Only the explicit reference is broken; `[shortcut]` is plain text.
        assert_eq!(index.broken_references.len(), 1);
        assert_eq!(index.broken_references[0].label, "nope");
        assert_eq!(
            &DOC[index.broken_references[0].range.clone()],
            "[missing][nope]"
        );
        assert_eq!(
            index
                .footnotes
                .iter()
                .map(|f| f.label.as_str())
                .collect::<Vec<_>>(),
            vec!["n"]
        );
        assert_eq!(&DOC[index.footnote_references[0].range.clone()], "[^n]");
    }
}
