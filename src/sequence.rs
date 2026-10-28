//! Leader sequences: the third layer of the keymap.
//!
//! `Ctrl+Space` starts a sequence; a mnemonic completes it. The grammar lives
//! here, apart from the editor, so it is testable without a terminal: a string
//! of characters goes in, an [`Action`] comes out.
//!
//! A sequence is a chord, not a mode. Nothing about editing changes while one
//! is pending, and the next key either completes the sequence or abandons it.

use crate::motion::{Motion, Object};

/// A command that needs no argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Save,
    SaveAs,
    OpenFile,
    RecentFiles,
    AlternateFile,
    SelectTheme,
    Quit,
    ForceQuit,
    Undo,
    Redo,
    ToggleGutter,
    ToggleRelativeGutter,
    ToggleWordWrap,
    ToggleScrollbar,
    GoToLine,
    Find,
    Replace,
    NextMatch,
    PreviousMatch,
    /// Puts one indent level in front of the cursor's line.
    IndentLine,
    /// Takes one indent level off the front of the cursor's line.
    UnindentLine,
    /// Comments the touched lines, or uncomments them (`Ctrl+/`).
    ///
    /// It has no leader mnemonic of its own: the chord is the binding, and
    /// the palette reaches it through this name.
    ToggleLineComment,
    /// Moves the current line or selection up one line (`Alt+Up`).
    MoveLineUp,
    /// Moves the current line or selection down one line (`Alt+Down`).
    MoveLineDown,
    /// Word auto-completion from the current buffer (`Ctrl+N`).
    CompleteWord,
    /// Trims trailing whitespace from all lines.
    TrimTrailingWhitespace,
    /// Deletes the current line entirely and pulls the next line up (`Ctrl+K`).
    DeleteLine,
    /// Deletes from the cursor to the end of the line (`Ctrl+Alt+K`).
    DeleteToEndOfLine,
    /// Duplicates the current line or selection down (`Shift+Alt+Down`, `Ctrl+D`).
    DuplicateLine,
    /// Duplicates the current line or selection up (`Shift+Alt+Up`).
    DuplicateLineUp,
    /// Joins the current line with the next, or joins selected lines (`Ctrl+J`, `Leader J`).
    JoinLines,
    /// Toggles git diff gutter signs.
    ToggleGitDiff,
    /// Refreshes git diff gutter signs from git HEAD.
    RefreshGitDiff,
    /// Jumps to the next git change hunk (`Alt+]`).
    NextGitChange,
    /// Jumps to the previous git change hunk (`Alt+[`).
    PreviousGitChange,
    /// Toggles highlighting occurrences of the word under cursor.
    ToggleWordHighlight,
    /// Toggles Zen (distraction-free) mode (`Shift+F11`, `Leader z`).
    ToggleZenMode,
    /// The help list (`Leader ?`).
    Help,
    /// The command palette (`F1`, `Leader p`).
    Palette,
}

