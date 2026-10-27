//! Recent files and buffer switching: track visited files and their viewport states.
//!
//! Maintains an MRU (Most Recently Used) list of files with their last known
//! cursor position and viewport scroll. Supports interactive filtering and
//! rapid switching between alternate buffers.

use std::path::{Path, PathBuf};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Hard cap on the number of remembered recent files.
pub const MAX_RECENT: usize = 50;

/// A recorded file entry with its last known cursor and viewport position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentFile {
    pub path: PathBuf,
    pub cursor_line: u64,
    pub cursor_col: usize,
    pub top_line: u64,
    pub left_col: usize,
}

/// The collection of most recently used files, sorted newest-first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentList {
    files: Vec<RecentFile>,
}

impl RecentList {
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Records or updates a file's viewport state and moves it to the front of MRU.
    pub fn record(
        &mut self,
        path: PathBuf,
        cursor_line: u64,
        cursor_col: usize,
        top_line: u64,
        left_col: usize,
    ) {
        if path.as_os_str().is_empty() {
            return;
        }

        // Remove if previously recorded so it can be moved to the front.
        if let Some(pos) = self.files.iter().position(|f| f.path == path) {
            self.files.remove(pos);
        }

        self.files.insert(
            0,
            RecentFile {
                path,
                cursor_line,
                cursor_col,
                top_line,
                left_col,
            },
        );

        if self.files.len() > MAX_RECENT {
            self.files.truncate(MAX_RECENT);
        }
    }

    /// Looks up the saved viewport state for `path`.
    pub fn get(&self, path: &Path) -> Option<&RecentFile> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Returns the most recently visited file that is not `current`.
    pub fn alternate(&self, current: &Path) -> Option<&RecentFile> {
        self.files.iter().find(|f| f.path != current)
    }

    /// An iterator over recent files in MRU order.
    pub fn iter(&self) -> std::slice::Iter<'_, RecentFile> {
        self.files.iter()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// A formatted item presented in the recent files picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentItem {
    pub display: String,
    pub path: PathBuf,
    pub cursor_line: u64,
    pub cursor_col: usize,
    pub is_current: bool,
}

/// Checks whether all characters in `needle` appear in `haystack` in order.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut haystack_chars = haystack.chars();
    for nc in needle.chars() {
        if !haystack_chars.any(|hc| hc == nc) {
            return false;
        }
    }
    true
}

/// Interactive picker state: query, items, filtered matches, and selection.
#[derive(Debug, Clone)]
pub struct RecentPicker {
    query: String,
    items: Vec<RecentItem>,
    matches: Vec<usize>,
    selected: usize,
}

impl Default for RecentPicker {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl RecentPicker {
    pub fn new(items: Vec<RecentItem>) -> Self {
        let matches = (0..items.len()).collect();
        Self {
            query: String::new(),
            items,
            matches,
            selected: 0,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn items(&self) -> &[RecentItem] {
        &self.items
    }

    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    pub fn selection(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&RecentItem> {
        self.matches.get(self.selected).map(|&idx| &self.items[idx])
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

    pub fn page_down(&mut self, step: usize) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = (self.selected + step).min(self.matches.len() - 1);
    }

    pub fn page_up(&mut self, step: usize) {
        self.selected = self.selected.saturating_sub(step);
    }

    fn refilter(&mut self) {
        if self.query.is_empty() {
            self.matches = (0..self.items.len()).collect();
            self.selected = 0;
            return;
        }

        let query = self.query.to_lowercase();
        let mut scored: Vec<(u8, usize)> = Vec::new();

        for (idx, item) in self.items.iter().enumerate() {
            let lower = item.display.to_lowercase();
            let file_name = lower.rsplit('/').next().unwrap_or(&lower);

            if file_name.contains(&query) {
                scored.push((0, idx));
            } else if lower.contains(&query) {
                scored.push((1, idx));
            } else if is_subsequence(&query, &lower) {
                scored.push((2, idx));
            }
        }

        scored.sort_by_key(|&(tier, idx)| (tier, idx));
        self.matches = scored.into_iter().map(|(_, idx)| idx).collect();
        self.selected = 0;
    }
}

fn heading() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn body() -> Style {
    Style::default()
}

fn selected_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Renders the recent files picker panel into screen lines.
pub fn recent_lines(picker: &RecentPicker, height: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("recent files: ", heading()),
        Span::styled(picker.query().to_owned(), body()),
        Span::styled(
            format!(" ({}/{})", picker.matches().len(), picker.items().len()),
            Style::default().fg(Color::DarkGray),
        ),
    ])];

