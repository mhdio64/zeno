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
use crate::doc::{Doc, col_to_byte};

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
    /// Start of the next paragraph: the next blank line, or the end of the
    /// file.
    ParagraphForward,
    /// Start of the previous paragraph: the blank line above, or the top of
    /// the file.
    ParagraphBack,
    /// One past the end of the current or next sentence.
    SentenceForward,
    /// Start of the current or previous sentence.
    SentenceBack,
    /// The bracket matching the one under (or after) the cursor.
    MatchingBracket,
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
            Motion::ParagraphForward => "next paragraph",
            Motion::ParagraphBack => "previous paragraph",
            Motion::SentenceForward => "next sentence",
            Motion::SentenceBack => "previous sentence",
            Motion::MatchingBracket => "matching bracket",
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
        Motion::ParagraphForward => paragraph_forward(doc, from),
        Motion::ParagraphBack => paragraph_back(doc, from),
        Motion::SentenceForward => sentence_forward(doc, from),
        Motion::SentenceBack => sentence_back(doc, from),
        Motion::MatchingBracket => matching_bracket(doc, from),
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

        if !stepped && before.next().is_some() {
            col -= 1;
            stepped = true;
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

        let kind = class(
            *before
                .peek()
                .expect("col > 0 means there is a character behind it"),
            big,
        );
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

/// How far a landmark motion may walk before giving up.
///
/// A paragraph or sentence search is a scan, and a file with no blank line in
/// it would otherwise walk 39 million lines to answer `}`. The cap is what
/// keeps one keystroke inside a frame; past it the motion reports "nowhere to
/// go" by staying put (docs/performance.md, ADR 0007).
const MAX_LANDMARK_LINES: u64 = 10_000;

/// True when a line holds nothing but whitespace.
fn is_blank(doc: &Doc, line: u64) -> bool {
    doc.line_text(line)
        .is_some_and(|text| text.trim().is_empty())
}

/// The next blank line, or the end of the file.
fn paragraph_forward(doc: &Doc, from: Pos) -> Pos {
    let mut line = from.line + 1;
    let mut scanned = 0u64;
    while doc.line_text(line).is_some() {
        if is_blank(doc, line) {
            return Pos::new(line, 0);
        }
        scanned += 1;
        if scanned > MAX_LANDMARK_LINES {
            return from;
        }
        line += 1;
    }
    // Ran off the end: the last line that exists is where this ends.
    let last = line.saturating_sub(1);
    Pos::new(last, line_len(doc, last))
}

/// The blank line above, or the top of the file.
fn paragraph_back(doc: &Doc, from: Pos) -> Pos {
    let mut line = from.line;
    let mut scanned = 0u64;
    while line > 0 {
        line -= 1;
        if is_blank(doc, line) {
            return Pos::new(line, 0);
        }
        scanned += 1;
        if scanned > MAX_LANDMARK_LINES {
            return from;
        }
    }
    Pos::new(0, 0)
}

/// Whether `c` ends a sentence.
fn ends_sentence(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// The first column after a sentence terminator and its trailing space.
fn after_terminator(chars: &[char], i: usize) -> usize {
    let mut at = i + 1;
    while at < chars.len() && chars[at].is_whitespace() {
        at += 1;
    }
    at
}

/// One past the end of the current or next sentence.
///
/// A terminator counts only when whitespace or the end of the line follows
/// it, so `2.5` and `e.g.` do not end anything.
fn sentence_forward(doc: &Doc, from: Pos) -> Pos {
    let mut line = from.line;
    let mut col = from.col;
    let mut scanned = 0u64;
    loop {
        let text = doc.line_text(line).unwrap_or_default();
        let chars: Vec<char> = text.chars().collect();
        let mut i = col;
        while i < chars.len() {
            if ends_sentence(chars[i]) && chars.get(i + 1).is_none_or(|c| c.is_whitespace()) {
                return Pos::new(line, after_terminator(&chars, i));
            }
            i += 1;
        }
        scanned += 1;
        if doc.line_text(line + 1).is_none() || scanned > MAX_LANDMARK_LINES {
            return Pos::new(line, chars.len());
        }
        line += 1;
        col = 0;
    }
}

/// Start of the current or previous sentence.
///
/// When the cursor already sits on a sentence start, the terminator that
/// opened it is stepped over first, so a second `(` moves a second sentence
/// back instead of staying where it is.
fn sentence_back(doc: &Doc, from: Pos) -> Pos {
    let mut line = from.line;
    let mut col = from.col;
    let mut scanned = 0u64;
    loop {
        let text = doc.line_text(line).unwrap_or_default();
        let chars: Vec<char> = text.chars().collect();
        let mut scan_from = col.min(chars.len());
        let mut probe = scan_from;
        while probe > 0 && chars[probe - 1].is_whitespace() {
            probe -= 1;
        }
        if probe > 0 && ends_sentence(chars[probe - 1]) {
            scan_from = probe - 1;
        }
        let mut i = scan_from;
        while i > 0 {
            i -= 1;
            if ends_sentence(chars[i]) && chars.get(i + 1).is_none_or(|c| c.is_whitespace()) {
                return Pos::new(line, after_terminator(&chars, i));
            }
        }
        scanned += 1;
        if line == 0 || scanned > MAX_LANDMARK_LINES {
            return Pos::new(line, 0);
        }
        line -= 1;
        col = line_len(doc, line);
    }
}

/// The character at a position, if there is one there.
fn char_at(doc: &Doc, pos: Pos) -> Option<char> {
    doc.line_text(pos.line)?.chars().nth(pos.col)
}

/// The bracket pairs `%` knows.
fn bracket_pair(c: char) -> Option<(char, char)> {
    Some(match c {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        '{' => ('{', '}'),
        ')' => ('(', ')'),
        ']' => ('[', ']'),
        '}' => ('{', '}'),
        _ => return None,
    })
}

fn is_opener(c: char) -> bool {
    matches!(c, '(' | '[' | '{')
}

/// The next position in reading order, or `None` at the ends of the file.
///
/// A boundary (column equal to the line's length) has no character of its
/// own: the newline lives between it and the next line's first character, so
/// the step crosses it.
fn step_char(doc: &Doc, pos: Pos, forward: bool) -> Option<Pos> {
    if forward {
        if pos.col < line_len(doc, pos.line) {
            return Some(Pos::new(pos.line, pos.col + 1));
        }
        doc.line_text(pos.line + 1)?;
        Some(Pos::new(pos.line + 1, 0))
    } else {
        if pos.col > 0 {
            return Some(Pos::new(pos.line, pos.col - 1));
        }
        if pos.line == 0 {
            return None;
        }
        let previous = pos.line - 1;
        Some(Pos::new(previous, line_len(doc, previous)))
    }
}

/// The bracket matching the one under the cursor, or the next one after it.
///
/// The scan is capped by [`MAX_LANDMARK_LINES`], so an unbalanced file cannot
/// make one keystroke walk to the end of a gigabyte.
fn matching_bracket(doc: &Doc, from: Pos) -> Pos {
    // The bracket under the cursor if there is one, else the next one along
    // the line — the way `%` behaves in vim.
    let text = doc.line_text(from.line).unwrap_or_default();
    let start = text
        .chars()
        .enumerate()
        .skip(from.col)
        .find(|(_, c)| bracket_pair(*c).is_some())
        .map(|(i, _)| Pos::new(from.line, i));
    let Some(start) = start else {
        return from;
    };
    let opening = char_at(doc, start).expect("a bracket was found there");
    let (open, close) = bracket_pair(opening).expect("bracket_pair answered for it");
    let forward = is_opener(opening);
    // The scan counts its own bracket up and the opposite one down, whichever
    // direction it walks, so it needs no special case for a closer.
    let (mine, other) = if forward {
        (open, close)
    } else {
        (close, open)
    };

    let mut depth: i32 = 0;
    let mut at = start;
    loop {
        if let Some(c) = char_at(doc, at) {
            if c == mine {
                depth += 1;
            } else if c == other {
                depth -= 1;
                if depth == 0 {
                    return at;
                }
            }
        }
        match step_char(doc, at, forward) {
            Some(next) => at = next,
            None => return from,
        }
        if at.line.abs_diff(start.line) > MAX_LANDMARK_LINES {
            return from;
        }
    }
}

/// What a text object names, independent of where inside it the cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Word,
    BigWord,
    Line,
    Paragraph,
    Sentence,
    Paren,
    Bracket,
    Brace,
    DoubleQuote,
    SingleQuote,
    Backtick,
    Tag,
}

impl Object {
    /// What the object is called, for messages.
    pub fn label(self) -> &'static str {
        match self {
            Object::Word => "word",
            Object::BigWord => "WORD",
            Object::Line => "line",
            Object::Paragraph => "paragraph",
            Object::Sentence => "sentence",
            Object::Paren => "parentheses",
            Object::Bracket => "brackets",
            Object::Brace => "braces",
            Object::DoubleQuote => "double quotes",
            Object::SingleQuote => "single quotes",
            Object::Backtick => "backticks",
            Object::Tag => "tag",
        }
    }
}

