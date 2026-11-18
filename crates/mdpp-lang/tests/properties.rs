//! Property tests (PLAN §8, the fuzzing goal of Phase 6): on any Markdown-ish text and any caret
//! or selection, the assists, completion, navigation, rename, code actions and diagnostics never
//! panic, and every edit they return is inside the text and on character boundaries.

use std::path::Path;

use mdpp_lang::assist::{self, Edit};
use mdpp_lang::{FileSystem, actions, context_at, diagnose, lines, navigate, rename, table};
use mdpp_markdown::{BlockState, DocIndex};
use proptest::prelude::*;
use ropey::Rope;

/// Text built from Markdown's punctuation, some letters, multi-byte characters and newlines.
fn markdownish() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            "[#*_`>|:!\\[\\]()\\- .1\\\\$^~]",
            "[a-zğüşöçı]",
            Just("\n".to_string()),
            Just("| a | b |\n| --- | --- |\n".to_string()),
            Just("# Title\n".to_string()),
            Just("- [ ] task\n".to_string()),
            Just("[x](#title)".to_string()),
            Just("```rust\nfn x() {}\n```\n".to_string()),
            Just("日本".to_string()),
        ],
        0..60,
    )
    .prop_map(|parts| parts.concat())
}

/// A byte offset into `text` on a character boundary.
fn boundary(text: &str, pick: usize) -> usize {
    let mut offset = pick % (text.len() + 1);
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn check(text: &str, edit: &Edit) -> Result<(), TestCaseError> {
    prop_assert!(edit.range.start <= edit.range.end && edit.range.end <= text.len());
    prop_assert!(text.is_char_boundary(edit.range.start) && text.is_char_boundary(edit.range.end));
    let mut after = text.to_string();
    after.replace_range(edit.range.clone(), &edit.text);
    prop_assert!(edit.select.start <= edit.select.end && edit.select.end <= after.len());
    prop_assert!(after.is_char_boundary(edit.select.start));
    prop_assert!(after.is_char_boundary(edit.select.end));
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(600))]

    #[test]
    fn edits_stay_inside_the_text(text in markdownish(), a: usize, b: usize) {
        let rope = Rope::from_str(&text);
        let (x, y) = (boundary(&text, a), boundary(&text, b));
        let selection = x.min(y)..x.max(y);
        let caret = selection.start;
        let mut edits = vec![
            assist::toggle_wrap(&rope, selection.clone(), "**"),
            assist::insert_link(&rope, selection.clone()),
            assist::toggle_task(&rope, caret),
            assist::shift_heading(&rope, caret, 1),
            assist::shift_heading(&rope, caret, -1),
            lines::duplicate_lines(&rope, selection.clone()),
            lines::delete_lines(&rope, selection.clone()),
        ];
        edits.extend(assist::on_enter(&rope, caret));
        edits.extend(assist::indent_list(&rope, selection.clone(), false));
        edits.extend(assist::indent_list(&rope, selection.clone(), true));
        edits.extend(assist::paste_link(&rope, selection.clone(), "https://e.com"));
        edits.extend(lines::move_lines(&rope, selection.clone(), true));
        edits.extend(lines::move_lines(&rope, selection.clone(), false));
        edits.extend(table::on_tab(&rope, selection.clone(), false));
        edits.extend(table::on_tab(&rope, selection.clone(), true));
        edits.extend(table::on_enter(&rope, caret));
        edits.extend(table::format(&rope, caret));
        for edit in &edits {
            check(&text, edit)?;
        }
    }

    #[test]
    fn analysis_never_panics(text in markdownish(), a: usize, name in "[a-z ]{0,8}") {
        let rope = Rope::from_str(&text);
        let caret = boundary(&text, a);
        let index = DocIndex::build(&text);
        let dir = Some(Path::new("/nonexistent-mdpp-dir"));
        let _ = context_at(&rope, caret, BlockState::Normal);
        let _ = diagnose(&text, &index, dir, &FileSystem);
        let _ = navigate::target_at(&index, caret, dir);
        let _ = navigate::hover_at(&index, &text, caret, dir, &FileSystem);
        for action in actions::code_actions(&text, &index, caret, dir, &FileSystem) {
            if let actions::ActionKind::Edit(edit) = &action.kind {
                check(&text, edit)?;
            }
        }
        if let Some((which, _)) = rename::heading_at(&text, &index, caret)
            && let Some(edit) = rename::rename_heading(&text, which, &name)
        {
            check(&text, &edit)?;
        }
    }
}
