//! Editing assists (PLAN §4.3): Markdown-aware Enter and Tab, formatting toggles, headings, links
//! and tasks.
//!
//! Each assist is a pure function from the current text and selection to one [`Edit`], so the UI
//! applies it as a single undo step and tests need no editor.

use std::borrow::Cow;
use std::ops::Range;

use ropey::{LineType, Rope};

/// Replace `range` with `text`, then put the cursor (or selection) at `select`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    /// Selection after the edit, in document bytes (empty = a caret).
    pub select: Range<usize>,
}

/// The container prefix of a line: indentation, blockquote markers, a list marker, a task box.
#[derive(Debug, PartialEq, Eq)]
struct Prefix<'a> {
    /// Indentation and `> ` markers, copied as-is to a continued line.
    lead: &'a str,
    marker: Option<Marker>,
    /// Space between the marker and the content (kept on continuation).
    gap: &'a str,
    task: bool,
    /// Byte length of the whole prefix.
    len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marker {
    Bullet(char),
    Ordered { number: u64, delimiter: char },
}

fn parse_prefix(line: &str) -> Prefix<'_> {
    let bytes = line.as_bytes();
    let mut pos = 0;
    loop {
        while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t') {
            pos += 1;
        }
        if bytes.get(pos) == Some(&b'>') {
            pos += 1;
            if bytes.get(pos) == Some(&b' ') {
                pos += 1;
            }
        } else {
            break;
        }
    }
    let lead_end = pos;
    let rest = &line[pos..];
    let marker = match rest.as_bytes().first() {
        Some(&c @ (b'-' | b'*' | b'+')) if matches!(rest.as_bytes().get(1), Some(b' ' | b'\t')) => {
            pos += 1;
            Some(Marker::Bullet(c as char))
        }
        Some(b'0'..=b'9') => {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            match (rest.as_bytes().get(digits), rest.as_bytes().get(digits + 1)) {
                (Some(&d @ (b'.' | b')')), Some(b' ' | b'\t')) if digits <= 9 => {
                    pos += digits + 1;
                    rest[..digits].parse().ok().map(|number| Marker::Ordered {
                        number,
                        delimiter: d as char,
                    })
                }
                _ => None,
            }
        }
        _ => None,
    };
    let gap_start = pos;
    if marker.is_some() {
        while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t') {
            pos += 1;
        }
    }
    let gap = &line[gap_start..pos];
    let task_box = marker.is_some()
        && bytes.len() >= pos + 3
        && bytes[pos] == b'['
        && matches!(bytes[pos + 1], b' ' | b'x' | b'X')
        && bytes[pos + 2] == b']'
        && matches!(bytes.get(pos + 3), None | Some(b' '));
    if task_box {
        pos = (pos + 4).min(line.len());
    }
    Prefix {
        lead: &line[..lead_end],
        marker,
        gap,
        task: task_box,
        len: pos,
    }
}

fn line_at(text: &Rope, offset: usize) -> (usize, Range<usize>, Cow<'_, str>) {
    let row = text.byte_to_line_idx(offset, LineType::LF);
    let start = text.line_to_byte_idx(row, LineType::LF);
    let line = Cow::from(text.line(row, LineType::LF));
    let content_len = line.trim_end_matches(['\n', '\r']).len();
    (row, start..start + content_len, line)
}

