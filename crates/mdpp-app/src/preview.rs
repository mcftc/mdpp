//! Native Markdown preview (PLAN D6): `mdpp-markdown` blocks drawn with GPUI/gpui-kit primitives in
//! a virtual list, so only visible blocks are laid out. Each top-level block knows its source
//! range; scroll sync maps an editor line to the block containing it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mdpp_markdown::blocks::{
    self, AlertKind, Alignment, Block, BlockKind, InlineStyle, Inlines, Segment,
};

/// Pixels rendered beyond the viewport so fast scrolling does not show blank space.
const OVERDRAW: f32 = 600.0;
/// Heading size relative to body text, H1…H6 (GitHub-like proportions).
const HEADING_SCALE: [f32; 6] = [1.9, 1.5, 1.25, 1.1, 1.0, 0.9];
/// Code, HTML and footnotes are set slightly smaller than body text.
const SMALL_SCALE: f32 = 0.9;

pub struct Preview {
    blocks: Arc<Vec<Block>>,
    list: ListState,
    doc_dir: Option<PathBuf>,
    max_width: Pixels,
    remote_images: bool,
}

impl Preview {
    pub fn new(max_width: f32, remote_images: bool) -> Self {
        Self {
            blocks: Arc::default(),
            list: ListState::new(0, ListAlignment::Top, px(OVERDRAW)),
            doc_dir: None,
            max_width: px(max_width),
            remote_images,
        }
    }

    /// Show a new parse of the document, keeping the reading position.
    pub fn set_document(
        &mut self,
        blocks: Arc<Vec<Block>>,
        doc_dir: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let top = self.list.logical_scroll_top();
        self.list.reset(blocks.len());
        if !blocks.is_empty() {
            self.list.scroll_to(ListOffset {
                item_ix: top.item_ix.min(blocks.len() - 1),
                offset_in_item: top.offset_in_item,
            });
        }
        self.blocks = blocks;
        self.doc_dir = doc_dir;
        cx.notify();
    }

    /// Scroll so the block containing source byte `byte` is at the top.
    pub fn reveal_source(&mut self, byte: usize, cx: &mut Context<Self>) {
        let Some(index) = blocks::block_at(&self.blocks, byte) else {
            return;
        };
        if self.list.logical_scroll_top().item_ix != index {
            self.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
            cx.notify();
        }
    }

    /// Number of top-level blocks and the index of the block at the top (for the checks).
    pub fn position(&self) -> (usize, usize) {
        (self.blocks.len(), self.list.logical_scroll_top().item_ix)
    }

    pub fn block_index_at(&self, byte: usize) -> Option<usize> {
        blocks::block_at(&self.blocks, byte)
    }

    fn render_item(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(block) = self.blocks.get(index) else {
            return div().into_any_element();
        };
        let style = Palette::new(cx, self.doc_dir.as_deref(), self.remote_images);
        div()
            .w_full()
            .flex()
            .justify_center()
            .px_6()
            .py_1p5()
            .child(
                div()
                    .w_full()
                    .max_w(self.max_width)
                    .child(render_block(block, &style)),
            )
            .into_any_element()
    }
}

impl Render for Preview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_size(theme.font_size)
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this, index, _window, cx| this.render_item(index, cx)),
                )
                .size_full()
                .py_4(),
            )
    }
}

/// Everything the block renderers need from the theme, read once per item.
struct Palette<'a> {
    muted: Hsla,
    border: Hsla,
    link: Hsla,
    code_background: Hsla,
    alert: [Hsla; 5],
    mono: SharedString,
    font_size: Pixels,
    doc_dir: Option<&'a Path>,
    remote_images: bool,
}

impl<'a> Palette<'a> {
    fn new(cx: &App, doc_dir: Option<&'a Path>, remote_images: bool) -> Self {
        let theme = cx.theme();
        Self {
            muted: theme.muted_foreground,
            border: theme.border,
            link: theme.link,
            code_background: theme.secondary,
            alert: [
                theme.info,
                theme.success,
                theme.magenta,
                theme.warning,
                theme.danger,
            ],
            mono: theme.mono_font_family.clone(),
            font_size: theme.font_size,
            doc_dir,
            remote_images,
        }
    }

    fn alert(&self, kind: AlertKind) -> (Hsla, &'static str) {
        match kind {
            AlertKind::Note => (self.alert[0], "Note"),
            AlertKind::Tip => (self.alert[1], "Tip"),
            AlertKind::Important => (self.alert[2], "Important"),
            AlertKind::Warning => (self.alert[3], "Warning"),
            AlertKind::Caution => (self.alert[4], "Caution"),
        }
    }

    fn highlight(&self, style: InlineStyle) -> HighlightStyle {
        let mut out = HighlightStyle::default();
        if style.strong {
            out.font_weight = Some(FontWeight::BOLD);
        }
        if style.emphasis {
            out.font_style = Some(FontStyle::Italic);
        }
        if style.strikethrough {
            out.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        if style.code || style.math {
            out.background_color = Some(self.code_background);
        }
        if style.link || style.footnote {
            out.color = Some(self.link);
            out.underline = style.link.then(|| UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            });
        }
        if style.html {
            out.color = Some(self.muted);
        }
        out
    }
}

