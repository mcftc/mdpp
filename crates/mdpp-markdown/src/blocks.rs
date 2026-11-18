//! The document as blocks for the preview (PLAN D6): built from the same `pulldown-cmark` events
//! as the structure index, so the preview and the editor agree on what the text means. Every
//! top-level block keeps the byte range of its source — that is what scroll sync maps through.
//!
//! The model is UI-agnostic: styled text runs, images, lists, tables, quotes/alerts, code.

use std::ops::Range;

use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, MetadataBlockKind, Parser, Tag, TagEnd,
};

/// Inline formatting of a run of text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InlineStyle {
    pub emphasis: bool,
    pub strong: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: bool,
    pub math: bool,
    pub footnote: bool,
    pub html: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// Text with styled byte ranges (unstyled text has no span).
    Text {
        text: String,
        spans: Vec<(Range<usize>, InlineStyle)>,
    },
    Image {
        dest: String,
        alt: String,
    },
    /// Display math (`$$…$$`), as written between the dollars.
    Math {
        tex: String,
    },
}

/// The inline content of a paragraph, heading or table cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inlines {
    pub segments: Vec<Segment>,
}

impl Inlines {
    fn push_text(&mut self, text: &str, style: InlineStyle) {
        if text.is_empty() {
            return;
        }
        if !matches!(self.segments.last(), Some(Segment::Text { .. })) {
            self.segments.push(Segment::Text {
                text: String::new(),
                spans: Vec::new(),
            });
        }
        let Some(Segment::Text {
            text: buffer,
            spans,
        }) = self.segments.last_mut()
        else {
            return;
        };
        let start = buffer.len();
        buffer.push_str(text);
        if style != InlineStyle::default() {
            match spans.last_mut() {
                Some((range, last)) if *last == style && range.end == start => {
                    range.end = buffer.len()
                }
                _ => spans.push((start..buffer.len(), style)),
            }
        }
    }