impl Command {
    /// The name `zeno.toml` uses for this command.
    pub fn name(self) -> &'static str {
        match self {
            Command::Save => "save",
            Command::SaveAs => "save_as",
            Command::OpenFile => "open_file",
            Command::RecentFiles => "recent_files",
            Command::AlternateFile => "alternate_file",
            Command::SelectTheme => "select_theme",
            Command::Quit => "quit",
            Command::ForceQuit => "force_quit",
            Command::Undo => "undo",
            Command::Redo => "redo",
            Command::ToggleGutter => "toggle_line_numbers",
            Command::ToggleRelativeGutter => "toggle_relative_line_numbers",
            Command::ToggleWordWrap => "toggle_word_wrap",
            Command::ToggleScrollbar => "toggle_scrollbar",
            Command::GoToLine => "go_to_line",
            Command::Find => "find",
            Command::Replace => "replace",
            Command::NextMatch => "next_match",
            Command::PreviousMatch => "previous_match",
            Command::IndentLine => "indent_line",
            Command::UnindentLine => "unindent_line",
            Command::ToggleLineComment => "toggle_line_comment",
            Command::MoveLineUp => "move_line_up",
            Command::MoveLineDown => "move_line_down",
            Command::CompleteWord => "complete_word",
            Command::TrimTrailingWhitespace => "trim_trailing_whitespace",
            Command::DeleteLine => "delete_line",
            Command::DeleteToEndOfLine => "delete_to_end_of_line",
            Command::DuplicateLine => "duplicate_line",
            Command::DuplicateLineUp => "duplicate_line_up",
            Command::JoinLines => "join_lines",
            Command::ToggleGitDiff => "toggle_git_diff",
            Command::RefreshGitDiff => "refresh_git_diff",
            Command::NextGitChange => "next_git_change",
            Command::PreviousGitChange => "previous_git_change",
            Command::ToggleWordHighlight => "toggle_word_highlight",
            Command::ToggleZenMode => "toggle_zen_mode",
            Command::Help => "help",
            Command::Palette => "palette",
        }
    }

    /// The command a name stands for.
    ///
    /// Spaces, dashes and underscores are the same separator, so `go to line`,
    /// `go-to-line` and `go_to_line` are one name: the point is that a person
    /// writing a config file should not have to guess the spelling.
    pub fn from_name(name: &str) -> Option<Self> {
        let normalised: String = name
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c == ' ' || c == '-' { '_' } else { c })
            .collect();
        Some(match normalised.as_str() {
            "save" => Command::Save,
            "save_as" | "saveas" => Command::SaveAs,
            "open_file" | "open" | "file_finder" => Command::OpenFile,
            "recent_files" | "recent" | "buffers" | "recent_files_picker" => Command::RecentFiles,
            "alternate_file" | "switch_buffer" | "toggle_buffer" | "other_file" => {
                Command::AlternateFile
            }
            "select_theme" | "theme" | "theme_picker" | "change_theme" => Command::SelectTheme,
            "quit" => Command::Quit,
            "force_quit" | "quit_without_saving" => Command::ForceQuit,
            "undo" => Command::Undo,
            "redo" => Command::Redo,
            "toggle_line_numbers" | "line_numbers" => Command::ToggleGutter,
            "toggle_relative_line_numbers" | "relative_line_numbers" => {
                Command::ToggleRelativeGutter
            }
            "toggle_word_wrap" | "word_wrap" => Command::ToggleWordWrap,
            "toggle_scrollbar" | "scrollbar" => Command::ToggleScrollbar,
            "go_to_line" | "goto_line" => Command::GoToLine,
            "find" => Command::Find,
            "replace" => Command::Replace,
            "next_match" => Command::NextMatch,
            "previous_match" => Command::PreviousMatch,
            "indent_line" => Command::IndentLine,
            "unindent_line" => Command::UnindentLine,
            "toggle_line_comment" | "comment" => Command::ToggleLineComment,
            "move_line_up" | "move_up" => Command::MoveLineUp,
            "move_line_down" | "move_down" => Command::MoveLineDown,
            "complete_word" | "complete" | "autocomplete" => Command::CompleteWord,
            "trim_trailing_whitespace" | "trim_whitespace" | "trim" => {
                Command::TrimTrailingWhitespace
            }
            "delete_line" | "cut_line" | "kill_line" => Command::DeleteLine,
            "delete_to_end_of_line" | "delete_to_eol" | "kill_to_end" => Command::DeleteToEndOfLine,
            "duplicate_line"
            | "duplicate"
            | "duplicate_down"
            | "duplicate_line_down"
            | "copy_line_down" => Command::DuplicateLine,
            "duplicate_line_up" | "duplicate_up" | "copy_line_up" => Command::DuplicateLineUp,
            "join_lines" | "join" | "join_line" => Command::JoinLines,
            "toggle_git_diff" | "toggle_git_gutter" | "git_diff" => Command::ToggleGitDiff,
            "refresh_git_diff" | "refresh_git" | "git_refresh" => Command::RefreshGitDiff,
            "next_git_change" | "next_change" | "next_hunk" => Command::NextGitChange,
            "previous_git_change"
            | "prev_git_change"
            | "prev_change"
            | "previous_hunk"
            | "prev_hunk" => Command::PreviousGitChange,
            "toggle_word_highlight" | "toggle_highlight_word" | "word_highlight" => {
                Command::ToggleWordHighlight
            }
            "toggle_zen_mode"
            | "zen_mode"
            | "zen"
            | "distraction_free"
            | "toggle_distraction_free" => Command::ToggleZenMode,
            "help" => Command::Help,
            "palette" | "command_palette" => Command::Palette,
            _ => return None,
        })
    }
}