/// Enter at `offset` inside a list item, task or quote: continue it on the next line (renumbering
/// the rest of an ordered list), or end it when the item is empty. `None` means "plain Enter".
pub fn on_enter(text: &Rope, offset: usize) -> Option<Edit> {
    let (row, line_range, line) = line_at(text, offset);
    let line = &line[..line_range.len()];
    let prefix = parse_prefix(line);
    let column = offset - line_range.start;
    if (prefix.marker.is_none() && prefix.lead.trim().is_empty()) || column < prefix.len {
        return None;
    }
    if line[prefix.len..].trim().is_empty() {
        // An empty item ends the list (or quote): drop the marker and leave a blank line, so the
        // next paragraph is not read as a lazy continuation of the last item.
        let caret = line_range.start + 1;
        return Some(Edit {
            range: line_range.clone(),
            text: "\n".to_string(),
            select: caret..caret,
        });
    }
    let marker = match prefix.marker {
        Some(Marker::Bullet(c)) => c.to_string(),
        Some(Marker::Ordered { number, delimiter }) => format!("{}{delimiter}", number + 1),
        None => String::new(),
    };
    let task = if prefix.task { "[ ] " } else { "" };
    let continuation = format!("\n{}{marker}{}{task}", prefix.lead, prefix.gap);
    let caret = offset + continuation.len();

    let Some(Marker::Ordered { number, delimiter }) = prefix.marker else {
        return Some(Edit {
            range: offset..offset,
            text: continuation,
            select: caret..caret,
        });
    };
    // Renumber the following items of the same list in the same edit (one undo step).
    let mut renumbered = String::new();
    let mut end = offset;
    let mut expected = number + 1;
    let mut next_row = row + 1;
    let rows = text.len_lines(LineType::LF);
    let tail = &line[column..];
    while next_row < rows {
        let start = text.line_to_byte_idx(next_row, LineType::LF);
        let raw = Cow::from(text.line(next_row, LineType::LF));
        let content = raw.trim_end_matches(['\n', '\r']);
        let next = parse_prefix(content);
        match next.marker {
            Some(Marker::Ordered {
                number: n,
                delimiter: d,
            }) if next.lead == prefix.lead && d == delimiter && n == expected => {
                renumbered.push('\n');
                renumbered.push_str(&format!(
                    "{}{}{d}{}",
                    next.lead,
                    n + 1,
                    &content[next.lead.len() + n.to_string().len() + 1..]
                ));
                end = start + content.len();
                expected += 1;
                next_row += 1;
            }
            _ => break,
        }
    }
    if renumbered.is_empty() {
        return Some(Edit {
            range: offset..offset,
            text: continuation,
            select: caret..caret,
        });
    }
    Some(Edit {
        range: offset..end,
        text: format!("{continuation}{tail}{renumbered}"),
        select: caret..caret,
    })
}

/// Toggle a symmetric inline marker (`**`, `*`, `` ` ``, `~~`) around the selection. An empty
/// selection inserts the pair with the caret between.
pub fn toggle_wrap(text: &Rope, selection: Range<usize>, marker: &str) -> Edit {
    let m = marker.len();
    let selected = text.slice(selection.clone()).to_string();
    // The `m` bytes next to the selection, when they are whole characters (a multi-byte
    // character there is not a marker).
    let around = |range: Range<usize>| {
        (range.end <= text.len()
            && text.is_char_boundary(range.start)
            && text.is_char_boundary(range.end))
        .then(|| text.slice(range).to_string())
    };
    let before = selection
        .start
        .checked_sub(m)
        .and_then(|start| around(start..selection.start));
    let after = around(selection.end..selection.end + m);
    // A single `*` must not be taken for half of `**` (and vice versa).
    let clean_edge = |outer: Option<usize>| {
        outer.is_none_or(|i| marker != "*" || around(i..i + 1).as_deref() != Some("*"))
    };
    if before.as_deref() == Some(marker)
        && after.as_deref() == Some(marker)
        && clean_edge(selection.start.checked_sub(m + 1))
        && clean_edge((selection.end + m < text.len()).then_some(selection.end + m))
    {
        // **[sel]** → [sel]
        let start = selection.start - m;
        return Edit {
            range: start..selection.end + m,
            text: selected,
            select: start..selection.end - m,
        };
    }
    if selected.len() >= 2 * m && selected.starts_with(marker) && selected.ends_with(marker) {
        // [**sel**] → [sel]
        let inner = selected[m..selected.len() - m].to_string();
        let end = selection.start + inner.len();
        return Edit {
            range: selection.clone(),
            text: inner,
            select: selection.start..end,
        };
    }
    let start = selection.start + m;
    Edit {
        range: selection.clone(),
        text: format!("{marker}{selected}{marker}"),
        select: start..start + selected.len(),
    }
}

/// Insert a link around the selection. A selected URL becomes the destination (caret in the text
/// part); other text becomes the link text (caret in the destination).
pub fn insert_link(text: &Rope, selection: Range<usize>) -> Edit {
    let selected = text.slice(selection.clone()).to_string();
    let is_url = selected.contains("://") || selected.starts_with("mailto:");
    if is_url {
        let caret = selection.start + 1;
        return Edit {
            range: selection,
            text: format!("[]({selected})"),
            select: caret..caret,
        };
    }
    let caret = selection.start + selected.len() + 3;
    Edit {
        range: selection,
        text: format!("[{selected}]()"),
        select: caret..caret,
    }
}

