//! Leader sequences: the third layer of the keymap.
//!
//! `Ctrl+Space` starts a sequence; a mnemonic completes it. The grammar lives
//! here, apart from the editor, so it is testable without a terminal: a string
//! of characters goes in, an [`Action`] comes out.
//!
//! A sequence is a chord, not a mode. Nothing about editing changes while one
//! is pending, and the next key either completes the sequence or abandons it.

use crate::motion::Motion;

/// A command that needs no argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Save,
    Quit,
    ForceQuit,
    Undo,
    Redo,
    ToggleGutter,
    GoToLine,
}

/// The half of a two-part sequence that says what happens to a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Select,
}

impl Operator {
    /// What the operator does, for messages.
    pub fn label(self) -> &'static str {
        match self {
            Operator::Delete => "delete",
            Operator::Change => "change",
            Operator::Yank => "yank",
            Operator::Select => "select",
        }
    }
}

/// What an operator acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Motion(Motion),
    /// The line the cursor is on, terminator included.
    Line,
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
        }
    }

    fn feed_leader(&mut self, key: char) -> Step {
        if let Some(digit) = key.to_digit(10) {
            self.count = (self.count.saturating_mul(10) + digit).min(MAX_COUNT);
            return Step::Pending("leader count…");
        }

        let command = match key {
            's' => Some(Command::Save),
            'q' => Some(Command::Quit),
            'Q' => Some(Command::ForceQuit),
            'u' => Some(Command::Undo),
            'U' => Some(Command::Redo),
            ',' => Some(Command::ToggleGutter),
            'g' => Some(Command::GoToLine),
            _ => None,
        };
        if let Some(command) = command {
            self.cancel();
            return Step::Action(Action::Command(command));
        }

        let operator = match key {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            'v' => Some(Operator::Select),
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

    fn feed_operator(&mut self, operator: Operator, key: char) -> Step {
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

fn repeats(operator: Operator, key: char) -> bool {
    match operator {
        Operator::Delete => key == 'd',
        Operator::Change => key == 'c',
        Operator::Yank => key == 'y',
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
    }
}

/// The motion a mnemonic names.
///
/// Only motions that stay near the cursor are here. A motion that can name a
/// multi-thousand-line range (`G`, `gg`, `}`) is deliberately absent until the
/// overlay stops paying a superlinear price for such a deletion; see
/// `docs/performance.md`.
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
    fn a_standalone_mnemonic_is_a_command() {
        assert_eq!(run("s"), Step::Action(Action::Command(Command::Save)));
        assert_eq!(run(","), Step::Action(Action::Command(Command::ToggleGutter)));
    }

    #[test]
    fn the_line_jump_is_a_standalone_command() {
        assert_eq!(
            run("g"),
            Step::Action(Action::Command(Command::GoToLine))
        );
    }

    #[test]
    fn an_operator_waits_for_a_motion() {
        let mut seq = Sequences::new();
        seq.start();
        assert_eq!(
            seq.feed('d'),
            Step::Pending("delete: motion?")
        );
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
    fn an_unknown_mnemonic_cancels() {
        assert_eq!(run("z"), Step::Cancelled);
    }

    #[test]
    fn an_unknown_motion_cancels() {
        assert_eq!(run("dz"), Step::Cancelled);
    }

    #[test]
    fn a_motion_outside_the_current_set_cancels() {
        // `G` would name a range the overlay cannot delete cheaply yet.
        assert_eq!(run("dG"), Step::Cancelled);
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
            seq.feed('w'),
            Step::Cancelled,
            "a bare motion is not a command"
        );
    }
}
