//! The editor describing itself: the help list and the command palette.
//!
//! Both read the same table. Help prints all of it; the palette filters it by
//! what has been typed. One table means a command cannot be reachable from
//! one and missing from the other, and it means the palette's answer is
//! always a binding help also shows.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::sequence::Command;

/// One command, with the keys that reach it and the name a person would use.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub keys: &'static str,
    pub label: &'static str,
    pub command: Command,
}

/// Every command the palette offers, in the order help lists them.
pub const ENTRIES: &[Entry] = &[
    Entry {
        keys: "ctrl+s / leader s",
        label: "Save",
        command: Command::Save,
    },
    Entry {
        keys: "leader q",
        label: "Quit",
        command: Command::Quit,
    },
    Entry {
        keys: "leader Q",
        label: "Quit without saving",
        command: Command::ForceQuit,
    },
    Entry {
        keys: "leader S",
        label: "Save as",
        command: Command::SaveAs,
    },
    Entry {
        keys: "ctrl+o / leader o",
        label: "Open file",
        command: Command::OpenFile,
    },
    Entry {
        keys: "leader b",
        label: "Recent files",
        command: Command::RecentFiles,
    },
    Entry {
        keys: "ctrl+6",
        label: "Switch to alternate file",
        command: Command::AlternateFile,
    },
    Entry {
        keys: "leader t",
        label: "Select theme",
        command: Command::SelectTheme,
    },
    Entry {
        keys: "ctrl+z / leader u",
        label: "Undo",
        command: Command::Undo,
    },
    Entry {
        keys: "ctrl+y / leader U",
        label: "Redo",
        command: Command::Redo,
    },
    Entry {
        keys: "ctrl+f / leader f",
        label: "Find",
        command: Command::Find,
    },
    Entry {
        keys: "ctrl+h / leader r",
        label: "Replace",
        command: Command::Replace,
    },
    Entry {
        keys: "leader n",
        label: "Next match",
        command: Command::NextMatch,
    },
    Entry {
        keys: "leader N",
        label: "Previous match",
        command: Command::PreviousMatch,
    },
    Entry {
        keys: "ctrl+g / leader g",
        label: "Go to line",
        command: Command::GoToLine,
    },
    Entry {
        keys: "ctrl+/",
        label: "Toggle line comment",
        command: Command::ToggleLineComment,
    },
    Entry {
        keys: "leader ,",
        label: "Toggle line numbers",
        command: Command::ToggleGutter,
    },
    Entry {
        keys: "leader .",
        label: "Toggle relative line numbers",
        command: Command::ToggleRelativeGutter,
    },
    Entry {
        keys: "alt+z / leader w",
        label: "Toggle word wrap",
        command: Command::ToggleWordWrap,
    },
    Entry {
        keys: "leader |",
        label: "Toggle scrollbar",
        command: Command::ToggleScrollbar,
    },
    Entry {
        keys: "ctrl+n",
        label: "Complete word",
        command: Command::CompleteWord,
    },
    Entry {
        keys: "",
        label: "Trim trailing whitespace",
        command: Command::TrimTrailingWhitespace,
    },
    Entry {
        keys: "alt+up",
        label: "Move line up",
        command: Command::MoveLineUp,
    },
    Entry {
        keys: "alt+down",
        label: "Move line down",
        command: Command::MoveLineDown,
    },
    Entry {
        keys: "leader ]",
        label: "Indent line",
        command: Command::IndentLine,
    },
    Entry {
        keys: "leader [",
        label: "Unindent line",
        command: Command::UnindentLine,
    },
    Entry {
        keys: "leader ?",
        label: "Help",
        command: Command::Help,
    },
    Entry {
        keys: "ctrl+p / f1 / leader p",
        label: "Command palette",
        command: Command::Palette,
    },
];

/// The reference rows help prints under the commands: things that are a
/// grammar rather than a single key.
const REFERENCE: &[(&str, &str)] = &[
    (
        "operators",
        "d delete   c change   y copy (yank)   v select   > <  reindent",
    ),
    ("motions", "h l j k   w W b B e E   $ 0 ^   { }   ( )   %"),
    (
        "text objects",
        "i / a then   w W l p s   ( ) [ ] { }   \" ' `   t",
    ),
    ("marks", "leader m <name> sets   leader ' <name> jumps"),
    ("counts", "leader 3 d w deletes three words"),
];

/// Heading style.
fn heading() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn body() -> Style {
    Style::default()
}

fn keys_style() -> Style {
    Style::default().fg(Color::Cyan)
}

fn selected() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// The command palette: a query, the entries it matches, and which of them is
/// under the cursor.
#[derive(Debug, Clone)]
pub struct Palette {
    query: String,
    matches: Vec<usize>,
    selected: usize,
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}

impl Palette {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            matches: (0..ENTRIES.len()).collect(),
            selected: 0,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The entry `Enter` would run.
    pub fn selected(&self) -> Option<&'static Entry> {
        self.matches
            .get(self.selected)
            .map(|index| &ENTRIES[*index])
    }

    /// The indices of the matching entries, in table order.
    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    pub fn selection(&self) -> usize {
        self.selected
    }

    pub fn insert(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if self.selected + 1 < self.matches.len() {
            self.selected += 1;
        }
    }

    /// Recomputes what the query matches, keeping the cursor in range.
    ///
    /// A case-insensitive substring is enough for a table this size: the
    /// query is a reminder of a name, not a program.
    fn refilter(&mut self) {
        let query = self.query.to_lowercase();
        self.matches = ENTRIES
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.label.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
    }
}

