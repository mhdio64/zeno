//! Progressive line index over a memory-mapped file.
//!
//! The index records the byte offset at which every discovered line starts.
//! Discovery is incremental: asking for line *n* scans only as far as needed
//! to reach it, and a background thread keeps scanning ahead so that later
//! lookups are cheap. Nothing in the index assumes the whole file has been
//! read — the mapped bytes are already resident, so "reading" means scanning
//! for newlines, and that can be spread over time.

use std::sync::Mutex;

use memchr::memchr_iter;

/// How much of the file one index step scans. Sized so that a single step
/// finds more lines than fit on any terminal, without meaningfully delaying
/// the foreground.
const SCAN_CHUNK: usize = 64 * 1024;

#[derive(Debug)]
struct State {
    /// Byte offset of the start of each discovered line. Always non-empty
    /// for a non-empty file: line 0 starts at offset 0.
    starts: Vec<u64>,
    /// How far into the file the index has scanned for newlines.
    scanned_to: u64,
    /// Total length of the file in bytes.
    len: u64,
}

impl State {
    /// Scans up to `budget` bytes starting at `scanned_to`, recording the
    /// start of every line it finds. Returns the number of lines discovered.
    fn scan(&mut self, data: &[u8], budget: usize) -> usize {
        let start = self.scanned_to as usize;
        if start >= self.len as usize {
            return 0;
        }
        let end = ((start.saturating_add(budget)) as u64).min(self.len) as usize;
        let mut found = 0usize;
        for pos in memchr_iter(b'\n', &data[start..end]) {
            let next = (start + pos) as u64 + 1;
            // A newline on the very last byte terminates the last line rather
            // than starting a new one.
            if next < self.len {
                self.starts.push(next);
            }
            found += 1;
        }
        self.scanned_to = end as u64;
        found
    }
}

/// A lazily-populated index of line start offsets.
///
/// Cloning is not implemented on purpose: the index is shared between the
/// foreground (which queries it to render) and the background scanner (which
/// fills it in), and every method takes `&self`.
#[derive(Debug)]
pub struct LineIndex {
    state: Mutex<State>,
}

impl LineIndex {
    pub fn new(len: u64) -> Self {
        let starts = if len == 0 { Vec::new() } else { vec![0] };
        Self {
            state: Mutex::new(State {
                starts,
                scanned_to: 0,
                len,
            }),
        }
    }

    /// Number of lines discovered so far.
    ///
    /// This is a lower bound while the file is still being scanned: the final
    /// line only becomes countable once the scan reaches the end of the file.
    pub fn known_lines(&self) -> u64 {
        self.state.lock().expect("line index poisoned").starts.len() as u64
    }

    /// Total number of lines, once the whole file has been scanned.
    pub fn total_lines(&self) -> Option<u64> {
        let s = self.state.lock().expect("line index poisoned");
        if s.scanned_to == s.len {
            Some(s.starts.len() as u64)
        } else {
            None
        }
    }

    /// Byte offset at which line `n` starts, scanning forward if needed.
    ///
    /// Returns `None` when the file ends before line `n` exists.
    pub fn line_start(&self, data: &[u8], n: u64) -> Option<u64> {
        let mut s = self.state.lock().expect("line index poisoned");
        while (s.starts.len() as u64) <= n {
            if s.scanned_to >= s.len {
                return None;
            }
            s.scan(data, SCAN_CHUNK);
        }
        Some(s.starts[n as usize])
    }

    /// Byte range of line `n`, excluding its trailing newline.
    ///
    /// The range always ends at a newline or at the end of the file.
    pub fn line_span(&self, data: &[u8], n: u64) -> Option<(u64, u64)> {
        let mut s = self.state.lock().expect("line index poisoned");
        // We need the start of line n+1 to know where line n ends, unless the
        // file ends first.
        while (s.starts.len() as u64) <= n + 1 && s.scanned_to < s.len {
            s.scan(data, SCAN_CHUNK);
        }
        if (s.starts.len() as u64) <= n {
            return None;
        }
        let start = s.starts[n as usize];
        let end = if (s.starts.len() as u64) > n + 1 {
            s.starts[(n + 1) as usize]
        } else {
            s.len
        };
        Some((start, end))
    }

    /// Scans up to `budget` more bytes. Returns `true` once the whole file
    /// has been scanned.
    pub fn scan_more(&self, data: &[u8], budget: usize) -> bool {
        let mut s = self.state.lock().expect("line index poisoned");
        if s.scanned_to >= s.len {
            return true;
        }
        s.scan(data, budget);
        s.scanned_to >= s.len
    }

    /// True when the whole file has been scanned.
    pub fn is_complete(&self) -> bool {
        let s = self.state.lock().expect("line index poisoned");
        s.scanned_to == s.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_of(text: &[u8]) -> LineIndex {
        LineIndex::new(text.len() as u64)
    }

    #[test]
    fn counts_lines() {
        let data = b"a\nb\nc\n";
        let ix = index_of(data);
        assert_eq!(ix.total_lines(), None); // not scanned yet
        assert_eq!(ix.line_span(data, 0), Some((0, 2)));
        assert_eq!(ix.line_span(data, 1), Some((2, 4)));
        assert_eq!(ix.line_span(data, 2), Some((4, 6)));
        assert_eq!(ix.line_span(data, 3), None);
    }

    #[test]
    fn file_without_trailing_newline() {
        let data = b"a\nb";
        let ix = index_of(data);
        assert_eq!(ix.line_span(data, 0), Some((0, 2)));
        assert_eq!(ix.line_span(data, 1), Some((2, 3)));
        assert_eq!(ix.line_span(data, 2), None);
    }

    #[test]
    fn empty_file_has_no_lines() {
        let data = b"";
        let ix = index_of(data);
        assert_eq!(ix.line_span(data, 0), None);
        assert_eq!(ix.total_lines(), Some(0));
    }

    #[test]
    fn single_line_no_newline() {
        let data = b"hello";
        let ix = index_of(data);
        assert_eq!(ix.line_span(data, 0), Some((0, 5)));
        assert_eq!(ix.total_lines(), Some(1));
    }

    #[test]
    fn total_lines_known_after_full_scan() {
        let data = b"one\ntwo\nthree\n";
        let ix = index_of(data);
        assert!(ix.scan_more(data, 1024));
        assert_eq!(ix.total_lines(), Some(3));
    }

    #[test]
    fn known_lines_grows_as_scan_advances() {
        let data = b"a\n".repeat(1000);
        let ix = index_of(&data);
        let mut last = 0;
        while !ix.scan_more(&data, 64) {
            let n = ix.known_lines();
            assert!(n >= last);
            last = n;
        }
        assert_eq!(ix.total_lines(), Some(1000));
    }
}