    if picker.matches().is_empty() {
        lines.push(Line::from(Span::styled(
            "  no recent files",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    let room = height.saturating_sub(lines.len()).max(1);
    let first = picker.selection().saturating_sub(room - 1);

    for (offset, &idx) in picker.matches().iter().enumerate().skip(first).take(room) {
        let item = &picker.items()[idx];
        let is_selected = offset == picker.selection();
        let style = if is_selected {
            selected_style()
        } else {
            body()
        };

        let mut spans = Vec::new();
        spans.push(Span::styled(if is_selected { "> " } else { "  " }, style));

        if let Some((dir, file)) = item.display.rsplit_once('/') {
            let dir_style = if is_selected {
                style
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let file_style = if is_selected {
                style
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(format!("{dir}/"), dir_style));
            spans.push(Span::styled(file.to_string(), file_style));
        } else {
            let file_style = if is_selected {
                style
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(item.display.clone(), file_style));
        }

        let line_num_str = format!(":{}", item.cursor_line + 1);
        let pos_style = if is_selected {
            style
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(line_num_str, pos_style));

        if item.is_current {
            let badge_style = if is_selected {
                style
            } else {
                Style::default().fg(Color::Cyan)
            };
            spans.push(Span::styled(" [current]", badge_style));
        }

        lines.push(Line::from(spans));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_list_records_mru_order() {
        let mut list = RecentList::new();
        list.record(PathBuf::from("a.txt"), 10, 2, 5, 0);
        list.record(PathBuf::from("b.txt"), 20, 4, 10, 0);
        list.record(PathBuf::from("c.txt"), 30, 6, 15, 0);

        let paths: Vec<_> = list.iter().map(|f| f.path.to_str().unwrap()).collect();
        assert_eq!(paths, vec!["c.txt", "b.txt", "a.txt"]);

        // Re-recording an existing path moves it to the front and updates its position.
        list.record(PathBuf::from("a.txt"), 99, 1, 50, 0);
        let paths: Vec<_> = list.iter().map(|f| f.path.to_str().unwrap()).collect();
        assert_eq!(paths, vec!["a.txt", "c.txt", "b.txt"]);
        assert_eq!(list.get(Path::new("a.txt")).unwrap().cursor_line, 99);
    }

    #[test]
    fn recent_list_alternate_finds_most_recent_other_file() {
        let mut list = RecentList::new();
        list.record(PathBuf::from("a.txt"), 10, 2, 5, 0);
        list.record(PathBuf::from("b.txt"), 20, 4, 10, 0);

        assert_eq!(
            list.alternate(Path::new("b.txt")).unwrap().path,
            PathBuf::from("a.txt")
        );
        assert_eq!(
            list.alternate(Path::new("a.txt")).unwrap().path,
            PathBuf::from("b.txt")
        );
        assert_eq!(
            list.alternate(Path::new("c.txt")).unwrap().path,
            PathBuf::from("b.txt")
        );
    }

    #[test]
    fn recent_list_respects_capacity_limit() {
        let mut list = RecentList::new();
        for i in 0..(MAX_RECENT + 10) {
            list.record(PathBuf::from(format!("file_{i}.txt")), 0, 0, 0, 0);
        }
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(
            list.iter().next().unwrap().path,
            PathBuf::from(format!("file_{}.txt", MAX_RECENT + 9))
        );
    }

    #[test]
    fn recent_picker_navigation_and_filtering() {
        let items = vec![
            RecentItem {
                display: "src/main.rs".into(),
                path: PathBuf::from("src/main.rs"),
                cursor_line: 42,
                cursor_col: 0,
                is_current: false,
            },
            RecentItem {
                display: "src/finder.rs".into(),
                path: PathBuf::from("src/finder.rs"),
                cursor_line: 10,
                cursor_col: 5,
                is_current: false,
            },
            RecentItem {
                display: "README.md".into(),
                path: PathBuf::from("README.md"),
                cursor_line: 0,
                cursor_col: 0,
                is_current: true,
            },
        ];

        let mut picker = RecentPicker::new(items);
        assert_eq!(picker.selection(), 0);
        assert_eq!(picker.selected_item().unwrap().display, "src/main.rs");

        picker.down();
        assert_eq!(picker.selection(), 1);
        assert_eq!(picker.selected_item().unwrap().display, "src/finder.rs");

        picker.insert('r');
        picker.insert('e');
        picker.insert('a');
        picker.insert('d');
        assert_eq!(picker.matches().len(), 1);
        assert_eq!(picker.selected_item().unwrap().display, "README.md");

        picker.backspace();
        picker.backspace();
        picker.backspace();
        picker.backspace();
        assert_eq!(picker.matches().len(), 3);
    }

    #[test]
    fn recent_lines_renders_header_items_and_current_badge() {
        let items = vec![
            RecentItem {
                display: "src/main.rs".into(),
                path: PathBuf::from("src/main.rs"),
                cursor_line: 41,
                cursor_col: 0,
                is_current: false,
            },
            RecentItem {
                display: "README.md".into(),
                path: PathBuf::from("README.md"),
                cursor_line: 0,
                cursor_col: 0,
                is_current: true,
            },
        ];
        let picker = RecentPicker::new(items);
        let lines = recent_lines(&picker, 10);

        assert_eq!(lines.len(), 3);
        let header_text = lines[0].to_string();
        assert!(header_text.contains("recent files:"));
        assert!(header_text.contains("(2/2)"));

        let first_text = lines[1].to_string();
        assert!(first_text.contains(">"));
        assert!(first_text.contains("src/main.rs:42"));

        let second_text = lines[2].to_string();
        assert!(second_text.contains("README.md:1"));
        assert!(second_text.contains("[current]"));
    }
}