    /// The text without formatting (images contribute their alt text).
    pub fn plain_text(&self) -> String {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Text { text, .. } => text.as_str(),
                Segment::Image { alt, .. } => alt.as_str(),
                Segment::Math { tex } => tex.as_str(),
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

/// GitHub alert types (`> [!NOTE]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Bytes of the source this block was parsed from.
    pub source: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Heading {
        level: u8,
        content: Inlines,
    },
    Paragraph(Inlines),
    /// Fenced, indented or front-matter code; `language` is the first word of the info string.
    Code {
        language: Option<String>,
        text: String,
    },
    Quote {
        alert: Option<AlertKind>,
        blocks: Vec<Block>,
    },
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Table {
        alignments: Vec<Alignment>,
        header: Vec<Inlines>,
        rows: Vec<Vec<Inlines>>,
    },
    Rule,
    Html(String),
    FootnoteDefinition {
        label: String,
        blocks: Vec<Block>,
    },
    /// Definition-list definitions.
    Indented(Vec<Block>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// `Some(checked)` for task-list items.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

/// Parse `text` into top-level blocks.
pub fn parse(text: &str) -> Vec<Block> {
    let mut builder = Builder::default();
    for (event, range) in Parser::new_ext(text, crate::parser_options()).into_offset_iter() {
        builder.event(event, range);
    }
    builder.finish()
}

enum Container {
    Quote {
        alert: Option<AlertKind>,
        source: Range<usize>,
        blocks: Vec<Block>,
    },
    List {
        start: Option<u64>,
        source: Range<usize>,
        items: Vec<ListItem>,
    },
    Item {
        task: Option<bool>,
        blocks: Vec<Block>,
    },
    Footnote {
        label: String,
        source: Range<usize>,
        blocks: Vec<Block>,
    },
    Indented {
        source: Range<usize>,
        blocks: Vec<Block>,
    },
    Table {
        alignments: Vec<Alignment>,
        source: Range<usize>,
        header: Vec<Inlines>,
        rows: Vec<Vec<Inlines>>,
        row: Vec<Inlines>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineKind {
    Paragraph,
    Heading(u8),
    Cell,
    /// Text directly inside a tight list item: becomes a paragraph.
    Implicit,
}

enum Leaf {
    Inline {
        kind: InlineKind,
        source: Range<usize>,
        inlines: Inlines,
    },
    Code {
        language: Option<String>,
        text: String,
        source: Range<usize>,
    },
    Html {
        text: String,
        source: Range<usize>,
    },
}

#[derive(Default)]
struct Builder {
    root: Vec<Block>,
    containers: Vec<Container>,
    leaf: Option<Leaf>,
    emphasis: u32,
    strong: u32,
    strikethrough: u32,
    link: u32,
    /// Image being read: destination, alt text, source start.
    image: Option<(String, String, usize)>,
}

impl Builder {
    fn style(&self) -> InlineStyle {
        InlineStyle {
            emphasis: self.emphasis > 0,
            strong: self.strong > 0,
            strikethrough: self.strikethrough > 0,
            link: self.link > 0,
            ..InlineStyle::default()
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match (&mut self.leaf, &mut self.image) {
                (Some(Leaf::Code { text: code, .. } | Leaf::Html { text: code, .. }), _) => {
                    code.push_str(&text)
                }
                (_, Some((_, alt, _))) => alt.push_str(&text),
                _ => {
                    let style = self.style();
                    self.inlines(range).push_text(&text, style);
                }
            },
            Event::Code(text) => self.styled(range, &text, |s| s.code = true),
            Event::InlineMath(text) => self.styled(range, &text, |s| s.math = true),
            // Display math becomes its own segment (drawn as a formula); inside an image's alt
            // text it stays text.
            Event::DisplayMath(text) if self.image.is_none() => {
                self.inlines(range).segments.push(Segment::Math {
                    tex: text.trim().to_string(),
                })
            }
            Event::DisplayMath(text) => self.styled(range, &text, |s| s.math = true),
            Event::Html(text) => match &mut self.leaf {
                Some(Leaf::Html { text: html, .. }) => html.push_str(&text),
                _ => self.styled(range, &text, |s| s.html = true),
            },
            Event::InlineHtml(text) => self.styled(range, &text, |s| s.html = true),
            Event::FootnoteReference(label) => {
                self.styled(range, &format!("[{label}]"), |s| s.footnote = true)
            }
            Event::SoftBreak => self.styled(range, " ", |_| {}),
            Event::HardBreak => self.styled(range, "\n", |_| {}),
            Event::Rule => {
                self.close_leaf();
                self.push_block(BlockKind::Rule, range);
            }
            Event::TaskListMarker(checked) => {
                if let Some(Container::Item { task, .. }) = self.containers.last_mut() {
                    *task = Some(checked);
                }
            }
        }
    }

    fn styled(&mut self, range: Range<usize>, text: &str, apply: impl FnOnce(&mut InlineStyle)) {
        if let Some((_, alt, _)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        let mut style = self.style();
        apply(&mut style);
        self.inlines(range).push_text(text, style);
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => self.open_inline(InlineKind::Paragraph, range),
            Tag::Heading { level, .. } => self.open_inline(InlineKind::Heading(level as u8), range),
            Tag::TableCell => {
                self.leaf = Some(Leaf::Inline {
                    kind: InlineKind::Cell,
                    source: range,
                    inlines: Inlines::default(),
                })
            }
            Tag::DefinitionListTitle => {
                self.strong += 1;
                self.open_inline(InlineKind::Paragraph, range);
            }
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_string)
                    }
                    CodeBlockKind::Indented => None,
                };
                self.open_code(language, range);
            }
            Tag::MetadataBlock(kind) => {
                let language = match kind {
                    MetadataBlockKind::YamlStyle => "yaml",
                    MetadataBlockKind::PlusesStyle => "toml",
                };
                self.open_code(Some(language.to_string()), range);
            }
            Tag::HtmlBlock => {
                self.close_leaf();
                self.leaf = Some(Leaf::Html {
                    text: String::new(),
                    source: range,
                });
            }
            Tag::BlockQuote(kind) => self.open(Container::Quote {
                alert: kind.map(alert),
                source: range,
                blocks: Vec::new(),
            }),
            Tag::List(start) => self.open(Container::List {
                start,
                source: range,
                items: Vec::new(),
            }),
            Tag::Item => self.open(Container::Item {
                task: None,
                blocks: Vec::new(),
            }),
            Tag::FootnoteDefinition(label) => self.open(Container::Footnote {
                label: label.to_string(),
                source: range,
                blocks: Vec::new(),
            }),
            Tag::DefinitionListDefinition => self.open(Container::Indented {
                source: range,
                blocks: Vec::new(),
            }),
            Tag::Table(alignments) => self.open(Container::Table {
                alignments: alignments.into_iter().map(alignment).collect(),
                source: range,
                header: Vec::new(),
                rows: Vec::new(),
                row: Vec::new(),
            }),
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strikethrough += 1,
            Tag::Link { .. } => self.link += 1,
            Tag::Image { dest_url, .. } => {
                self.image = Some((dest_url.to_string(), String::new(), range.start))
            }
            Tag::DefinitionList
            | Tag::TableHead
            | Tag::TableRow
            | Tag::Superscript
            | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::CodeBlock
            | TagEnd::MetadataBlock(_)
            | TagEnd::HtmlBlock => self.close_leaf(),
            TagEnd::DefinitionListTitle => {
                self.close_leaf();
                self.strong = self.strong.saturating_sub(1);
            }
            TagEnd::TableCell => {
                if let Some(Leaf::Inline { inlines, .. }) = self.leaf.take()
                    && let Some(Container::Table { row, .. }) = self.containers.last_mut()
                {
                    row.push(inlines);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(Container::Table {
                    header, rows, row, ..
                }) = self.containers.last_mut()
                {
                    let cells = std::mem::take(row);
                    if matches!(tag, TagEnd::TableHead) {
                        *header = cells;
                    } else {
                        rows.push(cells);
                    }
                }
            }
            TagEnd::Item => {
                self.close_leaf();
                if let Some(Container::Item { task, blocks }) = self.containers.pop() {
                    match self.containers.last_mut() {
                        Some(Container::List { items, .. }) => {
                            items.push(ListItem { task, blocks })
                        }
                        _ => self.root.extend(blocks), // malformed; keep the content
                    }
                }
            }
            TagEnd::BlockQuote(_)
            | TagEnd::List(_)
            | TagEnd::FootnoteDefinition
            | TagEnd::DefinitionListDefinition
            | TagEnd::Table => {
                self.close_leaf();
                if let Some(container) = self.containers.pop() {
                    self.close_container(container);
                }
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strikethrough = self.strikethrough.saturating_sub(1),
            TagEnd::Link => self.link = self.link.saturating_sub(1),
            TagEnd::Image => {
                if let Some((dest, alt, start)) = self.image.take() {
                    self.inlines(start..start)
                        .segments
                        .push(Segment::Image { dest, alt });
                }
            }
            TagEnd::DefinitionList | TagEnd::Superscript | TagEnd::Subscript => {}
        }
    }

    fn open(&mut self, container: Container) {
        self.close_leaf();
        self.containers.push(container);
    }

    fn open_inline(&mut self, kind: InlineKind, source: Range<usize>) {
        self.close_leaf();
        self.leaf = Some(Leaf::Inline {
            kind,
            source,
            inlines: Inlines::default(),
        });
    }

    fn open_code(&mut self, language: Option<String>, source: Range<usize>) {
        self.close_leaf();
        self.leaf = Some(Leaf::Code {
            language,
            text: String::new(),
            source,
        });
    }

    /// The inline content being built; text directly inside a list item starts an implicit
    /// paragraph.
    fn inlines(&mut self, at: Range<usize>) -> &mut Inlines {
        if !matches!(self.leaf, Some(Leaf::Inline { .. })) {
            self.close_leaf();
            self.leaf = Some(Leaf::Inline {
                kind: InlineKind::Implicit,
                source: at.clone(),
                inlines: Inlines::default(),
            });
        }
        match &mut self.leaf {
            Some(Leaf::Inline {
                inlines, source, ..
            }) => {
                source.end = source.end.max(at.end);
                inlines
            }
            _ => unreachable!("an inline leaf was just ensured"),
        }
    }

    fn close_leaf(&mut self) {
        let Some(leaf) = self.leaf.take() else {
            return;
        };
        let (kind, source) = match leaf {
            Leaf::Inline {
                kind: InlineKind::Heading(level),
                source,
                inlines,
            } => (
                BlockKind::Heading {
                    level,
                    content: inlines,
                },
                source,
            ),
            Leaf::Inline {
                kind: InlineKind::Cell,
                inlines,
                ..
            } => {
                if let Some(Container::Table { row, .. }) = self.containers.last_mut() {
                    row.push(inlines);
                }
                return;
            }
            Leaf::Inline {
                source, inlines, ..
            } => (BlockKind::Paragraph(inlines), source),
            Leaf::Code {
                language,
                mut text,
                source,
            } => {
                if text.ends_with('\n') {
                    text.pop();
                }
                (BlockKind::Code { language, text }, source)
            }
            Leaf::Html { text, source } => (BlockKind::Html(text), source),
        };
        self.push_block(kind, source);
    }

    fn close_container(&mut self, container: Container) {
        let (kind, source) = match container {
            Container::Quote {
                alert,
                source,
                blocks,
            } => (BlockKind::Quote { alert, blocks }, source),
            Container::List {
                start,
                source,
                items,
            } => (BlockKind::List { start, items }, source),
            Container::Footnote {
                label,
                source,
                blocks,
            } => (BlockKind::FootnoteDefinition { label, blocks }, source),
            Container::Indented { source, blocks } => (BlockKind::Indented(blocks), source),
            Container::Table {
                alignments,
                source,
                header,
                rows,
                ..
            } => (
                BlockKind::Table {
                    alignments,
                    header,
                    rows,
                },
                source,
            ),
            Container::Item { blocks, .. } => {
                self.root.extend(blocks);
                return;
            }
        };
        self.push_block(kind, source);
    }

    fn push_block(&mut self, kind: BlockKind, source: Range<usize>) {
        let block = Block { kind, source };
        match self.containers.last_mut() {
            Some(
                Container::Quote { blocks, .. }
                | Container::Item { blocks, .. }
                | Container::Footnote { blocks, .. }
                | Container::Indented { blocks, .. },
            ) => blocks.push(block),
            // Blocks never sit directly in a list or table; keep them rather than lose them.
            _ => self.root.push(block),
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.close_leaf();
        while let Some(container) = self.containers.pop() {
            self.close_container(container);
        }
        self.root
    }
}

fn alert(kind: BlockQuoteKind) -> AlertKind {
    match kind {
        BlockQuoteKind::Note => AlertKind::Note,
        BlockQuoteKind::Tip => AlertKind::Tip,
        BlockQuoteKind::Important => AlertKind::Important,
        BlockQuoteKind::Warning => AlertKind::Warning,
        BlockQuoteKind::Caution => AlertKind::Caution,
    }
}

fn alignment(a: pulldown_cmark::Alignment) -> Alignment {
    match a {
        pulldown_cmark::Alignment::None => Alignment::None,
        pulldown_cmark::Alignment::Left => Alignment::Left,
        pulldown_cmark::Alignment::Center => Alignment::Center,
        pulldown_cmark::Alignment::Right => Alignment::Right,
    }
}

/// Index of the top-level block whose source contains `byte`, or the last one starting before it.
pub fn block_at(blocks: &[Block], byte: usize) -> Option<usize> {
    blocks
        .partition_point(|b| b.source.start <= byte)
        .checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\
---
title: x
---
# Title *em*

Para with **bold**, `code`, [link](u) and ~~gone~~.

- tight
- [x] done
  - nested

1. loose

2. second

> [!WARNING]
> Careful.

| a | b |
|:--|--:|
| 1 | 2 |

```rust
fn main() {}
```

![alt text](img.png)

<div>html</div>

---

Note[^1].

[^1]: The note.
";

    fn kinds(blocks: &[Block]) -> Vec<&'static str> {
        blocks
            .iter()
            .map(|b| match &b.kind {
                BlockKind::Heading { .. } => "heading",
                BlockKind::Paragraph(_) => "paragraph",
                BlockKind::Code { .. } => "code",
                BlockKind::Quote { .. } => "quote",
                BlockKind::List { .. } => "list",
                BlockKind::Table { .. } => "table",
                BlockKind::Rule => "rule",
                BlockKind::Html(_) => "html",
                BlockKind::FootnoteDefinition { .. } => "footnote",
                BlockKind::Indented(_) => "indented",
            })
            .collect()
    }

    #[test]
    fn top_level_blocks_and_sources() {
        let blocks = parse(DOC);
        assert_eq!(
            kinds(&blocks),
            vec![
                "code",
                "heading",
                "paragraph",
                "list",
                "list",
                "quote",
                "table",
                "code",
                "paragraph",
                "html",
                "rule",
                "paragraph",
                "footnote"
            ]
        );
        assert!(DOC[blocks[1].source.clone()].starts_with("# Title"));
        assert!(DOC[blocks[6].source.clone()].starts_with("| a | b |"));
        // Sources are ordered, so scroll sync can binary-search them.
        assert!(
            blocks
                .windows(2)
                .all(|w| w[0].source.start <= w[1].source.start)
        );
        let table_start = blocks[6].source.start;
        assert_eq!(block_at(&blocks, table_start + 3), Some(6));
        assert_eq!(block_at(&blocks, 0), Some(0));
    }

    #[test]
    fn inline_styles() {
        let blocks = parse(DOC);
        let BlockKind::Paragraph(inlines) = &blocks[2].kind else {
            panic!()
        };
        let Segment::Text { text, spans } = &inlines.segments[0] else {
            panic!()
        };
        assert_eq!(text, "Para with bold, code, link and gone.");
        let styled = |word: &str| {
            let start = text.find(word).unwrap();
            spans
                .iter()
                .find(|(r, _)| r.start == start)
                .map(|(_, s)| *s)
                .unwrap()
        };
        assert!(styled("bold").strong);
        assert!(styled("code").code);
        assert!(styled("link").link);
        assert!(styled("gone").strikethrough);
    }

    #[test]
    fn lists_tasks_alerts_tables_code() {
        let blocks = parse(DOC);
        let BlockKind::List { start, items } = &blocks[3].kind else {
            panic!()
        };
        assert_eq!(*start, None);
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].task, items[1].task), (None, Some(true)));
        assert!(matches!(
            items[1].blocks.last().unwrap().kind,
            BlockKind::List { .. }
        ));
        let BlockKind::List { start, items } = &blocks[4].kind else {
            panic!()
        };
        assert_eq!((*start, items.len()), (Some(1), 2));
        assert!(matches!(
            blocks[5].kind,
            BlockKind::Quote {
                alert: Some(AlertKind::Warning),
                ..
            }
        ));
        let BlockKind::Table {
            alignments,
            header,
            rows,
        } = &blocks[6].kind
        else {
            panic!()
        };
        assert_eq!(alignments, &vec![Alignment::Left, Alignment::Right]);
        assert_eq!(
            header.iter().map(Inlines::plain_text).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(rows[0][1].plain_text(), "2");
        assert_eq!(
            blocks[7].kind,
            BlockKind::Code {
                language: Some("rust".into()),
                text: "fn main() {}".into()
            }
        );
        assert_eq!(
            blocks[0].kind,
            BlockKind::Code {
                language: Some("yaml".into()),
                text: "title: x".into()
            }
        );
    }

    #[test]
    fn display_math_is_its_own_segment() {
        let blocks = parse("Before $a^2$ and\n$$\n\\frac{1}{2}\n$$\nafter.\n");
        let BlockKind::Paragraph(inlines) = &blocks[0].kind else {
            panic!("{blocks:?}")
        };
        let kinds: Vec<_> = inlines
            .segments
            .iter()
            .map(|s| match s {
                Segment::Text { text, .. } => format!("text {text:?}"),
                Segment::Math { tex } => format!("math {tex:?}"),
                Segment::Image { .. } => "image".into(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "text \"Before a^2 and \"",
                "math \"\\\\frac{1}{2}\"",
                "text \" after.\""
            ]
        );
    }

    #[test]
    fn images_and_footnotes() {
        let blocks = parse(DOC);
        let BlockKind::Paragraph(inlines) = &blocks[8].kind else {
            panic!()
        };
        assert_eq!(
            inlines.segments,
            vec![Segment::Image {
                dest: "img.png".into(),
                alt: "alt text".into()
            }]
        );
        let BlockKind::FootnoteDefinition {
            label,
            blocks: inner,
        } = &blocks[12].kind
        else {
            panic!()
        };
        assert_eq!(label, "1");
        assert_eq!(inner.len(), 1);
    }

    #[test]
    fn empty_and_plain_documents() {
        assert!(parse("").is_empty());
        assert_eq!(kinds(&parse("just text")), vec!["paragraph"]);
    }
}