/// The range an object names from `from`, ordered and half-open.
///
/// `around` keeps the delimiters — the parentheses, the quotes, the line's
/// terminator — and `inside` leaves them out. `None` means the cursor is not
/// inside one of these, which is what makes `Leader d i t` on plain prose say
/// so instead of deleting nothing quietly.
pub fn object_range(
    doc: &Doc,
    from: Pos,
    object: Object,
    around: bool,
    count: u32,
) -> Option<(Pos, Pos)> {
    Some(match object {
        Object::Word => word_object(doc, from, false, around, count),
        Object::BigWord => word_object(doc, from, true, around, count),
        Object::Line => line_object(doc, from, around, count),
        Object::Paragraph => paragraph_object(doc, from, around),
        Object::Sentence => sentence_object(doc, from),
        Object::Paren => enclosing(doc, from, '(', ')', around)?,
        Object::Bracket => enclosing(doc, from, '[', ']', around)?,
        Object::Brace => enclosing(doc, from, '{', '}', around)?,
        Object::DoubleQuote => quote_object(doc, from, '"', around)?,
        Object::SingleQuote => quote_object(doc, from, '\'', around)?,
        Object::Backtick => quote_object(doc, from, '`', around)?,
        Object::Tag => tag_object(doc, from, around)?,
    })
}

