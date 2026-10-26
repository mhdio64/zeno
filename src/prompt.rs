//! The prompt line: the editor asking a question.
//!
//! Every command that needs an answer needs somewhere to ask. Without a
//! prompt, each one grows its own input handling — go-to-line, find, replace,
//! save-as, "the file changed on disk, what now?" — and they drift apart.
//! This is that place: a label, an answer, and a caret, all drawn in the
//! status line so the document never moves while a question is open.
//!
//! There are two kinds of question. A **text** question collects a string,
//! which is what a line number or a search pattern is. A **choice** question
//! collects one letter from a fixed set, which is what a decision is. Both are
//! answered with `Enter` or with the letter itself, and both are abandoned
//! with `Esc`.
//!
//! The prompt owns the keyboard while it is open (ADR 0014): the leader does
//! not start, chords do not fire, and every printable key is an answer.

/// Why the prompt is open, which decides what happens to the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    GoToLine,
    Search,
    /// The pattern to look for, before asking what to put in its place.
    Replace,
    /// The text to put in the pattern's place.
    ReplaceWith,
    ChangedOnDisk,
}

impl Purpose {
    /// The label drawn in front of the question.
    pub fn label(self) -> &'static str {
        match self {
            Purpose::GoToLine => "Go to line: ",
            Purpose::Search => "Find: ",
            Purpose::Replace => "Replace: ",
            Purpose::ReplaceWith => "With: ",
            Purpose::ChangedOnDisk => "Changed on disk",
        }
    }
}

/// What kind of answer the prompt accepts.
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Free text, edited with the caret.
    Text,
    /// One letter from a fixed set, each with a description.
    Choice(&'static [(char, &'static str)]),
}

/// A labelled question.
#[derive(Debug, Clone)]
pub struct Prompt {
    purpose: Purpose,
    kind: Kind,
    text: String,
    /// Caret position, counted in characters.
    cursor: usize,
    /// A note drawn after the answer, such as the reason the answer is not
    /// yet good enough. It sits past the caret, so typing is not disturbed
    /// by it.
    note: Option<String>,
}

impl Prompt {
    /// A prompt that collects a line of text.
    pub fn input(purpose: Purpose) -> Self {
        Self {
            purpose,
            kind: Kind::Text,
            text: String::new(),
            cursor: 0,
            note: None,
        }
    }

    /// A prompt that collects a line of text, starting with `text`.
    ///
    /// The caret sits at the end, so typing continues the answer rather than
    /// replacing it.
    pub fn with_text(purpose: Purpose, text: &str) -> Self {
        let mut prompt = Self::input(purpose);
        prompt.text = text.to_owned();
        prompt.cursor = prompt.len();
        prompt
    }

