//! Motions: where a command *would* go.
//!
//! A motion is a pure function from a cursor position to a destination. It
//! never moves anything itself. That separation is what lets one motion serve
//! two masters: the arrow keys move the cursor to the destination, and an
//! operator turns `cursor..destination` into the range it edits.
//!
//! Every destination is a **boundary**, not a character. `EndOfWord` lands one
//! past the word's last character and `LineEnd` lands one past the line's last
//! character, so an operator can treat every range as `[from, to)` without a
//! special case for "inclusive" motions.

use crate::doc::edit::Pos;
use crate::doc::{col_to_byte, Doc};

/// Something a command can move to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    /// Start of the next word.
    Word,
    /// Start of the next WORD (only whitespace separates).
    BigWord,
    /// Start of the previous word.
    BackWord,
    /// Start of the previous WORD.
    BackBigWord,
    /// One past the end of the current or next word.
    EndOfWord,
    /// One past the end of the current or next WORD.
    EndOfBigWord,
    LineEnd,
    LineStart,
    FirstNonBlank,
}

impl Motion {
    /// Long name, used in messages.
    pub fn label(self) -> &'static str {
        match self {
            Motion::Left => "left",
            Motion::Right => "right",
            Motion::Up => "up",
            Motion::Down => "down",
            Motion::Word => "word",
            Motion::BigWord => "WORD",
            Motion::BackWord => "back word",
            Motion::BackBigWord => "back WORD",
            Motion::EndOfWord => "end of word",
            Motion::EndOfBigWord => "end of WORD",
            Motion::LineEnd => "end of line",
            Motion::LineStart => "start of line",
            Motion::FirstNonBlank => "first non-blank",
        }
    }
}

/// Where `motion` taken from `from` ends up.
pub fn target(doc: &Doc, from: Pos, motion: Motion) -> Pos {
    match motion {
        Motion::Left => Pos::new(from.line, from.col.saturating_sub(1)),
        Motion::Right => Pos::new(from.line, (from.col + 1).min(line_len(doc, from.line))),
        Motion::Up => {
            if from.line == 0 {
                return from;
            }
            let line = from.line - 1;
            Pos::new(line, from.col.min(line_len(doc, line)))
        }
        Motion::Down => {
            let line = from.line + 1;
            if doc.line_text(line).is_none() {
                return from;
            }
            Pos::new(line, from.col.min(line_len(doc, line)))
        }
        Motion::LineStart => Pos::new(from.line, 0),
        Motion::LineEnd => Pos::new(from.line, line_len(doc, from.line)),
        Motion::FirstNonBlank => {
            let text = doc.line_text(from.line).unwrap_or_default();
            let col = text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or_else(|| text.chars().count());
            Pos::new(from.line, col)
        }
        Motion::Word => word_forward(doc, from, false),
        Motion::BigWord => word_forward(doc, from, true),
        Motion::BackWord => word_back(doc, from, false),
        Motion::BackBigWord => word_back(doc, from, true),
        Motion::EndOfWord => word_end(doc, from, false),
        Motion::EndOfBigWord => word_end(doc, from, true),
    }
}

/// How a character groups with its neighbours.
///
/// Words are runs of alphanumerics and underscores; punctuation is its own
/// run, so `foo.bar` is three words and `.` is one of them. A WORD ignores
/// punctuation and is broken only by whitespace.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

fn line_len(doc: &Doc, line: u64) -> usize {
    doc.line_text(line).map_or(0, |t| t.chars().count())
}

/// The start of the next word: finish the run the cursor is in, then skip
/// whitespace. Line breaks count as whitespace, so this crosses lines.
fn word_forward(doc: &Doc, from: Pos, big: bool) -> Pos {
    let mut line = from.line;
    let mut col = from.col;
    let mut inside_run = true;

    loop {
        let text = doc.line_text(line).unwrap_or_default();
        let mut chars = text.chars().skip(col).peekable();
        let mut at = col;

        if inside_run {
            if let Some(&first) = chars.peek() {
                let kind = class(first, big);
                while let Some(&c) = chars.peek() {
                    if class(c, big) != kind {
                        break;
                    }
                    chars.next();
                    at += 1;
                }
            }
            inside_run = false;
        }

        while let Some(&c) = chars.peek() {
            if class(c, big) != Class::Space {
                return Pos::new(line, at);
            }
            chars.next();
            at += 1;
        }

        // End of the line: the line break is whitespace too.
        if doc.line_text(line + 1).is_none() {
            return Pos::new(line, at);
        }
        line += 1;
        col = 0;
    }
}

/// The start of the previous word.
///
/// Stepping backwards never needs the cursor's absolute column, only "the
/// character behind me", which is why this walks a reversed slice of the line
/// instead of collecting the line into a `Vec<char>`.
fn word_back(doc: &Doc, from: Pos, big: bool) -> Pos {
    let mut line = from.line;
    let mut col = from.col;
    let mut stepped = false;

    loop {
        let text = doc.line_text(line).unwrap_or_default();
        let mut before = text[..col_to_byte(&text, col)].chars().rev().peekable();

        if !stepped {
            if before.next().is_some() {
                col -= 1;
                stepped = true;
            }
        }

        let mut skipped_space = false;
        while let Some(&c) = before.peek() {
            if class(c, big) != Class::Space {
                break;
            }
            before.next();
            col -= 1;
            skipped_space = true;
        }

        if col == 0 {
            // Already at the start of a word on this line: that is the answer.
            if stepped && !skipped_space {
                return Pos::new(line, 0);
            }
            if line == 0 {
                return Pos::new(0, 0);
            }
            line -= 1;
            col = line_len(doc, line);
            continue;
        }

        let kind = class(*before.peek().expect("col > 0 means there is a character behind it"), big);
        while let Some(&c) = before.peek() {
            if class(c, big) != kind {
                break;
            }
            before.next();
            col -= 1;
        }
        return Pos::new(line, col);
    }
}

