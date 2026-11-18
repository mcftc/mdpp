//! Table assist (PLAN §4.3, v1.0): in a GFM table, Tab and Shift+Tab move between cells and
//! align the columns, Tab past the last cell adds a row, Enter in a row adds a row below (on an
//! empty row it leaves the table). Each is one [`Edit`] of the whole table: one undo step.

use std::ops::Range;

use ropey::{LineType, Rope};
use unicode_width::UnicodeWidthStr;

use crate::assist::Edit;

/// Narrowest column: the delimiter row needs three dashes.
const MIN_WIDTH: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
    None,
    Left,
    Center,
    Right,
}

/// A table found around the caret.
struct Table {
    /// Bytes of the document from the first row's start to the last row's end (no newline).
    range: Range<usize>,
    /// Indentation and `>` markers of every row, kept as-is.
    lead: String,
    /// Cells of each row (the delimiter row excluded), trimmed.
    rows: Vec<Vec<String>>,
    aligns: Vec<Align>,
    /// Caret: row (in `rows`) and cell.
    at: (usize, usize),
}

/// Tab (`backwards`: Shift+Tab) in a table: align the table and select the next (previous)
/// cell's text; Tab in the last cell adds a row. `None` outside tables.
pub fn on_tab(text: &Rope, selection: Range<usize>, backwards: bool) -> Option<Edit> {
    let mut table = find(text, selection.start)?;
    let columns = table.aligns.len();
    let (row, cell) = table.at;
    let target = if backwards {
        match (row, cell) {
            (0, 0) => (0, 0),
            (_, 0) => (row - 1, columns - 1),
            _ => (row, cell - 1),
        }
    } else if cell + 1 < columns {
        (row, cell + 1)
    } else {
        if row + 1 == table.rows.len() {
            table.rows.push(vec![String::new(); columns]);
        }
        (row + 1, 0)
    };
    Some(table.render(target))
}

/// Enter in a table row: a new row below it; on an empty row, leave the table (the row goes,
/// a blank line follows the table). `None` outside tables and on the header or delimiter row.
pub fn on_enter(text: &Rope, offset: usize) -> Option<Edit> {
    let mut table = find(text, offset)?;
    let (row, _) = table.at;
    if row == 0 {
        return None;
    }
    let columns = table.aligns.len();
    if table.rows[row].iter().all(String::is_empty) {
        table.rows.remove(row);
        let mut edit = table.render((0, 0));
        edit.text.push('\n');
        let end = table.range.start + edit.text.len();
        // The caret goes on the blank line after the table.
        edit.select = end..end;
        return Some(edit);
    }
    table.rows.insert(row + 1, vec![String::new(); columns]);
    Some(table.render((row + 1, 0)))
}

/// Align the table around `offset`, keeping the caret's cell. `None` outside tables.
pub fn format(text: &Rope, offset: usize) -> Option<Edit> {
    let table = find(text, offset)?;
    let at = table.at;
    Some(table.render(at))
}

/// The table containing byte `offset`: rows are consecutive lines with a `|`, and the second
/// one must be a delimiter row (`| --- | :-: |`).
fn find(text: &Rope, offset: usize) -> Option<Table> {
    let lines = text.len_lines(LineType::LF);
    let row_of = |i: usize| -> String {
        let line = text.line(i, LineType::LF).to_string();
        line.trim_end_matches(['\n', '\r']).to_string()
    };
    let caret_line = text.byte_to_line_idx(offset.min(text.len()), LineType::LF);
    let is_row = |line: &str| line.contains('|') && !line.trim().is_empty();
    if !is_row(&row_of(caret_line)) {
        return None;
    }
    let mut first = caret_line;
    while first > 0 && is_row(&row_of(first - 1)) {
        first -= 1;
    }
    let mut last = caret_line;
    while last + 1 < lines && is_row(&row_of(last + 1)) {
        last += 1;
    }
    let raw: Vec<String> = (first..=last).map(row_of).collect();
    let lead = lead_of(&raw[0]);
    let bodies: Vec<&str> = raw
        .iter()
        .map(|line| line.strip_prefix(lead.as_str()))
        .collect::<Option<_>>()?;
    let aligns = parse_delimiter(bodies.get(1)?)?;
    let caret_in_line = offset - text.line_to_byte_idx(caret_line, LineType::LF);
    let caret_row = caret_line - first;
    if caret_row == 1 {
        return None; // the delimiter row: nothing to edit there
    }
    let mut rows: Vec<Vec<String>> = bodies
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, body)| split_cells(body))
        .collect();
    let columns = aligns
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    for row in &mut rows {
        row.resize(columns, String::new());
    }
    let mut aligns = aligns;
    aligns.resize(columns, Align::None);
    let body = bodies[caret_row];
    let cell = cell_at(body, caret_in_line.saturating_sub(lead.len())).min(columns - 1);
    let row = if caret_row == 0 { 0 } else { caret_row - 1 };
    let start = text.line_to_byte_idx(first, LineType::LF);
    let end = text.line_to_byte_idx(last, LineType::LF) + raw[raw.len() - 1].len();
    Some(Table {
        range: start..end,
        lead,
        rows,
        aligns,
        at: (row, cell),
    })
}

