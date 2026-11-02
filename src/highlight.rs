//! Syntax highlighting: the colours of the lines in the viewport.
//!
//! The colours are computed by a worker, never by the editor. Loading the
//! syntax definitions and the theme takes tens of milliseconds, and parsing
//! a line takes longer still; the editor has a budget of fifty milliseconds
//! for its first frame and sixteen for a keystroke. So the editor paints
//! plain text first and the colours arrive a moment later — the same
//! bargain the search made (see `search.rs`).
//!
//! Three things keep the worker honest:
//!
//! - **It is cancelable.** Every request carries a number, and the worker
//!   abandons a request the moment a newer one arrives. A window the user
//!   has already scrolled past is not worth finishing, and neither is a
//!   window that an edit has made out of date.
//! - **A result is only about the document it was computed from.** The
//!   request carries the document's version, and a result whose version is
//!   old describes text that is no longer there.
//! - **A line over the limit is not parsed.** A line of a megabyte would
//!   take seconds in the parser, and would break the frame budget on its
//!   own. It is left plain, which is the same bargain `bat` makes.
//!
//! ## Getting to a line
//!
//! A TextMate parser is a state machine: what a line looks like depends on
//! the lines before it — a line inside a block comment is a comment because
//! the line above opened the comment. So the colours of a window cannot be
//! computed from the window alone; the parser has to be walked from
//! somewhere it has already been.
//!
//! The worker therefore keeps *checkpoints*: the parser's state at the start
//! of every few hundred lines it has walked past. A window is coloured by
//! resuming from the nearest checkpoint above it and walking down to the
//! window's last line. The cost of a window is therefore the distance from
//! the checkpoint to the window, not the size of the file.
//!
//! An edit invalidates the checkpoints below it: the text they were
//! computed from is no longer the text of the file. The checkpoints above
//! the edit are still true, and are kept. Whether *a* checkpoint survives
//! is therefore a question about that checkpoint and its own version — it
//! is kept exactly when nothing above it has been written since it was
//! built. Asking per checkpoint is what stops one edit near the top of a
//! file from discarding, on every request for ever after, the checkpoints
//! rebuilt after that edit.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use ratatui::style::{Color, Modifier, Style};
use syntect::highlighting::{FontStyle, HighlightState, RangedHighlightIterator, Theme, ThemeSet};
use syntect::parsing::{ParseState, ScopeStack, SyntaxDefinition, SyntaxReference, SyntaxSet};

use crate::doc::{Doc, Snapshot};
use crate::theme::ThemeColors;

/// The theme the colours come from.
///
/// A dark theme, because the editor's own chrome is dark, and a light
/// syntax theme on top of it reads as a mistake. The theme's *background*
/// is deliberately not used: the editor keeps the terminal's own, so that
/// the selection (which is painted in reverse video) and the cursor line stay
/// readable.
const THEME: &str = "Dark Modern";

const VSCODE_DARK_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>Dark Modern</string>
    <key>settings</key>
    <array>
        <dict>
            <key>settings</key>
            <dict>
                <key>background</key>
                <string>#000000</string>
                <key>foreground</key>
                <string>#E0E0E0</string>
                <key>lineHighlight</key>
                <string>#181818</string>
                <key>selection</key>
                <string>#264F78</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Comment</string>
            <key>scope</key>
            <string>comment, punctuation.definition.comment</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#72B35B</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>String</string>
            <key>scope</key>
            <string>string, punctuation.definition.string</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#E29878</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>String Escape</string>
            <key>scope</key>
            <string>constant.character.escape, string.regexp</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#E8C87E</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Number</string>
            <key>scope</key>
            <string>constant.numeric</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#BFE2B0</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Built-in Constant</string>
            <key>scope</key>
            <string>constant.language</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#5DA8ED</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Control Keyword</string>
            <key>scope</key>
            <string>keyword.control</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#D888D4</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Keyword</string>
            <key>scope</key>
            <string>keyword, storage, storage.type, storage.modifier</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#5DA8ED</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Function</string>
            <key>scope</key>
            <string>entity.name.function, support.function, entity.name.macro</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#ECECA0</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Type</string>
            <key>scope</key>
            <string>entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, support.type, support.class</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#4FE0BF</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Variable</string>
            <key>scope</key>
            <string>variable, variable.parameter, variable.other</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#A3E2FF</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Property</string>
            <key>scope</key>
            <string>variable.other.property, variable.other.member, support.type.property-name</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#A3E2FF</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Tag</string>
            <key>scope</key>
            <string>entity.name.tag</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#5DA8ED</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Attribute</string>
            <key>scope</key>
            <string>entity.other.attribute-name</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#A3E2FF</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Operator</string>
            <key>scope</key>
            <string>keyword.operator</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#E2E2E2</string>
            </dict>
        </dict>
    </array>
</dict>
</plist>
"#;

const MONOKAI_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>Monokai</string>
    <key>settings</key>
    <array>
        <dict>
            <key>settings</key>
            <dict>
                <key>background</key>
                <string>#272822</string>
                <key>foreground</key>
                <string>#f8f8f2</string>
                <key>lineHighlight</key>
                <string>#3e3d32</string>
                <key>selection</key>
                <string>#49483e</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Comment</string>
            <key>scope</key>
            <string>comment</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#75715e</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>String</string>
            <key>scope</key>
            <string>string</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#e6db74</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Number</string>
            <key>scope</key>
            <string>constant.numeric</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#ae81ff</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Keyword</string>
            <key>scope</key>
            <string>keyword, storage</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#f92672</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Function</string>
            <key>scope</key>
            <string>entity.name.function, support.function</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#a6e22e</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Type</string>
            <key>scope</key>
            <string>entity.name.type, entity.name.class, support.type, support.class</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#66d9ef</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Variable</string>
            <key>scope</key>
            <string>variable</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#fd971f</string>
            </dict>
        </dict>
    </array>
</dict>
</plist>
"#;