/// One past the end of the current or next word.
fn word_end(doc: &Doc, from: Pos, big: bool) -> Pos {
    let mut line = from.line;
    let mut col = from.col;

    loop {
        let text = doc.line_text(line).unwrap_or_default();
        let mut chars = text.chars().skip(col).peekable();
        let mut at = col;

        while let Some(&c) = chars.peek() {
            if class(c, big) != Class::Space {
                break;
            }
            chars.next();
            at += 1;
        }

        if let Some(&first) = chars.peek() {
            let kind = class(first, big);
            while let Some(&c) = chars.peek() {
                if class(c, big) != kind {
                    break;
                }
                chars.next();
                at += 1;
            }
            return Pos::new(line, at);
        }

        if doc.line_text(line + 1).is_none() {
            return Pos::new(line, at);
        }
        line += 1;
        col = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(name: &str, text: &str) -> Doc {
        let dir = std::env::temp_dir().join("zeno-motion-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        doc
    }

    fn at(line: u64, col: usize) -> Pos {
        Pos::new(line, col)
    }

    #[test]
    fn word_forward_lands_on_the_next_word() {
        let d = doc("w.txt", "alpha beta gamma");
        assert_eq!(target(&d, at(0, 0), Motion::Word), at(0, 6));
        assert_eq!(target(&d, at(0, 6), Motion::Word), at(0, 11));
        // From the last word there is nowhere left to go but the end.
        assert_eq!(target(&d, at(0, 11), Motion::Word), at(0, 16));
    }

    #[test]
    fn word_forward_crosses_a_line_break() {
        let d = doc("w-lines.txt", "one\ntwo\n");
        assert_eq!(target(&d, at(0, 0), Motion::Word), at(1, 0));
    }

    #[test]
    fn word_forward_skips_blank_lines() {
        let d = doc("w-blank.txt", "a\n\nb");
        assert_eq!(target(&d, at(0, 0), Motion::Word), at(2, 0));
    }

    #[test]
    fn punctuation_is_its_own_word() {
        let d = doc("w-punct.txt", "foo.bar baz");
        assert_eq!(target(&d, at(0, 0), Motion::Word), at(0, 3));
        assert_eq!(target(&d, at(0, 3), Motion::Word), at(0, 4));
        assert_eq!(target(&d, at(0, 3), Motion::BigWord), at(0, 8));
    }

    #[test]
    fn word_back_lands_on_the_start_of_a_word() {
        let d = doc("b.txt", "alpha beta gamma");
        assert_eq!(target(&d, at(0, 11), Motion::BackWord), at(0, 6));
        assert_eq!(target(&d, at(0, 6), Motion::BackWord), at(0, 0));
        // Already at the start of the first word: nowhere to go.
        assert_eq!(target(&d, at(0, 0), Motion::BackWord), at(0, 0));
    }

    #[test]
    fn word_back_from_mid_word_returns_to_its_start() {
        let d = doc("b-mid.txt", "alpha beta");
        assert_eq!(target(&d, at(0, 4), Motion::BackWord), at(0, 0));
        assert_eq!(target(&d, at(0, 9), Motion::BackWord), at(0, 6));
    }

    #[test]
    fn word_back_crosses_a_line_break() {
        let d = doc("b-lines.txt", "one\ntwo\n");
        assert_eq!(target(&d, at(1, 0), Motion::BackWord), at(0, 0));
    }

    #[test]
    fn word_back_from_an_empty_line_reaches_the_previous_word() {
        let d = doc("b-blank.txt", "a\n\nb");
        assert_eq!(target(&d, at(1, 0), Motion::BackWord), at(0, 0));
    }

    #[test]
    fn word_end_is_one_past_the_last_character() {
        let d = doc("e.txt", "alpha beta");
        assert_eq!(target(&d, at(0, 0), Motion::EndOfWord), at(0, 5));
        assert_eq!(target(&d, at(0, 2), Motion::EndOfWord), at(0, 5));
        assert_eq!(target(&d, at(0, 5), Motion::EndOfWord), at(0, 10));
    }

    #[test]
    fn line_motions_are_boundaries() {
        let d = doc("lines.txt", "   indented\nsecond\n");
        assert_eq!(target(&d, at(0, 6), Motion::LineEnd), at(0, 11));
        assert_eq!(target(&d, at(0, 6), Motion::LineStart), at(0, 0));
        assert_eq!(target(&d, at(0, 6), Motion::FirstNonBlank), at(0, 3));
    }

    #[test]
    fn vertical_motions_keep_the_column_when_they_can() {
        let d = doc("vertical.txt", "long line\nab\nlong line\n");
        assert_eq!(target(&d, at(0, 7), Motion::Down), at(1, 2));
        assert_eq!(target(&d, at(1, 2), Motion::Down), at(2, 2));
        assert_eq!(target(&d, at(0, 0), Motion::Up), at(0, 0));
    }

    #[test]
    fn horizontal_motions_stay_inside_the_line() {
        let d = doc("h.txt", "ab\ncd\n");
        assert_eq!(target(&d, at(0, 0), Motion::Left), at(0, 0));
        assert_eq!(target(&d, at(0, 1), Motion::Right), at(0, 2));
        assert_eq!(target(&d, at(0, 2), Motion::Right), at(0, 2));
    }
}
