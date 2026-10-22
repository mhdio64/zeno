//! Line-level edit overlay.
//!
//! The mapped file is never modified while editing. Instead, the original
//! lines stay where they are, and each *segment* records one original line
//! together with the current lines that stand in for it. A segment with no
//! lines means the original line was deleted; a segment with several lines
//! means the original line was split.
//!
//! The important detail is the granularity of an edit. An edit addresses a
//! **current** line, not an original one, and it replaces only that line
//! inside its segment. Editing the second half of a split line must not
//! destroy the first half — which is exactly what happens if the edit
//! granularity is the original line.
//!
//! This is deliberately a whole-line model: it is enough to prove the
//! performance claims of the design (editing never rewrites the file, and
//! the viewport cost is independent of file size) without building a
//! rope-based text model that the real implementation may or may not need.

/// A replacement of one original line by zero or more current lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEdit {
    /// The original line this segment stands in for.
    pub original_line: u64,
    /// The current lines that take its place.
    pub lines: Vec<String>,
}

/// Where a current line lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Location {
    /// The line is one of the lines of a segment.
    InSegment {
        segment: usize,
        entry: usize,
        original: u64,
    },
    /// The line is an original line that has not been edited.
    Original { original: u64 },
}

/// The set of edits applied on top of the mapped file.
#[derive(Debug, Default, Clone)]
pub struct Overlay {
    segments: Vec<LineEdit>,
}