/// The half of a two-part sequence that says what happens to a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Select,
    /// Adds one indent level to every line the range touches.
    Indent,
    /// Removes one indent level from every line the range touches.
    Unindent,
}

impl Operator {
    /// What the operator does, for messages.
    pub fn label(self) -> &'static str {
        match self {
            Operator::Delete => "delete",
            Operator::Change => "change",
            Operator::Yank => "yank",
            Operator::Select => "select",
            Operator::Indent => "indent",
            Operator::Unindent => "unindent",
        }
    }
}

/// What an operator acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Motion(Motion),
    /// The line the cursor is on, terminator included.
    Line,
    /// A text object: `i` selects inside it, `a` selects around it.
    Object {
        object: Object,
        around: bool,
    },
}

/// A complete sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Command(Command),
    Apply {
        operator: Operator,
        target: Target,
        count: u32,
    },
    /// Remember the cursor's position under a name (`Leader m <mark>`).
    SetMark(char),
    /// Return to the position remembered under a name (`Leader ' <mark>`).
    JumpToMark(char),
}

/// What a fed key produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The key is part of the sequence and the sequence is still open.
    Pending(&'static str),
    /// The sequence completed.
    Action(Action),
    /// The key abandoned the sequence.
    Cancelled,
}

/// The longest repeat count a sequence accepts.
///
/// A count is a multiplier applied to a motion, so an unbounded one is a way
/// to ask for a very long loop. The cap is far past any count a person types.
const MAX_COUNT: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Leader,
    Operator(Operator),
    /// Waiting for the object after `i` or `a`.
    Object {
        operator: Operator,
        around: bool,
    },
    /// Waiting for the mark's name, after `m` or `'`.
    Mark {
        set: bool,
    },
}

/// The leader-sequence state machine.
#[derive(Debug, Clone)]
pub struct Sequences {
    stage: Stage,
    count: u32,
}

impl Default for Sequences {
    fn default() -> Self {
        Self::new()
    }
}

impl Sequences {
    pub fn new() -> Self {
        Self {
            stage: Stage::Idle,
            count: 0,
        }
    }

    /// True when no sequence is in progress.
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    /// True when the sequence has collected digits but no operator yet.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Opens a sequence with the leader key.
    pub fn start(&mut self) {
        self.stage = Stage::Leader;
        self.count = 0;
    }

    /// Abandons the sequence without running anything.
    pub fn cancel(&mut self) {
        self.stage = Stage::Idle;
        self.count = 0;
    }

    /// Feeds one key into the sequence.
    pub fn feed(&mut self, key: char) -> Step {
        match self.stage {
            Stage::Idle => {
                // Nothing is pending, so no key belongs to a sequence.
                Step::Pending("")
            }
            Stage::Leader => self.feed_leader(key),
            Stage::Operator(operator) => self.feed_operator(operator, key),
            Stage::Object { operator, around } => self.feed_object(operator, around, key),
            Stage::Mark { set } => {
                self.cancel();
                if is_mark(key) {
                    Step::Action(if set {
                        Action::SetMark(key)
                    } else {
                        Action::JumpToMark(key)
                    })
                } else {
                    Step::Cancelled
                }
            }
        }
    }