const HIGH_CONTRAST_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>High Contrast</string>
    <key>settings</key>
    <array>
        <dict>
            <key>settings</key>
            <dict>
                <key>background</key>
                <string>#000000</string>
                <key>foreground</key>
                <string>#ffffff</string>
                <key>lineHighlight</key>
                <string>#232323</string>
                <key>selection</key>
                <string>#ffffff</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Keyword</string>
            <key>scope</key>
            <string>keyword, storage</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#00ffff</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>String</string>
            <key>scope</key>
            <string>string</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#00ff00</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Number</string>
            <key>scope</key>
            <string>constant.numeric, constant</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#ffff00</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Function</string>
            <key>scope</key>
            <string>entity.name.function, support.function</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#ff00ff</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Type</string>
            <key>scope</key>
            <string>entity.name.type, entity.name.class, support.type, support.class</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#00e5ff</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Comment</string>
            <key>scope</key>
            <string>comment</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#ffff77</string>
            </dict>
        </dict>
    </array>
</dict>
</plist>
"#;

/// Lines longer than this are not highlighted.
///
/// The parser would spend seconds on a line of a megabyte, and the editor
/// has a frame budget of sixteen milliseconds. The line is left plain
/// instead, so the cost of one long line is bounded and the rest of the
/// file still gets its colours.
const MAX_LINE_BYTES: usize = 16 * 1024;

/// What the parser is shown in place of a line that is over the limit.
///
/// The syntax set was loaded without newlines (see `work`), so an empty line
/// is the empty string. Parsing it advances the parser for no cost.
const EMPTY_LINE: &str = "";

/// How many lines apart the parser's state is kept.
///
/// The state is what makes a re-colour cheap: it says what the parser knows
/// about the text *before* a line. Keeping one every few hundred lines
/// means an edit costs a few hundred lines of re-walking, rather than the
/// whole file.
const CHECKPOINT_LINES: u64 = 512;

/// How many checkpoints are kept.
///
/// The state is small, but there is no reason to keep the whole history of
/// a file's parse: the checkpoints that matter are the ones near the
/// window the user is looking at.
const MAX_CHECKPOINTS: usize = 64;

/// A run of characters on one line that share a style.
///
/// The columns are character columns, like every other column in the
/// editor, so a run of Persian text is measured the way the user sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    /// The first character of the run.
    pub start: usize,
    /// One past the last character of the run.
    pub end: usize,
    /// How the run is painted.
    pub style: Style,
}

/// The colours of a window of lines, by line number.
pub type Highlighted = HashMap<u64, Vec<LineSpan>>;

/// A window of lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Window {
    /// The version of the document the window belongs to.
    version: u64,
    first: u64,
    last: u64,
}

/// What the editor asks the worker for.
struct Request {
    /// The number of this request. A newer one makes this one pointless.
    epoch: u64,
    /// The window being asked about.
    window: Window,
    /// The document, as the worker should read it.
    snapshot: Snapshot,
}

/// The colours of a window, as the worker reports them.
struct Colours {
    /// The window they were computed for.
    ///
    /// The answer says which question it answers. The editor is scrolling
    /// while the worker is parsing, so an answer about a window the editor
    /// has already left is not an answer about anything on screen.
    window: Window,
    /// The lines that were coloured.
    lines: Highlighted,
    /// The name of the syntax in use, once it is known.
    syntax: Option<String>,
}

/// The editor's side of the highlighter.
///
/// It asks for the window that is on screen and holds the answer until the
/// next one arrives. It never parses anything itself.
pub struct Highlighter {
    requests: Sender<Request>,
    colours: Receiver<Colours>,
    worker: Option<JoinHandle<()>>,
    /// The number of the last request sent.
    epoch: Arc<AtomicU64>,
    /// The window that was last asked for.
    asked: Option<Window>,
    /// The colours of the lines that have been coloured.
    lines: Highlighted,
    /// The version of the document the colours on hand belong to.
    lines_version: u64,
    /// The name of the syntax in use.
    syntax: Option<String>,
    /// The theme name requested for this highlighter.
    theme: Option<String>,
    /// The active theme's colors.
    theme_colors: ThemeColors,
}

impl Highlighter {
    /// Starts the worker.
    ///
    /// The worker is started even for a file with no syntax: the editor
    /// does not know yet whether the file is plain text, and a shebang on
    /// the first line may say otherwise.
    pub fn start(path: &Path) -> Self {
        let theme = crate::config::load().settings.theme;
        Self::start_with_theme(path, theme)
    }

    /// Starts the worker with an explicitly chosen theme name.
    pub fn start_with_theme(path: &Path, theme: Option<String>) -> Self {
        let (requests, request_rx) = channel();
        let (colour_tx, colours) = channel();
        let epoch = Arc::new(AtomicU64::new(0));
        let worker = {
            let epoch = Arc::clone(&epoch);
            let path = path.to_path_buf();
            let theme = theme.clone();
            std::thread::Builder::new()
                .name("zeno-highlight".into())
                .spawn(move || work(request_rx, colour_tx, epoch, path, theme))
                .ok()
        };
        let theme_colors = ThemeColors::for_theme(theme.as_deref());
        Self {
            requests,
            colours,
            worker,
            epoch,
            asked: None,
            lines: Highlighted::new(),
            lines_version: 0,
            syntax: None,
            theme,
            theme_colors,
        }
    }

