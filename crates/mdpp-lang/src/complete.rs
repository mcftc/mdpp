//! Completion (PLAN §4.3): what is being typed is read from the current line of the current
//! snapshot, never from a possibly stale index (PLAN §3.2); the index only supplies names.

use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use mdpp_markdown::{BlockState, DocIndex};
use ropey::{LineType, Rope};

use crate::data::{ALERTS, FENCE_LANGUAGES, IMAGE_EXTENSIONS};
use crate::paths::{PathSource, headings_of, percent_decode, percent_encode_path};

/// Only this much of the line before the cursor is inspected.
const MAX_CONTEXT_BYTES: usize = 1_024;
/// Characters after `:` before emoji suggestions appear (fewer would fire on ordinary colons).
pub const EMOJI_MIN_PREFIX: usize = 2;
/// Upper bound on emoji suggestions per request.
pub const EMOJI_MAX_RESULTS: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Heading,
    File,
    Folder,
    Reference,
    Footnote,
    Emoji,
    Language,
    Alert,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub insert: String,
    /// Bytes of the document replaced by `insert`.
    pub replace: Range<usize>,
    pub kind: CompletionKind,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextKind {
    /// `](#…` — a heading of this document.
    Anchor,
    /// `](other.md#…` — a heading of another file (`file` as typed, relative to the document).
    FileAnchor { file: String },
    /// `](…` or `![](…` — a file or folder; `dir` is what was typed up to the last `/`.
    Path { dir: String, images: bool },
    /// `[text][…`
    Reference,
    /// `[^…`
    Footnote,
    /// `:…`
    Emoji,
    /// ```` ```… ````
    FenceLanguage,
    /// `> [!…`
    Alert,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub kind: ContextKind,
    /// What has been typed so far; candidates are filtered by it.
    pub prefix: String,
    /// Bytes an accepted completion replaces.
    pub replace: Range<usize>,
}

impl Context {
    /// Whether [`candidates`] needs a [`DocIndex`] for this context.
    pub fn needs_index(&self) -> bool {
        matches!(
            self.kind,
            ContextKind::Anchor | ContextKind::Reference | ContextKind::Footnote
        )
    }
}

/// The completion context at byte `offset`. `line_state` is the highlighter's block state of the
/// cursor's line: nothing is offered inside code blocks, front matter, comments or math.
pub fn context_at(text: &Rope, offset: usize, line_state: BlockState) -> Option<Context> {
    if line_state != BlockState::Normal || offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let row = text.byte_to_line_idx(offset, LineType::LF);
    let mut start = text.line_to_byte_idx(row, LineType::LF);
    if offset - start > MAX_CONTEXT_BYTES {
        start = offset - MAX_CONTEXT_BYTES;
        while !text.is_char_boundary(start) {
            start += 1;
        }
    }
    let before = Cow::from(text.slice(start..offset));
    context_in_line(&before, start)
}

