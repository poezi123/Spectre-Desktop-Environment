//! Ranking launcher results.
//!
//! A subsequence match with a score, the same shape every fuzzy finder uses.
//! Kept pure so the ranking can be pinned down by tests instead of by feel:
//! typing `fire` must put Firefox first, every time.

use crate::entry::Entry;

/// How well a query matches, higher is better.
pub type Score = i32;

/// Points for a character that continues the previous match.
const CONSECUTIVE: Score = 8;
/// Points for a character at the start of a word.
const WORD_START: Score = 12;
/// Points for the very first character of the name.
const PREFIX: Score = 20;
/// Penalty per character skipped between matches.
const GAP: Score = -1;
/// Multiplier applied when the match came from the keywords rather than the
/// name, so a name match always outranks a category match.
const KEYWORD_PENALTY: Score = 3;

/// Score `query` against `text`, or `None` if it is not a subsequence.
///
/// Matching is case-insensitive; an empty query matches everything at zero.
pub fn score(query: &str, text: &str) -> Option<Score> {
    if query.is_empty() {
        return Some(0);
    }
    let haystack: Vec<char> = text.chars().flat_map(|c| c.to_lowercase()).collect();
    let needle: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();

    let mut total: Score = 0;
    let mut cursor = 0usize;
    let mut previous_match: Option<usize> = None;

    for wanted in needle {
        let found = haystack[cursor..].iter().position(|&c| c == wanted)? + cursor;

        total += match previous_match {
            Some(previous) if found == previous + 1 => CONSECUTIVE,
            _ if found == 0 => PREFIX,
            _ if is_word_start(&haystack, found) => WORD_START,
            _ => 0,
        };
        if let Some(previous) = previous_match {
            total += GAP * (found - previous - 1) as Score;
        }

        previous_match = Some(found);
        cursor = found + 1;
    }

    // Shorter names win ties: "Files" should beat "File Roller" for "file".
    Some(total - (haystack.len() as Score / 8))
}

fn is_word_start(haystack: &[char], index: usize) -> bool {
    index == 0
        || haystack
            .get(index - 1)
            .is_some_and(|c| !c.is_alphanumeric())
}

/// Filter and rank entries for a query.
///
/// An empty query returns everything in its existing (alphabetical) order,
/// which is what a launcher should show before the user has typed anything.
pub fn rank<'a>(query: &str, entries: &'a [Entry]) -> Vec<&'a Entry> {
    if query.trim().is_empty() {
        return entries.iter().collect();
    }
    let query = query.trim();

    let mut scored: Vec<(Score, &Entry)> = entries
        .iter()
        .filter_map(|entry| {
            let by_name = score(query, &entry.name);
            let by_keyword = score(query, &entry.keywords).map(|s| s / KEYWORD_PENALTY);
            let by_exec = score(query, &entry.exec).map(|s| s / KEYWORD_PENALTY);
            let best = [by_name, by_keyword, by_exec].into_iter().flatten().max()?;
            Some((best, entry))
        })
        .collect();

    // Sort by score, then by name, so equal scores come out in a stable and
    // predictable order rather than whatever the filter happened to produce.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().map(|(_, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, keywords: &str, exec: &str) -> Entry {
        Entry {
            name: name.into(),
            comment: String::new(),
            exec: exec.into(),
            terminal: false,
            keywords: keywords.into(),
            id: name.into(),
        }
    }

    fn apps() -> Vec<Entry> {
        vec![
            entry("Firefox", "Network;WebBrowser", "firefox"),
            entry("Files", "System;FileManager", "nautilus"),
            entry("File Roller", "Utility;Archiving", "file-roller"),
            entry("Konsole", "System;TerminalEmulator", "konsole"),
            entry("Wireshark", "Network;Monitor", "wireshark"),
        ]
    }

    fn names(query: &str) -> Vec<String> {
        rank(query, &apps()).into_iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn an_empty_query_keeps_everything_in_order() {
        assert_eq!(names("").len(), 5);
        assert_eq!(names("   "), names(""));
        assert_eq!(names("")[0], "Firefox", "the given order is preserved");
    }

    #[test]
    fn a_prefix_beats_a_match_in_the_middle() {
        let ranked = names("fire");
        assert_eq!(ranked[0], "Firefox");
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(names("FIREFOX")[0], "Firefox");
        assert_eq!(names("firefox")[0], "Firefox");
    }

    #[test]
    fn a_shorter_name_wins_a_tie() {
        let ranked = names("file");
        assert_eq!(ranked[0], "Files", "got {ranked:?}");
    }

    #[test]
    fn gaps_are_allowed_but_cost_something() {
        let tight = score("kon", "Konsole").unwrap();
        let loose = score("kne", "Konsole").unwrap();
        assert!(tight > loose);
    }

    #[test]
    fn a_word_start_scores_better_than_the_middle_of_a_word() {
        assert!(score("r", "File Roller").unwrap() > score("l", "File Roller").unwrap());
    }

    #[test]
    fn a_query_that_is_not_a_subsequence_matches_nothing() {
        assert_eq!(score("zzz", "Konsole"), None);
        assert!(names("zzzz").is_empty());
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(score("olesnok", "Konsole").is_none());
        assert!(score("konsole", "Konsole").is_some());
    }

    #[test]
    fn categories_and_commands_are_searchable_but_rank_below_names() {
        // "terminal" appears only in Konsole's categories.
        assert_eq!(names("terminal")[0], "Konsole");
        // A name match must still beat a category match for the same query.
        let mixed = vec![
            entry("Network Tools", "Utility", "nettools"),
            entry("Wireshark", "Network;Monitor", "wireshark"),
        ];
        let ranked: Vec<String> =
            rank("network", &mixed).into_iter().map(|e| e.name.clone()).collect();
        assert_eq!(ranked[0], "Network Tools");
    }

    #[test]
    fn ranking_is_deterministic() {
        assert_eq!(names("f"), names("f"));
    }

    #[test]
    fn an_empty_name_does_not_panic() {
        assert!(score("x", "").is_none());
        assert_eq!(score("", ""), Some(0));
    }
}
