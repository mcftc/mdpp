//! Whole-line commands (PLAN §6.3): duplicate, delete and move the lines a selection touches.
//!
//! Like the assists, each is one [`Edit`] (one undo step) computed from the text and selection.

use std::ops::Range;

use ropey::{LineType, Rope};

use crate::assist::{Edit, selected_rows};

/// The selected rows as a byte range, including the final row's line break when it has one.
fn block(text: &Rope, selection: &Range<usize>) -> (usize, usize, Range<usize>) {
    let (first, last) = selected_rows(text, selection);
    let start = text.line_to_byte_idx(first, LineType::LF);
    let end = text.line_to_byte_idx(last + 1, LineType::LF);
    (first, last, start..end)
}

fn shifted(selection: &Range<usize>, by: isize) -> Range<usize> {
    selection.start.saturating_add_signed(by)..selection.end.saturating_add_signed(by)
}

/// Copy the selected lines below themselves; the selection moves to the copy.
pub fn duplicate_lines(text: &Rope, selection: Range<usize>) -> Edit {
    let (_, _, range) = block(text, &selection);
    let lines = text.slice(range.clone()).to_string();
    let copy = if lines.ends_with('\n') {
        lines
    } else {
        format!("\n{lines}") // the last line of the text has no line break to copy
    };
    let by = copy.len() as isize;
    Edit {
        range: range.end..range.end,
        text: copy,
        select: shifted(&selection, by),
    }
}

/// Delete the selected lines; the caret keeps its column on the line that takes their place.
pub fn delete_lines(text: &Rope, selection: Range<usize>) -> Edit {
    let (first, last, mut range) = block(text, &selection);
    // The column in characters, so the caret lands on a character of the other line (found by
    // the property tests: a byte column fell inside `日`).
    let column = text.slice(range.start..selection.start).chars().count();
    let at_column = |row: usize| {
        let line = text.line(row, LineType::LF);
        let chars = line.chars().take_while(|&c| c != '\n').take(column);
        chars.map(char::len_utf8).sum::<usize>()
    };
    let rows = text.len_lines(LineType::LF);
    let caret = if last + 1 < rows && text.slice(range.clone()).chars().last() == Some('\n') {
        range.start + at_column(last + 1)
    } else if first > 0 {
        // Deleting the last lines: also remove the break before them; land on the line above.
        range.start -= 1;
        let above = text.line_to_byte_idx(first - 1, LineType::LF);
        above + at_column(first - 1)
    } else {
        0
    };
    Edit {
        range,
        text: String::new(),
        select: caret..caret,
    }
}