    /// The theme name requested for this highlighter.
    pub fn theme(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    /// The active theme's colors for editor chrome, gutter, and document body.
    pub fn theme_colors(&self) -> ThemeColors {
        self.theme_colors
    }

    /// Asks for the window that is on screen to be coloured.
    ///
    /// Asking twice for the same window of the same document does nothing:
    /// the editor draws every thirty milliseconds, and the worker should not
    /// be told to do the same work twice.
    pub fn request(&mut self, doc: &Doc, first: u64, last: u64) {
        let window = Window {
            version: doc.version(),
            first,
            last,
        };
        if self.asked == Some(window) {
            return;
        }
        self.asked = Some(window);
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.requests.send(Request {
            epoch,
            window,
            snapshot: doc.snapshot(),
        });
    }

    /// Takes the colours that have arrived, if they are still about this
    /// document and this window.
    ///
    /// Returns whether anything changed, so the editor knows to paint again.
    pub fn poll(&mut self, doc: &Doc) -> bool {
        let version = doc.version();
        let mut changed = false;
        if self.lines_version != version {
            // An edit may have moved the text, so the colours from the
            // first line written since these colours were computed down
            // are not to be believed. The ones above it are still the
            // truth, and keeping them is what stops a keystroke from
            // flashing the screen plain while the worker catches up. It is
            // the same rule the worker applies to its own checkpoints.
            // When nothing has been written since, nothing is dropped: an
            // edit that did not touch this window must not blank it.
            if let Some(changed_line) = doc.first_changed_line_since(self.lines_version) {
                self.lines.retain(|line, _| *line < changed_line);
            }
            self.lines_version = version;
            changed = true;
        }
        while let Ok(colours) = self.colours.try_recv() {
            if Some(colours.window) != self.asked || colours.window.version != version {
                // Either the editor has already scrolled on to another
                // window, or the document has moved on from the text these
                // colours describe. A result about a window nobody is
                // looking at must not be installed over the one that is.
                continue;
            }
            self.lines = colours.lines;
            self.syntax = colours.syntax;
            changed = true;
        }
        changed
    }

    /// The colours of the lines that have been coloured.
    pub fn lines(&self) -> &Highlighted {
        &self.lines
    }

    /// The name of the syntax in use, once it is known.
    pub fn syntax(&self) -> Option<&str> {
        self.syntax.as_deref()
    }
}

impl Drop for Highlighter {
    fn drop(&mut self) {
        // The worker stops when the channel closes, which happens when this
        // handle is dropped. It is not joined: a worker stuck on one
        // enormous line must not be able to hold up the editor.
        self.worker.take();
    }
}

/// The parser's state at the start of a line.
#[derive(Clone)]
struct Checkpoint {
    parse: ParseState,
    state: HighlightState,
    /// The document version the state was computed from.
    ///
    /// The state at a line is only as true as the text above it was at
    /// this version; see [`Checkpoints::before`].
    version: u64,
}

/// The checkpoints the worker keeps.
struct Checkpoints {
    /// The state at the start of each line, by line number.
    states: BTreeMap<u64, Checkpoint>,
}

impl Checkpoints {
    fn new() -> Self {
        Self {
            states: BTreeMap::new(),
        }
    }

    /// The checkpoint to resume from for a window starting at `first`.
    ///
    /// The nearest one above the window, if one is still true: a
    /// checkpoint is the state at the start of its line as of its own
    /// version, so it survives exactly when nothing above it has been
    /// written since. One that does not survive is dropped here, and the
    /// search continues to the next. Validating the one actually chosen —
    /// rather than clearing the whole file by one global number — is what
    /// stops a single edit near the top of a large file from discarding
    /// every request for ever the checkpoints rebuilt after it.
    fn before(&mut self, first: u64, snapshot: &Snapshot) -> Option<(u64, Checkpoint)> {
        loop {
            let line = *self.states.range(..=first).next_back()?.0;
            let stale = {
                let checkpoint = &self.states[&line];
                snapshot
                    .first_changed_line_since(checkpoint.version)
                    .is_some_and(|changed| line >= changed)
            };
            if stale {
                self.states.remove(&line);
                continue;
            }
            return Some((line, self.states[&line].clone()));
        }
    }

    /// Records the state at the start of `line`.
    ///
    /// The checkpoints furthest from the window are dropped when there are
    /// too many: the ones near the user are the ones worth keeping.
    fn insert(&mut self, line: u64, checkpoint: Checkpoint, window: u64) {
        self.states.insert(line, checkpoint);
        while self.states.len() > MAX_CHECKPOINTS {
            let furthest = self
                .states
                .keys()
                .max_by_key(|at| at.abs_diff(window))
                .copied();
            match furthest {
                Some(at) => {
                    self.states.remove(&at);
                }
                None => break,
            }
        }
    }
}

/// The worker's state between requests.
struct Worker {
    syntaxes: SyntaxSet,
    theme: Theme,
    /// The style the theme gives to text it says nothing about.
    plain: syntect::highlighting::Style,
    /// The syntax in use, once it has been decided.
    syntax: Option<SyntaxReference>,
    /// The name of that syntax.
    name: Option<String>,
    /// Whether the syntax has been decided yet.
    detected: bool,
    /// Where the syntaxes read from the config directory start in
    /// `syntaxes`. The user's own are the last ones added, and are looked at
    /// first.
    user_from: usize,
    checkpoints: Checkpoints,
}

impl Worker {
    /// The syntax of the file, decided once from the file itself.
    fn detect(&mut self, request: &Request, path: &Path) {
        self.detected = true;
        let first_line = request.snapshot.line_text(0).map(|line| line.into_owned());
        let syntax = detect_syntax(&self.syntaxes, self.user_from, path, first_line.as_deref());
        self.name = syntax.as_ref().map(|syntax| syntax.name.clone());
        self.syntax = syntax;
    }

    /// Colours the window of a request.
    ///
    /// `None` when the request was overtaken by a newer one: the colours
    /// would be for a window nobody is looking at any more.
    fn colour(&mut self, request: &Request, epoch: &AtomicU64) -> Option<Highlighted> {
        let Some(syntax) = self.syntax.clone() else {
            // The file has no syntax: nothing to colour, but the editor
            // should hear that the question was answered rather than wait.
            return Some(Highlighted::new());
        };
        // An edit makes the checkpoints below it untrue. Deciding which
        // ones those are belongs to `Checkpoints::before`, per checkpoint
        // and against the version each was built at.

        // The theme's rules, resolved once per request. The parser needs
        // them for every line it walks, and building them is not free.
        let styler = syntect::highlighting::Highlighter::new(&self.theme);

        // Resume from the nearest checkpoint above the window.
        let (from, mut parse, mut state) = match self
            .checkpoints
            .before(request.window.first, &request.snapshot)
        {
            Some((line, checkpoint)) => (line, checkpoint.parse.clone(), checkpoint.state.clone()),
            None => (
                0,
                ParseState::new(&syntax),
                HighlightState::new(&styler, ScopeStack::new()),
            ),
        };
        let mut lines = Highlighted::new();

        let mut line = from;
        while line <= request.window.last {
            if epoch.load(Ordering::SeqCst) != request.epoch {
                // A newer window was asked for, and this one is not worth
                // finishing.
                return None;
            }
            if line == request.window.first || line % CHECKPOINT_LINES == 0 {
                self.checkpoints.insert(
                    line,
                    Checkpoint {
                        parse: parse.clone(),
                        state: state.clone(),
                        version: request.window.version,
                    },
                    request.window.first,
                );
            }
            let Some(text) = request.snapshot.line_text(line) else {
                // Past the end of the document.
                break;
            };
            // A line over the limit is not coloured — but the parser is
            // still walked over an *empty* line in its place, which is
            // exactly what `bat` does (`too_long = line.len() > 1024 *
            // 16`, parsed as `"\n"`). An empty line costs the parser
            // nothing and still advances it, so a megabyte cannot open a
            // block comment or close one and leave the rest of the file
            // painted as a comment.
            let too_long = text.len() > MAX_LINE_BYTES;
            let parsed: &str = if too_long { EMPTY_LINE } else { &text };
            if let Some(spans) = self.spans_of(&mut parse, &mut state, &styler, parsed)
                && !too_long
                && line >= request.window.first
            {
                lines.insert(line, spans);
            }
            line += 1;
        }
        Some(lines)
    }