/// The palette as screen lines: the query, then the matches.
///
/// The list shows as much as `height` allows and says how many are left, so a
/// small window is never a mystery.
pub fn palette_lines(palette: &Palette, height: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("palette: ", heading()),
        Span::styled(palette.query().to_owned(), body()),
    ])];
    if palette.matches().is_empty() {
        lines.push(Line::from(Span::styled(
            "  no command matches",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    let room = height.saturating_sub(lines.len()).max(1);
    // Keep the selected entry on screen when the list is longer than the
    // window, the way the viewport keeps the cursor.
    let first = palette.selection().saturating_sub(room - 1);
    for (offset, index) in palette.matches().iter().enumerate().skip(first).take(room) {
        let entry = &ENTRIES[*index];
        let style = if offset == palette.selection() {
            selected()
        } else {
            body()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<18}", entry.keys),
                if offset == palette.selection() {
                    style
                } else {
                    keys_style()
                },
            ),
            Span::styled(entry.label, style),
        ]));
    }
    let left = palette.matches().len().saturating_sub(first + room);
    if left > 0 {
        lines.push(Line::from(Span::styled(
            format!("  … {left} more"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

/// How many lines [`help_lines`] would produce without a window.
pub fn help_height() -> usize {
    2 + ENTRIES.len() + 1 + REFERENCE.len() + 2
}

/// The help list: every binding, then the grammar.
///
/// `offset` is the first line shown, so the list scrolls in a short window
/// instead of being cut off.
pub fn help_lines(offset: usize, height: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(" zeno — keys", heading())));
    for entry in ENTRIES {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<20}", entry.keys), keys_style()),
            Span::styled(entry.label, body()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" grammar", heading())));
    for (name, description) in REFERENCE {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<14}", name), keys_style()),
            Span::styled(*description, body()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " esc closes",
        Style::default().fg(Color::DarkGray),
    )));

    // The window shows `height` lines starting at `offset`; the rest is not
    // drawn but is still there, one scroll away.
    let offset = offset.min(lines.len().saturating_sub(1));
    lines.drain(0..offset);
    lines.truncate(height.max(1));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_starts_with_every_command() {
        let palette = Palette::new();
        assert_eq!(palette.matches().len(), ENTRIES.len());
        assert_eq!(palette.selected().map(|entry| entry.label), Some("Save"));
    }

    #[test]
    fn typing_filters_by_name() {
        let mut palette = Palette::new();
        for c in "wrap".chars() {
            palette.insert(c);
        }
        let labels: Vec<&str> = palette
            .matches()
            .iter()
            .map(|index| ENTRIES[*index].label)
            .collect();
        assert_eq!(labels, vec!["Toggle word wrap"]);
    }

    #[test]
    fn filtering_is_case_insensitive() {
        let mut palette = Palette::new();
        for c in "UNDO".chars() {
            palette.insert(c);
        }
        assert_eq!(palette.matches().len(), 1);
        assert_eq!(palette.selected().map(|entry| entry.label), Some("Undo"));
    }

    #[test]
    fn a_query_nobody_matches_selects_nothing() {
        let mut palette = Palette::new();
        for c in "zzz".chars() {
            palette.insert(c);
        }
        assert!(palette.selected().is_none());
    }

    #[test]
    fn the_cursor_walks_the_matches_and_stops_at_the_ends() {
        let mut palette = Palette::new();
        palette.down();
        assert_eq!(palette.selected().map(|entry| entry.label), Some("Quit"));
        palette.up();
        palette.up();
        assert_eq!(palette.selected().map(|entry| entry.label), Some("Save"));
        for _ in 0..ENTRIES.len() * 2 {
            palette.down();
        }
        assert_eq!(
            palette.selected().map(|entry| entry.label),
            ENTRIES.last().map(|entry| entry.label)
        );
    }

    #[test]
    fn backspace_widens_the_search_again() {
        let mut palette = Palette::new();
        for c in "quit".chars() {
            palette.insert(c);
        }
        assert_eq!(palette.matches().len(), 2, "Quit and Quit without saving");
        palette.backspace();
        assert_eq!(palette.matches().len(), 2);
        palette.backspace();
        palette.backspace();
        palette.backspace();
        assert_eq!(palette.matches().len(), ENTRIES.len());
    }

    #[test]
    fn the_palette_draws_its_query_and_matches() {
        let mut palette = Palette::new();
        for c in "save".chars() {
            palette.insert(c);
        }
        let lines = palette_lines(&palette, 10);
        assert!(lines[0].to_string().contains("palette: save"));
        assert!(lines.iter().any(|line| line.to_string().contains("Save")));
    }

    #[test]
    fn help_lists_every_command_and_scrolls() {
        let all = help_lines(0, help_height());
        assert!(all.len() >= ENTRIES.len(), "every command is listed");
        assert!(all.iter().any(|line| line.to_string().contains("grammar")));
        assert!(
            all.iter()
                .any(|line| line.to_string().contains("copy (yank)"))
        );
        let scrolled = help_lines(3, 4);
        assert_eq!(scrolled.len(), 4);
        assert_ne!(
            scrolled[0].to_string(),
            all[0].to_string(),
            "the offset moves the window"
        );
    }
}
