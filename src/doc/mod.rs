//! Document access: the mapped file, its line index and the edit overlay.

pub mod edit;
pub mod index;
pub mod overlay;

use std::borrow::Cow;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use memmap2::Mmap;

use edit::{Edit, History, Pos};

pub use edit::Pos as Cursor;
pub use index::LineIndex;
pub use overlay::{LineEdit, Overlay};

/// The byte offset of a character column.
///
/// Columns count characters, and slicing a string needs bytes, so every
/// slice goes through here. It never allocates: on a single enormous line
/// the difference between this and collecting a `Vec<char>` is hundreds of
/// megabytes per keystroke.
pub(crate) fn col_to_byte(text: &str, col: usize) -> usize {
    text.char_indices()
        .nth(col)
        .map_or(text.len(), |(byte, _)| byte)
}

/// The text of a current line, given the pieces of a document.
///
/// The foreground and the background scanners both need this, and they must
/// agree exactly: a match the scanner reports has to be where the user sees
/// it, in the same characters and in the same columns.
pub(crate) fn line_text_in<'a>(
    overlay: &'a Overlay,
    index: &LineIndex,
    bytes: &'a [u8],
    ending: LineEnding,
    line: u64,
) -> Option<Cow<'a, str>> {
    if let Some(edited) = overlay.get(line) {
        return Some(Cow::Borrowed(edited));
    }
    let original = overlay.current_to_original(line);
    let (start, end) = index.line_span(bytes, original)?;
    let mut slice = &bytes[start as usize..end as usize];
    if slice.last() == Some(&b'\n') {
        slice = &slice[..slice.len() - 1];
        // A CRLF file keeps the carriage return in the mapped bytes; it is
        // part of the terminator, not part of the text.
        if ending == LineEnding::CrLf && slice.last() == Some(&b'\r') {
            slice = &slice[..slice.len() - 1];
        }
    }
    match std::str::from_utf8(slice) {
        Ok(s) => Some(Cow::Borrowed(s)),
        Err(_) => Some(Cow::Owned(String::from_utf8_lossy(slice).into_owned())),
    }
}

/// Where a string inserted at `start` ends, counting newlines as line breaks.
fn inserted_end(start: Pos, text: &str) -> Pos {
    let mut parts = text.split('\n');
    let first = parts.next().unwrap_or("");
    let mut pos = Pos::new(start.line, start.col + first.chars().count());
    for part in parts {
        pos = Pos::new(pos.line + 1, part.chars().count());
    }
    pos
}

/// The line terminator a file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// Looks at the first few kilobytes of a file to decide how its lines end.
fn detect_line_ending(bytes: &[u8]) -> LineEnding {
    let probe = bytes.len().min(8192);
    for i in 0..probe {
        if bytes[i] == b'\n' {
            return if i > 0 && bytes[i - 1] == b'\r' {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            };
        }
    }
    LineEnding::Lf
}

/// Number of decimal digits in `n` (0 has one digit).
pub fn digits_of(n: u64) -> u16 {
    let mut n = n;
    let mut digits = 1u16;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
}

/// A file open for editing.
///
/// The file bytes are mapped read-only; nothing is written back until the
/// document is saved. Edits live in an [`Overlay`] and are addressed by
/// original line numbers, so the cost of an edit never depends on the size
/// of the file.
pub struct Doc {
    mmap: Arc<Mmap>,
    index: Arc<LineIndex>,
    overlay: Overlay,
    history: History,
    len: u64,
    ending: LineEnding,
    identity: FileIdentity,
    /// Bumped every time the document changes.
    ///
    /// A background scan that started before an edit describes a document
    /// that no longer exists, so the editor needs to know that something
    /// moved under it.
    version: u64,
}

