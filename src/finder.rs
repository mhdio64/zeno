//! The file finder: discover and filter project files for quick switching.
//!
//! Scans the project directory recursively with a depth limit and skips
//! heavy build, vcs, and dependency trees (`.git`, `target`, `node_modules`).

use std::path::{Path, PathBuf};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Hard cap on the number of files scanned to keep memory and latency bounded.
pub const MAX_FILES: usize = 2000;

/// Maximum directory recursion depth.
pub const MAX_DEPTH: usize = 8;

/// Directory names to skip entirely.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    "vendor",
    ".freebuff",
    ".gemini",
    ".idea",
    ".vscode",
    "venv",
    ".venv",
    "__pycache__",
];

/// A single file discovered in the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative path as displayed to the user (using `/`).
    pub display: String,
    /// Absolute or relative path on disk.
    pub path: PathBuf,
    /// Whether this file is among the recently visited files.
    pub is_recent: bool,
}

/// Whether a directory name should be skipped during scan.
fn should_ignore_dir(name: &str) -> bool {
    if IGNORED_DIRS.contains(&name) {
        return true;
    }
    name.starts_with('.') && name != "." && name != ".."
}

/// Scans `root` recursively for files, skipping ignored directories.
pub fn scan_dir(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs_to_visit = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = dirs_to_visit.pop() {
        if files.len() >= MAX_FILES || depth > MAX_DEPTH {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    if !should_ignore_dir(&file_name_str) {
                        subdirs.push((path, depth + 1));
                    }
                } else if file_type.is_file() {
                    let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                    files.push(relative);
                    if files.len() >= MAX_FILES {
                        break;
                    }
                }
            }
        }
        subdirs.sort_by(|a, b| b.0.cmp(&a.0));
        dirs_to_visit.extend(subdirs);
    }

    files.sort();
    files
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

/// The file finder overlay state: query, scanned entries, and current selection.
#[derive(Debug, Clone)]
pub struct FileFinder {
    query: String,
    entries: Vec<FileEntry>,
    matches: Vec<usize>,
    selected: usize,
}

impl Default for FileFinder {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl FileFinder {
    pub fn new(entries: Vec<FileEntry>) -> Self {
        let matches = (0..entries.len()).collect();
        Self {
            query: String::new(),
            entries,
            matches,
            selected: 0,
        }
    }

    /// Scans `root` and builds a finder with all discovered files.
    pub fn scan(root: &Path) -> Self {
        Self::scan_with_recents(root, &[])
    }

