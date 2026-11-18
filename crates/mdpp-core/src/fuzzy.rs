//! Fuzzy matching for the palette and path completion: the query's characters must appear in
//! order (ignoring case); matches that start words, run together or start early score higher.
//! Small on purpose — a few hundred candidates per keystroke, no index needed.

/// Score bonuses and penalties, in one place so the ranking can be tuned without hunting.
const WORD_START: i32 = 8;
const CONSECUTIVE: i32 = 5;
const FIRST_CHARACTER: i32 = 6;
const GAP: i32 = 1;
/// Characters after which a new word starts.
const SEPARATORS: &[char] = &[' ', '/', '\\', '_', '-', '.', ':', '›', '#'];

/// How well `query` matches `candidate`, or `None` when it does not. Higher is better; an empty
/// query matches everything with 0.
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    let mut query = query.chars().filter(|c| !c.is_whitespace()).peekable();
    let mut total = 0;
    let mut previous: Option<char> = None;
    let mut last_match: Option<usize> = None;
    for (index, c) in candidate.chars().enumerate() {
        let Some(&wanted) = query.peek() else {
            break;
        };
        if c.to_lowercase().eq(wanted.to_lowercase()) {
            let word_start = previous
                .is_none_or(|p| SEPARATORS.contains(&p) || (p.is_lowercase() && c.is_uppercase()));
            total += match last_match {
                Some(last) if last + 1 == index => CONSECUTIVE,
                Some(last) => -GAP * (index - last - 1).min(10) as i32,
                None => -GAP * index.min(10) as i32,
            };
            if word_start {
                total += WORD_START;
            }
            if index == 0 {
                total += FIRST_CHARACTER;
            }
            last_match = Some(index);
            query.next();
        }
        previous = Some(c);
    }
    query.peek().is_none().then_some(total)
}

/// `candidates` that match `query`, best first (ties keep their order), as indices.
pub fn rank<'a>(query: &str, candidates: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = candidates
        .into_iter()
        .enumerate()
        .filter_map(|(index, text)| score(query, text).map(|s| (s, index)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_in_order_ignoring_case() {
        assert!(score("tgw", "Toggle Word Wrap").is_some());
        assert!(score("wrap", "Toggle Word Wrap").is_some());
        assert!(score("wt", "Toggle Word Wrap").is_none());
        assert_eq!(score("", "anything"), Some(0));
        assert!(score("ı", "I").is_none()); // no locale folding: dotless i is its own letter
    }

    #[test]
    fn word_starts_and_runs_rank_first() {
        let names = ["Save All", "Select All", "Close Saved Tabs", "Save"];
        let ranked: Vec<&str> = rank("sa", names).into_iter().map(|i| names[i]).collect();
        assert_eq!(ranked[..2], ["Save All", "Save"]);
        let files = ["docs/notes/readme.md", "README.md", "src/reader.rs"];
        assert_eq!(files[rank("readme", files)[0]], "README.md");
        // Initials of words.
        let commands = ["Reopen Closed Tab", "Report a Bug", "Reset Zoom"];
        assert_eq!(commands[rank("rct", commands)[0]], "Reopen Closed Tab");
    }
}