    /// The styles of one line, as runs of character columns.
    fn spans_of(
        &self,
        parse: &mut ParseState,
        state: &mut HighlightState,
        styler: &syntect::highlighting::Highlighter<'_>,
        text: &str,
    ) -> Option<Vec<LineSpan>> {
        let operations = parse.parse_line(text, &self.syntaxes).ok()?;
        let mut spans: Vec<LineSpan> = Vec::new();
        let mut column = 0usize;
        for (style, piece, _range) in RangedHighlightIterator::new(state, &operations, text, styler)
        {
            let width = piece.chars().count();
            if width == 0 {
                continue;
            }
            if let Some(editor_style) = editor_style(style, self.plain) {
                match spans.last_mut() {
                    Some(last) if last.style == editor_style && last.end == column => {
                        last.end = column + width;
                    }
                    _ => spans.push(LineSpan {
                        start: column,
                        end: column + width,
                        style: editor_style,
                    }),
                }
            }
            column += width;
        }
        Some(spans)
    }
}

/// The syntax of a file, if it has one.
///
/// The order is the one the specification names: the file name and its
/// extension first, then the first line for a shebang, and plain text when
/// neither says anything.
fn detect_syntax(
    syntaxes: &SyntaxSet,
    user_from: usize,
    path: &Path,
    first_line: Option<&str>,
) -> Option<SyntaxReference> {
    if let Some(name) = path.file_name().and_then(|name| name.to_str())
        && let Some(syntax) = user_syntax_by_extension(syntaxes, user_from, name)
            .or_else(|| syntaxes.find_syntax_by_extension(name).cloned())
    {
        return Some(syntax);
    }
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str())
        && let Some(syntax) = user_syntax_by_extension(syntaxes, user_from, extension)
            .or_else(|| syntaxes.find_syntax_by_extension(extension).cloned())
    {
        return Some(syntax);
    }
    if let Some(line) = first_line
        && let Some(syntax) = syntaxes.find_syntax_by_first_line(line)
    {
        return Some(syntax.clone());
    }
    None
}

/// A syntax of the user's own that claims this extension.
///
/// The bundled syntaxes are consulted first by the set itself, so a file the
/// user drops in for an extension syntect already knows would never be seen.
/// Looking at their syntaxes first is what makes the directory an override
/// and not just an addition.
fn user_syntax_by_extension(
    syntaxes: &SyntaxSet,
    user_from: usize,
    extension: &str,
) -> Option<SyntaxReference> {
    syntaxes
        .syntaxes()
        .get(user_from..)?
        .iter()
        .find(|syntax| {
            syntax
                .file_extensions
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
        .cloned()
}

/// The bundled syntaxes, plus any `.sublime-syntax` files in the config
/// directory's `syntaxes/` (spec §4.3).
///
/// Returns the set and where the user's syntaxes start in it. A file that
/// cannot be read or parsed is skipped: one broken syntax must not cost the
/// user every language they had.
fn load_syntaxes() -> (SyntaxSet, usize) {
    load_syntaxes_from(crate::config::syntaxes_dir().as_deref())
}

/// The same, reading the user's syntaxes from one directory.
fn load_syntaxes_from(dir: Option<&Path>) -> (SyntaxSet, usize) {
    let mut builder = SyntaxSet::load_defaults_nonewlines().into_builder();
    let user_from = builder.syntaxes().len();
    let Some(dir) = dir else {
        return (builder.build(), user_from);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (builder.build(), user_from);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sublime-syntax")
        })
        .collect();
    // Sorted, so the same directory always produces the same set: two files
    // claiming one extension would otherwise resolve by directory order.
    paths.sort();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let fallback = path.file_stem().and_then(|stem| stem.to_str());
        // A syntax that will not parse is left out; the rest still load.
        if let Ok(syntax) = SyntaxDefinition::load_from_str(&text, false, fallback) {
            builder.add(syntax);
        }
    }
    (builder.build(), user_from)
}

/// The bundled themes, plus any `.tmTheme` files in the config directory's
/// `themes/` (spec §6, ADR 0008).
///
/// Returns the full theme set and a list of theme keys loaded from the user's
/// directory. A file that cannot be read or parsed is skipped: one broken theme
/// must not cost the user every theme they had.
fn load_themes() -> (ThemeSet, Vec<String>) {
    load_themes_from(crate::config::themes_dir().as_deref())
}

