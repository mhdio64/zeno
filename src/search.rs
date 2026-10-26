//! Finding text: a compiled pattern, and a background scan of the document.
//!
//! A document can be a gigabyte, so the foreground never scans all of it.
//! Two scans share the work, and each answers the question it is good at:
//!
//! - The **viewport scan** is the foreground's, and it is small: the
//!   renderer matches the pattern against the lines the user can see, which
//!   is what makes the highlight appear as the pattern is typed rather than
//!   after the file has been read.
//! - The **whole-file scan** is a worker, and it counts: it walks the entire
//!   document in the background, reports how far it has come, and can be
//!   cancelled at any moment. A 1 GB file never freezes the editor, because
//!   the editor is never the thing doing the scanning.
//!
//! The worker is given the bytes and the line index the editor already has,
//! so a match it reports is a match in the document on screen. The overlay
//! is a snapshot, so a scan that finished after the document changed is
//! discarded rather than believed.
//!
//! The pattern is a Rust `regex`. An invalid one is an error the editor
//! shows, never a silent failure: a search that quietly finds nothing looks
//! exactly like a search that found nothing, and the user cannot tell the
//! two apart.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use regex::Regex;

use crate::doc::Snapshot;
use crate::doc::edit::Pos;

/// A match, as a line and a range of character columns.
///
/// Columns count characters, like every other column in the editor, so a
/// match in Persian or emoji text is where the user sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchPos {
    pub line: u64,
    pub start: usize,
    pub end: usize,
}

impl MatchPos {
    /// Whether the match covers the position.
    pub fn contains(&self, pos: Pos) -> bool {
        pos.line == self.line && self.start <= pos.col && pos.col < self.end
    }
}

/// The matches of a pattern in one line of text.
///
/// The columns are character columns, and they are computed as the iterator
/// walks, so matching a long line does not cost one pass per match.
pub struct Matches<'a> {
    text: &'a str,
    inner: Option<regex::Matches<'a, 'a>>,
    /// The character column of `byte_at`.
    char_at: usize,
    /// The byte offset of `char_at`.
    byte_at: usize,
}

impl<'a> Matches<'a> {
    fn empty(text: &'a str) -> Self {
        Self {
            text,
            inner: None,
            char_at: 0,
            byte_at: 0,
        }
    }
}

impl Iterator for Matches<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let found = self.inner.as_mut()?.next()?;
        let start = self.char_at + self.text[self.byte_at..found.start()].chars().count();
        let end = start + found.as_str().chars().count();
        self.byte_at = found.end();
        self.char_at = end;
        Some((start, end))
    }
}

/// A compiled search pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    regex: Regex,
    source: String,
}

impl Pattern {
    /// Compiles a pattern, or explains why it cannot be compiled.
    pub fn compile(source: &str) -> Result<Self, String> {
        match Regex::new(source) {
            Ok(regex) => Ok(Self {
                regex,
                source: source.to_owned(),
            }),
            Err(error) => {
                // The regex error spans several lines; the first one says
                // what is wrong, and the status line has room for one.
                let message = error.to_string();
                let first = message.lines().next().unwrap_or("invalid pattern");
                Err(format!("invalid pattern: {first}"))
            }
        }
    }

    /// The text the pattern was compiled from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The matches in one line of text, as character-column ranges.
    ///
    /// An empty pattern matches nothing, rather than matching the empty
    /// string at every position — which is what the regex engine would do,
    /// and what would turn an empty search box into a screenful of
    /// highlights.
    pub fn matches_in<'r>(&'r self, text: &'r str) -> Matches<'r> {
        if self.source.is_empty() {
            return Matches::empty(text);
        }
        Matches {
            text,
            inner: Some(self.regex.find_iter(text)),
            char_at: 0,
            byte_at: 0,
        }
    }

    /// The matches in one line of text, collected.
    pub fn find_in(&self, text: &str) -> Vec<(usize, usize)> {
        self.matches_in(text).collect()
    }
}

/// What a scan is looking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every match in the document. The scan counts them and reports how far
    /// it has come.
    All,
    /// The first match starting strictly after the position.
    After { pos: Pos },
    /// The first match starting at or after the position.
    From { pos: Pos },
    /// The last match starting strictly before the position.
    Before { pos: Pos },
    /// The last match in the document.
    ///
    /// Finding it means knowing where the document ends, which means the
    /// whole file has to be indexed first. The scan does that itself, in the
    /// background, so the editor never has to wait for it.
    Last,
}