/// What a file looked like when it was opened.
///
/// Size and modification time are enough to answer the one question the save
/// path needs: is this still the file we mapped? Comparing content would cost
/// a read of the whole file on every save, which a 1 GB file cannot pay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// Reads the identity of the file at `path` right now.
pub fn identity_of(path: &Path) -> std::io::Result<FileIdentity> {
    let meta = std::fs::metadata(path)?;
    Ok(FileIdentity {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

impl Doc {
    /// Creates an empty document with no backing file on disk.
    pub fn empty() -> std::io::Result<Self> {
        let mut attempts = 0;
        let file = loop {
            attempts += 1;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "zeno-empty-{}-{}-{}.tmp",
                std::process::id(),
                now,
                attempts
            ));
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    let _ = std::fs::remove_file(&path);
                    break file;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempts < 10 => {
                    continue;
                }
                Err(e) => return Err(e),
            }
        };
        let mmap = Arc::new(unsafe { Mmap::map(&file)? });
        Ok(Self {
            index: Arc::new(LineIndex::new(0)),
            overlay: Overlay::new(),
            history: History::new(),
            mmap,
            len: 0,
            ending: LineEnding::Lf,
            identity: FileIdentity {
                len: 0,
                modified: None,
            },
            version: 0,
        })
    }

    /// Opens a file and maps it into memory, or creates an empty document if it does not exist.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::empty(),
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }
        let len = meta.len();
        let identity = FileIdentity {
            len,
            modified: meta.modified().ok(),
        };
        // SAFETY: the handle is opened read-only and the mapping is read-only,
        // so this process cannot change the bytes under itself. It does *not*
        // make the mapping safe from other processes: a concurrent truncation
        // invalidates it, which is why the identity is recorded and checked
        // before every save (ADR 0015).
        let mmap = Arc::new(unsafe { Mmap::map(&file)? });
        let ending = detect_line_ending(&mmap[..]);
        Ok(Self {
            index: Arc::new(LineIndex::new(len)),
            overlay: Overlay::new(),
            history: History::new(),
            mmap,
            len,
            ending,
            identity,
            version: 0,
        })
    }

    /// How many times the document has changed.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The lowest current line written since `version`.
    ///
    /// Everything above it is the text as it stood at `version`. A worker
    /// holding answers computed at that version — the colour of a line, the
    /// parser's state at a line — can keep the half of its work that is
    /// above this line and drop the rest, which is what stops a keystroke
    /// from blanking the whole screen. Asked about the whole file
    /// (`version` 0), it is the lowest line that was ever edited.
    pub fn first_changed_line_since(&self, version: u64) -> Option<u64> {
        self.overlay.first_changed_line_since(version)
    }

    /// What the file looked like when it was opened, or last saved.
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Records that the file on disk is now exactly this document.
    pub fn set_identity(&mut self, identity: FileIdentity) {
        self.identity = identity;
    }

    /// Starts a background thread that keeps the line index up to date.
    ///
    /// The thread shares the mapping and fills the shared index. Sharing
    /// rather than mapping the file a second time is not only cheaper: it
    /// makes it impossible for the index to describe bytes other than the
    /// ones the editor reads.
    ///
    /// This is a convenience, not a requirement: the foreground scans forward
    /// on demand whenever it needs a line it has not seen yet. Disabling it
    /// keeps opening a huge file cheap at the price of an accurate line
    /// count.
    pub fn spawn_indexer(&self) -> JoinHandle<()> {
        let index = Arc::clone(&self.index);
        let bytes = Arc::clone(&self.mmap);
        std::thread::spawn(move || {
            while !index.scan_more(&bytes[..], 1 << 20) {
                std::thread::sleep(Duration::from_millis(1));
            }
        })
    }

    /// The mapped bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.mmap[..]
    }

    /// The mapped bytes, shared with a background worker.
    pub fn shared_bytes(&self) -> Arc<Mmap> {
        Arc::clone(&self.mmap)
    }

    /// Length of the file in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The line index.
    pub fn index(&self) -> &LineIndex {
        &self.index
    }

    /// A shared handle to the line index, for a background scanner.
    pub fn index_handle(&self) -> Arc<LineIndex> {
        Arc::clone(&self.index)
    }

    /// The edit overlay.
    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    pub fn overlay_mut(&mut self) -> &mut Overlay {
        &mut self.overlay
    }

    /// Number of lines in the document, as far as they are known.
    ///
    /// While the background scan is still running this number grows; the
    /// final value is only known once the whole file has been scanned.
    pub fn line_count(&self) -> u64 {
        self.overlay.line_count(self.index.known_lines())
    }

    /// Total number of lines, once the whole file has been scanned.
    pub fn total_lines(&self) -> Option<u64> {
        self.index.total_lines()
    }

    /// True while the background scan is still running.
    pub fn is_indexing(&self) -> bool {
        !self.index.is_complete()
    }

    /// Scans the whole file, so that the line count becomes exact.
    ///
    /// This costs a pass over the file. It is what "go to the end of the
    /// file" has to pay for in order to know where the end is.
    pub fn index_fully(&mut self) {
        while !self.index.scan_more(self.bytes(), 1 << 20) {}
    }

    /// Number of decimal digits the line-number gutter has to reserve.
    ///
    /// Derived from the size of the file, not from the number of lines: a
    /// file of `n` bytes has at most `n` lines, so the digit count of the
    /// file size is always wide enough. The advantage over the exact line
    /// count is that the gutter never changes width while the background
    /// scan is running, which would make the text area jump under the
    /// user's eyes.
    pub fn gutter_digits(&self) -> u16 {
        digits_of(self.len)
    }

    /// The text of a current line, without its trailing newline.
    pub fn line_text(&self, line: u64) -> Option<Cow<'_, str>> {
        line_text_in(&self.overlay, &self.index, self.bytes(), self.ending, line)
    }

    /// The text between two positions, with lines joined by newlines.
    pub fn text_in_range(&self, start: Pos, end: Pos) -> String {
        let (start, end) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        if start.line == end.line {
            let text = self.line_text(start.line).unwrap_or_default();
            let from = col_to_byte(&text, start.col);
            let to = col_to_byte(&text, end.col).max(from);
            return text[from..to].to_owned();
        }
        let mut out = String::new();
        let first = self.line_text(start.line).unwrap_or_default();
        out.push_str(&first[col_to_byte(&first, start.col)..]);
        for line in start.line + 1..end.line {
            out.push('\n');
            out.push_str(&self.line_text(line).unwrap_or_default());
        }
        out.push('\n');
        let last = self.line_text(end.line).unwrap_or_default();
        out.push_str(&last[..col_to_byte(&last, end.col)]);
        out
    }

    /// Replaces the text in `[start, end)` with `text`, and records the change
    /// so it can be undone. Returns where the inserted text ends.
    ///
    /// This is the only way the document changes. Every command — typing,
    /// Enter, Backspace, Delete, duplicate, and later paste and the operators
    /// — is a range replaced by a string, which is what makes one undo journal
    /// enough to reverse all of them.
    pub fn edit(&mut self, start: Pos, end: Pos, text: &str) -> Pos {
        let (start, end) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        // Both ends must be bounded by the same document. Reading a line can
        // extend the index, so clamping them one after the other would let the
        // second bound be larger than the first.
        let last_line = self.line_count().saturating_sub(1);
        let start = self.clamp(start, last_line);
        let end = self.clamp(end, last_line);
        let (start, end) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        let (removed, inserted_end) = self.apply(start, end, text);
        self.history.record(Edit {
            start,
            end,
            inserted_end,
            removed,
            inserted: text.to_owned(),
        });
        inserted_end
    }

    /// Reverses the last edit, returning the position it started at.
    pub fn undo(&mut self) -> Option<Pos> {
        let edit = self.history.take_undo()?;
        self.apply(edit.start, edit.inserted_end, &edit.removed);
        Some(edit.start)
    }

    /// Reapplies the last undone edit, returning its start position.
    pub fn redo(&mut self) -> Option<Pos> {
        let edit = self.history.take_redo()?;
        self.apply(edit.start, edit.end, &edit.inserted);
        Some(edit.start)
    }

    /// Whether there is anything to undo or redo.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Clamps a position into the document.
    fn clamp(&self, pos: Pos, last_line: u64) -> Pos {
        let line = pos.line.min(last_line);
        let len = self.line_text(line).map_or(0, |t| t.chars().count());
        Pos::new(line, pos.col.min(len))
    }

    /// Applies a clamp-validated replacement without touching the journal.
    ///
    /// Returns the removed text and the end of the inserted text.
    fn apply(&mut self, start: Pos, end: Pos, text: &str) -> (String, Pos) {
        self.version += 1;
        // Everything written below is stamped with this version, so that
        // `first_changed_line_since` can later be asked what an answer
        // computed at the old version is still worth.
        self.overlay.set_version(self.version);
        let removed = self.text_in_range(start, end);
        let swallowed = end.line - start.line;

        // Build the replacement line in a single allocation, and let the
        // borrow of the document end before the overlay is written. On a
        // single enormous line this is the difference between one copy and
        // four per keystroke.
        let combined = {
            let first = self.line_text(start.line).unwrap_or_default();
            let mut out = String::with_capacity(first.len() + text.len());
            out.push_str(&first[..col_to_byte(&first, start.col)]);
            out.push_str(text);
            if start.line == end.line {
                out.push_str(&first[col_to_byte(&first, end.col)..]);
            } else {
                let last = self.line_text(end.line).unwrap_or_default();
                out.push_str(&last[col_to_byte(&last, end.col)..]);
            }
            out
        };

        let lines: Vec<String> = combined.split('\n').map(str::to_owned).collect();
        let replacement = lines.len() as u64;
        self.overlay.replace_lines(start.line, lines);
        // The lines the range spanned now sit *after* the replacement's own
        // lines — at `start.line + replacement`, not at `start.line + 1`.
        // The two are the same place only when the replacement is a single
        // line; with a multi-line replacement the short answer would delete
        // one of the new lines and leave an original in its place, which is
        // what made a multi-line indent over a multi-line range lose a line.
        for _ in 0..swallowed {
            self.overlay.delete_line(start.line + replacement);
        }

        (removed, inserted_end(start, text))
    }

    /// The line ending style of the original file.
    pub fn line_ending(&self) -> LineEnding {
        self.ending
    }

    /// Whether the original file ends with a line terminator.
    pub fn ends_with_newline(&self) -> bool {
        self.bytes().last() == Some(&b'\n')
    }

    /// True when the first `probe` bytes contain a NUL, which is how a binary
    /// file is recognised.
    pub fn looks_binary(&self, probe: usize) -> bool {
        let end = probe.min(self.bytes().len());
        self.bytes()[..end].contains(&0)
    }

    /// The document, as a background worker sees it.
    ///
    /// The bytes and the line index are shared, so the worker reads exactly
    /// the text the user is looking at. The overlay is copied, so the worker
    /// reads the document as it was when the snapshot was taken — a result
    /// that arrives after the document has moved on is discarded rather than
    /// believed.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            bytes: Arc::clone(&self.mmap),
            index: Arc::clone(&self.index),
            overlay: self.overlay.clone(),
            ending: self.ending,
        }
    }
}