/// The same, reading the user's themes from one directory.
fn load_themes_from(dir: Option<&Path>) -> (ThemeSet, Vec<String>) {
    let mut themes = ThemeSet::load_defaults();
    if let Ok(dark_modern) =
        ThemeSet::load_from_reader(&mut std::io::Cursor::new(VSCODE_DARK_THEME.as_bytes()))
    {
        themes
            .themes
            .insert("Dark Modern".into(), dark_modern.clone());
        themes
            .themes
            .insert("dark-modern".into(), dark_modern.clone());
        themes
            .themes
            .insert("Dark (Visual Studio)".into(), dark_modern.clone());
        themes
            .themes
            .insert("VS Code Dark".into(), dark_modern.clone());
        themes.themes.insert("vscode".into(), dark_modern);
    }
    if let Ok(monokai) =
        ThemeSet::load_from_reader(&mut std::io::Cursor::new(MONOKAI_THEME.as_bytes()))
    {
        themes.themes.insert("Monokai".into(), monokai);
    }
    if let Ok(hc) =
        ThemeSet::load_from_reader(&mut std::io::Cursor::new(HIGH_CONTRAST_THEME.as_bytes()))
    {
        themes.themes.insert("High Contrast".into(), hc.clone());
        themes.themes.insert("high-contrast".into(), hc);
    }
    let mut user_themes = Vec::new();
    let Some(dir) = dir else {
        return (themes, user_themes);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (themes, user_themes);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("tmTheme"))
        })
        .collect();
    // Sorted, so the same directory always produces the same set.
    paths.sort();
    for path in paths {
        let Ok(theme) = ThemeSet::get_theme(&path) else {
            continue;
        };
        let fallback_stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let theme_name = theme.name.clone();
        if !fallback_stem.is_empty() {
            themes
                .themes
                .insert(fallback_stem.to_owned(), theme.clone());
            user_themes.push(fallback_stem.to_owned());
        }
        if let Some(name) = theme_name
            && name != fallback_stem
        {
            themes.themes.insert(name.clone(), theme);
            user_themes.push(name);
        }
    }
    (themes, user_themes)
}

/// Resolves the theme to use from the theme set.
///
/// When an explicit theme is requested (from configuration), it is looked up
/// by exact key and then case-insensitively. If not found or not specified,
/// any theme from the user's directory is chosen first; otherwise it falls
/// back to the default dark theme (`Dark Modern`).
fn resolve_theme<'a>(
    themes: &'a ThemeSet,
    user_themes: &[String],
    requested: Option<&str>,
) -> &'a Theme {
    if let Some(name) = requested {
        if let Some(theme) = themes.themes.get(name) {
            return theme;
        }
        for (k, theme) in &themes.themes {
            if k.eq_ignore_ascii_case(name) {
                return theme;
            }
        }
    }
    if let Some(first_user) = user_themes.first()
        && let Some(theme) = themes.themes.get(first_user)
    {
        return theme;
    }
    if let Some(theme) = themes.themes.get(THEME) {
        return theme;
    }
    themes
        .themes
        .values()
        .next()
        .expect("bundled themes to contain at least one theme")
}

/// A style from the theme, in the editor's terms.
///
/// Only the foreground and the weight of the text are taken. The editor
/// keeps its own background, so that the selection and the cursor line stay
/// readable, and a token the theme says nothing about is left in the
/// terminal's own colours rather than painted in the theme's default.
fn editor_style(
    style: syntect::highlighting::Style,
    plain: syntect::highlighting::Style,
) -> Option<Style> {
    let bold = style.font_style.contains(FontStyle::BOLD);
    let italic = style.font_style.contains(FontStyle::ITALIC);
    let underline = style.font_style.contains(FontStyle::UNDERLINE);
    if style.foreground == plain.foreground && !bold && !italic && !underline {
        return None;
    }
    let mut result = Style::default();
    if style.foreground.a > 0 {
        result = result.fg(Color::Rgb(
            style.foreground.r,
            style.foreground.g,
            style.foreground.b,
        ));
    }
    if bold {
        result = result.add_modifier(Modifier::BOLD);
    }
    if italic {
        result = result.add_modifier(Modifier::ITALIC);
    }
    if underline {
        result = result.add_modifier(Modifier::UNDERLINED);
    }
    Some(result)
}