/// Swap the selected lines with the line above (`up`) or below. `None` at the edge of the text.
pub fn move_lines(text: &Rope, selection: Range<usize>, up: bool) -> Option<Edit> {
    let (first, last, range) = block(text, &selection);
    let rows = text.len_lines(LineType::LF);
    let lines = text.slice(range.clone()).to_string();
    let (other, edit_range) = if up {
        let above = text.line_to_byte_idx(first.checked_sub(1)?, LineType::LF);
        (above..range.start, above..range.end)
    } else {
        if last + 1 >= rows || range.end == text.len() && !lines.ends_with('\n') {
            return None;
        }
        let below = text.line_to_byte_idx(last + 2, LineType::LF);
        (range.end..below, range.start..below)
    };
    let other_text = text.slice(other).to_string();
    // The text's last line has no break: the break moves to whichever line is now above it.
    let (first_part, second_part) = if up {
        (lines.as_str(), other_text.as_str())
    } else {
        (other_text.as_str(), lines.as_str())
    };
    let joined = if first_part.ends_with('\n') {
        format!("{first_part}{second_part}")
    } else {
        format!(
            "{first_part}\n{}",
            second_part.strip_suffix('\n').unwrap_or(second_part)
        )
    };
    let moved_start = if up {
        edit_range.start
    } else {
        edit_range.start + other_text.len() + usize::from(!other_text.ends_with('\n'))
    };
    let by = moved_start as isize - range.start as isize;
    // A selection that took the moved block's final line break along loses it with the break
    // (found by the property tests: it ended one past the text).
    let end = edit_range.start + joined.len();
    let select = shifted(&selection, by);
    let select = select.start.min(end)..select.end.min(end);
    Some(Edit {
        range: edit_range,
        text: joined,
        select,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `|` marks a caret, `[`…`]` a selection; the result is rendered the same way.
    fn run(marked: &str, command: impl Fn(&Rope, Range<usize>) -> Option<Edit>) -> Option<String> {
        let (text, selection) = match marked.find('|') {
            Some(caret) => (marked.replace('|', ""), caret..caret),
            None => {
                let start = marked.find('[').unwrap();
                let end = marked.find(']').unwrap() - 1;
                (marked.replacen('[', "", 1).replacen(']', "", 1), start..end)
            }
        };
        let edit = command(&Rope::from_str(&text), selection)?;
        let mut out = text.clone();
        out.replace_range(edit.range.clone(), &edit.text);
        if edit.select.is_empty() {
            out.insert(edit.select.start, '|');
        } else {
            out.insert(edit.select.end, ']');
            out.insert(edit.select.start, '[');
        }
        Some(out)
    }

    fn duplicate(marked: &str) -> String {
        run(marked, |t, s| Some(duplicate_lines(t, s))).unwrap()
    }

    fn delete(marked: &str) -> String {
        run(marked, |t, s| Some(delete_lines(t, s))).unwrap()
    }

    fn up(marked: &str) -> Option<String> {
        run(marked, |t, s| move_lines(t, s, true))
    }

    fn down(marked: &str) -> Option<String> {
        run(marked, |t, s| move_lines(t, s, false))
    }

    #[test]
    fn deleting_keeps_the_column_in_characters() {
        // Found by the property tests: the caret fell inside a multi-byte character.
        let text = Rope::from_str("abc\n日本語\n");
        let edit = delete_lines(&text, 2..2);
        assert_eq!(edit.select, 6..6); // after `日本`, column 2
    }

    #[test]
    fn selection_with_the_final_break_stays_inside() {
        // Found by the property tests: the selection ended past the text.
        let text = Rope::from_str("a\nb\n");
        let edit = move_lines(&text, 0..4, false).unwrap();
        assert_eq!(edit.text, "\na\nb");
        assert_eq!(edit.select, 1..4);
    }

    #[test]
    fn duplicates() {
        assert_eq!(duplicate("a|b\nc"), "ab\na|b\nc");
        assert_eq!(duplicate("a\nb|"), "a\nb\nb|");
        assert_eq!(duplicate("[a\nb]\nc"), "a\nb\n[a\nb]\nc");
        // A selection ending at the start of a line does not take that line.
        assert_eq!(duplicate("[a\n]b"), "a\n[a\n]b");
    }

    #[test]
    fn deletes() {
        assert_eq!(delete("a\nb|c\nd"), "a\nd|");
        assert_eq!(delete("ab|c\ndefg"), "de|fg");
        assert_eq!(delete("a\nbc|"), "a|");
        assert_eq!(delete("on|ly"), "|");
        assert_eq!(delete("[a\nb]\nc"), "|c");
    }

    #[test]
    fn moves() {
        assert_eq!(up("a\nb|\nc").as_deref(), Some("b|\na\nc"));
        assert_eq!(down("a|\nb\nc").as_deref(), Some("b\na|\nc"));
        assert_eq!(up("a\nb|").as_deref(), Some("b|\na"));
        assert_eq!(down("a\nb|\nc").as_deref(), Some("a\nc\nb|"));
        assert_eq!(down("[a\nb]\nc").as_deref(), Some("c\n[a\nb]"));
        assert_eq!(up("a|\nb"), None);
        assert_eq!(down("a\nb|"), None);
        // Moving a line down past the empty line after a final line break.
        assert_eq!(down("a\nb|\n").as_deref(), Some("a\n\nb|"));
    }
}