/// A word, or `count` words; the whole run the cursor is in, or the next one.
///
/// `around` takes the whitespace after it too, which is what makes `Leader d a
/// w` close the gap it leaves instead of leaving two spaces where a word was.
fn word_object(doc: &Doc, from: Pos, big: bool, around: bool, count: u32) -> (Pos, Pos) {
    let text = doc.line_text(from.line).unwrap_or_default();
    let chars: Vec<char> = text.chars().collect();
    let mut start = from.col.min(chars.len());
    while start < chars.len() && class(chars[start], big) == Class::Space {
        start += 1;
    }
    if start >= chars.len() {
        // Nothing ahead on this line: the word behind the cursor, if any.
        let mut end = from.col.min(chars.len());
        while end > 0 && class(chars[end - 1], big) == Class::Space {
            end -= 1;
        }
        if end == 0 {
            return (from, from);
        }
        let kind = class(chars[end - 1], big);
        let mut begin = end;
        while begin > 0 && class(chars[begin - 1], big) == kind {
            begin -= 1;
        }
        return (Pos::new(from.line, begin), Pos::new(from.line, end));
    }

    let kind = class(chars[start], big);
    // The cursor may be in the middle of the run, so the object starts where
    // the run does, not where the cursor does.
    let mut begin = start;
    while begin > 0 && class(chars[begin - 1], big) == kind {
        begin -= 1;
    }
    let mut end = start;
    while end < chars.len() && class(chars[end], big) == kind {
        end += 1;
    }
    // A count takes that many words, the gap between them included.
    let mut taken = 1;
    while taken < count.max(1) {
        let mut peek = end;
        while peek < chars.len() && class(chars[peek], big) == Class::Space {
            peek += 1;
        }
        if peek >= chars.len() {
            break;
        }
        let next = class(chars[peek], big);
        let mut stop = peek;
        while stop < chars.len() && class(chars[stop], big) == next {
            stop += 1;
        }
        end = stop;
        taken += 1;
    }
    if around {
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    }
    (Pos::new(from.line, begin), Pos::new(from.line, end))
}