/// The worker's loop.
fn work(
    requests: Receiver<Request>,
    sender: Sender<Colours>,
    epoch: Arc<AtomicU64>,
    path: PathBuf,
    requested_theme: Option<String>,
) {
    // Loading the syntax definitions and the theme takes tens of
    // milliseconds, and the editor has fifty for its whole first frame. It
    // happens here, in the worker, so the editor paints plain text and the
    // colours arrive a moment later.
    let (syntaxes, user_from) = load_syntaxes();
    let (themes, user_themes) = load_themes();
    let theme = resolve_theme(&themes, &user_themes, requested_theme.as_deref()).clone();
    let plain = syntect::highlighting::Highlighter::new(&theme).get_default();
    let mut worker = Worker {
        syntaxes,
        theme,
        plain,
        syntax: None,
        name: None,
        detected: false,
        user_from,
        checkpoints: Checkpoints::new(),
    };

    for request in requests {
        if !worker.detected {
            worker.detect(&request, &path);
        }
        // A request that was overtaken reports nothing at all. Sending an
        // empty answer would be indistinguishable from "this window has no
        // colours", and would wipe the ones already on screen.
        let Some(lines) = worker.colour(&request, &epoch) else {
            continue;
        };
        let colours = Colours {
            window: request.window,
            lines,
            syntax: worker.name.clone(),
        };
        if sender.send(colours).is_err() {
            // The editor is gone.
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::edit::Pos;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_file(name: &str, contents: &[u8]) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("zeno-hl-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn doc_of(name: &str, contents: &[u8]) -> (Doc, PathBuf) {
        let path = temp_file(name, contents);
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        (doc, path)
    }

    /// Colours a window and waits for the answer.
    fn colours_of(highlighter: &mut Highlighter, doc: &Doc, first: u64, last: u64) -> Highlighted {
        highlighter.request(doc, first, last);
        for _ in 0..10_000 {
            highlighter.poll(doc);
            if !highlighter.lines().is_empty() {
                return highlighter.lines().clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        highlighter.lines().clone()
    }

    #[test]
    fn a_rust_file_is_recognised() {
        let (doc, path) = doc_of("main.rs", b"fn main() {}\n");
        let mut highlighter = Highlighter::start(&path);
        colours_of(&mut highlighter, &doc, 0, 10);
        assert_eq!(highlighter.syntax(), Some("Rust"));
    }

    /// A directory of the user's own syntaxes, with the given files in it.
    fn syntax_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("zeno-syntax-{}-{name}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        dir
    }

    /// A directory of the user's own themes, with the given files in it.
    fn theme_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("zeno-theme-{}-{name}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        dir
    }

    fn theme_plist(name: &str, keyword_color: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>{name}</string>
    <key>settings</key>
    <array>
        <dict>
            <key>settings</key>
            <dict>
                <key>background</key>
                <string>#000000</string>
                <key>foreground</key>
                <string>#ffffff</string>
                <key>lineHighlight</key>
                <string>#232323</string>
                <key>selection</key>
                <string>#ffffff</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Keyword</string>
            <key>scope</key>
            <string>keyword</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>{keyword_color}</string>
            </dict>
        </dict>
    </array>
</dict>
</plist>
"#
        )
    }

    #[test]
    fn a_theme_in_the_config_directory_is_picked_up() {
        let dir = theme_dir(
            "custom",
            &[("zeno-amber.tmTheme", &theme_plist("Zeno Amber", "#ffaa00"))],
        );
        let (themes, user_themes) = load_themes_from(Some(&dir));
        assert!(themes.themes.contains_key("zeno-amber"));
        assert!(themes.themes.contains_key("Zeno Amber"));
        assert!(user_themes.contains(&"zeno-amber".to_string()));

        let chosen = resolve_theme(&themes, &user_themes, Some("zeno-amber"));
        assert_eq!(chosen.name.as_deref(), Some("Zeno Amber"));

        let auto_chosen = resolve_theme(&themes, &user_themes, None);
        assert_eq!(auto_chosen.name.as_deref(), Some("Zeno Amber"));
    }

    #[test]
    fn a_users_theme_beats_the_bundled_one_for_the_same_name() {
        let dir = theme_dir(
            "override",
            &[(
                "base16-ocean.dark.tmTheme",
                &theme_plist("Base16 Ocean Dark", "#123456"),
            )],
        );
        let (themes, user_themes) = load_themes_from(Some(&dir));
        let chosen = resolve_theme(&themes, &user_themes, Some("base16-ocean.dark"));
        let kw_color = chosen
            .scopes
            .iter()
            .find(|item| item.scope == std::str::FromStr::from_str("keyword").unwrap());
        assert!(kw_color.is_some());
    }

    #[test]
    fn a_broken_theme_file_does_not_take_the_others_with_it() {
        let dir = theme_dir(
            "broken",
            &[
                ("a-broken.tmTheme", "this is not xml <plist>"),
                ("b-good.tmTheme", &theme_plist("Good Theme", "#00ff00")),
            ],
        );
        let (themes, user_themes) = load_themes_from(Some(&dir));
        assert!(themes.themes.contains_key("b-good"));
        assert!(user_themes.contains(&"b-good".to_string()));
        assert!(themes.themes.contains_key("base16-ocean.dark"));
    }

    #[test]
    fn a_missing_theme_directory_is_the_bundled_set() {
        let (themes, user_themes) = load_themes_from(Some(Path::new("/no/such/zeno/themes")));
        assert!(user_themes.is_empty());
        assert!(themes.themes.contains_key("Dark Modern"));
        assert!(themes.themes.contains_key("base16-ocean.dark"));
        let chosen = resolve_theme(&themes, &user_themes, None);
        assert_eq!(chosen.name.as_deref(), Some("Dark Modern"));
    }

    #[test]
    fn resolve_theme_falls_back_gracefully_on_unknown_theme() {
        let (themes, user_themes) = load_themes_from(None);
        let chosen = resolve_theme(&themes, &user_themes, Some("completely-unknown-theme-xyz"));
        assert_eq!(chosen.name.as_deref(), Some("Dark Modern"));
    }

    #[test]
    fn highlighter_uses_explicit_theme() {
        let (doc, path) = doc_of("main.rs", b"fn main() {}\n");
        let mut highlighter = Highlighter::start_with_theme(&path, Some("Solarized (dark)".into()));
        let colours = colours_of(&mut highlighter, &doc, 0, 1);
        assert!(!colours.is_empty());
        let spans = colours.get(&0).expect("line 0 colours");
        assert!(!spans.is_empty());
    }

    /// A minimal syntax definition that claims one extension.
    fn syntax_text(name: &str, extension: &str) -> String {
        format!(
            "%YAML 1.2\n---\nname: {name}\nfile_extensions: [{extension}]\nscope: source.{extension}\ncontexts:\n  main:\n    - match: '\\\\b\\\\d+\\\\b'\n      scope: constant.numeric.zeno\n"
        )
    }

    #[test]
    fn a_syntax_in_the_config_directory_is_picked_up() {
        let dir = syntax_dir(
            "custom",
            &[("note.sublime-syntax", &syntax_text("Zeno Note", "zenonote"))],
        );
        let (syntaxes, user_from) = load_syntaxes_from(Some(&dir));
        let path = Path::new("anything.zenonote");
        let syntax = detect_syntax(&syntaxes, user_from, path, None).expect("the user's syntax");
        assert_eq!(syntax.name, "Zeno Note");
        assert_eq!(user_from, syntaxes.syntaxes().len() - 1);
    }

    #[test]
    fn a_users_syntax_beats_the_bundled_one_for_the_same_extension() {
        // `rs` is bundled, so the file would be Rust without the override.
        let dir = syntax_dir(
            "override",
            &[("rs.sublime-syntax", &syntax_text("Not Rust", "rs"))],
        );
        let (syntaxes, user_from) = load_syntaxes_from(Some(&dir));
        let syntax = detect_syntax(&syntaxes, user_from, Path::new("main.rs"), None)
            .expect("a syntax for rs");
        assert_eq!(syntax.name, "Not Rust", "the user's file is the answer");
    }

    #[test]
    fn a_broken_syntax_file_does_not_take_the_others_with_it() {
        let dir = syntax_dir(
            "broken",
            &[
                ("a-broken.sublime-syntax", "this: is not [a syntax\n"),
                (
                    "b-good.sublime-syntax",
                    &syntax_text("Zeno Good", "zenogood"),
                ),
            ],
        );
        let (syntaxes, user_from) = load_syntaxes_from(Some(&dir));
        let syntax = detect_syntax(&syntaxes, user_from, Path::new("x.zenogood"), None)
            .expect("the syntax that parsed");
        assert_eq!(syntax.name, "Zeno Good");
        // The bundled languages are still there.
        let rust = detect_syntax(&syntaxes, user_from, Path::new("lib.rs"), None);
        assert_eq!(rust.map(|syntax| syntax.name), Some("Rust".to_string()));
    }

    #[test]
    fn a_missing_syntax_directory_is_the_bundled_set() {
        let (syntaxes, user_from) = load_syntaxes_from(Some(Path::new("/no/such/zeno/syntaxes")));
        assert_eq!(user_from, syntaxes.syntaxes().len());
        assert_eq!(
            detect_syntax(&syntaxes, user_from, Path::new("main.rs"), None)
                .map(|syntax| syntax.name),
            Some("Rust".to_string())
        );
    }

    #[test]
    fn a_file_without_a_syntax_stays_plain() {
        let (doc, path) = doc_of("mystery.zzz", b"nothing here\n");
        let mut highlighter = Highlighter::start(&path);
        colours_of(&mut highlighter, &doc, 0, 10);
        assert_eq!(highlighter.syntax(), None);
    }

    #[test]
    fn a_shebang_decides_the_syntax() {
        let (doc, path) = doc_of("script", b"#!/bin/sh\necho hi\n");
        let mut highlighter = Highlighter::start(&path);
        colours_of(&mut highlighter, &doc, 0, 10);
        assert_eq!(highlighter.syntax(), Some("Bourne Again Shell (bash)"));
    }

    #[test]
    fn a_keyword_is_painted_differently_from_the_text_around_it() {
        let (doc, path) = doc_of("keyword.rs", b"fn main() {}\n");
        let mut highlighter = Highlighter::start(&path);
        let lines = colours_of(&mut highlighter, &doc, 0, 10);
        let spans = lines.get(&0).expect("the first line is coloured");
        let keyword = spans
            .iter()
            .find(|span| span.start == 0 && span.end == 2)
            .expect("the keyword is a run of its own");
        assert!(
            keyword.style.fg.is_some(),
            "the keyword has a colour of its own"
        );
        assert!(
            !spans.iter().any(|span| span.start == 3 && span.end == 4),
            "the space after the keyword is not coloured"
        );
    }

    /// The colour covering `column` on `line`, if that column is coloured.
    ///
    /// Runs are merged when their styles agree, so asking for the run that
    /// *starts* at a column would depend on how the merging happened to fall.
    fn colour_at(lines: &Highlighted, line: u64, column: usize) -> Option<Style> {
        lines
            .get(&line)?
            .iter()
            .find(|span| span.start <= column && column < span.end)
            .map(|span| span.style)
    }

    /// Pins the newline convention, in a language where it actually bites.
    ///
    /// The syntax set and the lines handed to it must agree about whether a
    /// line carries its trailing newline: syntect rewrites `\\n` into `$` for
    /// one dump and `$` into `(?m:$)` for the other. The document hands back
    /// lines *without* the newline, so the set must be the one built for
    /// that.
    ///
    /// With the wrong set a `#` comment's end pattern — which wants a `\\n` —
    /// never matches, the comment context never pops, and every line below
    /// comes back as one flat comment-grey run. Rust's own syntax happens
    /// not to depend on it, so a Rust-only test would pass either way;
    /// shell, Python, C, Go, SQL, PHP and R all do (7 of 29 languages tried
    /// while finding this).
    #[test]
    fn a_line_comment_does_not_swallow_the_lines_below_it() {
        let (doc, path) = doc_of(
            "leak.sh",
            b"echo one # note\nif true; then\n  echo two\nfi\n",
        );
        let mut highlighter = Highlighter::start(&path);
        let lines = colours_of(&mut highlighter, &doc, 0, 10);

        let comment = colour_at(&lines, 0, 11).expect("the comment is coloured");
        let spans = lines.get(&1).expect("the line below is coloured");
        assert_ne!(
            colour_at(&lines, 1, 0),
            Some(comment),
            "the comment's colour leaked onto the line below it: the syntax \
             set and the lines disagree about the trailing newline, so the \
             comment's end pattern never matched and its context never popped"
        );
        assert!(
            spans.len() > 1,
            "the line below came back as one flat run — '{:?}' — which is \
             what a swallowed comment looks like",
            spans.first()
        );
    }

    /// The parser's state is what makes a window cheaper than a whole file,
    /// and a block comment is the state that has to survive the walk.
    #[test]
    fn a_block_comment_spans_the_lines_it_covers() {
        let (doc, path) = doc_of("block.rs", b"/* opened\n   still inside */\nfn main() {}\n");
        let mut highlighter = Highlighter::start(&path);
        let lines = colours_of(&mut highlighter, &doc, 0, 10);

        let inside = colour_at(&lines, 1, 0).expect("the second line is coloured");
        let opener = colour_at(&lines, 0, 0).expect("the first line is coloured");
        assert_eq!(
            inside, opener,
            "the middle line carries the comment's colour"
        );

        let keyword = colour_at(&lines, 2, 0).expect("the line after is coloured");
        assert_ne!(
            keyword, opener,
            "the comment was closed, so 'fn' is a keyword again"
        );
    }

    /// A cancelled request must be silent.
    ///
    /// The worker abandons a request the moment a newer one arrives. If it
    /// reported the abandoned one as an empty answer instead, that answer
    /// would be indistinguishable from a real one and the editor would blank
    /// the screen it had already painted — a flash of plain text every time
    /// the user scrolled. So: `poll` may report a change only when it has
    /// colours to show for it.
    #[test]
    fn a_cancelled_request_never_reports_an_empty_answer() {
        let lines: Vec<u8> = (0..40_000)
            .map(|n| format!("fn f{n}() {{}} // note\n"))
            .collect::<Vec<_>>()
            .concat()
            .into_bytes();
        let (doc, path) = doc_of("racing.rs", &lines);
        let mut highlighter = Highlighter::start(&path);

        // Two windows asked for back to back: the first is still being
        // coloured when the second supersedes it, so the first is cancelled.
        highlighter.request(&doc, 0, 40_000);
        highlighter.request(&doc, 0, 40_000 + 5_000);

        for _ in 0..60_000 {
            if highlighter.poll(&doc) {
                assert!(
                    !highlighter.lines().is_empty(),
                    "poll reported a change but handed back no colours — \
                     a cancelled request answered as an empty one"
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
        panic!("no answer ever arrived");
    }

    #[test]
    fn a_line_over_the_limit_is_not_highlighted() {
        let long = "x".repeat(MAX_LINE_BYTES + 1);
        let (doc, path) = doc_of("long.rs", format!("fn main() {{}}\n{long}\n").as_bytes());
        let mut highlighter = Highlighter::start(&path);
        let lines = colours_of(&mut highlighter, &doc, 0, 10);
        assert!(
            !lines.contains_key(&1),
            "the long line is left plain rather than parsed"
        );
        assert!(lines.contains_key(&0), "the rest of the file is coloured");
    }

    #[test]
    fn a_result_from_an_older_document_is_discarded() {
        let (mut doc, path) = doc_of("stale.rs", b"fn main() {}\n");
        let mut highlighter = Highlighter::start(&path);
        colours_of(&mut highlighter, &doc, 0, 10);
        assert!(!highlighter.lines().is_empty());
        // The document changes: the colours on hand are about the old text.
        doc.edit(Pos::new(0, 0), Pos::new(0, 2), "let");
        highlighter.poll(&doc);
        assert!(
            highlighter.lines().is_empty(),
            "the colours of the old text are dropped"
        );
    }

    #[test]
    fn the_window_is_coloured_where_it_was_asked_for() {
        let contents: Vec<u8> = (0..100)
            .map(|n| format!("fn f{n}() {{}}\n"))
            .collect::<Vec<_>>()
            .concat()
            .into_bytes();
        let (doc, path) = doc_of("window.rs", &contents);
        let mut highlighter = Highlighter::start(&path);
        let lines = colours_of(&mut highlighter, &doc, 40, 45);
        assert!(
            lines.contains_key(&40),
            "{:?}",
            lines.keys().collect::<Vec<_>>()
        );
        assert!(lines.contains_key(&45));
        assert!(
            !lines.contains_key(&39),
            "the lines before the window are not coloured"
        );
    }

    #[test]
    fn the_colours_follow_an_edit() {
        let (mut doc, path) = doc_of("edited.rs", b"fn main() {}\n");
        let mut highlighter = Highlighter::start(&path);
        colours_of(&mut highlighter, &doc, 0, 10);
        // Typing turns the keyword into something else.
        doc.edit(Pos::new(0, 0), Pos::new(0, 2), "xx");
        for _ in 0..2000 {
            highlighter.request(&doc, 0, 10);
            highlighter.poll(&doc);
            if let Some(spans) = highlighter.lines().get(&0)
                && !spans.iter().any(|span| span.start == 0 && span.end == 2)
            {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the keyword was not re-coloured: {:?}", highlighter.lines());
    }

    #[test]
    fn a_checkpoint_is_kept_only_until_the_text_above_it_changes() {
        let contents: String = (0..8200).map(|n| format!("fn f{n}() {{}}\n")).collect();
        let (mut doc, _path) = doc_of("checkpoints.rs", contents.as_bytes());

        // The pieces a checkpoint is made of.
        let syntaxes = SyntaxSet::load_defaults_nonewlines();
        let themes = ThemeSet::load_defaults();
        let theme = themes.themes.values().next().expect("a theme");
        let styler = syntect::highlighting::Highlighter::new(theme);
        let syntax = &syntaxes.syntaxes()[0];
        let fresh = |version: u64| Checkpoint {
            parse: ParseState::new(syntax),
            state: HighlightState::new(&styler, ScopeStack::new()),
            version,
        };

        let mut checkpoints = Checkpoints::new();
        checkpoints.insert(4000, fresh(doc.version()), 4000);

        // An edit above the checkpoint makes it false: the state at line
        // 4000 was computed from text that has changed.
        doc.edit(Pos::new(0, 0), Pos::new(0, 0), "// the top\n");
        let after_edit = doc.snapshot();
        assert!(
            checkpoints.before(5000, &after_edit).is_none(),
            "the checkpoint below the edit is dropped"
        );

        // The request that follows rebuilds it, at the version it answers.
        checkpoints.insert(4000, fresh(doc.version()), 5000);
        assert!(
            checkpoints.before(5000, &after_edit).is_some(),
            "the rebuilt checkpoint is history's, not the edit's: it must survive"
        );

        // An edit below the checkpoint leaves it true...
        doc.edit(Pos::new(8000, 0), Pos::new(8000, 0), "// below");
        let later = doc.snapshot();
        assert!(
            checkpoints.before(5000, &later).is_some(),
            "an edit below the checkpoint does not touch it"
        );

        // ...until the text above it changes again.
        doc.edit(Pos::new(3, 0), Pos::new(3, 0), "// again");
        let newest = doc.snapshot();
        assert!(
            checkpoints.before(5000, &newest).is_none(),
            "the next edit above it drops it"
        );
    }

    #[test]
    fn an_edit_below_the_window_keeps_the_colours_above_it() {
        let contents: String = (0..60).map(|n| format!("fn f{n}() {{}}\n")).collect();
        let (mut doc, path) = doc_of("kept.rs", contents.as_bytes());
        let mut highlighter = Highlighter::start(&path);

        // The document was edited near the top some time ago...
        doc.edit(Pos::new(2, 0), Pos::new(2, 0), "x");
        // ...the window on screen was coloured after that edit...
        colours_of(&mut highlighter, &doc, 40, 50);
        assert!(highlighter.lines().get(&40).is_some());

        // ...and now a keystroke lands inside it. Only the lines from the
        // edit down are no longer to be believed; the early edit from long
        // ago must not be remembered against this one.
        doc.edit(Pos::new(48, 0), Pos::new(48, 0), "y");
        highlighter.poll(&doc);
        assert!(
            highlighter.lines().get(&40).is_some(),
            "the colours above the edit are kept"
        );
        assert!(
            highlighter.lines().get(&48).is_none(),
            "the edited line is not"
        );
    }
}
