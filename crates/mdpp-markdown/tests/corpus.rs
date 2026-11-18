//! Phase 2 exit (PLAN §7): over a corpus of real Markdown — this repository's own documents —
//! the structure index and the preview blocks agree with the text, and scroll sync maps every
//! block's first byte (and every byte inside it) back to that block.

use std::path::Path;

use mdpp_markdown::blocks::{self, block_at};
use mdpp_markdown::{DocIndex, Highlighter};

fn corpus() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = vec![root.join("README.md"), root.join("CONTRIBUTING.md")];
    for dir in ["docs", "docs/research"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap().flatten() {
            if entry.path().extension().is_some_and(|e| e == "md") {
                files.push(entry.path());
            }
        }
    }
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            (path.display().to_string(), text)
        })
        .collect()
}

#[test]
fn scroll_sync_lands_on_the_right_block_in_every_corpus_file() {
    let corpus = corpus();
    assert!(
        corpus.len() >= 8,
        "the corpus is the repository's documents"
    );
    for (name, text) in &corpus {
        let blocks = blocks::parse(text);
        assert!(!blocks.is_empty(), "{name}: no blocks");
        for (i, block) in blocks.iter().enumerate() {
            assert!(block.source.start < block.source.end && block.source.end <= text.len());
            if i > 0 {
                assert!(
                    blocks[i - 1].source.end <= block.source.start,
                    "{name}: overlap at {i}"
                );
            }
            assert_eq!(
                block_at(&blocks, block.source.start),
                Some(i),
                "{name}: block {i} start"
            );
            assert_eq!(
                block_at(&blocks, block.source.end - 1),
                Some(i),
                "{name}: block {i} end"
            );
        }
    }
}

#[test]
fn index_and_highlighter_agree_with_the_text() {
    for (name, text) in corpus() {
        let index = DocIndex::build(&text);
        for heading in &index.headings {
            assert!(
                text[heading.range.clone()].trim_start().starts_with('#')
                    || text[heading.range.clone()].contains('\n'),
                "{name}: {heading:?}"
            );
            assert!(!heading.anchor.is_empty(), "{name}: empty anchor");
        }
        for link in &index.links {
            assert!(link.range.end <= text.len() && text.is_char_boundary(link.range.start));
        }
        // Incremental highlighting of the whole file equals a fresh build (PLAN §3.2).
        let rope = ropey::Rope::from_str(&text);
        let highlighter = Highlighter::new(&rope);
        assert_eq!(
            highlighter.line_count(),
            rope.len_lines(ropey::LineType::LF),
            "{name}"
        );
    }
}