/// Something a scan reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEvent {
    /// A match, in document order.
    Found(MatchPos),
    /// How far the scan has come: the lines it has looked at and the matches
    /// it has found so far.
    Progress { lines: u64, matches: u64 },
    /// The scan is over.
    Finished { matches: u64 },
}

/// A scan running in the background.
pub struct SearchJob {
    cancel: Arc<AtomicBool>,
    receiver: Receiver<SearchEvent>,
    handle: Option<JoinHandle<()>>,
}

impl SearchJob {
    /// Starts a scan.
    pub fn start(context: Snapshot, pattern: Pattern, scope: Scope) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let (sender, receiver) = channel();
        let handle = std::thread::spawn(move || scan(context, pattern, scope, flag, sender));
        Self {
            cancel,
            receiver,
            handle: Some(handle),
        }
    }

    /// The events that have arrived, without blocking.
    pub fn drain(&self) -> Vec<SearchEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
        }
        events
    }

    /// Stops the scan.
    ///
    /// The thread notices the flag between lines; it is joined only if it has
    /// already finished, because a scan stuck on one enormous line must not
    /// be able to hold up the editor.
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take()
            && handle.is_finished()
        {
            let _ = handle.join();
        }
    }
}

impl Drop for SearchJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// How often the scan reports its progress, in lines.
///
/// Often enough that the status line moves, rarely enough that a file with
/// tens of millions of lines does not spend its time in the channel.
const PROGRESS_EVERY: u64 = 8192;

/// Walks the document looking for matches.
fn scan(
    context: Snapshot,
    pattern: Pattern,
    scope: Scope,
    cancel: Arc<AtomicBool>,
    sender: Sender<SearchEvent>,
) {
    match scope {
        Scope::All => scan_all(&context, &pattern, &cancel, &sender),
        Scope::After { pos } | Scope::From { pos } => {
            scan_forward(&context, &pattern, scope, pos, &cancel, &sender)
        }
        Scope::Before { pos } => scan_backward(&context, &pattern, pos, &cancel, &sender),
        Scope::Last => scan_last(&context, &pattern, &cancel, &sender),
    }
}

/// Counts every match in the document.
fn scan_all(
    context: &Snapshot,
    pattern: &Pattern,
    cancel: &AtomicBool,
    sender: &Sender<SearchEvent>,
) {
    let mut matches: u64 = 0;
    let mut line = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some(text) = context.line_text(line) else {
            break;
        };
        matches += pattern.matches_in(&text).count() as u64;
        if line % PROGRESS_EVERY == PROGRESS_EVERY - 1 {
            let _ = sender.send(SearchEvent::Progress {
                lines: line + 1,
                matches,
            });
        }
        line += 1;
    }
    let _ = sender.send(SearchEvent::Finished { matches });
}

/// Finds the first match at or after the position.
///
/// The scan starts at the position's own line and stops at the first match
/// it finds, so the cost is the distance to the next match rather than the
/// size of the file.
fn scan_forward(
    context: &Snapshot,
    pattern: &Pattern,
    scope: Scope,
    pos: Pos,
    cancel: &AtomicBool,
    sender: &Sender<SearchEvent>,
) {
    let strict = matches!(scope, Scope::After { .. });
    let mut line = pos.line;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some(text) = context.line_text(line) else {
            break;
        };
        for (start, end) in pattern.matches_in(&text) {
            // A match on the caret's own line counts only if it starts past
            // the caret; a match on a later line always counts.
            let after = if line > pos.line {
                true
            } else {
                start > pos.col || (start == pos.col && !strict)
            };
            if after {
                let _ = sender.send(SearchEvent::Found(MatchPos { line, start, end }));
                let _ = sender.send(SearchEvent::Finished { matches: 1 });
                return;
            }
        }
        line += 1;
    }
    let _ = sender.send(SearchEvent::Finished { matches: 0 });
}

