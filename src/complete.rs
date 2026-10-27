//! Word auto-completion within the current file.
//!
//! Scans nearby lines in the document for words starting with the prefix at the
//! cursor, prioritizing words closest to the cursor and deduplicating matches.

use crate::doc::Doc;
use crate::doc::edit::Pos;

/// Active completion session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Line the completion was triggered on.
    pub line: u64,
    /// Column where the prefix begins.
    pub start_col: usize,
    /// The original prefix typed by the user before completion started.
    pub original: String,
    /// Matching candidate words, ordered by proximity.
    pub candidates: Vec<String>,
    /// Current selected index. `0..candidates.len()` is a candidate;
    /// `candidates.len()` represents the original prefix.
    pub index: usize,
}

impl Completion {
    /// Creates a new completion state with the given prefix and candidate list.
    pub fn new(line: u64, start_col: usize, original: String, candidates: Vec<String>) -> Self {
        Self {
            line,
            start_col,
            original,
            candidates,
            index: 0,
        }
    }

    /// Returns the currently active completion text.
    pub fn current_text(&self) -> &str {
        if self.index < self.candidates.len() {
            &self.candidates[self.index]
        } else {
            &self.original
        }
    }

    /// Advances to the next candidate (cycling through original).
    pub fn cycle_next(&mut self) -> &str {
        if self.candidates.is_empty() {
            return &self.original;
        }
        self.index = (self.index + 1) % (self.candidates.len() + 1);
        self.current_text()
    }

    /// Moves to the previous candidate (cycling through original).
    pub fn cycle_prev(&mut self) -> &str {
        if self.candidates.is_empty() {
            return &self.original;
        }
        if self.index == 0 {
            self.index = self.candidates.len();
        } else {
            self.index -= 1;
        }
        self.current_text()
    }

    /// Human-friendly status message describing current match.
    pub fn status_message(&self) -> String {
        if self.index < self.candidates.len() {
            format!(
                "[{}/{}] {}  (^N next, ^P prev, Esc cancel)",
                self.index + 1,
                self.candidates.len(),
                self.candidates[self.index]
            )
        } else {
            format!(
                "[original] {}  (^N next, ^P prev, Esc cancel)",
                self.original
            )
        }
    }
}

/// Whether `c` is treated as part of a word or identifier.
#[inline]
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Finds the word prefix immediately preceding `col` on the line.
///
/// Returns `Some((start_col, prefix))` if the character right before `col` is a
/// word character, or `None` if the cursor is at the line start or preceded by
/// non-word characters.
pub fn find_prefix(line_text: &str, col: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = line_text.chars().collect();
    let col = col.min(chars.len());
    if col == 0 {
        return None;
    }
    if !is_word_char(chars[col - 1]) {
        return None;
    }
    let mut start = col - 1;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let prefix: String = chars[start..col].iter().collect();
    Some((start, prefix))
}

/// Extracts all word slices from `line`.
pub fn words_in_line(line: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = None;
    for (idx, c) in line.char_indices() {
        if is_word_char(c) {
            if start.is_none() {
                start = Some(idx);
            }
        } else if let Some(s) = start.take() {
            words.push(&line[s..idx]);
        }
    }
    if let Some(s) = start {
        words.push(&line[s..]);
    }
    words
}