/// How many cells a table row has as written (indentation and `>` markers ignored).
pub(crate) fn cell_count(line: &str) -> usize {
    split_cells(&line[lead_of(line).len()..]).len()
}

/// Indentation and blockquote markers at the start of a row.
fn lead_of(line: &str) -> String {
    let end = line
        .char_indices()
        .find(|&(_, c)| !matches!(c, ' ' | '\t' | '>'))
        .map_or(line.len(), |(i, _)| i);
    line[..end].to_string()
}

/// Cell texts of a row: split at `|` not escaped by `\`, outer pipes optional.
fn split_cells(body: &str) -> Vec<String> {
    let trimmed = body.trim();
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = match inner.strip_suffix('|') {
        Some(rest) if !rest.ends_with('\\') => rest,
        _ => inner,
    };
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for c in inner.chars() {
        if c == '|' && !escaped {
            cells.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(c);
        }
        escaped = c == '\\' && !escaped;
    }
    cells.push(cell.trim().to_string());
    cells
}

/// Which cell byte `at` of a row is in.
fn cell_at(body: &str, at: usize) -> usize {
    let leading = body.trim_start().starts_with('|');
    let mut escaped = false;
    let mut pipes: usize = 0;
    for (i, c) in body.char_indices() {
        if i >= at {
            break;
        }
        if c == '|' && !escaped {
            pipes += 1;
        }
        escaped = c == '\\' && !escaped;
    }
    if leading {
        pipes.saturating_sub(1)
    } else {
        pipes
    }
}

fn parse_delimiter(body: &str) -> Option<Vec<Align>> {
    split_cells(body)
        .iter()
        .map(|cell| {
            let dashes = cell.trim_matches(':');
            if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
                return None;
            }
            Some(match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                (false, false) => Align::None,
            })
        })
        .collect()
}

impl Table {
    /// The aligned table as one edit, selecting the text of cell `target`.
    fn render(&self, target: (usize, usize)) -> Edit {
        let widths: Vec<usize> = (0..self.aligns.len())
            .map(|c| {
                self.rows
                    .iter()
                    .map(|row| row[c].width())
                    .max()
                    .unwrap_or(0)
                    .max(MIN_WIDTH)
            })
            .collect();
        let mut out = String::new();
        let mut select = 0..0;
        for (r, row) in self.rows.iter().enumerate() {
            if r == 1 {
                out.push_str(&self.delimiter_row(&widths));
                out.push('\n');
            }
            out.push_str(&self.lead);
            out.push('|');
            for (c, cell) in row.iter().enumerate() {
                let pad = widths[c] - cell.width();
                let (left, right) = match self.aligns[c] {
                    Align::Right => (pad, 0),
                    Align::Center => (pad / 2, pad - pad / 2),
                    Align::None | Align::Left => (0, pad),
                };
                out.push(' ');
                out.push_str(&" ".repeat(left));
                if (r, c) == target {
                    select = out.len()..out.len() + cell.len();
                }
                out.push_str(cell);
                out.push_str(&" ".repeat(right));
                out.push_str(" |");
            }
            if r + 1 < self.rows.len() || self.rows.len() == 1 {
                out.push('\n');
            }
        }
        if self.rows.len() == 1 {
            out.push_str(&self.delimiter_row(&widths));
        }
        let base = self.range.start;
        Edit {
            range: self.range.clone(),
            text: out,
            select: base + select.start..base + select.end,
        }
    }

