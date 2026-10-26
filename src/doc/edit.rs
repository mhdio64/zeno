//! Span edits, and the journal that makes them reversible.
//!
//! Every change to the document goes through one primitive: replace the text
//! in a range with a string. Inserting a character is an empty range
//! replaced by that character; pressing Enter is an empty range replaced by a
//! newline; joining two lines is the range across the line break replaced by
//! nothing.
//!
//! Having a single mutation point is what makes undo affordable. A journal of
//! snapshots would cost the size of the file per keystroke, which a 1 GB file
//! cannot pay. A journal of *edits* costs only the text that moved, so memory
//! is bounded by how much the user typed, not by how big the file is.

use std::fmt;

/// A position in the document: a line, and a character column within it.
///
/// Columns count characters, not bytes, so a line of Persian or emoji moves
/// the cursor one column per visible glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: u64,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: u64, col: usize) -> Self {
        Self { line, col }
    }
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line + 1, self.col + 1)
    }
}

/// One resolved replacement, with enough information to reverse it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Start of the replaced range.
    pub start: Pos,
    /// End of the replaced range as it was *before* the edit.
    pub end: Pos,
    /// End of the inserted text.
    pub inserted_end: Pos,
    /// The text that was removed.
    pub removed: String,
    /// The text that was inserted.
    pub inserted: String,
}

impl Edit {
    /// True when this edit is a continuation of a typing run.
    ///
    /// Typing "hello" should be one undo step, not five, so adjacent
    /// single-line insertions are folded together.
    fn continues(&self, previous: &Edit) -> bool {
        previous.removed.is_empty()
            && self.removed.is_empty()
            && !previous.inserted.contains('\n')
            && !self.inserted.contains('\n')
            && previous.inserted_end == self.start
    }
}

/// The undo and redo stacks.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an edit, folding it into the previous one when it continues a
    /// typing run.
    pub fn record(&mut self, edit: Edit) {
        // A new edit invalidates anything that was undone.
        self.redo.clear();
        if let Some(last) = self.undo.last_mut()
            && edit.continues(last)
        {
            last.inserted.push_str(&edit.inserted);
            last.inserted_end = edit.inserted_end;
            return;
        }
        self.undo.push(edit);
    }

    /// Takes the edit to reverse, moving it onto the redo stack.
    pub fn take_undo(&mut self) -> Option<Edit> {
        let edit = self.undo.pop()?;
        self.redo.push(edit.clone());
        Some(edit)
    }

    /// Takes the edit to reapply, moving it back onto the undo stack.
    pub fn take_redo(&mut self) -> Option<Edit> {
        let edit = self.redo.pop()?;
        self.undo.push(edit.clone());
        Some(edit)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// The last edit on the undo stack, if any.
    pub fn last_undo(&self) -> Option<&Edit> {
        self.undo.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(at: Pos, text: &str) -> Edit {
        let mut end = at;
        for part in text.split('\n').skip(1) {
            end = Pos::new(end.line + 1, part.chars().count());
        }
        if !text.contains('\n') {
            end = Pos::new(at.line, at.col + text.chars().count());
        }
        Edit {
            start: at,
            end: at,
            inserted_end: end,
            removed: String::new(),
            inserted: text.into(),
        }
    }

    #[test]
    fn a_typing_run_folds_into_one_step() {
        let mut h = History::new();
        h.record(insert(Pos::new(0, 0), "h"));
        h.record(insert(Pos::new(0, 1), "i"));
        assert_eq!(h.undo_depth(), 1);
        let e = h.last_undo().unwrap();
        assert_eq!(e.inserted, "hi");
        assert_eq!(e.inserted_end, Pos::new(0, 2));
    }

    #[test]
    fn a_newline_breaks_the_run() {
        let mut h = History::new();
        h.record(insert(Pos::new(0, 0), "a"));
        h.record(insert(Pos::new(0, 1), "\n"));
        h.record(insert(Pos::new(1, 0), "b"));
        assert_eq!(h.undo_depth(), 3);
    }

    #[test]
    fn typing_somewhere_else_breaks_the_run() {
        let mut h = History::new();
        h.record(insert(Pos::new(0, 0), "a"));
        h.record(insert(Pos::new(5, 0), "b"));
        assert_eq!(h.undo_depth(), 2);
    }

    #[test]
    fn a_deletion_breaks_the_run() {
        let mut h = History::new();
        h.record(insert(Pos::new(0, 0), "a"));
        h.record(Edit {
            start: Pos::new(0, 0),
            end: Pos::new(0, 1),
            inserted_end: Pos::new(0, 0),
            removed: "a".into(),
            inserted: String::new(),
        });
        assert_eq!(h.undo_depth(), 2);
    }

    #[test]
    fn undone_edits_move_to_the_redo_stack() {
        let mut h = History::new();
        h.record(insert(Pos::new(0, 0), "a"));
        assert!(h.can_undo() && !h.can_redo());
        let e = h.take_undo().unwrap();
        assert_eq!(e.inserted, "a");
        assert!(!h.can_undo() && h.can_redo());
        let r = h.take_redo().unwrap();
        assert_eq!(r.inserted, "a");
        assert!(h.can_undo() && !h.can_redo());
    }

    #[test]
    fn a_new_edit_discards_the_redo_stack() {
        let mut h = History::new();
        h.record(insert(Pos::new(0, 0), "a"));
        h.take_undo();
        assert!(h.can_redo());
        h.record(insert(Pos::new(0, 0), "b"));
        assert!(!h.can_redo());
    }
}
