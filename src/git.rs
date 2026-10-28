//! Git diff gutter signs.
//!
//! Provides minimal, responsive indicators in the line-number gutter showing
//! lines added, modified, or deleted compared to git `HEAD`.
//!
//! To keep the editor's 16ms keystroke budget and avoid spawning external
//! processes during typing, the baseline content from `HEAD` is fetched
//! asynchronously on a worker thread. Diffing between the in-memory baseline
//! and the current document is done in pure Rust using Myers diff algorithm
//! with common prefix/suffix trimming, completing in microseconds.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use ratatui::style::Color;

use crate::doc::Doc;

/// A diff sign indicating how a line changed compared to git HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSign {
    /// Newly added line.
    Added,
    /// Modified line.
    Modified,
    /// Deleted line (indicator placed on neighboring line).
    Deleted,
}

/// The visual style of git diff gutter symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitGutterStyle {
    /// Subtle vertical bar for added/modified (`▎`) and arrow for deleted (`▾`).
    #[default]
    Bar,
    /// Traditional ASCII signs (`+`, `~`, `-`).
    Ascii,
}

impl std::str::FromStr for GitGutterStyle {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "ascii" => Ok(Self::Ascii),
            _ => Ok(Self::Bar),
        }
    }
}

impl DiffSign {
    /// Returns the 1-column string symbol for this diff sign.
    pub fn symbol(&self, style: GitGutterStyle) -> &'static str {
        match style {
            GitGutterStyle::Bar => match self {
                DiffSign::Added => "▎",
                DiffSign::Modified => "▎",
                DiffSign::Deleted => "▾",
            },
            GitGutterStyle::Ascii => match self {
                DiffSign::Added => "+",
                DiffSign::Modified => "~",
                DiffSign::Deleted => "-",
            },
        }
    }

    /// Color associated with this diff sign.
    pub fn color(&self) -> Color {
        match self {
            DiffSign::Added => Color::Green,
            DiffSign::Modified => Color::Yellow,
            DiffSign::Deleted => Color::Red,
        }
    }
}

/// State managing git diff tracking for an open file.
pub struct GitState {
    /// The root directory of the git repository, if inside one.
    pub repo_root: Option<PathBuf>,
    /// Relative path of the file from `repo_root`.
    pub rel_path: Option<String>,
    /// Hashed lines of the file at git `HEAD`.
    pub base_lines: Option<Arc<Vec<u64>>>,
    /// Receiver for asynchronously fetched baseline.
    pub base_rx: Option<Receiver<Option<Vec<u64>>>>,
    /// Cached diff signs mapped by 0-based document line index.
    pub signs: HashMap<usize, DiffSign>,
    /// Document version that `signs` was computed against.
    pub signs_version: u64,
}

impl GitState {
    /// Initializes git tracking for the given path and starts background fetch.
    pub fn new(path: &Path) -> Self {
        let repo_root = find_repo_root(path);
        let rel_path = repo_root
            .as_ref()
            .and_then(|root| relative_git_path(root, path));

        let mut state = Self {
            repo_root,
            rel_path,
            base_lines: None,
            base_rx: None,
            signs: HashMap::new(),
            signs_version: 0,
        };

        state.spawn_fetch();
        state
    }

    /// Spawns a background thread to fetch the file's HEAD contents from git.
    pub fn spawn_fetch(&mut self) {
        let (root, rel) = match (&self.repo_root, &self.rel_path) {
            (Some(r), Some(p)) => (r.clone(), p.clone()),
            _ => return,
        };

        let (tx, rx) = channel();
        self.base_rx = Some(rx);

        let _ = std::thread::Builder::new()
            .name("zeno-git-diff".into())
            .spawn(move || {
                let lines = fetch_head_lines(&root, &rel);
                let _ = tx.send(lines);
            });
    }

    /// Refreshes the baseline from git.
    pub fn refresh(&mut self) {
        self.spawn_fetch();
    }