    /// A prompt that collects one of `options`, each a key and a description.
    pub fn choice(purpose: Purpose, options: &'static [(char, &'static str)]) -> Self {
        Self {
            purpose,
            kind: Kind::Choice(options),
            text: String::new(),
            cursor: 0,
            note: None,
        }
    }

    /// Attaches a note to the question, or removes it.
    ///
    /// The note explains why the answer is not accepted yet — a pattern that
    /// does not compile, say. It is drawn after the answer, so the caret
    /// still sits exactly where the user is typing.
    pub fn set_note(&mut self, note: Option<String>) {
        self.note = note;
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    pub fn is_text(&self) -> bool {
        matches!(self.kind, Kind::Text)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The byte offset of a character position, for editing the string.
    fn byte_of(&self, col: usize) -> usize {
        self.text
            .char_indices()
            .nth(col)
            .map_or(self.text.len(), |(byte, _)| byte)
    }

    /// Appends to the answer. A choice prompt has nothing to append to.
    pub fn insert(&mut self, c: char) {
        if !self.is_text() {
            return;
        }
        let at = self.byte_of(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    /// Removes the character before the caret. Returns whether it removed one.
    pub fn backspace(&mut self) -> bool {
        if !self.is_text() || self.cursor == 0 {
            return false;
        }
        let from = self.byte_of(self.cursor - 1);
        let to = self.byte_of(self.cursor);
        self.text.replace_range(from..to, "");
        self.cursor -= 1;
        true
    }

    /// Removes the character under the caret.
    pub fn delete(&mut self) -> bool {
        if !self.is_text() || self.cursor >= self.len() {
            return false;
        }
        let from = self.byte_of(self.cursor);
        let to = self.byte_of(self.cursor + 1);
        self.text.replace_range(from..to, "");
        true
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    /// The option `c` names, if this is a choice prompt and `c` is one of its
    /// keys.
    pub fn choose(&self, c: char) -> Option<char> {
        match self.kind {
            Kind::Choice(options) => options
                .iter()
                .find(|(key, _)| *key == c)
                .map(|(key, _)| *key),
            Kind::Text => None,
        }
    }

    /// The line as it is drawn, without the leading space.
    fn body(&self) -> String {
        match self.kind {
            Kind::Text => match &self.note {
                Some(note) => format!("{}{} — {note}", self.purpose.label(), self.text),
                None => format!("{}{}", self.purpose.label(), self.text),
            },
            Kind::Choice(options) => {
                let listed: Vec<String> = options
                    .iter()
                    .map(|(key, description)| format!("[{key}]{description}"))
                    .collect();
                format!("{} — {}", self.purpose.label(), listed.join("  "))
            }
        }
    }

    /// The line as it is drawn: a leading space, then the question.
    pub fn line(&self) -> String {
        format!(" {}", self.body())
    }

    /// The screen column of the caret within [`Prompt::line`], or `None` when
    /// the question is answered with a letter rather than typed.
    pub fn cursor_column(&self) -> Option<u16> {
        match self.kind {
            Kind::Text => Some((1 + self.purpose.label().chars().count() + self.cursor) as u16),
            Kind::Choice(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTIONS: &[(char, &str)] = &[('r', "reload"), ('o', "overwrite"), ('c', "cancel")];

    fn typed(text: &str) -> Prompt {
        let mut prompt = Prompt::input(Purpose::GoToLine);
        for c in text.chars() {
            prompt.insert(c);
        }
        prompt
    }

    #[test]
    fn typing_builds_the_line() {
        let prompt = typed("120");
        assert_eq!(prompt.text(), "120");
        assert_eq!(prompt.cursor(), 3);
        assert_eq!(prompt.line(), " Go to line: 120");
        // one leading space + "Go to line: " (12) + three typed characters
        assert_eq!(prompt.cursor_column(), Some(16));
    }

    #[test]
    fn a_note_is_drawn_after_the_answer() {
        let mut prompt = Prompt::input(Purpose::Search);
        for c in "ab".chars() {
            prompt.insert(c);
        }
        prompt.set_note(Some("invalid pattern".into()));
        assert_eq!(prompt.line(), " Find: ab — invalid pattern");
        assert_eq!(
            prompt.cursor_column(),
            Some(9),
            "the caret stays where the user is typing"
        );
        prompt.set_note(None);
        assert_eq!(prompt.line(), " Find: ab");
    }

    #[test]
    fn backspace_removes_before_the_caret() {
        let mut prompt = typed("42");
        assert!(prompt.backspace());
        assert_eq!(prompt.text(), "4");
        assert!(prompt.backspace());
        assert!(!prompt.backspace(), "nothing left to remove");
        assert_eq!(prompt.text(), "");
    }

    #[test]
    fn delete_removes_under_the_caret() {
        let mut prompt = typed("423");
        prompt.home();
        prompt.right();
        assert!(prompt.delete());
        assert_eq!(prompt.text(), "43");
        assert_eq!(prompt.cursor(), 1);
    }

    #[test]
    fn the_caret_moves_within_the_line_only() {
        let mut prompt = typed("ab");
        prompt.home();
        prompt.left();
        assert_eq!(prompt.cursor(), 0);
        prompt.end();
        prompt.right();
        assert_eq!(prompt.cursor(), 2, "the caret cannot pass the end");
    }

    #[test]
    fn typing_in_the_middle_inserts_at_the_caret() {
        let mut prompt = typed("ac");
        prompt.left();
        prompt.insert('b');
        assert_eq!(prompt.text(), "abc");
        assert_eq!(prompt.cursor(), 2);
    }

    #[test]
    fn a_caret_over_non_ascii_text_moves_one_character() {
        let mut prompt = typed("سلام");
        assert_eq!(prompt.cursor(), 4, "four characters, not eight bytes");
        assert!(
            prompt.backspace(),
            "removes the last character, not the last byte"
        );
        assert_eq!(prompt.text(), "سلا");
        assert_eq!(prompt.cursor(), 3);
    }

    #[test]
    fn a_choice_lists_its_options_instead_of_a_caret() {
        let prompt = Prompt::choice(Purpose::ChangedOnDisk, OPTIONS);
        assert!(!prompt.is_text());
        assert_eq!(
            prompt.line(),
            " Changed on disk — [r]reload  [o]overwrite  [c]cancel"
        );
        assert_eq!(prompt.cursor_column(), None);
    }

    #[test]
    fn only_listed_keys_are_answers() {
        let prompt = Prompt::choice(Purpose::ChangedOnDisk, OPTIONS);
        assert_eq!(prompt.choose('r'), Some('r'));
        assert_eq!(prompt.choose('x'), None);
        assert_eq!(prompt.choose('O'), None, "answers are the lowercase keys");
    }

    #[test]
    fn a_choice_has_no_text_to_edit() {
        let mut prompt = Prompt::choice(Purpose::ChangedOnDisk, OPTIONS);
        prompt.insert('r');
        assert!(!prompt.backspace(), "a choice has no text to delete");
        assert_eq!(prompt.text(), "");
        assert_eq!(prompt.len(), 0);
    }
}