    fn feed_leader(&mut self, key: char) -> Step {
        if let Some(digit) = key.to_digit(10) {
            self.count = (self.count.saturating_mul(10) + digit).min(MAX_COUNT);
            return Step::Pending("leader count…");
        }

        let command = match key {
            's' => Some(Command::Save),
            'S' => Some(Command::SaveAs),
            'o' => Some(Command::OpenFile),
            'b' => Some(Command::RecentFiles),
            't' => Some(Command::SelectTheme),
            'q' => Some(Command::Quit),
            'Q' => Some(Command::ForceQuit),
            'u' => Some(Command::Undo),
            'U' => Some(Command::Redo),
            ',' => Some(Command::ToggleGutter),
            '.' => Some(Command::ToggleRelativeGutter),
            'w' => Some(Command::ToggleWordWrap),
            '|' => Some(Command::ToggleScrollbar),
            'g' => Some(Command::GoToLine),
            'f' => Some(Command::Find),
            'r' => Some(Command::Replace),
            'n' => Some(Command::NextMatch),
            'N' => Some(Command::PreviousMatch),
            ']' => Some(Command::IndentLine),
            '[' => Some(Command::UnindentLine),
            'J' | 'j' => Some(Command::JoinLines),
            'z' | 'Z' => Some(Command::ToggleZenMode),
            '?' => Some(Command::Help),
            'p' => Some(Command::Palette),
            _ => None,
        };
        if let Some(command) = command {
            self.cancel();
            return Step::Action(Action::Command(command));
        }

        // `m <mark>` names a spot; `' <mark>` returns to it. The name is
        // collected by the next key, whichever it is.
        if key == 'm' || key == '\'' {
            self.stage = Stage::Mark { set: key == 'm' };
            return Step::Pending("mark: which one?");
        }

        let operator = match key {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            'v' => Some(Operator::Select),
            '>' => Some(Operator::Indent),
            '<' => Some(Operator::Unindent),
            _ => None,
        };
        match operator {
            Some(operator) => {
                self.stage = Stage::Operator(operator);
                Step::Pending(pending_hint(operator))
            }
            None => {
                self.cancel();
                Step::Cancelled
            }
        }
    }

    /// Collects the object's name, after `i` or `a`.
    fn feed_object(&mut self, operator: Operator, around: bool, key: char) -> Step {
        let count = self.count.max(1);
        self.cancel();
        match object_of(key) {
            Some(object) => Step::Action(Action::Apply {
                operator,
                target: Target::Object { object, around },
                count,
            }),
            None => Step::Cancelled,
        }
    }

    fn feed_operator(&mut self, operator: Operator, key: char) -> Step {
        // `i` selects inside a text object and `a` around it, so the
        // operator's next key is a name rather than a motion.
        if key == 'i' || key == 'a' {
            self.stage = Stage::Object {
                operator,
                around: key == 'a',
            };
            return Step::Pending(if key == 'a' {
                "around what?"
            } else {
                "inside what?"
            });
        }
        // Repeating the operator means the line, the way `dd` and `yy` do.
        let target = if repeats(operator, key) {
            Some(Target::Line)
        } else {
            motion_of(key).map(Target::Motion)
        };
        match target {
            Some(target) => {
                let count = self.count.max(1);
                self.cancel();
                Step::Action(Action::Apply {
                    operator,
                    target,
                    count,
                })
            }
            None => {
                self.cancel();
                Step::Cancelled
            }
        }
    }
}

/// Whether `key` can name a mark.
///
/// Letters and digits: enough names for any file, and every one of them is
/// a key the hand can find without looking.
fn is_mark(key: char) -> bool {
    key.is_ascii_alphanumeric()
}

fn repeats(operator: Operator, key: char) -> bool {
    match operator {
        Operator::Delete => key == 'd',
        Operator::Change => key == 'c',
        Operator::Yank => key == 'y',
        Operator::Indent => key == '>',
        Operator::Unindent => key == '<',
        // Selecting the line is what `Shift+Down` already does; a repeated
        // `v` has nothing left to mean.
        Operator::Select => false,
    }
}

fn pending_hint(operator: Operator) -> &'static str {
    match operator {
        Operator::Delete => "delete: motion?",
        Operator::Change => "change: motion?",
        Operator::Yank => "yank: motion?",
        Operator::Select => "select: motion?",
        Operator::Indent => "indent: motion?",
        Operator::Unindent => "unindent: motion?",
    }
}

/// The text object a mnemonic names.
fn object_of(key: char) -> Option<Object> {
    Some(match key {
        'w' => Object::Word,
        'W' => Object::BigWord,
        'l' => Object::Line,
        'p' => Object::Paragraph,
        's' => Object::Sentence,
        '(' | ')' => Object::Paren,
        '[' | ']' => Object::Bracket,
        '{' | '}' => Object::Brace,
        '"' => Object::DoubleQuote,
        '\'' => Object::SingleQuote,
        '`' => Object::Backtick,
        't' => Object::Tag,
        _ => return None,
    })
}