    /// Checks if a new baseline arrived, and if needed, recomputes signs against `doc`.
    pub fn update(&mut self, doc: &Doc) {
        if let Some(rx) = &self.base_rx
            && let Ok(res) = rx.try_recv()
        {
            self.base_lines = res.map(Arc::new);
            self.base_rx = None;
            self.signs_version = 0; // force recompute
        }

        let doc_ver = doc.version();
        if self.signs_version != doc_ver {
            if let Some(base) = &self.base_lines {
                let current_hashes = doc_line_hashes(doc);
                self.signs = compute_diff_signs(base, &current_hashes);
            } else {
                self.signs.clear();
            }
            self.signs_version = doc_ver;
        }
    }

    /// Returns the sorted start line numbers of all contiguous change hunks.
    pub fn hunk_starts(&self) -> Vec<usize> {
        if self.signs.is_empty() {
            return Vec::new();
        }
        let mut lines: Vec<usize> = self.signs.keys().copied().collect();
        lines.sort_unstable();

        let mut starts = Vec::new();
        let mut prev = None;
        for line in lines {
            if let Some(p) = prev {
                if line > p + 1 {
                    starts.push(line);
                }
            } else {
                starts.push(line);
            }
            prev = Some(line);
        }
        starts
    }

    /// Finds the target hunk index for moving forward from `current_line`.
    /// Returns `Some((hunk_index, is_wrapped))` or `None` if no changes exist.
    pub fn next_hunk_index(&self, current_line: usize) -> Option<(usize, bool)> {
        let starts = self.hunk_starts();
        if starts.is_empty() {
            return None;
        }
        if starts.len() == 1 && self.is_in_hunk(current_line, starts[0]) {
            return Some((0, false));
        }
        for (i, &start) in starts.iter().enumerate() {
            if start > current_line {
                return Some((i, false));
            }
        }
        Some((0, true))
    }

    /// Finds the target hunk index for moving backward from `current_line`.
    /// Returns `Some((hunk_index, is_wrapped))` or `None` if no changes exist.
    pub fn previous_hunk_index(&self, current_line: usize) -> Option<(usize, bool)> {
        let starts = self.hunk_starts();
        if starts.is_empty() {
            return None;
        }
        if starts.len() == 1 && self.is_in_hunk(current_line, starts[0]) {
            return Some((0, false));
        }
        for (i, &start) in starts.iter().enumerate().rev() {
            if start < current_line {
                return Some((i, false));
            }
        }
        Some((starts.len() - 1, true))
    }

    /// Returns true if `line` is part of the contiguous hunk starting at `hunk_start`.
    pub fn is_in_hunk(&self, line: usize, hunk_start: usize) -> bool {
        if line < hunk_start {
            return false;
        }
        for curr in hunk_start..=line {
            if !self.signs.contains_key(&curr) {
                return false;
            }
        }
        true
    }
}

/// Finds the root directory of the git repository containing `path`.
pub fn find_repo_root(path: &Path) -> Option<PathBuf> {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };

    let start = if abs_path.is_dir() {
        abs_path.as_path()
    } else {
        abs_path.parent()?
    };

    let mut current = start;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Computes the forward-slash separated relative path for git commands.
fn relative_git_path(repo_root: &Path, file_path: &Path) -> Option<String> {
    let abs = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(file_path)
    };

    let rel = abs.strip_prefix(repo_root).ok()?;
    let s = rel.to_str()?;
    Some(s.replace('\\', "/"))
}