    /// Scans `root` and prioritizes recent files at the top of the list.
    pub fn scan_with_recents(root: &Path, recents: &[PathBuf]) -> Self {
        let files = scan_dir(root);
        let mut recent_entries = Vec::new();
        let mut other_entries = Vec::new();

        for rel in files {
            let display = rel.to_string_lossy().replace('\\', "/");
            let full = root.join(&rel);
            let recent_idx = recents
                .iter()
                .position(|r| r == &full || r == &rel || full.ends_with(r));
            let is_recent = recent_idx.is_some();
            let entry = FileEntry {
                display,
                path: full,
                is_recent,
            };
            if let Some(idx) = recent_idx {
                recent_entries.push((idx, entry));
            } else {
                other_entries.push(entry);
            }
        }

        recent_entries.sort_by_key(|&(idx, _)| idx);
        let mut entries: Vec<FileEntry> = recent_entries.into_iter().map(|(_, e)| e).collect();
        entries.extend(other_entries);
        Self::new(entries)
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    pub fn total_files(&self) -> usize {
        self.entries.len()
    }

    pub fn selection(&self) -> usize {
        self.selected
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.matches
            .get(self.selected)
            .map(|&idx| self.entries[idx].path.as_path())
    }

    pub fn selected_entry(&self) -> Option<&FileEntry> {
        self.matches
            .get(self.selected)
            .map(|&idx| &self.entries[idx])
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
            self.matches = (0..self.entries.len()).collect();
            self.selected = 0;
            return;
        }

        let query = self.query.to_lowercase();
        let mut scored: Vec<(u8, usize)> = Vec::new();

        for (idx, entry) in self.entries.iter().enumerate() {
            let lower = entry.display.to_lowercase();
            let file_name = lower.rsplit('/').next().unwrap_or(&lower);

            let bonus = if entry.is_recent { 0 } else { 1 };
            if file_name.contains(&query) {
                scored.push((bonus, idx));
            } else if lower.contains(&query) {
                scored.push((2 + bonus, idx));
            } else if is_subsequence(&query, &lower) {
                scored.push((4 + bonus, idx));
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

/// Renders the file finder panel into screen lines.
pub fn finder_lines(finder: &FileFinder, height: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("open file: ", heading()),
        Span::styled(finder.query().to_owned(), body()),
        Span::styled(
            format!(" ({}/{})", finder.matches().len(), finder.total_files()),
            Style::default().fg(Color::DarkGray),
        ),
    ])];

    if finder.matches().is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching files",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    let room = height.saturating_sub(lines.len()).max(1);
    let first = finder.selection().saturating_sub(room - 1);

    for (offset, &idx) in finder.matches().iter().enumerate().skip(first).take(room) {
        let entry = &finder.entries()[idx];
        let is_selected = offset == finder.selection();
        let style = if is_selected {
            selected_style()
        } else {
            body()
        };

        let mut spans = Vec::new();
        spans.push(Span::styled(if is_selected { "> " } else { "  " }, style));

        if let Some((dir, file)) = entry.display.rsplit_once('/') {
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
            spans.push(Span::styled(entry.display.clone(), file_style));
        }

        if entry.is_recent {
            let badge_style = if is_selected {
                style
            } else {
                Style::default().fg(Color::Cyan)
            };
            spans.push(Span::styled(" [recent]", badge_style));
        }

        lines.push(Line::from(spans));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_files_and_ignores_hidden_and_build_dirs() {
        let dir = std::env::temp_dir().join("zeno-finder-tests-scan");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();

        std::fs::write(dir.join("Cargo.toml"), b"").unwrap();
        std::fs::write(dir.join("src/main.rs"), b"").unwrap();
        std::fs::write(dir.join(".git/config"), b"").unwrap();
        std::fs::write(dir.join("target/debug.exe"), b"").unwrap();
        std::fs::write(dir.join("node_modules/pkg.js"), b"").unwrap();

        let files = scan_dir(&dir);
        let as_strings: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();

        assert!(
            as_strings.contains(&"Cargo.toml".to_string()),
            "Cargo.toml should be found: {as_strings:?}"
        );
        assert!(
            as_strings.contains(&"src/main.rs".to_string()),
            "src/main.rs should be found: {as_strings:?}"
        );
        assert!(
            !as_strings.iter().any(|p| p.contains(".git")),
            ".git must be ignored"
        );
        assert!(
            !as_strings.iter().any(|p| p.contains("target")),
            "target must be ignored"
        );
        assert!(
            !as_strings.iter().any(|p| p.contains("node_modules")),
            "node_modules must be ignored"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_filters_and_scores_matches() {
        let entries = vec![
            FileEntry {
                display: "src/main.rs".into(),
                path: PathBuf::from("src/main.rs"),
                is_recent: false,
            },
            FileEntry {
                display: "src/bin/zeno-bench.rs".into(),
                path: PathBuf::from("src/bin/zeno-bench.rs"),
                is_recent: false,
            },
            FileEntry {
                display: "Cargo.toml".into(),
                path: PathBuf::from("Cargo.toml"),
                is_recent: false,
            },
        ];
        let mut finder = FileFinder::new(entries);
        assert_eq!(finder.matches().len(), 3);
        assert_eq!(finder.selected_path(), Some(Path::new("src/main.rs")));

        // Filter for "bench": should match "src/bin/zeno-bench.rs"
        for c in "bench".chars() {
            finder.insert(c);
        }
        assert_eq!(finder.matches().len(), 1);
        assert_eq!(
            finder.selected_path(),
            Some(Path::new("src/bin/zeno-bench.rs"))
        );

        // Backspace should widen matches
        finder.backspace();
        finder.backspace();
        finder.backspace();
        finder.backspace();
        finder.backspace();
        assert_eq!(finder.matches().len(), 3);

        // Fuzzy subsequence "sm" should match "src/main.rs"
        finder.insert('s');
        finder.insert('m');
        assert!(!finder.matches().is_empty());
        assert_eq!(finder.selected_path(), Some(Path::new("src/main.rs")));
    }

    #[test]
    fn cursor_navigation_and_paging() {
        let entries = vec![
            FileEntry {
                display: "a.txt".into(),
                path: PathBuf::from("a.txt"),
                is_recent: false,
            },
            FileEntry {
                display: "b.txt".into(),
                path: PathBuf::from("b.txt"),
                is_recent: false,
            },
            FileEntry {
                display: "c.txt".into(),
                path: PathBuf::from("c.txt"),
                is_recent: false,
            },
        ];
        let mut finder = FileFinder::new(entries);
        assert_eq!(finder.selection(), 0);

        finder.down();
        assert_eq!(finder.selection(), 1);
        finder.down();
        assert_eq!(finder.selection(), 2);
        finder.down(); // Clamps at bottom
        assert_eq!(finder.selection(), 2);

        finder.up();
        assert_eq!(finder.selection(), 1);
        finder.page_up(10); // Clamps at top
        assert_eq!(finder.selection(), 0);

        finder.page_down(10);
        assert_eq!(finder.selection(), 2);
    }

    #[test]
    fn finder_lines_draws_ui() {
        let entries = vec![
            FileEntry {
                display: "src/main.rs".into(),
                path: PathBuf::from("src/main.rs"),
                is_recent: true,
            },
            FileEntry {
                display: "Cargo.toml".into(),
                path: PathBuf::from("Cargo.toml"),
                is_recent: false,
            },
        ];
        let mut finder = FileFinder::new(entries);
        let initial_lines = finder_lines(&finder, 10);
        assert!(initial_lines[1].to_string().contains("[recent]"));

        for c in "main".chars() {
            finder.insert(c);
        }

        let lines = finder_lines(&finder, 10);
        assert!(lines[0].to_string().contains("open file: main (1/2)"));
        assert!(lines.iter().any(|l| l.to_string().contains("main.rs")));
    }

    #[test]
    fn scan_with_recents_prioritizes_recent_files() {
        let dir = std::env::temp_dir().join("zeno-finder-recents-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("a.txt"), b"").unwrap();
        std::fs::write(dir.join("b.txt"), b"").unwrap();
        std::fs::write(dir.join("c.txt"), b"").unwrap();

        let recents = vec![dir.join("b.txt")];
        let finder = FileFinder::scan_with_recents(&dir, &recents);

        assert_eq!(finder.entries().len(), 3);
        assert_eq!(finder.entries()[0].display, "b.txt");
        assert!(finder.entries()[0].is_recent);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