/// The motion a mnemonic names.
///
/// A landmark motion (`}` , `%`) can name a long range in a file with no
/// blanks or brackets in it, so the sequence accepts it and the editor
/// refuses the *range* at the moment it is applied, with a message, once it
/// crosses the affordable span — see `Editor::range_is_affordable`. A motion
/// that names a whole-file range by definition (`G`, `gg`) stays absent:
/// there is no reachable range it could ever be allowed to edit.
fn motion_of(key: char) -> Option<Motion> {
    Some(match key {
        'h' => Motion::Left,
        'l' => Motion::Right,
        'j' => Motion::Down,
        'k' => Motion::Up,
        'w' => Motion::Word,
        'W' => Motion::BigWord,
        'b' => Motion::BackWord,
        'B' => Motion::BackBigWord,
        'e' => Motion::EndOfWord,
        'E' => Motion::EndOfBigWord,
        '$' => Motion::LineEnd,
        '0' => Motion::LineStart,
        '^' => Motion::FirstNonBlank,
        '}' => Motion::ParagraphForward,
        '{' => Motion::ParagraphBack,
        ')' => Motion::SentenceForward,
        '(' => Motion::SentenceBack,
        '%' => Motion::MatchingBracket,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a whole sequence and returns what it produced.
    fn run(keys: &str) -> Step {
        let mut seq = Sequences::new();
        seq.start();
        let mut last = Step::Pending("");
        for key in keys.chars() {
            last = seq.feed(key);
        }
        last
    }

    fn apply(operator: Operator, target: Target, count: u32) -> Step {
        Step::Action(Action::Apply {
            operator,
            target,
            count,
        })
    }

    #[test]
    fn a_command_has_a_name_and_answers_to_it() {
        assert_eq!(Command::ToggleWordWrap.name(), "toggle_word_wrap");
        assert_eq!(
            Command::from_name("Toggle Word Wrap"),
            Some(Command::ToggleWordWrap)
        );
        assert_eq!(Command::from_name("goto-line"), Some(Command::GoToLine));
        assert_eq!(Command::from_name("nonsense"), None);
    }

    #[test]
    fn every_command_name_round_trips() {
        for command in [
            Command::Save,
            Command::SaveAs,
            Command::OpenFile,
            Command::RecentFiles,
            Command::AlternateFile,
            Command::SelectTheme,
            Command::Quit,
            Command::ForceQuit,
            Command::Undo,
            Command::Redo,
            Command::ToggleGutter,
            Command::ToggleRelativeGutter,
            Command::ToggleWordWrap,
            Command::ToggleScrollbar,
            Command::GoToLine,
            Command::Find,
            Command::Replace,
            Command::NextMatch,
            Command::PreviousMatch,
            Command::IndentLine,
            Command::UnindentLine,
            Command::ToggleLineComment,
            Command::MoveLineUp,
            Command::MoveLineDown,
            Command::CompleteWord,
            Command::TrimTrailingWhitespace,
            Command::DeleteLine,
            Command::DeleteToEndOfLine,
            Command::DuplicateLine,
            Command::DuplicateLineUp,
            Command::JoinLines,
            Command::ToggleGitDiff,
            Command::RefreshGitDiff,
            Command::NextGitChange,
            Command::PreviousGitChange,
            Command::ToggleWordHighlight,
            Command::ToggleZenMode,
            Command::Help,
            Command::Palette,
        ] {
            assert_eq!(
                Command::from_name(command.name()),
                Some(command),
                "{}",
                command.name()
            );
        }
    }

    #[test]
    fn a_standalone_mnemonic_is_a_command() {
        assert_eq!(run("s"), Step::Action(Action::Command(Command::Save)));
        assert_eq!(run("S"), Step::Action(Action::Command(Command::SaveAs)));
        assert_eq!(run("o"), Step::Action(Action::Command(Command::OpenFile)));
        assert_eq!(
            run("b"),
            Step::Action(Action::Command(Command::RecentFiles))
        );
        assert_eq!(
            run("t"),
            Step::Action(Action::Command(Command::SelectTheme))
        );
        assert_eq!(
            run(","),
            Step::Action(Action::Command(Command::ToggleGutter))
        );
        assert_eq!(
            run("."),
            Step::Action(Action::Command(Command::ToggleRelativeGutter))
        );
    }

    #[test]
    fn help_and_the_palette_are_standalone_commands() {
        assert_eq!(run("?"), Step::Action(Action::Command(Command::Help)));
        assert_eq!(run("p"), Step::Action(Action::Command(Command::Palette)));
    }

    #[test]
    fn word_wrap_is_a_standalone_command() {
        assert_eq!(
            run("w"),
            Step::Action(Action::Command(Command::ToggleWordWrap))
        );
    }

    #[test]
    fn join_lines_is_a_standalone_command() {
        assert_eq!(run("J"), Step::Action(Action::Command(Command::JoinLines)));
        assert_eq!(run("j"), Step::Action(Action::Command(Command::JoinLines)));
    }

    #[test]
    fn zen_mode_is_a_standalone_command() {
        assert_eq!(
            run("z"),
            Step::Action(Action::Command(Command::ToggleZenMode))
        );
        assert_eq!(
            run("Z"),
            Step::Action(Action::Command(Command::ToggleZenMode))
        );
    }

    #[test]
    fn scrollbar_is_a_standalone_command() {
        assert_eq!(
            run("|"),
            Step::Action(Action::Command(Command::ToggleScrollbar))
        );
    }

    #[test]
    fn the_line_jump_is_a_standalone_command() {
        assert_eq!(run("g"), Step::Action(Action::Command(Command::GoToLine)));
    }

    #[test]
    fn the_search_mnemonics_are_standalone_commands() {
        assert_eq!(run("f"), Step::Action(Action::Command(Command::Find)));
        assert_eq!(run("r"), Step::Action(Action::Command(Command::Replace)));
        assert_eq!(run("n"), Step::Action(Action::Command(Command::NextMatch)));
        assert_eq!(
            run("N"),
            Step::Action(Action::Command(Command::PreviousMatch))
        );
    }

    #[test]
    fn an_operator_waits_for_a_motion() {
        let mut seq = Sequences::new();
        seq.start();
        assert_eq!(seq.feed('d'), Step::Pending("delete: motion?"));
        assert!(!seq.is_idle());
        assert_eq!(
            seq.feed('w'),
            apply(Operator::Delete, Target::Motion(Motion::Word), 1)
        );
        assert!(seq.is_idle(), "a completed sequence leaves nothing pending");
    }

    #[test]
    fn repeating_the_operator_targets_the_line() {
        assert_eq!(run("dd"), apply(Operator::Delete, Target::Line, 1));
        assert_eq!(run("yy"), apply(Operator::Yank, Target::Line, 1));
        assert_eq!(run("cc"), apply(Operator::Change, Target::Line, 1));
    }

    #[test]
    fn select_is_an_operator() {
        assert_eq!(
            run("vw"),
            apply(Operator::Select, Target::Motion(Motion::Word), 1)
        );
    }

    #[test]
    fn indent_and_unindent_are_operators_and_commands() {
        assert_eq!(
            run(">"),
            Step::Pending("indent: motion?"),
            "the operator waits for its motion"
        );
        assert_eq!(
            run(">j"),
            apply(Operator::Indent, Target::Motion(Motion::Down), 1)
        );
        assert_eq!(
            run("<w"),
            apply(Operator::Unindent, Target::Motion(Motion::Word), 1)
        );
        assert_eq!(run(">>"), apply(Operator::Indent, Target::Line, 1));
        assert_eq!(run("]"), Step::Action(Action::Command(Command::IndentLine)));
        assert_eq!(
            run("["),
            Step::Action(Action::Command(Command::UnindentLine))
        );
    }

    fn object(object: Object, around: bool) -> Step {
        Step::Action(Action::Apply {
            operator: Operator::Delete,
            target: Target::Object { object, around },
            count: 1,
        })
    }

    #[test]
    fn text_objects_follow_an_operator() {
        assert_eq!(run("diw"), object(Object::Word, false));
        assert_eq!(run("daw"), object(Object::Word, true));
        assert_eq!(run("dil"), object(Object::Line, false));
        assert_eq!(run("dip"), object(Object::Paragraph, false));
        assert_eq!(run("dis"), object(Object::Sentence, false));
        assert_eq!(run("di("), object(Object::Paren, false));
        assert_eq!(run("da["), object(Object::Bracket, true));
        assert_eq!(run("di{"), object(Object::Brace, false));
        assert_eq!(run("di\""), object(Object::DoubleQuote, false));
        assert_eq!(run("di'"), object(Object::SingleQuote, false));
        assert_eq!(run("di`"), object(Object::Backtick, false));
        assert_eq!(run("dat"), object(Object::Tag, true));
    }

    #[test]
    fn a_text_object_waits_for_its_name() {
        let mut seq = Sequences::new();
        seq.start();
        assert_eq!(seq.feed('d'), Step::Pending("delete: motion?"));
        assert_eq!(seq.feed('i'), Step::Pending("inside what?"));
        assert!(!seq.is_idle(), "the object's name is still being collected");
        assert_eq!(seq.feed('w'), object(Object::Word, false));
        assert!(seq.is_idle());
    }

    #[test]
    fn a_sequence_that_cannot_name_an_object_cancels() {
        assert_eq!(run("diz"), Step::Cancelled);
        assert_eq!(run("caz"), Step::Cancelled);
    }

    #[test]
    fn a_text_object_keeps_the_count() {
        assert_eq!(
            run("2diw"),
            Step::Action(Action::Apply {
                operator: Operator::Delete,
                target: Target::Object {
                    object: Object::Word,
                    around: false
                },
                count: 2,
            })
        );
    }

    #[test]
    fn a_count_multiplies_the_motion() {
        assert_eq!(
            run("3dw"),
            apply(Operator::Delete, Target::Motion(Motion::Word), 3)
        );
    }

    #[test]
    fn a_count_is_capped() {
        let step = run("99999dw");
        match step {
            Step::Action(Action::Apply { count, .. }) => assert_eq!(count, MAX_COUNT),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_mark_takes_a_name() {
        assert_eq!(run("ma"), Step::Action(Action::SetMark('a')));
        assert_eq!(run("'z"), Step::Action(Action::JumpToMark('z')));
    }

    #[test]
    fn a_mark_waits_for_its_name() {
        let mut seq = Sequences::new();
        seq.start();
        assert_eq!(seq.feed('m'), Step::Pending("mark: which one?"));
        assert!(!seq.is_idle(), "the name is still being collected");
        assert_eq!(seq.feed('q'), Step::Action(Action::SetMark('q')));
        assert!(seq.is_idle());
    }

    #[test]
    fn a_key_that_cannot_name_a_mark_cancels() {
        assert_eq!(run("m!"), Step::Cancelled);
        assert_eq!(run("' "), Step::Cancelled);
    }

    #[test]
    fn an_unknown_mnemonic_cancels() {
        assert_eq!(run("x"), Step::Cancelled);
    }

    #[test]
    fn an_unknown_motion_cancels() {
        assert_eq!(run("dz"), Step::Cancelled);
    }

    #[test]
    fn a_motion_outside_the_current_set_cancels() {
        // `G` would name a whole-file range the overlay cannot delete at all.
        assert_eq!(run("dG"), Step::Cancelled);
    }

    #[test]
    fn landmark_motions_are_operator_targets() {
        assert_eq!(
            run("d}"),
            apply(
                Operator::Delete,
                Target::Motion(Motion::ParagraphForward),
                1
            )
        );
        assert_eq!(
            run("y{"),
            apply(Operator::Yank, Target::Motion(Motion::ParagraphBack), 1)
        );
        assert_eq!(
            run("c)"),
            apply(Operator::Change, Target::Motion(Motion::SentenceForward), 1)
        );
        assert_eq!(
            run("v("),
            apply(Operator::Select, Target::Motion(Motion::SentenceBack), 1)
        );
        assert_eq!(
            run("d%"),
            apply(Operator::Delete, Target::Motion(Motion::MatchingBracket), 1)
        );
    }

    #[test]
    fn cancelling_leaves_no_state_behind() {
        let mut seq = Sequences::new();
        seq.start();
        seq.feed('d');
        seq.cancel();
        assert!(seq.is_idle());
        assert_eq!(seq.count(), 0);
        seq.start();
        assert_eq!(
            seq.feed('k'),
            Step::Cancelled,
            "a bare motion is not a command"
        );
    }
}