/// Fetches the file contents at `HEAD` and returns hashed lines.
fn fetch_head_lines(repo_root: &Path, rel_path: &str) -> Option<Vec<u64>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["show", &format!("HEAD:{}", rel_path)])
        .output();

    match output {
        Ok(out) if out.status.success() => Some(split_and_hash_lines(&out.stdout)),
        Ok(_) => {
            // Check if file is untracked
            if is_untracked(repo_root, rel_path) {
                // An untracked file has an empty baseline: every line is Added
                Some(Vec::new())
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// Checks if a file is untracked in git.
fn is_untracked(repo_root: &Path, rel_path: &str) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["ls-files", rel_path])
        .output();

    match output {
        Ok(out) if out.status.success() => out.stdout.is_empty(),
        _ => false,
    }
}

/// Hashes a single line string, ignoring trailing `\r` and `\n`.
pub fn hash_line(s: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let trimmed = s.trim_end_matches(['\r', '\n']);
    trimmed.hash(&mut hasher);
    hasher.finish()
}

/// Splits raw bytes into lines and hashes each one.
pub fn split_and_hash_lines(bytes: &[u8]) -> Vec<u64> {
    let text = String::from_utf8_lossy(bytes);
    text.lines().map(hash_line).collect()
}

/// Extracts hashed lines from a `Doc`.
pub fn doc_line_hashes(doc: &Doc) -> Vec<u64> {
    let count = doc.line_count() as usize;
    // Cap to 50,000 lines to preserve performance budget on extreme files
    if count > 50_000 {
        return Vec::new();
    }
    let mut hashes = Vec::with_capacity(count);
    for line_no in 0..count {
        if let Some(text) = doc.line_text(line_no as u64) {
            hashes.push(hash_line(&text));
        } else {
            hashes.push(0);
        }
    }
    hashes
}

/// Computes line-level diff signs between baseline lines and current lines.
pub fn compute_diff_signs(base: &[u64], current: &[u64]) -> HashMap<usize, DiffSign> {
    let mut signs = HashMap::new();
    let n = base.len();
    let m = current.len();

    if n == 0 && m == 0 {
        return signs;
    }

    // Common prefix
    let mut prefix = 0;
    while prefix < n && prefix < m && base[prefix] == current[prefix] {
        prefix += 1;
    }

    // Common suffix
    let mut suffix = 0;
    while suffix < (n - prefix)
        && suffix < (m - prefix)
        && base[n - 1 - suffix] == current[m - 1 - suffix]
    {
        suffix += 1;
    }

    let base_slice = &base[prefix..n - suffix];
    let current_slice = &current[prefix..m - suffix];

    let n_mid = base_slice.len();
    let m_mid = current_slice.len();

    if n_mid == 0 && m_mid == 0 {
        return signs;
    }

    if n_mid == 0 {
        // Pure additions
        for i in 0..m_mid {
            signs.insert(prefix + i, DiffSign::Added);
        }
        return signs;
    }

    if m_mid == 0 {
        // Pure deletion
        if prefix < m {
            signs.insert(prefix, DiffSign::Deleted);
        } else if prefix > 0 {
            signs.insert(prefix - 1, DiffSign::Deleted);
        }
        return signs;
    }

    // Myers diff on the middle slice
    let max_d = (n_mid + m_mid).min(500);
    let mut v = vec![0isize; 2 * max_d + 1];
    let mut trace = Vec::with_capacity(max_d + 1);
    let mut found_d = None;

    'outer: for d in 0..=max_d {
        let mut v_copy = v.clone();
        let d_isize = d as isize;
        for k in (-d_isize..=d_isize).step_by(2) {
            let k_idx = (k + max_d as isize) as usize;
            let mut x = if k == -d_isize
                || (k != d_isize
                    && v[(k - 1 + max_d as isize) as usize] < v[(k + 1 + max_d as isize) as usize])
            {
                v[(k + 1 + max_d as isize) as usize]
            } else {
                v[(k - 1 + max_d as isize) as usize] + 1
            };
            let mut y = x - k;
            while (x as usize) < n_mid
                && (y as usize) < m_mid
                && base_slice[x as usize] == current_slice[y as usize]
            {
                x += 1;
                y += 1;
            }
            v_copy[k_idx] = x;
            if x as usize >= n_mid && y as usize >= m_mid {
                trace.push(v_copy);
                found_d = Some(d);
                break 'outer;
            }
        }
        v = v_copy;
        trace.push(v.clone());
    }

    let Some(d_total) = found_d else {
        // Fallback for > 500 diffs: mark overlapping as modified and rest added
        let overlap = n_mid.min(m_mid);
        for i in 0..overlap {
            signs.insert(prefix + i, DiffSign::Modified);
        }
        for i in overlap..m_mid {
            signs.insert(prefix + i, DiffSign::Added);
        }
        return signs;
    };

    // Backtrack to extract edit operations
    let mut x = n_mid as isize;
    let mut y = m_mid as isize;

    #[derive(Debug)]
    enum Edit {
        Delete,
        Insert(usize),
        Equal,
    }
    let mut edits = Vec::new();

    for d in (1..=d_total).rev() {
        let d_isize = d as isize;
        let k = x - y;
        let v_prev = &trace[d - 1];
        let prev_k = if k == -d_isize
            || (k != d_isize
                && v_prev[(k - 1 + max_d as isize) as usize]
                    < v_prev[(k + 1 + max_d as isize) as usize])
        {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v_prev[(prev_k + max_d as isize) as usize];
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y && x > 0 && y > 0 {
            x -= 1;
            y -= 1;
            edits.push(Edit::Equal);
        }

        if prev_x == x {
            y -= 1;
            edits.push(Edit::Insert(y as usize));
        } else {
            x -= 1;
            edits.push(Edit::Delete);
        }
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        edits.push(Edit::Equal);
    }
    edits.reverse();

    // Group contiguous edits into hunks
    let mut i = 0;
    while i < edits.len() {
        match &edits[i] {
            Edit::Equal => {
                i += 1;
            }
            Edit::Delete | Edit::Insert(_) => {
                let mut dels = 0;
                let mut inss = Vec::new();
                while i < edits.len() {
                    match &edits[i] {
                        Edit::Delete => dels += 1,
                        Edit::Insert(curr_idx) => inss.push(*curr_idx),
                        Edit::Equal => break,
                    }
                    i += 1;
                }

                if dels > 0 && !inss.is_empty() {
                    let mod_count = dels.min(inss.len());
                    for &idx in &inss[..mod_count] {
                        signs.insert(prefix + idx, DiffSign::Modified);
                    }
                    for &idx in &inss[mod_count..] {
                        signs.insert(prefix + idx, DiffSign::Added);
                    }
                } else if !inss.is_empty() {
                    for idx in inss {
                        signs.insert(prefix + idx, DiffSign::Added);
                    }
                } else if dels > 0 {
                    // Lines were deleted at this position
                    let pos = if i < edits.len() {
                        // Find the next Insert or Equal position in current_slice
                        let mut next_pos = m_mid;
                        for op in &edits[i..] {
                            if let Edit::Insert(curr_idx) = op {
                                next_pos = *curr_idx;
                                break;
                            }
                        }
                        next_pos
                    } else {
                        m_mid
                    };
                    let line_in_curr = prefix + pos;
                    if line_in_curr < m {
                        signs.insert(line_in_curr, DiffSign::Deleted);
                    } else if line_in_curr > 0 {
                        signs.insert(line_in_curr - 1, DiffSign::Deleted);
                    }
                }
            }
        }
    }

    signs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_line_ignores_crlf() {
        assert_eq!(hash_line("hello\n"), hash_line("hello\r\n"));
        assert_eq!(hash_line("hello"), hash_line("hello\n"));
    }

    #[test]
    fn compute_diff_empty_and_identical() {
        let empty: Vec<u64> = vec![];
        assert!(compute_diff_signs(&empty, &empty).is_empty());

        let lines = vec![hash_line("a"), hash_line("b"), hash_line("c")];
        assert!(compute_diff_signs(&lines, &lines).is_empty());
    }

    #[test]
    fn compute_diff_all_added_for_untracked() {
        let base: Vec<u64> = vec![];
        let curr = vec![hash_line("a"), hash_line("b")];
        let diff = compute_diff_signs(&base, &curr);
        assert_eq!(diff.len(), 2);
        assert_eq!(diff.get(&0), Some(&DiffSign::Added));
        assert_eq!(diff.get(&1), Some(&DiffSign::Added));
    }

    #[test]
    fn compute_diff_single_addition() {
        let base = vec![hash_line("a"), hash_line("c")];
        let curr = vec![hash_line("a"), hash_line("b"), hash_line("c")];
        let diff = compute_diff_signs(&base, &curr);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get(&1), Some(&DiffSign::Added));
    }

    #[test]
    fn compute_diff_single_modification() {
        let base = vec![hash_line("a"), hash_line("b"), hash_line("c")];
        let curr = vec![hash_line("a"), hash_line("b_mod"), hash_line("c")];
        let diff = compute_diff_signs(&base, &curr);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get(&1), Some(&DiffSign::Modified));
    }

    #[test]
    fn compute_diff_deletion_in_middle() {
        let base = vec![hash_line("a"), hash_line("b"), hash_line("c")];
        let curr = vec![hash_line("a"), hash_line("c")];
        let diff = compute_diff_signs(&base, &curr);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get(&1), Some(&DiffSign::Deleted));
    }

    #[test]
    fn compute_diff_deletion_at_eof() {
        let base = vec![hash_line("a"), hash_line("b")];
        let curr = vec![hash_line("a")];
        let diff = compute_diff_signs(&base, &curr);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get(&0), Some(&DiffSign::Deleted));
    }

    #[test]
    fn compute_diff_deletion_at_top() {
        let base = vec![hash_line("a"), hash_line("b")];
        let curr = vec![hash_line("b")];
        let diff = compute_diff_signs(&base, &curr);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get(&0), Some(&DiffSign::Deleted));
    }

    #[test]
    fn compute_diff_mixed() {
        let base = vec![hash_line("a"), hash_line("b"), hash_line("d")];
        let curr = vec![
            hash_line("a"),
            hash_line("b_mod"),
            hash_line("b_new"),
            hash_line("d"),
        ];
        let diff = compute_diff_signs(&base, &curr);
        assert_eq!(diff.get(&1), Some(&DiffSign::Modified));
        assert_eq!(diff.get(&2), Some(&DiffSign::Added));
    }

    #[test]
    fn find_repo_root_discovers_git_dir() {
        let current_file = Path::new("src/main.rs");
        let root = find_repo_root(current_file);
        assert!(root.is_some());
        assert!(root.unwrap().join(".git").exists());
    }

    #[test]
    fn hunk_navigation_logic() {
        let mut state = GitState {
            repo_root: None,
            rel_path: None,
            base_lines: None,
            base_rx: None,
            signs: HashMap::new(),
            signs_version: 0,
        };

        // Initially empty
        assert_eq!(state.hunk_starts(), Vec::<usize>::new());
        assert_eq!(state.next_hunk_index(0), None);
        assert_eq!(state.previous_hunk_index(0), None);

        // Hunk 0: lines 10, 11, 12
        state.signs.insert(10, DiffSign::Modified);
        state.signs.insert(11, DiffSign::Added);
        state.signs.insert(12, DiffSign::Added);
        // Hunk 1: line 25
        state.signs.insert(25, DiffSign::Deleted);
        // Hunk 2: lines 40, 41
        state.signs.insert(40, DiffSign::Added);
        state.signs.insert(41, DiffSign::Added);

        assert_eq!(state.hunk_starts(), vec![10, 25, 40]);

        // Forward from line 0 -> Hunk 0 (10)
        assert_eq!(state.next_hunk_index(0), Some((0, false)));
        // Forward from line 10 -> Hunk 1 (25)
        assert_eq!(state.next_hunk_index(10), Some((1, false)));
        // Forward from line 11 (inside Hunk 0) -> Hunk 1 (25)
        assert_eq!(state.next_hunk_index(11), Some((1, false)));
        // Forward from line 25 -> Hunk 2 (40)
        assert_eq!(state.next_hunk_index(25), Some((2, false)));
        // Forward from line 40 -> wraps to Hunk 0 (10)
        assert_eq!(state.next_hunk_index(40), Some((0, true)));
        // Forward from line 50 -> wraps to Hunk 0 (10)
        assert_eq!(state.next_hunk_index(50), Some((0, true)));

        // Backward from line 50 -> Hunk 2 (40)
        assert_eq!(state.previous_hunk_index(50), Some((2, false)));
        // Backward from line 40 -> Hunk 1 (25)
        assert_eq!(state.previous_hunk_index(40), Some((1, false)));
        // Backward from line 25 -> Hunk 0 (10)
        assert_eq!(state.previous_hunk_index(25), Some((0, false)));
        // Backward from line 12 (inside Hunk 0) -> start of Hunk 0 (10)
        assert_eq!(state.previous_hunk_index(12), Some((0, false)));
        // Backward from line 10 -> wraps to Hunk 2 (40)
        assert_eq!(state.previous_hunk_index(10), Some((2, true)));
    }
}