/// Collects candidate completions for `prefix` from `doc` around `cursor`.
///
/// Scans outwards from `cursor.line` (proximity order: current line, line - 1,
/// line + 1, line - 2, line + 2...), collecting unique words starting with
/// `prefix` (excluding exact matches of `prefix`).
pub fn find_candidates(doc: &Doc, cursor: Pos, prefix: &str) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let total_lines = doc.total_lines().unwrap_or_else(|| doc.line_count());
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(prefix.to_string());

    // First scan current line
    if let Some(text) = doc.line_text(cursor.line) {
        for word in words_in_line(&text) {
            if word.starts_with(prefix) && seen.insert(word.to_string()) {
                candidates.push(word.to_string());
            }
        }
    }

    // Maximum distance to search (2000 lines up and 2000 lines down)
    // and maximum candidate count to guarantee instant microsecond response.
    let max_distance = 2000u64;
    let max_candidates = 50;

    let mut dist = 1u64;
    while dist <= max_distance && candidates.len() < max_candidates {
        let mut checked_any = false;
        // Check line above
        if cursor.line >= dist
            && let Some(text) = doc.line_text(cursor.line - dist)
        {
            checked_any = true;
            for word in words_in_line(&text) {
                if word.starts_with(prefix) && seen.insert(word.to_string()) {
                    candidates.push(word.to_string());
                    if candidates.len() >= max_candidates {
                        break;
                    }
                }
            }
        }
        // Check line below
        let line_below = cursor.line + dist;
        if line_below < total_lines
            && let Some(text) = doc.line_text(line_below)
        {
            checked_any = true;
            for word in words_in_line(&text) {
                if word.starts_with(prefix) && seen.insert(word.to_string()) {
                    candidates.push(word.to_string());
                    if candidates.len() >= max_candidates {
                        break;
                    }
                }
            }
        }
        if !checked_any && line_below >= total_lines {
            break;
        }
        dist += 1;
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_of(name: &str, contents: &[u8]) -> Doc {
        use std::io::Write;
        let dir = std::env::temp_dir().join("zeno-complete-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        doc
    }

    #[test]
    fn find_prefix_extracts_word_immediately_before_cursor() {
        assert_eq!(
            find_prefix("let my_variable = 10", 15),
            Some((4, "my_variable".into()))
        );
        assert_eq!(find_prefix("hello", 5), Some((0, "hello".into())));
        assert_eq!(find_prefix("hello", 2), Some((0, "he".into())));
        assert_eq!(find_prefix("foo.bar", 7), Some((4, "bar".into())));
        assert_eq!(find_prefix("foo.bar", 4), None);
        assert_eq!(find_prefix("hello ", 6), None);
        assert_eq!(find_prefix("", 0), None);
    }

    #[test]
    fn words_in_line_extracts_all_word_tokens() {
        let line = "fn calculate_sum(a: i32, b_1: i32) -> i64 {";
        let words = words_in_line(line);
        assert_eq!(
            words,
            vec!["fn", "calculate_sum", "a", "i32", "b_1", "i32", "i64"]
        );
    }

    #[test]
    fn find_candidates_orders_by_proximity_and_deduplicates() {
        let content = b"let far_away = 1;\nlet process_data = 2;\nlet processor = 3;\nlet pro\nlet proceed = 4;\n";
        let doc = doc_of("proximity.txt", content);
        // Cursor at line 3, col 7 (right after "pro")
        let candidates = find_candidates(&doc, Pos::new(3, 7), "pro");

        // "pro" itself should be excluded
        assert!(!candidates.contains(&"pro".to_string()));

        // Nearby lines (processor on line 2, proceed on line 4, process_data on line 1)
        assert_eq!(candidates, vec!["processor", "proceed", "process_data"]);
    }

    #[test]
    fn completion_cycling_through_candidates_and_original() {
        let mut comp =
            Completion::new(0, 0, "pro".into(), vec!["process".into(), "proceed".into()]);
        assert_eq!(comp.current_text(), "process");
        assert_eq!(
            comp.status_message(),
            "[1/2] process  (^N next, ^P prev, Esc cancel)"
        );

        assert_eq!(comp.cycle_next(), "proceed");
        assert_eq!(
            comp.status_message(),
            "[2/2] proceed  (^N next, ^P prev, Esc cancel)"
        );

        // Next cycles to original
        assert_eq!(comp.cycle_next(), "pro");
        assert_eq!(
            comp.status_message(),
            "[original] pro  (^N next, ^P prev, Esc cancel)"
        );

        // Next cycles back to first candidate
        assert_eq!(comp.cycle_next(), "process");

        // Prev cycles back to original
        assert_eq!(comp.cycle_prev(), "pro");

        // Prev from original cycles to last candidate
        assert_eq!(comp.cycle_prev(), "proceed");
    }
}