fn render_block(block: &Block, p: &Palette) -> AnyElement {
    match &block.kind {
        BlockKind::Heading { level, content } => {
            let level = usize::from(*level).clamp(1, HEADING_SCALE.len());
            div()
                .pt_2()
                .text_size(p.font_size * HEADING_SCALE[level - 1])
                .font_weight(FontWeight::BOLD)
                .when(level <= 2, |d| d.pb_1().border_b_1().border_color(p.border))
                .child(render_inlines(content, p))
                .into_any_element()
        }
        BlockKind::Paragraph(content) => render_inlines(content, p),
        BlockKind::Code { language, text } => v_flex()
            .w_full()
            .p_3()
            .rounded_md()
            .bg(p.code_background)
            .font_family(p.mono.clone())
            .text_size(p.font_size * SMALL_SCALE)
            .when_some(language.clone(), |d, language| {
                d.child(div().text_xs().text_color(p.muted).child(language))
            })
            .child(text.clone())
            .into_any_element(),
        BlockKind::Quote { alert, blocks } => {
            let (color, title) = match alert {
                Some(kind) => {
                    let (color, title) = p.alert(*kind);
                    (color, Some(title))
                }
                None => (p.border, None),
            };
            v_flex()
                .gap_2()
                .pl_3()
                .border_l_4()
                .border_color(color)
                .when(alert.is_none(), |d| d.text_color(p.muted))
                .when_some(title, |d, title| {
                    d.child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(color)
                            .child(title),
                    )
                })
                .children(blocks.iter().map(|b| render_block(b, p)))
                .into_any_element()
        }
        BlockKind::List { start, items } => v_flex()
            .gap_1()
            .children(items.iter().enumerate().map(|(i, item)| {
                let marker = match (item.task, start) {
                    (Some(true), _) => "☑".to_string(),
                    (Some(false), _) => "☐".to_string(),
                    (None, Some(first)) => format!("{}.", first + i as u64),
                    (None, None) => "•".to_string(),
                };
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(div().min_w(px(20.)).text_color(p.muted).child(marker))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .children(item.blocks.iter().map(|b| render_block(b, p))),
                    )
            }))
            .into_any_element(),
        BlockKind::Table {
            alignments,
            header,
            rows,
        } => {
            let row = |cells: &[Inlines], head: bool| {
                h_flex()
                    .w_full()
                    .border_b_1()
                    .border_color(p.border)
                    .when(head, |d| {
                        d.font_weight(FontWeight::BOLD).bg(p.code_background)
                    })
                    .children(cells.iter().enumerate().map(|(i, cell)| {
                        let cell_div = div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .child(render_inlines(cell, p));
                        match alignments.get(i) {
                            Some(Alignment::Center) => cell_div.text_center(),
                            Some(Alignment::Right) => cell_div.text_right(),
                            _ => cell_div,
                        }
                    }))
            };
            v_flex()
                .w_full()
                .border_1()
                .border_color(p.border)
                .rounded_sm()
                .child(row(header, true))
                .children(rows.iter().map(|cells| row(cells, false)))
                .into_any_element()
        }
        BlockKind::Rule => div()
            .my_2()
            .h(px(1.))
            .w_full()
            .bg(p.border)
            .into_any_element(),
        BlockKind::Html(html) => div()
            .font_family(p.mono.clone())
            .text_size(p.font_size * SMALL_SCALE)
            .text_color(p.muted)
            .child(html.trim_end().to_string())
            .into_any_element(),
        BlockKind::FootnoteDefinition { label, blocks } => h_flex()
            .items_start()
            .gap_2()
            .text_size(p.font_size * SMALL_SCALE)
            .child(div().text_color(p.link).child(format!("[{label}]")))
            .child(
                v_flex()
                    .flex_1()
                    .children(blocks.iter().map(|b| render_block(b, p))),
            )
            .into_any_element(),
        BlockKind::Indented(blocks) => v_flex()
            .pl_6()
            .children(blocks.iter().map(|b| render_block(b, p)))
            .into_any_element(),
    }
}

fn render_inlines(content: &Inlines, p: &Palette) -> AnyElement {
    let mut parts = content.segments.iter().map(|segment| match segment {
        Segment::Text { text, spans } => {
            let highlights = spans
                .iter()
                .map(|(range, style)| (range.clone(), p.highlight(*style)));
            StyledText::new(text.clone())
                .with_highlights(highlights)
                .into_any_element()
        }
        Segment::Image { dest, alt } => render_image(dest, alt, p),
    });
    match content.segments.len() {
        0 => div().into_any_element(),
        1 => parts.next().unwrap_or_else(|| div().into_any_element()),
        _ => v_flex().gap_1().children(parts).into_any_element(),
    }
}

fn render_image(dest: &str, alt: &str, p: &Palette) -> AnyElement {
    let remote = dest.contains("://");
    let source: Option<ImageSource> = if remote {
        p.remote_images
            .then(|| SharedUri::from(dest.to_string()).into())
    } else {
        p.doc_dir.map(|dir| dir.join(dest).into())
    };
    match source {
        Some(source) => img(source).max_w_full().into_any_element(),
        None => div()
            .text_color(p.muted)
            .child(format!(
                "[image: {}]",
                if alt.is_empty() { dest } else { alt }
            ))
            .into_any_element(),
    }
}