/// One line, or `count` lines.
fn line_object(doc: &Doc, from: Pos, around: bool, count: u32) -> (Pos, Pos) {
    let mut last = from.line.saturating_add(count.max(1) as u64 - 1);
    while last > from.line && doc.line_text(last).is_none() {
        last -= 1;
    }
    let finish = Pos::new(last, line_len(doc, last));
    let start = Pos::new(from.line, 0);
    if !around {
        return (start, finish);
    }
    // Around the line means its terminator as well, so deleting leaves no
    // empty line behind.
    if doc.line_text(last + 1).is_some() {
        (start, Pos::new(last + 1, 0))
    } else {
        (start, finish)
    }
}

/// The paragraph the cursor is in: its first line to its last non-blank one.
fn paragraph_object(doc: &Doc, from: Pos, around: bool) -> (Pos, Pos) {
    let mut first = from.line;
    while first > 0 && !is_blank(doc, first - 1) {
        first -= 1;
    }
    let mut last = from.line;
    while doc.line_text(last + 1).is_some() && !is_blank(doc, last + 1) {
        last += 1;
    }
    let start = Pos::new(first, 0);
    if !around {
        return (start, Pos::new(last, line_len(doc, last)));
    }
    if doc.line_text(last + 1).is_some() {
        (start, Pos::new(last + 1, 0))
    } else {
        (start, Pos::new(last, line_len(doc, last)))
    }
}

/// The sentence the cursor is in.
fn sentence_object(doc: &Doc, from: Pos) -> (Pos, Pos) {
    let start = sentence_back(doc, from);
    let end = sentence_forward(doc, start);
    (start, end)
}

/// The innermost pair of `open`/`close` around the cursor.
///
/// The search walks backwards for an opener that is not already closed, then
/// forwards for its partner. Both walks are capped by
/// [`MAX_LANDMARK_LINES`], so an unbalanced file answers "not inside one"
/// instead of walking a gigabyte.
fn enclosing(doc: &Doc, from: Pos, open: char, close: char, around: bool) -> Option<(Pos, Pos)> {
    let mut depth: i32 = 0;
    let mut at = from;
    let mut first = true;
    let start = loop {
        match char_at(doc, at) {
            Some(c) if c == close && !first => depth += 1,
            Some(c) if c == open => {
                if depth == 0 {
                    break at;
                }
                depth -= 1;
            }
            _ => {}
        }
        first = false;
        at = step_char(doc, at, false)?;
    };
    let inside_start = step_char(doc, start, true)?;

    let mut depth: i32 = 0;
    let mut at = start;
    loop {
        match char_at(doc, at) {
            Some(c) if c == open => depth += 1,
            Some(c) if c == close => {
                depth -= 1;
                if depth == 0 {
                    if around {
                        return Some((start, step_char(doc, at, true)?));
                    }
                    return Some((inside_start, at));
                }
            }
            _ => {}
        }
        at = step_char(doc, at, true)?;
        if at.line.abs_diff(start.line) > MAX_LANDMARK_LINES {
            return None;
        }
    }
}