/// `before` is the line up to the cursor; it starts at document byte `base`.
fn context_in_line(before: &str, base: usize) -> Option<Context> {
    let ctx = |kind: ContextKind, from: usize| {
        Some(Context {
            kind,
            prefix: before[from..].to_string(),
            replace: base + from..base + before.len(),
        })
    };
    let is_word =
        |s: &str, extra: &str| s.chars().all(|c| c.is_alphanumeric() || extra.contains(c));

    // A code-fence line offers languages and nothing else.
    let body = strip_block_prefix(before);
    for fence in ['`', '~'] {
        let run = body.chars().take_while(|&c| c == fence).count();
        if run >= 3 {
            let info = &body[run..];
            return is_word(info, "+#.-_")
                .then(|| ctx(ContextKind::FenceLanguage, before.len() - info.len()))?;
        }
    }

    // `> [!NOTE`
    if let Some(quoted) = before.trim_start().strip_prefix('>')
        && let Some(kind) = quoted.trim_start().strip_prefix("[!")
        && kind.chars().all(|c| c.is_ascii_alphabetic())
    {
        return ctx(ContextKind::Alert, before.len() - kind.len());
    }

    // Link destinations: `](…`
    if let Some(p) = before.rfind("](") {
        let target = &before[p + 2..];
        if !target.contains([')', ' ', '\t', '<', '>', '"']) {
            let target_start = p + 2;
            if let Some(hash) = target.find('#') {
                let kind = match hash {
                    0 => ContextKind::Anchor,
                    _ => ContextKind::FileAnchor {
                        file: target[..hash].to_string(),
                    },
                };
                return ctx(kind, target_start + hash + 1);
            }
            let name_start = target.rfind('/').map_or(0, |s| s + 1);
            let images = before[..p]
                .rfind('[')
                .is_some_and(|open| open > 0 && before.as_bytes()[open - 1] == b'!');
            let dir = target[..name_start].to_string();
            return ctx(ContextKind::Path { dir, images }, target_start + name_start);
        }
    }

    // `[text][label`
    if let Some(p) = before.rfind("][") {
        let label = &before[p + 2..];
        if !label.contains([']', '[']) {
            return ctx(ContextKind::Reference, p + 2);
        }
    }

    // `[^label`
    if let Some(p) = before.rfind("[^") {
        let label = &before[p + 2..];
        if !label.contains([']', ' ', '\t']) {
            return ctx(ContextKind::Footnote, p + 2);
        }
    }

    // `:shortcode` at the start of a word.
    if let Some(colon) = before.rfind(':') {
        let code = &before[colon + 1..];
        let at_word_start = before[..colon]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || "([".contains(c));
        if at_word_start
            && code.len() >= EMOJI_MIN_PREFIX
            && code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_+-".contains(c))
        {
            // The colon is replaced too: `:smile` becomes the emoji.
            return Some(Context {
                kind: ContextKind::Emoji,
                prefix: code.to_string(),
                replace: base + colon..base + before.len(),
            });
        }
    }
    None
}

/// The line without indentation, blockquote markers and a list marker.
fn strip_block_prefix(line: &str) -> &str {
    let mut s = line.trim_start();
    while let Some(rest) = s.strip_prefix('>') {
        s = rest.trim_start();
    }
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(bullet) {
            return rest.trim_start();
        }
    }
    s
}