/// Finds the last match strictly before the position.
///
/// The scan walks backwards from the position's own line, so the cost is the
/// distance to the previous match and not the size of the file.
fn scan_backward(
    context: &Snapshot,
    pattern: &Pattern,
    pos: Pos,
    cancel: &AtomicBool,
    sender: &Sender<SearchEvent>,
) {
    let mut line = pos.line;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some(text) = context.line_text(line) else {
            break;
        };
        let mut closest: Option<MatchPos> = None;
        for (start, end) in pattern.matches_in(&text) {
            if line == pos.line && start >= pos.col {
                break;
            }
            closest = Some(MatchPos { line, start, end });
        }
        if let Some(found) = closest {
            let _ = sender.send(SearchEvent::Found(found));
            let _ = sender.send(SearchEvent::Finished { matches: 1 });
            return;
        }
        if line == 0 {
            break;
        }
        line -= 1;
    }
    let _ = sender.send(SearchEvent::Finished { matches: 0 });
}

/// Finds the last match in the document.
fn scan_last(
    context: &Snapshot,
    pattern: &Pattern,
    cancel: &AtomicBool,
    sender: &Sender<SearchEvent>,
) {
    // The end of the document is only known once the whole file has been
    // indexed, so the scan finishes the index here rather than making the
    // editor wait for it.
    context.index_fully();
    let Some(mut line) = context.last_line() else {
        let _ = sender.send(SearchEvent::Finished { matches: 0 });
        return;
    };
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some(text) = context.line_text(line) else {
            break;
        };
        let mut closest: Option<MatchPos> = None;
        for (start, end) in pattern.matches_in(&text) {
            closest = Some(MatchPos { line, start, end });
        }
        if let Some(found) = closest {
            let _ = sender.send(SearchEvent::Found(found));
            let _ = sender.send(SearchEvent::Finished { matches: 1 });
            return;
        }
        if line == 0 {
            break;
        }
        line -= 1;
    }
    let _ = sender.send(SearchEvent::Finished { matches: 0 });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;
    use std::path::PathBuf;

    fn temp_file(name: &str, contents: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join("zeno-search-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn doc_of(name: &str, contents: &[u8]) -> Doc {
        let path = temp_file(name, contents);
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        doc
    }

    fn context_of(doc: &Doc) -> Snapshot {
        doc.snapshot()
    }

    fn pattern(source: &str) -> Pattern {
        Pattern::compile(source).expect("a valid pattern")
    }

    /// Runs a scan to completion and returns everything it reported.
    fn run(context: Snapshot, pattern: Pattern, scope: Scope) -> Vec<SearchEvent> {
        let job = SearchJob::start(context, pattern, scope);
        let mut events = Vec::new();
        for _ in 0..2000 {
            events.extend(job.drain());
            if events
                .iter()
                .any(|e| matches!(e, SearchEvent::Finished { .. }))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        events
    }

    fn found(events: &[SearchEvent]) -> Vec<MatchPos> {
        events
            .iter()
            .filter_map(|e| match e {
                SearchEvent::Found(m) => Some(*m),
                _ => None,
            })
            .collect()
    }

    fn finished(events: &[SearchEvent]) -> Option<u64> {
        events.iter().find_map(|e| match e {
            SearchEvent::Finished { matches } => Some(*matches),
            _ => None,
        })
    }

    #[test]
    fn an_invalid_pattern_is_an_error() {
        let error = Pattern::compile("a(").unwrap_err();
        assert!(
            error.starts_with("invalid pattern"),
            "{error:?} should say what is wrong"
        );
    }

    #[test]
    fn an_empty_pattern_matches_nothing() {
        let pattern = pattern("");
        assert!(pattern.find_in("anything at all").is_empty());
    }

    #[test]
    fn matches_are_reported_in_character_columns() {
        let pattern = pattern("b");
        assert_eq!(pattern.find_in("abc"), vec![(1, 2)]);
    }

    #[test]
    fn a_match_after_non_ascii_text_is_where_it_looks() {
        let pattern = pattern("x");
        // "سلامx" — the match starts after four characters, not eight bytes.
        assert_eq!(pattern.find_in("سلامx"), vec![(4, 5)]);
    }

    #[test]
    fn a_regex_matches_more_than_a_literal() {
        let pattern = pattern(r"\d+");
        assert_eq!(pattern.find_in("a12b345"), vec![(1, 3), (4, 7)]);
    }

    #[test]
    fn a_whole_file_scan_counts_every_match() {
        let doc = doc_of("scan.txt", b"one\ntwo\nthree\n");
        let events = run(context_of(&doc), pattern("t"), Scope::All);
        // "two" and "three" contain a "t".
        assert_eq!(finished(&events), Some(2));
    }

    #[test]
    fn a_scan_for_the_last_match_finds_the_last_one() {
        let doc = doc_of("last.txt", b"hit\nx\nhit\n");
        let events = run(context_of(&doc), pattern("hit"), Scope::Last);
        let matches = found(&events);
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(matches[0].line, 2, "the last match in the document");
    }

    #[test]
    fn the_last_match_on_a_line_is_the_one_found() {
        let doc = doc_of("last-line.txt", b"hit hit\n");
        let events = run(context_of(&doc), pattern("hit"), Scope::Last);
        let matches = found(&events);
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(matches[0].start, 4, "the second one on the line");
    }

    #[test]
    fn a_scan_for_the_last_match_of_a_file_without_one_says_so() {
        let doc = doc_of("last-none.txt", b"one\ntwo\n");
        let events = run(context_of(&doc), pattern("needle"), Scope::Last);
        assert!(found(&events).is_empty());
        assert_eq!(finished(&events), Some(0));
    }

    #[test]
    fn a_scan_after_a_position_stops_at_the_first_match() {
        let doc = doc_of("after.txt", b"aaa\nbbb\n");
        let events = run(
            context_of(&doc),
            pattern("b"),
            Scope::After {
                pos: Pos::new(0, 0),
            },
        );
        let matches = found(&events);
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(matches[0].line, 1);
    }

    #[test]
    fn a_scan_after_a_position_skips_the_match_under_the_cursor() {
        let doc = doc_of("after-self.txt", b"x\nx\n");
        let events = run(
            context_of(&doc),
            pattern("x"),
            Scope::After {
                pos: Pos::new(0, 0),
            },
        );
        let matches = found(&events);
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(
            matches[0].line, 1,
            "the match under the cursor is not the next one"
        );
    }

    #[test]
    fn a_scan_from_a_position_takes_the_match_under_the_cursor() {
        let doc = doc_of("from-self.txt", b"x\nx\n");
        let events = run(
            context_of(&doc),
            pattern("x"),
            Scope::From {
                pos: Pos::new(0, 0),
            },
        );
        let matches = found(&events);
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(matches[0].line, 0);
    }

    #[test]
    fn a_scan_before_a_position_finds_the_closest_match() {
        let doc = doc_of("before.txt", b"x\nx\nx\n");
        let events = run(
            context_of(&doc),
            pattern("x"),
            Scope::Before {
                pos: Pos::new(2, 0),
            },
        );
        let matches = found(&events);
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(matches[0].line, 1, "the closest match before line 2");
    }

    #[test]
    fn a_scan_before_a_position_finds_a_match_on_the_same_line() {
        let doc = doc_of("before-same.txt", b"ab\n");
        let events = run(
            context_of(&doc),
            pattern("a"),
            Scope::Before {
                pos: Pos::new(0, 1),
            },
        );
        let matches = found(&events);
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(
            matches[0],
            MatchPos {
                line: 0,
                start: 0,
                end: 1
            }
        );
    }

    #[test]
    fn a_scan_that_finds_nothing_says_so() {
        let doc = doc_of("none.txt", b"aaa\n");
        let events = run(
            context_of(&doc),
            pattern("zzz"),
            Scope::After {
                pos: Pos::new(0, 0),
            },
        );
        assert!(found(&events).is_empty());
        assert_eq!(finished(&events), Some(0));
    }

    #[test]
    fn a_cancelled_scan_stops() {
        let doc = doc_of("cancel.txt", &b"x\n".repeat(100_000));
        let mut job = SearchJob::start(context_of(&doc), pattern("x"), Scope::All);
        job.cancel();
        // A cancelled scan reports nothing more than it already had.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let events = job.drain();
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, SearchEvent::Finished { .. })),
            "{events:?}"
        );
    }

    #[test]
    fn a_scan_sees_the_edited_text() {
        let mut doc = doc_of("edited.txt", b"one\ntwo\n");
        doc.edit(Pos::new(1, 0), Pos::new(1, 3), "twoo");
        let events = run(context_of(&doc), pattern("twoo"), Scope::All);
        assert_eq!(
            finished(&events),
            Some(1),
            "the scan reads the document as it is now"
        );
    }
}