/// The text between a pair of quotes on the cursor's line.
///
/// Quotes pair off from the left, and a quote behind a backslash is escaped
/// text rather than a delimiter.
fn quote_object(doc: &Doc, from: Pos, quote: char, around: bool) -> Option<(Pos, Pos)> {
    let text = doc.line_text(from.line)?;
    let chars: Vec<char> = text.chars().collect();
    let marks: Vec<usize> = chars
        .iter()
        .enumerate()
        .filter(|(i, c)| **c == quote && (*i == 0 || chars[i - 1] != '\\'))
        .map(|(i, _)| i)
        .collect();
    for pair in marks.chunks(2) {
        if pair.len() < 2 {
            return None;
        }
        let (open, close) = (pair[0], pair[1]);
        if from.col > open && from.col <= close {
            return Some(if around {
                (Pos::new(from.line, open), Pos::new(from.line, close + 1))
            } else {
                (Pos::new(from.line, open + 1), Pos::new(from.line, close))
            });
        }
    }
    None
}

/// A `<tag …> … </tag>` pair around the cursor.
fn tag_object(doc: &Doc, from: Pos, around: bool) -> Option<(Pos, Pos)> {
    let (open_start, open_end, name) = enclosing_tag(doc, from)?;
    let close_start = closing_tag(doc, open_end, &name)?;
    if !around {
        return Some((open_end, close_start));
    }
    // Around the tag means both delimiters: `</name>` and its `>`.
    let mut at = close_start;
    while at != open_end {
        if char_at(doc, at) == Some('>') {
            return Some((open_start, step_char(doc, at, true)?));
        }
        at = step_char(doc, at, true)?;
    }
    None
}

/// Reads a tag whose `<` is at `at`.
///
/// Returns where the tag ends (just past its `>`), its name, whether it closes
/// a tag, and whether it closes itself.
fn read_tag(doc: &Doc, at: Pos) -> Option<(Pos, String, bool, bool)> {
    if char_at(doc, at) != Some('<') {
        return None;
    }
    let mut cursor = step_char(doc, at, true)?;
    let mut body = String::new();
    let mut scanned = 0;
    while let Some(c) = char_at(doc, cursor) {
        if c == '>' {
            let closing = body.starts_with('/');
            let self_closing = body.trim_end().ends_with('/');
            let name: String = body
                .trim_start_matches('/')
                .trim()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == ':' || *c == '_')
                .collect();
            if name.is_empty() {
                return None;
            }
            return Some((step_char(doc, cursor, true)?, name, closing, self_closing));
        }
        body.push(c);
        scanned += 1;
        if scanned > 500 {
            return None;
        }
        cursor = step_char(doc, cursor, true)?;
    }
    None
}

/// The nearest tag opener at or before the cursor that is still open there.
fn enclosing_tag(doc: &Doc, from: Pos) -> Option<(Pos, Pos, String)> {
    let mut at = from;
    let mut budget = 0u32;
    loop {
        if char_at(doc, at) == Some('<')
            && let Some((end, name, closing, self_closing)) = read_tag(doc, at)
            && !closing
            && !self_closing
            && closing_tag(doc, end, &name).is_some_and(|close| from < close)
        {
            return Some((at, end, name));
        }
        at = step_char(doc, at, false)?;
        budget += 1;
        if budget > 4_000 {
            return None;
        }
    }
}