/// A copy of the document, for a worker to read.
///
/// The copy is of the *edits*, not of the text: the file's bytes are mapped
/// and shared, and the line index is shared too, so taking a snapshot costs
/// as much as the edits so far and no more. That is what lets the editor
/// hand the same kind of snapshot to the search and to the highlighter
/// without either of them owning a copy of the file.
#[derive(Clone)]
pub struct Snapshot {
    bytes: Arc<Mmap>,
    index: Arc<LineIndex>,
    overlay: Overlay,
    ending: LineEnding,
}

/// How much of the file one background indexing step scans.
///
/// Larger than the foreground's, because the background has no frame to
/// finish: it would rather do the work in fewer, bigger steps.
const BACKGROUND_SCAN_CHUNK: usize = 1 << 20;

impl Snapshot {
    pub fn new(
        bytes: Arc<Mmap>,
        index: Arc<LineIndex>,
        overlay: Overlay,
        ending: LineEnding,
    ) -> Self {
        Self {
            bytes,
            index,
            overlay,
            ending,
        }
    }

    /// The text of a current line, as the editor would show it.
    pub fn line_text(&self, line: u64) -> Option<Cow<'_, str>> {
        line_text_in(&self.overlay, &self.index, &self.bytes, self.ending, line)
    }

    /// The lowest line written since `version`.
    ///
    /// Everything above it is the text as it was at `version`, which is
    /// what lets a worker keep the half of its work that is still true.
    pub fn first_changed_line_since(&self, version: u64) -> Option<u64> {
        self.overlay.first_changed_line_since(version)
    }

    /// Scans the rest of the file, so that the number of lines is known.
    ///
    /// The index is shared with the editor, so this fills it in for the
    /// editor too rather than doing the work twice.
    pub fn index_fully(&self) {
        while !self.index.scan_more(&self.bytes[..], BACKGROUND_SCAN_CHUNK) {}
    }

    /// The last line of the document, once the file has been scanned.
    pub fn last_line(&self) -> Option<u64> {
        let total = self.index.total_lines()?;
        Some(self.overlay.line_count(total).saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("zeno-doc-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path
    }

    #[test]
    fn open_empty_file_on_disk() {
        let path = temp_file("empty_zero_bytes.txt", b"");
        let doc = Doc::open(&path);
        assert!(doc.is_ok(), "opening empty file failed: {:?}", doc.err());
    }

    #[test]
    fn open_directory_returns_error() {
        let dir = std::env::temp_dir().join("zeno-doc-tests-dir");
        let _ = std::fs::create_dir_all(&dir);
        let res = Doc::open(&dir);
        assert!(res.is_err(), "opening directory must return error");
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn open_nonexistent_file_returns_empty_doc() {
        let path = std::env::temp_dir()
            .join("zeno-doc-tests")
            .join("does_not_exist.txt");
        let _ = std::fs::remove_file(&path);
        let doc = Doc::open(&path).expect("opening non-existent file should yield empty doc");
        assert_eq!(doc.line_count(), 0);
        assert_eq!(doc.len(), 0);
        assert!(doc.is_empty());
    }

    #[test]
    fn the_identity_notices_a_replaced_file() {
        let path = temp_file("identity.txt", b"one\n");
        let doc = Doc::open(&path).unwrap();
        assert_eq!(identity_of(&path).unwrap(), doc.identity());
        // A replace by rename, which is how git and most editors write: on
        // Windows a mapped file cannot even be opened for writing, so this is
        // the realistic shape of somebody else changing the file.
        let replacement = temp_file("identity.replacement", b"two three\n");
        std::fs::rename(&replacement, &path).unwrap();
        assert_ne!(identity_of(&path).unwrap(), doc.identity());
    }

    #[test]
    fn reads_lines() {
        let path = temp_file("read_lines.txt", b"alpha\nbeta\ngamma\n");
        let doc = Doc::open(&path).unwrap();
        // The line count grows as the index is built; scan the whole file.
        assert_eq!(doc.line_count(), 1);
        assert!(doc.index().scan_more(doc.bytes(), usize::MAX));
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.total_lines(), Some(3));
        assert_eq!(doc.line_text(0).as_deref(), Some("alpha"));
        assert_eq!(doc.line_text(2).as_deref(), Some("gamma"));
        assert_eq!(doc.line_text(3), None);
    }

    /// The whole document as text, for round-trip comparisons.
    fn whole_text(doc: &Doc) -> String {
        let mut out = String::new();
        for line in 0..doc.line_count() {
            if line > 0 {
                out.push('\n');
            }
            out.push_str(&doc.line_text(line).unwrap_or_default());
        }
        out
    }

    #[test]
    fn undo_restores_the_document_exactly() {
        let path = temp_file("undo-all.txt", b"alpha\nbeta\ngamma\n");
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        let before = whole_text(&doc);

        doc.edit(Pos::new(0, 0), Pos::new(0, 0), "xyz");
        doc.edit(Pos::new(1, 0), Pos::new(1, 4), "B");
        doc.edit(Pos::new(1, 1), Pos::new(1, 1), "\nnew");
        doc.edit(Pos::new(0, 3), Pos::new(1, 0), "");
        assert_ne!(whole_text(&doc), before);

        while doc.can_undo() {
            doc.undo();
        }
        assert_eq!(whole_text(&doc), before);
    }

    #[test]
    fn redo_returns_to_the_edited_document() {
        let path = temp_file("redo-all.txt", b"one\ntwo\n");
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        doc.edit(Pos::new(0, 0), Pos::new(0, 3), "1");
        doc.edit(Pos::new(1, 3), Pos::new(1, 3), "!");
        doc.edit(Pos::new(0, 1), Pos::new(1, 0), "\n");
        let edited = whole_text(&doc);

        while doc.can_undo() {
            doc.undo();
        }
        while doc.can_redo() {
            doc.redo();
        }
        assert_eq!(whole_text(&doc), edited);
    }

    #[test]
    fn a_typing_run_is_one_undo_step() {
        let path = temp_file("undo-run.txt", b"ab\n");
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        for (i, c) in "hello".chars().enumerate() {
            doc.edit(Pos::new(0, i), Pos::new(0, i), &c.to_string());
        }
        assert_eq!(doc.history().undo_depth(), 1);
        doc.undo();
        assert_eq!(whole_text(&doc), "ab");
        assert!(!doc.can_undo());
    }

    #[test]
    fn a_multi_line_replacement_is_reversible() {
        let path = temp_file("undo-range.txt", b"a\nb\nc\nd\n");
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        let before = whole_text(&doc);
        doc.edit(Pos::new(0, 0), Pos::new(2, 1), "X");
        assert_eq!(whole_text(&doc), "X\nd");
        doc.undo();
        assert_eq!(whole_text(&doc), before);
    }

    /// The replacement's own line breaks and the range's must not be
    /// confused for each other.
    ///
    /// A range spanning two lines replaced by three: the two originals the
    /// range swallowed are deleted *after* the three new lines, so none of
    /// the new lines is eaten and no original survives beside them. This is
    /// the shape of pasting a multi-line clipboard over a multi-line
    /// selection — and of the indent operator rewriting a range.
    #[test]
    fn a_multi_line_replacement_over_a_span_keeps_its_own_lines() {
        let path = temp_file("replace-span.txt", b"ab\ncd\nef\n");
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        let before = whole_text(&doc);
        doc.edit(Pos::new(0, 1), Pos::new(1, 1), "X\nY\nZ");
        assert_eq!(whole_text(&doc), "aX\nY\nZd\nef");
        doc.undo();
        assert_eq!(whole_text(&doc), before, "the undo restores it exactly");
        doc.redo();
        assert_eq!(whole_text(&doc), "aX\nY\nZd\nef", "and the redo agrees");
    }

    #[test]
    fn deleting_across_a_line_break_joins_the_lines() {
        let path = temp_file("join-lines.txt", b"ab\ncd\n");
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        doc.edit(Pos::new(0, 2), Pos::new(1, 0), "");
        assert_eq!(whole_text(&doc), "abcd");
        assert_eq!(doc.line_count(), 1);
    }

    #[test]
    fn text_in_range_spans_lines() {
        let path = temp_file("range-text.txt", b"abc\ndef\nghi\n");
        let doc = {
            let mut doc = Doc::open(&path).unwrap();
            doc.index_fully();
            doc
        };
        assert_eq!(
            doc.text_in_range(Pos::new(0, 1), Pos::new(2, 2)),
            "bc\ndef\ngh"
        );
        assert_eq!(
            doc.text_in_range(Pos::new(2, 2), Pos::new(0, 1)),
            "bc\ndef\ngh"
        );
        assert_eq!(doc.text_in_range(Pos::new(1, 0), Pos::new(1, 0)), "");
    }

    #[test]
    fn detects_binary() {
        let path = temp_file("binary.bin", b"\x7fELF\x00\x01\x02");
        let doc = Doc::open(&path).unwrap();
        assert!(doc.looks_binary(64));
    }

    #[test]
    fn editing_keeps_original_anchoring() {
        let path = temp_file("edit.txt", b"one\ntwo\nthree\n");
        let mut doc = Doc::open(&path).unwrap();
        // Editing addresses lines the index knows about.
        doc.index_fully();
        doc.edit(Pos::new(1, 0), Pos::new(1, 3), "TWO");
        assert_eq!(doc.line_text(1).as_deref(), Some("TWO"));
        assert_eq!(doc.line_text(2).as_deref(), Some("three"));
    }

    #[test]
    fn crlf_files_do_not_leak_the_carriage_return() {
        let path = temp_file("crlf.txt", b"one\r\ntwo\r\n");
        let doc = Doc::open(&path).unwrap();
        assert_eq!(doc.line_ending(), LineEnding::CrLf);
        assert_eq!(doc.line_text(0).as_deref(), Some("one"));
        assert_eq!(doc.line_text(1).as_deref(), Some("two"));
        assert!(doc.ends_with_newline());
    }

    #[test]
    fn lf_files_keep_a_lone_carriage_return() {
        let path = temp_file("lone-cr.txt", b"a\rb\n");
        let doc = Doc::open(&path).unwrap();
        assert_eq!(doc.line_ending(), LineEnding::Lf);
        assert_eq!(doc.line_text(0).as_deref(), Some("a\rb"));
    }
}