    fn delimiter_row(&self, widths: &[usize]) -> String {
        let mut out = format!("{}|", self.lead);
        for (align, width) in self.aligns.iter().zip(widths) {
            let (left, right) = match align {
                Align::None => ("-", "-"),
                Align::Left => (":", "-"),
                Align::Right => ("-", ":"),
                Align::Center => (":", ":"),
            };
            out.push(' ');
            out.push_str(left);
            out.push_str(&"-".repeat(width - 2));
            out.push_str(right);
            out.push_str(" |");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply `edit` to `doc`, marking the selection with `[` and `]`.
    fn show(doc: &str, edit: &Edit) -> String {
        let mut out = doc.to_string();
        out.replace_range(edit.range.clone(), &edit.text);
        out.insert(edit.select.end, ']');
        out.insert(edit.select.start, '[');
        out
    }

    fn at(doc: &str) -> (Rope, usize) {
        let caret = doc.find('^').expect("caret marker");
        (Rope::from_str(&doc.replace('^', "")), caret)
    }

    #[test]
    fn tab_aligns_and_selects_the_next_cell() {
        let (text, caret) =
            at("intro\n| Name | Qty |\n|:--|--:|\n| apple^ | 3 |\n| fig | 12 |\nafter");
        let edit = on_tab(&text, caret..caret, false).unwrap();
        assert_eq!(
            show(&text.to_string(), &edit),
            "intro\n| Name  | Qty |\n| :---- | --: |\n| apple |   [3] |\n| fig   |  12 |\nafter"
        );
        let edit = on_tab(&text, caret..caret, true).unwrap();
        assert!(show(&text.to_string(), &edit).contains("| Name  | [Qty] |"));
    }

    #[test]
    fn tab_in_the_last_cell_adds_a_row() {
        let (text, caret) = at("| a | b |\n| --- | --- |\n| 1 | 2^ |");
        let edit = on_tab(&text, caret..caret, false).unwrap();
        assert_eq!(
            show(&text.to_string(), &edit),
            "| a   | b   |\n| --- | --- |\n| 1   | 2   |\n| []    |     |"
        );
    }

    #[test]
    fn enter_adds_a_row_and_leaves_on_an_empty_one() {
        let (text, caret) = at("| a | b |\n|---|---|\n| one^ | two |\ntext");
        let edit = on_enter(&text, caret).unwrap();
        assert_eq!(
            show(&text.to_string(), &edit),
            "| a   | b   |\n| --- | --- |\n| one | two |\n| []    |     |\ntext"
        );
        let (text, caret) = at("| a | b |\n|---|---|\n| one | two |\n|  ^ |  |");
        let edit = on_enter(&text, caret).unwrap();
        assert_eq!(
            show(&text.to_string(), &edit),
            "| a   | b   |\n| --- | --- |\n| one | two |\n[]"
        );
        // The header row keeps the editor's Enter.
        let (text, caret) = at("| a^ | b |\n|---|---|\n| 1 | 2 |");
        assert_eq!(on_enter(&text, caret), None);
    }

    #[test]
    fn escaped_pipes_wide_text_quotes_and_non_tables() {
        let (text, caret) = at("> | x \\| y | 日本 |\n> | - | - |\n> | ^1 | 2 |");
        let edit = format(&text, caret).unwrap();
        assert_eq!(
            show(&text.to_string(), &edit),
            "> | x \\| y | 日本 |\n> | ------ | ---- |\n> | [1]      | 2    |"
        );
        let (text, caret) = at("a | b^\nnot a delimiter\n");
        assert_eq!(on_tab(&text, caret..caret, false), None);
        let (text, caret) = at("plain^ text");
        assert_eq!(format(&text, caret), None);
    }
}