impl Overlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when nothing has been edited.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Number of edited original lines.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Finds where a current line lives.
    fn locate(&self, current_line: u64) -> Location {
        let mut delta: i64 = 0;
        for (segment, edit) in self.segments.iter().enumerate() {
            let start = edit.original_line as i64 + delta;
            let end = start + edit.lines.len() as i64;
            if (current_line as i64) < start {
                break;
            }
            if (current_line as i64) < end {
                return Location::InSegment {
                    segment,
                    entry: (current_line as i64 - start) as usize,
                    original: edit.original_line,
                };
            }
            delta += edit.lines.len() as i64 - 1;
        }
        Location::Original {
            original: (current_line as i64 - delta).max(0) as u64,
        }
    }

    /// Replaces the text of a current line.
    pub fn set_line(&mut self, current_line: u64, text: String) {
        match self.locate(current_line) {
            Location::InSegment { segment, entry, .. } => {
                self.segments[segment].lines[entry] = text;
            }
            Location::Original { original } => {
                self.upsert(original, vec![text]);
            }
        }
    }

    /// Replaces a current line with any number of current lines.
    ///
    /// An empty vector deletes the line.
    pub fn replace_lines(&mut self, current_line: u64, lines: Vec<String>) {
        match self.locate(current_line) {
            Location::InSegment { segment, entry, .. } => {
                self.segments[segment].lines.splice(entry..entry + 1, lines);
            }
            Location::Original { original } => {
                self.upsert(original, lines);
            }
        }
    }

    /// Deletes a current line.
    pub fn delete_line(&mut self, current_line: u64) {
        self.replace_lines(current_line, Vec::new());
    }

    /// Creates or replaces the segment of an original line.
    fn upsert(&mut self, original: u64, lines: Vec<String>) {
        match self
            .segments
            .binary_search_by_key(&original, |e| e.original_line)
        {
            Ok(idx) => self.segments[idx].lines = lines,
            Err(idx) => self
                .segments
                .insert(idx, LineEdit { original_line: original, lines }),
        }
    }

    /// The edited text of a current line, if that line came from an edit.
    pub fn get(&self, current_line: u64) -> Option<&str> {
        match self.locate(current_line) {
            Location::InSegment { segment, entry, .. } => {
                Some(self.segments[segment].lines[entry].as_str())
            }
            Location::Original { .. } => None,
        }
    }

    /// Maps a current line number to the original line it came from.
    ///
    /// When a current line falls inside a multi-line replacement, the
    /// original line it replaced is returned.
    pub fn current_to_original(&self, current_line: u64) -> u64 {
        match self.locate(current_line) {
            Location::InSegment { original, .. } => original,
            Location::Original { original } => original,
        }
    }

    /// Maps an original line number to the current line number it became.
    ///
    /// When an original line was replaced by several lines, the current line
    /// of its first replacement is returned.
    pub fn original_to_current(&self, original_line: u64) -> u64 {
        let mut delta: i64 = 0;
        for edit in &self.segments {
            if original_line < edit.original_line {
                break;
            }
            if original_line == edit.original_line {
                return (edit.original_line as i64 + delta) as u64;
            }
            delta += edit.lines.len() as i64 - 1;
        }
        (original_line as i64 + delta).max(0) as u64
    }

    /// How many lines the document has, given the number of original lines.
    pub fn line_count(&self, original_lines: u64) -> u64 {
        let delta: i64 = self.segments.iter().map(|e| e.lines.len() as i64 - 1).sum();
        (original_lines as i64 + delta).max(0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_overlay_is_identity() {
        let ov = Overlay::new();
        assert_eq!(ov.current_to_original(7), 7);
        assert_eq!(ov.original_to_current(7), 7);
        assert_eq!(ov.line_count(10), 10);
        assert!(ov.get(7).is_none());
    }

    #[test]
    fn single_line_replacement_keeps_numbering() {
        let mut ov = Overlay::new();
        ov.set_line(3, "x".into());
        assert_eq!(ov.current_to_original(3), 3);
        assert_eq!(ov.original_to_current(3), 3);
        assert_eq!(ov.current_to_original(4), 4);
        assert_eq!(ov.line_count(10), 10);
        assert_eq!(ov.get(3), Some("x"));
        assert_eq!(ov.get(4), None);
    }

    #[test]
    fn multi_line_insert_shifts_later_lines() {
        let mut ov = Overlay::new();
        ov.replace_lines(3, vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(ov.line_count(10), 12);
        assert_eq!(ov.current_to_original(3), 3);
        assert_eq!(ov.current_to_original(4), 3);
        assert_eq!(ov.current_to_original(5), 3);
        assert_eq!(ov.current_to_original(6), 4);
        assert_eq!(ov.original_to_current(4), 6);
        assert_eq!(ov.get(5), Some("c"));
        assert_eq!(ov.get(6), None);
    }

    #[test]
    fn deleting_a_line_shortens_the_document() {
        let mut ov = Overlay::new();
        ov.delete_line(3);
        assert_eq!(ov.line_count(10), 9);
        assert_eq!(ov.current_to_original(3), 4);
        assert_eq!(ov.original_to_current(4), 3);
        assert_eq!(ov.get(3), None);
    }

    #[test]
    fn replacing_the_same_line_twice_overwrites() {
        let mut ov = Overlay::new();
        ov.replace_lines(3, vec!["a".into(), "b".into()]);
        ov.set_line(3, "z".into());
        // Two edits, one segment: the second must not add a duplicate.
        assert_eq!(ov.len(), 1);
        assert_eq!(ov.line_count(10), 11);
        assert_eq!(ov.get(3), Some("z"));
        assert_eq!(ov.get(4), Some("b"));
        assert_eq!(ov.current_to_original(4), 3);
        assert_eq!(ov.current_to_original(5), 4);
    }

    /// The bug this model exists to prevent: editing one half of a split
    /// line must not destroy the other half.
    #[test]
    fn editing_one_half_of_a_split_keeps_the_other_half() {
        let mut ov = Overlay::new();
        ov.replace_lines(2, vec!["}".into(), String::new()]);
        assert_eq!(ov.line_count(6), 7);
        assert_eq!(ov.get(2), Some("}"));
        assert_eq!(ov.get(3), Some(""));

        ov.set_line(3, "N".into());
        assert_eq!(ov.get(2), Some("}"), "the first half must survive");
        assert_eq!(ov.get(3), Some("N"));
        assert_eq!(ov.line_count(6), 7);
    }

    #[test]
    fn splitting_a_split_line_keeps_all_the_pieces() {
        let mut ov = Overlay::new();
        ov.replace_lines(2, vec!["}".into(), String::new()]);
        ov.replace_lines(3, vec!["a".into(), "b".into()]);
        assert_eq!(ov.get(2), Some("}"));
        assert_eq!(ov.get(3), Some("a"));
        assert_eq!(ov.get(4), Some("b"));
        assert_eq!(ov.line_count(6), 8);
    }

    #[test]
    fn joining_two_split_halves_puts_them_back_together() {
        let mut ov = Overlay::new();
        ov.replace_lines(2, vec!["ab".into(), "cd".into()]);
        // Merge line 2 into line 3 and drop line 3.
        ov.set_line(2, "abcd".into());
        ov.delete_line(3);
        assert_eq!(ov.get(2), Some("abcd"));
        assert_eq!(ov.get(3), None);
        assert_eq!(ov.line_count(6), 6);
        assert_eq!(ov.current_to_original(3), 3);
    }

    #[test]
    fn edits_are_kept_sorted() {
        let mut ov = Overlay::new();
        ov.set_line(9, "a".into());
        ov.set_line(1, "b".into());
        ov.set_line(5, "c".into());
        let order: Vec<u64> = ov.segments.iter().map(|e| e.original_line).collect();
        assert_eq!(order, vec![1, 5, 9]);
    }

    #[test]
    fn deleting_an_edited_line_leaves_its_neighbours_alone() {
        let mut ov = Overlay::new();
        ov.replace_lines(2, vec!["a".into(), "b".into(), "c".into()]);
        ov.delete_line(3);
        assert_eq!(ov.get(2), Some("a"));
        assert_eq!(ov.get(3), Some("c"));
        assert_eq!(ov.line_count(6), 7);
    }
}