/// Pasting a URL over selected text makes a link of it: `[selection](url)`. `None` means "paste
/// normally" (no selection, a multi-line selection, or the clipboard is not a single URL).
pub fn paste_link(text: &Rope, selection: Range<usize>, clipboard: &str) -> Option<Edit> {
    let url = clipboard.trim();
    let is_url =
        (url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:"))
            && !url.contains(char::is_whitespace);
    let selected = text.slice(selection.clone()).to_string();
    if !is_url || selected.is_empty() || selected.contains('\n') {
        return None;
    }
    let link = format!("[{selected}]({url})");
    let caret = selection.start + link.len();
    Some(Edit {
        range: selection,
        text: link,
        select: caret..caret,
    })
}

/// Toggle a task on the line at `offset`: `- [ ]` ↔ `- [x]`; a list item gets a box; a plain line
/// becomes an open task.
pub fn toggle_task(text: &Rope, offset: usize) -> Edit {
    let (_, line_range, line) = line_at(text, offset);
    let line = &line[..line_range.len()];
    let prefix = parse_prefix(line);
    let keep_caret = |delta: isize| {
        let caret = offset.saturating_add_signed(delta).max(line_range.start);
        caret..caret
    };
    if prefix.task {
        let box_at = line_range.start + prefix.len - 4 + 1; // the character inside [ ]
        let checked = line.as_bytes()[prefix.len - 3] != b' ';
        let mark = if checked { " " } else { "x" };
        return Edit {
            range: box_at..box_at + 1,
            text: mark.to_string(),
            select: keep_caret(0),
        };
    }
    if prefix.marker.is_some() {
        let at = line_range.start + prefix.len;
        return Edit {
            range: at..at,
            text: "[ ] ".to_string(),
            select: keep_caret(4),
        };
    }
    let at = line_range.start + prefix.lead.len();
    Edit {
        range: at..at,
        text: "- [ ] ".to_string(),
        select: keep_caret(6),
    }
}

/// Rows touched by `selection`. A selection ending at the start of a row does not include that
/// row (selecting whole lines by dragging to the next line's start is the common case).
pub(crate) fn selected_rows(text: &Rope, selection: &Range<usize>) -> (usize, usize) {
    let first = text.byte_to_line_idx(selection.start, LineType::LF);
    let mut last = text.byte_to_line_idx(selection.end, LineType::LF);
    if last > first && text.line_to_byte_idx(last, LineType::LF) == selection.end {
        last -= 1;
    }
    (first, last)
}

/// Length of the blockquote part (`> > `) at the start of a prefix's `lead`.
fn quote_len(lead: &str) -> usize {
    lead.rfind('>')
        .map_or(0, |i| i + 1 + usize::from(lead[i + 1..].starts_with(' ')))
}

/// A list item's position: its indentation inside any quote, and where its content starts.
struct Item {
    quote: usize,
    indent: usize,
    content: usize,
}

fn list_item(line: &str) -> Option<Item> {
    let prefix = parse_prefix(line);
    prefix.marker?;
    let quote = quote_len(prefix.lead);
    let task = if prefix.task { 4 } else { 0 };
    Some(Item {
        quote,
        indent: prefix.lead.len() - quote,
        content: prefix.len - task - quote,
    })
}

/// Tab / Shift+Tab on list items: indent the item under its previous sibling (to that sibling's
/// content column, as CommonMark nesting requires) or outdent it to its parent's level. Every
/// line the selection touches moves by the same amount, so nested items and continuation lines
/// keep their shape. `None` when the first line is not a list item, or a top-level item is
/// outdented: the editor's own indent applies.
pub fn indent_list(text: &Rope, selection: Range<usize>, outdent: bool) -> Option<Edit> {
    let (first, last) = selected_rows(text, &selection);
    let line = |row: usize| {
        let raw = Cow::from(text.line(row, LineType::LF));
        raw.trim_end_matches(['\n', '\r']).to_string()
    };
    let item = list_item(&line(first))?;
    // Nearest earlier item of the same list: the sibling (same indent) when indenting, the parent
    // (smaller indent) when outdenting. Blank and indented continuation lines are skipped; a
    // top-level paragraph or another quote depth ends the list.
    let mut target = None;
    for row in (0..first).rev() {
        let previous = line(row);
        if previous.trim().is_empty() {
            continue;
        }
        let Some(other) = list_item(&previous) else {
            let lead = parse_prefix(&previous).lead;
            if lead.len() > quote_len(lead) {
                continue;
            }
            break;
        };
        if other.quote != item.quote {
            break;
        }
        if other.indent < item.indent || (!outdent && other.indent == item.indent) {
            target = Some(other);
            break;
        }
    }
    let delta: isize = if outdent {
        if item.indent == 0 {
            return None;
        }
        let parent = target.map_or(0, |parent| parent.indent);
        -((item.indent - parent) as isize)
    } else {
        match target {
            Some(sibling) if sibling.indent == item.indent => {
                (sibling.content - item.indent) as isize
            }
            // The first item of a list (or of a nested list): indent by its own marker width.
            _ => (item.content - item.indent) as isize,
        }
    };

    let start = text.line_to_byte_idx(first, LineType::LF);
    let mut out = String::new();
    let mut changes = Vec::new(); // (row start, change position, bytes added (+) or removed (−))
    let mut old_end = start;
    for row in first..=last {
        let row_start = text.line_to_byte_idx(row, LineType::LF);
        let content = line(row);
        let at = quote_len(parse_prefix(&content).lead);
        let change = if content.trim().is_empty() {
            0
        } else if delta > 0 {
            delta
        } else {
            let spaces = content[at..].bytes().take_while(|b| *b == b' ').count();
            -(spaces.min(delta.unsigned_abs()) as isize)
        };
        out.push_str(&content[..at]);
        out.extend(std::iter::repeat_n(' ', change.max(0) as usize));
        out.push_str(&content[at + (-change).max(0) as usize..]);
        if row < last {
            out.push('\n');
        }
        changes.push((row_start, row_start + at, change));
        old_end = row_start + content.len();
    }
    // Where an old offset ends up: text after a change point moves with it; an offset inside
    // removed spaces lands where they were.
    let moved = |offset: usize| {
        let mut shift = 0isize;
        for &(row_start, at, change) in &changes {
            if offset < row_start {
                break;
            }
            let removed = (-change).max(0) as usize;
            if offset >= at + removed {
                shift += change;
            } else if offset >= at {
                return at.saturating_add_signed(shift);
            }
        }
        offset.saturating_add_signed(shift)
    };
    Some(Edit {
        range: start..old_end,
        text: out,
        select: moved(selection.start)..moved(selection.end),
    })
}

/// Change the heading level of the line at `offset` by `delta`: a paragraph is level 0, so +1
/// makes it `# `, and −1 on `# ` makes it a paragraph again. Levels stay within 0–6.
pub fn shift_heading(text: &Rope, offset: usize, delta: i8) -> Edit {
    let (_, line_range, line) = line_at(text, offset);
    let line = &line[..line_range.len()];
    let at = parse_prefix(line).len;
    let rest = &line[at..];
    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
    let is_heading = (1..=6).contains(&hashes)
        && matches!(rest.as_bytes().get(hashes), None | Some(b' ' | b'\t'));
    let (level, old_len) = if is_heading {
        (hashes as i8, hashes + usize::from(rest.len() > hashes))
    } else {
        (0, 0)
    };
    let new_level = (level + delta).clamp(0, 6) as usize;
    let marker = if new_level == 0 {
        String::new()
    } else {
        format!("{} ", "#".repeat(new_level))
    };
    let from = line_range.start + at;
    let caret = if offset >= from + old_len {
        offset - old_len + marker.len()
    } else {
        from + marker.len()
    };
    Edit {
        range: from..from + old_len,
        text: marker,
        select: caret..caret,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply an edit and render the caret/selection as `|` or `[…]` for readable assertions.
    fn show(text: &str, edit: &Edit) -> String {
        let mut out = text.to_string();
        out.replace_range(edit.range.clone(), &edit.text);
        if edit.select.is_empty() {
            out.insert(edit.select.start, '|');
        } else {
            out.insert(edit.select.end, ']');
            out.insert(edit.select.start, '[');
        }
        out
    }

    fn enter(text_with_caret: &str) -> Option<String> {
        let offset = text_with_caret.find('|').unwrap();
        let text = text_with_caret.replace('|', "");
        on_enter(&Rope::from_str(&text), offset).map(|edit| show(&text, &edit))
    }

    #[test]
    fn markers_next_to_multibyte_characters() {
        // Found by the property tests: a selection right after `ö` crashed Ctrl+B.
        let text = Rope::from_str("ö x ş");
        // `ö` is two bytes: the two bytes before `x` start inside it.
        assert_eq!(toggle_wrap(&text, 3..4, "**").text, "**x**");
        assert_eq!(toggle_wrap(&text, 3..4, "*").text, "*x*");
    }

    #[test]
    fn continues_lists_tasks_and_quotes() {
        assert_eq!(enter("- one|").as_deref(), Some("- one\n- |"));
        assert_eq!(enter("  * nested|").as_deref(), Some("  * nested\n  * |"));
        assert_eq!(enter("- [x] done|").as_deref(), Some("- [x] done\n- [ ] |"));
        assert_eq!(enter("> quoted|").as_deref(), Some("> quoted\n> |"));
        assert_eq!(
            enter("> - in quote|").as_deref(),
            Some("> - in quote\n> - |")
        );
        assert_eq!(enter("9) nine|").as_deref(), Some("9) nine\n10) |"));
    }

    #[test]
    fn splits_an_item_at_the_caret() {
        assert_eq!(enter("- ab|cd").as_deref(), Some("- ab\n- |cd"));
    }

    #[test]
    fn empty_item_ends_the_list() {
        assert_eq!(enter("- a\n- |").as_deref(), Some("- a\n\n|"));
        assert_eq!(enter("- a\n- [ ] |").as_deref(), Some("- a\n\n|"));
        assert_eq!(enter(">|").as_deref(), Some("\n|"));
    }

    #[test]
    fn plain_lines_and_carets_in_the_marker_use_plain_enter() {
        assert_eq!(enter("text|"), None);
        assert_eq!(enter("|- item"), None);
        assert_eq!(enter("-| item"), None);
        assert_eq!(enter("-not a list|"), None);
    }

    #[test]
    fn ordered_lists_renumber() {
        assert_eq!(
            enter("1. a|\n2. b\n3. c\n\ntext").as_deref(),
            Some("1. a\n2. |\n3. b\n4. c\n\ntext")
        );
        // Only the same list (same indentation and delimiter) is renumbered.
        assert_eq!(
            enter("1. a|\n  2. nested").as_deref(),
            Some("1. a\n2. |\n  2. nested")
        );
    }

    fn wrap(text_with_selection: &str, marker: &str) -> String {
        let start = text_with_selection.find('[').unwrap();
        let end = text_with_selection.find(']').unwrap() - 1;
        let text = text_with_selection.replace(['[', ']'], "");
        show(
            &text,
            &toggle_wrap(&Rope::from_str(&text), start..end, marker),
        )
    }

    #[test]
    fn formatting_toggles() {
        assert_eq!(wrap("a [word] b", "**"), "a **[word]** b");
        assert_eq!(wrap("a **[word]** b", "**"), "a [word] b");
        assert_eq!(wrap("a [**word**] b", "**"), "a [word] b");
        assert_eq!(wrap("x [] y", "`"), "x `|` y");
        // Italic inside bold is added, not mistaken for removing half of the bold.
        assert_eq!(wrap("**[w]**", "*"), "***[w]***");
    }

    #[test]
    fn links() {
        let text = Rope::from_str("see docs here");
        assert_eq!(
            show("see docs here", &insert_link(&text, 4..8)),
            "see [docs](|) here"
        );
        let url = Rope::from_str("https://e.com");
        assert_eq!(
            show("https://e.com", &insert_link(&url, 0..13)),
            "[|](https://e.com)"
        );
    }

    #[test]
    fn pasting_a_url_over_text_links_it() {
        let text = Rope::from_str("read the docs now");
        let edit = paste_link(&text, 9..13, " https://e.com/x \n").unwrap();
        assert_eq!(
            show("read the docs now", &edit),
            "read the [docs](https://e.com/x)| now"
        );
        assert_eq!(paste_link(&text, 9..9, "https://e.com"), None); // nothing selected
        assert_eq!(paste_link(&text, 9..13, "not a url"), None);
        assert_eq!(paste_link(&text, 9..13, "https://a b"), None);
    }

    #[test]
    fn task_toggles() {
        let toggle =
            |text: &str, offset: usize| show(text, &toggle_task(&Rope::from_str(text), offset));
        assert_eq!(toggle("- [ ] job", 7), "- [x] j|ob");
        assert_eq!(toggle("- [x] job", 7), "- [ ] j|ob");
        assert_eq!(toggle("- job", 3), "- [ ] j|ob");
        assert_eq!(toggle("job", 1), "- [ ] j|ob");
        assert_eq!(toggle("> 1. job", 6), "> 1. [ ] j|ob");
    }

    /// `|` marks a caret, `[`…`]` a selection.
    fn parse_marked(marked: &str) -> (String, Range<usize>) {
        if let Some(caret) = marked.find('|') {
            return (marked.replace('|', ""), caret..caret);
        }
        let start = marked.find('[').unwrap();
        let end = marked.find(']').unwrap() - 1;
        (marked.replacen('[', "", 1).replacen(']', "", 1), start..end)
    }

    fn tab(marked: &str, outdent: bool) -> Option<String> {
        let (text, selection) = parse_marked(marked);
        indent_list(&Rope::from_str(&text), selection, outdent).map(|edit| show(&text, &edit))
    }

    #[test]
    fn tab_nests_items_under_their_sibling() {
        assert_eq!(tab("- a\n- b|", false).as_deref(), Some("- a\n  - b|"));
        assert_eq!(tab("1. a\n1. b|", false).as_deref(), Some("1. a\n   1. b|"));
        assert_eq!(
            tab("- [ ] a\n- [ ] b|", false).as_deref(),
            Some("- [ ] a\n  - [ ] b|")
        );
        assert_eq!(tab("- a\n\n- b|", false).as_deref(), Some("- a\n\n  - b|"));
        assert_eq!(
            tab("> - a\n> - b|", false).as_deref(),
            Some("> - a\n>   - b|")
        );
        // The first item has no sibling: it moves by its own marker width.
        assert_eq!(tab("- a|", false).as_deref(), Some("  - a|"));
    }

    #[test]
    fn tab_moves_every_selected_line() {
        assert_eq!(
            tab("- a\n[- b\n  - c]", false).as_deref(),
            Some("- a\n  [- b\n    - c]")
        );
        assert_eq!(
            tab("- a\n  [- b\n    text\n    - c]", true).as_deref(),
            Some("- a\n[- b\n  text\n  - c]")
        );
    }

    #[test]
    fn shift_tab_outdents_to_the_parent() {
        assert_eq!(tab("- a\n  - b|", true).as_deref(), Some("- a\n- b|"));
        assert_eq!(
            tab("- a\n  - b\n    - c|", true).as_deref(),
            Some("- a\n  - b\n  - c|")
        );
        assert_eq!(tab("- a|", true), None); // already top level
    }

    #[test]
    fn tab_outside_lists_is_the_editors() {
        assert_eq!(tab("text|", false), None);
        assert_eq!(tab("    code|", true), None);
    }

    fn heading(marked: &str, delta: i8) -> String {
        let (text, selection) = parse_marked(marked);
        show(
            &text,
            &shift_heading(&Rope::from_str(&text), selection.start, delta),
        )
    }

    #[test]
    fn heading_levels() {
        assert_eq!(heading("Title|", 1), "# Title|");
        assert_eq!(heading("# Title|", -1), "Title|");
        assert_eq!(heading("## T|", 1), "### T|");
        assert_eq!(heading("###### T|", 1), "###### T|");
        assert_eq!(heading("Ti|tle", -1), "Ti|tle");
        assert_eq!(heading("> ## Q|", -1), "> # Q|");
        assert_eq!(heading("#tag|", 1), "# #tag|"); // not a heading: no space after #
        assert_eq!(heading("#|# T", 1), "### |T"); // caret inside the marker
        assert_eq!(heading("a\n## b|\nc", -1), "a\n# b|\nc");
    }
}