/// Candidates for `ctx`. `index` is needed when [`Context::needs_index`]; `doc_dir` is the folder
/// of the document (`None` for an unsaved one, which gets no path completion).
pub fn candidates(
    ctx: &Context,
    index: Option<&DocIndex>,
    files: &dyn PathSource,
    doc_dir: Option<&Path>,
) -> Vec<Completion> {
    let item = |label: String, insert: String, kind, detail: Option<String>| Completion {
        label,
        insert,
        replace: ctx.replace.clone(),
        kind,
        detail,
    };
    let prefix = ctx.prefix.to_lowercase();
    let mut out: Vec<Completion> = match &ctx.kind {
        ContextKind::Anchor => index
            .map(|index| {
                index
                    .headings
                    .iter()
                    .filter(|h| h.anchor.contains(&prefix))
                    .map(|h| {
                        let detail = format!("{} {}", "#".repeat(usize::from(h.level)), h.text);
                        item(
                            h.anchor.clone(),
                            h.anchor.clone(),
                            CompletionKind::Heading,
                            Some(detail),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ContextKind::FileAnchor { file } => doc_dir
            .and_then(|dir| headings_of(files, &dir.join(percent_decode(file))))
            .unwrap_or_default()
            .into_iter()
            .filter(|h| h.anchor.contains(&prefix))
            .map(|h| {
                let detail = format!("{} {}", "#".repeat(usize::from(h.level)), h.text);
                item(
                    h.anchor.clone(),
                    h.anchor,
                    CompletionKind::Heading,
                    Some(detail),
                )
            })
            .collect(),
        ContextKind::Path { dir, images } => {
            let Some(doc_dir) = doc_dir else {
                return Vec::new();
            };
            let folder = doc_dir.join(percent_decode(dir));
            let typed = percent_decode(&prefix);
            let mut entries: Vec<_> = files
                .list(&folder)
                .into_iter()
                .filter(|e| typed.starts_with('.') || !e.name.starts_with('.'))
                .filter(|e| e.name.to_lowercase().starts_with(&typed))
                .filter(|e| e.is_dir || !images || is_image(&e.name))
                .collect();
            entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
            entries
                .into_iter()
                .map(|e| {
                    let encoded = percent_encode_path(&e.name);
                    if e.is_dir {
                        item(
                            format!("{}/", e.name),
                            format!("{encoded}/"),
                            CompletionKind::Folder,
                            None,
                        )
                    } else {
                        item(e.name, encoded, CompletionKind::File, None)
                    }
                })
                .collect()
        }
        ContextKind::Reference => index
            .map(|index| {
                index
                    .references
                    .iter()
                    .filter(|r| r.label.to_lowercase().contains(&prefix))
                    .map(|r| {
                        item(
                            r.label.clone(),
                            r.label.clone(),
                            CompletionKind::Reference,
                            Some(r.dest.clone()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ContextKind::Footnote => index
            .map(|index| {
                index
                    .footnotes
                    .iter()
                    .filter(|f| f.label.to_lowercase().contains(&prefix))
                    .map(|f| {
                        item(
                            f.label.clone(),
                            f.label.clone(),
                            CompletionKind::Footnote,
                            None,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ContextKind::Emoji => {
            let mut found: Vec<(&str, &str)> = emojis::iter()
                .flat_map(|e| e.shortcodes().map(move |code| (code, e.as_str())))
                .filter(|(code, _)| code.starts_with(&prefix))
                .collect();
            found.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(b.0)));
            found
                .into_iter()
                .take(EMOJI_MAX_RESULTS)
                .map(|(code, emoji)| {
                    item(
                        format!("{emoji} :{code}:"),
                        emoji.to_string(),
                        CompletionKind::Emoji,
                        None,
                    )
                })
                .collect()
        }
        ContextKind::FenceLanguage => FENCE_LANGUAGES
            .iter()
            .filter(|lang| lang.starts_with(&prefix))
            .map(|lang| {
                item(
                    (*lang).to_string(),
                    (*lang).to_string(),
                    CompletionKind::Language,
                    None,
                )
            })
            .collect(),
        ContextKind::Alert => ALERTS
            .iter()
            .filter(|alert| alert.to_lowercase().starts_with(&prefix))
            .map(|alert| {
                item(
                    (*alert).to_string(),
                    format!("{alert}]"),
                    CompletionKind::Alert,
                    None,
                )
            })
            .collect(),
    };
    // Exact-prefix matches first; otherwise keep document/alphabetical order.
    out.sort_by_key(|c| !c.label.to_lowercase().starts_with(&prefix));
    out
}

fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::fake::FakeFs;

    /// Context at the `|` marker of `line` (single-line document).
    fn context(line: &str) -> Option<Context> {
        let cursor = line.find('|').expect("cursor marker");
        let text = Rope::from_str(&line.replace('|', ""));
        context_at(&text, cursor, BlockState::Normal)
    }

    fn kind(line: &str) -> Option<ContextKind> {
        context(line).map(|c| c.kind)
    }

    #[test]
    fn detects_contexts() {
        assert_eq!(kind("see [x](#ins|"), Some(ContextKind::Anchor));
        assert_eq!(
            kind("see [x](docs/gui|"),
            Some(ContextKind::Path {
                dir: "docs/".into(),
                images: false
            })
        );
        assert_eq!(
            kind("![logo](|"),
            Some(ContextKind::Path {
                dir: String::new(),
                images: true
            })
        );
        assert_eq!(kind("[a][re|"), Some(ContextKind::Reference));
        assert_eq!(kind("text[^no|"), Some(ContextKind::Footnote));
        assert_eq!(kind("hi :smi|"), Some(ContextKind::Emoji));
        assert_eq!(kind("```py|"), Some(ContextKind::FenceLanguage));
        assert_eq!(kind("- ~~~|"), Some(ContextKind::FenceLanguage));
        assert_eq!(kind("> [!WAR|"), Some(ContextKind::Alert));
    }

    #[test]
    fn ignores_non_contexts() {
        assert_eq!(kind("[x](done) and more|"), None);
        assert_eq!(kind("[x](a b|"), None);
        assert_eq!(kind("time 12:30|"), None);
        assert_eq!(kind("note:s|"), None);
        assert_eq!(kind(":s|"), None); // too short
        assert_eq!(kind("```rust {x}|"), None);
        let text = Rope::from_str("[x](#a");
        assert_eq!(context_at(&text, 6, BlockState::HtmlComment), None);
    }

    #[test]
    fn replace_ranges() {
        let c = context("ab [x](#in|").unwrap();
        assert_eq!((c.prefix.as_str(), c.replace), ("in", 8..10));
        let c = context("hi :smi|").unwrap();
        assert_eq!((c.prefix.as_str(), c.replace), ("smi", 3..7));
        let c = context("[x](a/b/c|").unwrap();
        assert_eq!(c.replace, 8..9);
        // Multi-line: offsets are document offsets.
        let text = Rope::from_str("line one\n[x](#");
        assert_eq!(
            context_at(&text, 14, BlockState::Normal).unwrap().replace,
            14..14
        );
    }

    #[test]
    fn anchor_candidates() {
        let index = DocIndex::build("# Install guide\n## Usage\n## Install guide\n");
        let ctx = context("[x](#inst|").unwrap();
        let items = candidates(&ctx, Some(&index), &FakeFs::default(), None);
        let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["install-guide", "install-guide-1"]);
        assert_eq!(items[0].detail.as_deref(), Some("# Install guide"));
    }

    #[test]
    fn anchors_of_another_file() {
        assert_eq!(
            kind("[x](docs/other%20one.md#a|"),
            Some(ContextKind::FileAnchor {
                file: "docs/other%20one.md".into()
            })
        );
        let fs = FakeFs::with(&["/d/docs/other one.md", "/d/a.txt"])
            .text(
                "/d/docs/other one.md",
                "# Setup\r\n## Set up again\n# Use\n",
            )
            .text("/d/a.txt", "# Not markdown\n");
        let dir = Some(Path::new("/d"));
        let ctx = context("[x](docs/other%20one.md#set|").unwrap();
        let items = candidates(&ctx, None, &fs, dir);
        let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["setup", "set-up-again"]);
        assert_eq!(items[1].detail.as_deref(), Some("## Set up again"));
        assert_eq!(items[0].replace, 24..27); // just "set"
        // Not Markdown, unreadable or unsaved document: nothing.
        assert!(candidates(&context("[x](a.txt#|").unwrap(), None, &fs, dir).is_empty());
        assert!(candidates(&context("[x](none.md#|").unwrap(), None, &fs, dir).is_empty());
        assert!(candidates(&ctx, None, &fs, None).is_empty());
    }

    #[test]
    fn path_candidates() {
        let fs = FakeFs::with(&[
            "/d/notes.md",
            "/d/img/",
            "/d/img/a b.png",
            "/d/img/c.txt",
            "/d/.git/",
        ]);
        let dir = Some(Path::new("/d"));
        let ctx = context("[x](|").unwrap();
        let labels: Vec<_> = candidates(&ctx, None, &fs, dir)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, vec!["img/", "notes.md"]);
        let ctx = context("![x](img/|").unwrap();
        let items = candidates(&ctx, None, &fs, dir);
        assert_eq!(items.len(), 1);
        assert_eq!(
            (items[0].label.as_str(), items[0].insert.as_str()),
            ("a b.png", "a%20b.png")
        );
        // Unsaved documents have no folder.
        assert!(candidates(&ctx, None, &fs, None).is_empty());
    }

    #[test]
    fn other_candidates() {
        let index = DocIndex::build("[r1]: https://a.b\n[Ref2]: /x\n\nT[^note].\n\n[^note]: n\n");
        let fs = FakeFs::default();
        let refs = candidates(&context("[t][re|").unwrap(), Some(&index), &fs, None);
        assert_eq!(
            refs.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            vec!["Ref2"]
        );
        let notes = candidates(&context("x[^n|").unwrap(), Some(&index), &fs, None);
        assert_eq!(notes[0].insert, "note");
        let emoji = candidates(&context(":rocke|").unwrap(), None, &fs, None);
        assert_eq!(emoji[0].insert, "🚀");
        let langs = candidates(&context("```ru|").unwrap(), None, &fs, None);
        assert_eq!(langs[0].insert, "ruby");
        let alerts = candidates(&context("> [!w|").unwrap(), None, &fs, None);
        assert_eq!(alerts[0].insert, "WARNING]");
    }
}