/// Where the `</name>` that closes an opener ends up.
fn closing_tag(doc: &Doc, from: Pos, name: &str) -> Option<Pos> {
    let mut at = from;
    let mut budget = 0u32;
    loop {
        if char_at(doc, at) == Some('<')
            && let Some((_, found, closing, _)) = read_tag(doc, at)
            && closing
            && found == name
        {
            return Some(at);
        }
        at = step_char(doc, at, true)?;
        budget += 1;
        if budget > 200_000 {
            return None;
        }
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
    fn paragraph_motions_land_on_blank_lines() {
        let d = doc("paras.txt", "one\ntwo\n\nthree\nfour\n");
        assert_eq!(target(&d, at(0, 0), Motion::ParagraphForward), at(2, 0));
        assert_eq!(target(&d, at(4, 0), Motion::ParagraphBack), at(2, 0));
        // From the blank line itself, backward has only the top of the file
        // left to offer: the blank line above it is the same one.
        assert_eq!(target(&d, at(2, 0), Motion::ParagraphBack), at(0, 0));
        // Nowhere above the first paragraph: the top of the file.
        assert_eq!(target(&d, at(1, 0), Motion::ParagraphBack), at(0, 0));
    }

    #[test]
    fn a_paragraph_motion_without_one_runs_to_the_end() {
        let d = doc("paras-none.txt", "one\ntwo\n");
        assert_eq!(target(&d, at(0, 0), Motion::ParagraphForward), at(1, 3));
    }

    #[test]
    fn sentence_motions_follow_terminators() {
        let d = doc("sentences.txt", "One. Two! Three\n");
        assert_eq!(target(&d, at(0, 0), Motion::SentenceForward), at(0, 5));
        assert_eq!(target(&d, at(0, 5), Motion::SentenceForward), at(0, 10));
        // On a sentence start, a second `(` moves a second sentence back.
        assert_eq!(target(&d, at(0, 5), Motion::SentenceBack), at(0, 0));
        assert_eq!(target(&d, at(0, 10), Motion::SentenceBack), at(0, 5));
    }

    #[test]
    fn a_full_stop_inside_a_number_is_not_a_terminator() {
        let d = doc("sentences-num.txt", "pi is 3.14159 and more\n");
        assert_eq!(target(&d, at(0, 0), Motion::SentenceForward), at(0, 22));
    }

    #[test]
    fn sentence_motion_crosses_a_line_break() {
        let d = doc("sentences-lines.txt", "no stop here\nNext one. End\n");
        assert_eq!(target(&d, at(0, 0), Motion::SentenceForward), at(1, 10));
        // `Next one.` and `no stop here` are one sentence: nothing between
        // them ends it, so the sentence before `End` starts at the top.
        assert_eq!(target(&d, at(1, 10), Motion::SentenceBack), at(0, 0));
    }

    #[test]
    fn the_matching_bracket_motion_finds_the_partner() {
        let d = doc("brackets.txt", "fn f(a, [b]) { g() }\n");
        assert_eq!(target(&d, at(0, 4), Motion::MatchingBracket), at(0, 11));
        assert_eq!(target(&d, at(0, 11), Motion::MatchingBracket), at(0, 4));
        assert_eq!(target(&d, at(0, 13), Motion::MatchingBracket), at(0, 19));
        // A cursor just before a bracket searches forward for the next one.
        assert_eq!(target(&d, at(0, 5), Motion::MatchingBracket), at(0, 10));
    }

    #[test]
    fn matching_bracket_reaches_across_lines() {
        let d = doc("brackets-lines.txt", "if (a\n  && b) {\n}\n");
        assert_eq!(target(&d, at(0, 3), Motion::MatchingBracket), at(1, 6));
        assert_eq!(target(&d, at(1, 6), Motion::MatchingBracket), at(0, 3));
        assert_eq!(target(&d, at(1, 8), Motion::MatchingBracket), at(2, 0));
    }

    #[test]
    fn matching_bracket_without_a_partner_stays_put() {
        let d = doc("brackets-open.txt", "unclosed (\n");
        assert_eq!(target(&d, at(0, 9), Motion::MatchingBracket), at(0, 9));
        let plain = doc("brackets-none.txt", "nothing here\n");
        assert_eq!(target(&plain, at(0, 0), Motion::MatchingBracket), at(0, 0));
    }

    fn range(
        d: &Doc,
        at_line: u64,
        at_col: usize,
        object: Object,
        around: bool,
    ) -> Option<(Pos, Pos)> {
        object_range(d, Pos::new(at_line, at_col), object, around, 1)
    }

    #[test]
    fn the_word_object_is_the_run_under_the_cursor() {
        let d = doc("obj-word.txt", "one two three\n");
        assert_eq!(
            range(&d, 0, 5, Object::Word, false),
            Some((at(0, 4), at(0, 7)))
        );
        assert_eq!(
            range(&d, 0, 0, Object::Word, false),
            Some((at(0, 0), at(0, 3)))
        );
        assert_eq!(
            range(&d, 0, 5, Object::Word, true),
            Some((at(0, 4), at(0, 8)))
        );
    }

    #[test]
    fn a_word_object_takes_the_previous_word_from_the_end_of_the_line() {
        let d = doc("obj-word-end.txt", "one two   \n");
        assert_eq!(
            range(&d, 0, 10, Object::Word, false),
            Some((at(0, 4), at(0, 7)))
        );
    }

    #[test]
    fn a_line_object_takes_the_terminator_only_when_around() {
        let d = doc("obj-line.txt", "alpha\nbeta\n");
        assert_eq!(
            range(&d, 0, 2, Object::Line, false),
            Some((at(0, 0), at(0, 5)))
        );
        assert_eq!(
            range(&d, 0, 2, Object::Line, true),
            Some((at(0, 0), at(1, 0)))
        );
        // The last line has no terminator to take.
        assert_eq!(
            range(&d, 1, 2, Object::Line, true),
            Some((at(1, 0), at(1, 4)))
        );
    }

    #[test]
    fn the_paragraph_object_is_the_whole_block() {
        let d = doc("obj-para.txt", "one\ntwo\n\nthree\n");
        assert_eq!(
            range(&d, 0, 1, Object::Paragraph, false),
            Some((at(0, 0), at(1, 3)))
        );
    }

    #[test]
    fn the_paren_object_is_the_innermost_pair() {
        let d = doc("obj-paren.txt", "f(a, b) + g(c)\n");
        assert_eq!(
            range(&d, 0, 3, Object::Paren, false),
            Some((at(0, 2), at(0, 6)))
        );
        assert_eq!(
            range(&d, 0, 3, Object::Paren, true),
            Some((at(0, 1), at(0, 7)))
        );
        // A nested pair wins over the one around it.
        let nested = doc("obj-paren-nested.txt", "((x))\n");
        assert_eq!(
            range(&nested, 0, 2, Object::Paren, false),
            Some((at(0, 2), at(0, 3)))
        );
        assert_eq!(
            range(&nested, 0, 2, Object::Paren, true),
            Some((at(0, 1), at(0, 4)))
        );
    }

    #[test]
    fn objects_that_are_not_around_the_cursor_are_none() {
        let d = doc("obj-none.txt", "plain prose\n");
        assert_eq!(range(&d, 0, 2, Object::Paren, false), None);
        assert_eq!(range(&d, 0, 2, Object::Brace, false), None);
        assert_eq!(range(&d, 0, 2, Object::DoubleQuote, false), None);
        assert_eq!(range(&d, 0, 2, Object::Tag, false), None);
    }

    #[test]
    fn the_quote_object_pairs_quotes_from_the_left() {
        let d = doc("obj quotes.txt", "say \"hi there\" now\n");
        assert_eq!(
            range(&d, 0, 7, Object::DoubleQuote, false),
            Some((at(0, 5), at(0, 13)))
        );
        assert_eq!(
            range(&d, 0, 7, Object::DoubleQuote, true),
            Some((at(0, 4), at(0, 14)))
        );
    }

    #[test]
    fn an_escaped_quote_is_text_not_a_delimiter() {
        let d = doc("obj-escaped.txt", "say \"a\\\"b\" now\n");
        assert_eq!(
            range(&d, 0, 6, Object::DoubleQuote, false),
            Some((at(0, 5), at(0, 9)))
        );
    }

    #[test]
    fn the_tag_object_is_the_element() {
        let d = doc("obj-tag.txt", "<b>bold</b> tail\n");
        assert_eq!(
            range(&d, 0, 4, Object::Tag, false),
            Some((at(0, 3), at(0, 7)))
        );
        assert_eq!(
            range(&d, 0, 4, Object::Tag, true),
            Some((at(0, 0), at(0, 11)))
        );
    }

    #[test]
    fn horizontal_motions_stay_inside_the_line() {
        let d = doc("h.txt", "ab\ncd\n");
        assert_eq!(target(&d, at(0, 0), Motion::Left), at(0, 0));
        assert_eq!(target(&d, at(0, 1), Motion::Right), at(0, 2));
        assert_eq!(target(&d, at(0, 2), Motion::Right), at(0, 2));
    }
}
