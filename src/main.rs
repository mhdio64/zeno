//! zeno — a fast, non-modal terminal text editor.
//!
//! One file per run. The file is mapped into memory, the visible part is
//! indexed on demand, and the viewport is repainted after every keystroke.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget};

use zeno_cli::clipboard;
use zeno_cli::config::Config;
use zeno_cli::doc::edit::Pos;
use zeno_cli::doc::{Doc, Snapshot, identity_of};
use zeno_cli::highlight::Highlighter;
use zeno_cli::indent::{self, IndentStyle};
use zeno_cli::keys::Keymap;
use zeno_cli::motion::{self, Motion};
use zeno_cli::panel::{self, Palette};
use zeno_cli::prompt::{Prompt, Purpose};
use zeno_cli::render::{self, View, ViewState};
use zeno_cli::search::{MatchPos, Pattern, Scope, SearchEvent, SearchJob};
use zeno_cli::sequence::{Action, Command, Operator, Sequences, Step, Target};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The line-comment prefix for a file, from its extension.
///
/// A file type with no line comment — HTML, Markdown, JSON — has no answer,
/// and the command says so rather than inventing one: `<!-- -->` and `{/* */}`
/// are not lines a reader would call commented.
fn line_comment_prefix(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "rs" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "js" | "mjs" | "cjs" | "jsx" | "ts"
        | "tsx" | "java" | "go" | "kt" | "kts" | "swift" | "scss" | "less" | "php" | "cs"
        | "dart" | "zig" | "groovy" | "gradle" => "//",
        "py" | "pyi" | "rb" | "sh" | "bash" | "zsh" | "fish" | "toml" | "yaml" | "yml" | "ini"
        | "cfg" | "conf" | "pl" | "pm" | "r" | "jl" | "ps1" | "gitignore" => "#",
        "sql" | "lua" | "hs" | "elm" | "ada" => "--",
        "lisp" | "clj" | "cljs" | "el" | "scm" | "asm" | "s" => ";",
        "vim" => "\"",
        "erl" | "hrl" => "%",
        "bat" | "cmd" => "REM",
        _ => return None,
    })
}

/// The most lines one command may edit at once.
///
/// Not a product limit — it is the point where the line-anchored model stops
/// being able to answer inside a frame (see `docs/performance.md`, finding 8).
/// Ten thousand lines costs about 20 ms to delete; a hundred thousand would
/// cost seconds.
const MAX_SPAN_LINES: u64 = 10_000;

/// The answers offered when the file changed on disk under the editor.
const CHANGED_ON_DISK: &[(char, &str)] = &[('r', "reload"), ('o', "overwrite"), ('c', "cancel")];

/// What the user said to a prompt.
enum Answer {
    Text(String),
    Choice(char),
}

#[cfg(windows)]
unsafe extern "system" {
    /// Maps a character to the virtual-key code of the key that types it on
    /// the active keyboard layout. Returns -1 when no key types it.
    fn VkKeyScanW(ch: u16) -> i16;
}

/// The letter a `Ctrl` chord stands for.
///
/// Chords must not be identified by the character the layout produces for
/// them. On Windows, `Ctrl+S` arrives as the character on the S key of the
/// *active* layout — `'س'` on a Persian layout, `'ы'` on Russian — so
/// matching on `'s'` would leave every chord dead for anyone not typing on
/// a Latin layout. The physical key is the stable identity, so the character
/// is mapped back to the key that types it.
fn chord_letter(code: KeyCode) -> Option<char> {
    let KeyCode::Char(c) = code else {
        return None;
    };
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_lowercase());
    }
    // A bare control character, as a VT-style terminal sends it.
    let n = c as u32;
    if (1..=26).contains(&n) {
        return char::from_u32(n + 0x60);
    }
    #[cfg(windows)]
    {
        layout_letter(c)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The ASCII letter typed by the key that produces `c` on the active layout.
#[cfg(windows)]
fn layout_letter(c: char) -> Option<char> {
    let mut buf = [0u16; 2];
    let encoded = c.encode_utf16(&mut buf);
    if encoded.len() != 1 {
        return None;
    }
    let vk = unsafe { VkKeyScanW(encoded[0]) };
    if vk == -1 {
        return None;
    }
    // The low byte is the virtual-key code, the high byte the shift state.
    let vk = (vk as u16) & 0xFF;
    match vk {
        // 'A'..='Z'
        0x41..=0x5A => char::from_u32(vk as u32).map(|ch| ch.to_ascii_lowercase()),
        _ => None,
    }
}

/// The prompt as a widget: the label and the text being typed.
///
/// A search prompt also shows how many matches the pattern has, as soon as
/// the background scan has counted them: that is the number the user is
/// really looking for while typing.
fn prompt_line(prompt: &Prompt, search: Option<&Search>) -> Paragraph<'static> {
    let style = Style::default()
        .bg(Color::Rgb(28, 28, 34))
        .fg(Color::Yellow);
    let mut spans = vec![Span::styled(prompt.line(), style)];
    if prompt.purpose() == Purpose::Search
        && let Some(total) = search.and_then(|search| search.total)
    {
        let count = if total == 1 {
            "1 match".to_string()
        } else {
            format!("{total} matches")
        };
        spans.push(Span::styled(format!("  {count}"), style));
    }
    Paragraph::new(Line::from(spans)).style(style)
}

/// What an operator's target is called, for messages.
fn target_label(target: Target) -> &'static str {
    match target {
        Target::Line => "line",
        Target::Motion(motion) => motion.label(),
        Target::Object { object, .. } => object.label(),
    }
}

/// The same range, always running forwards.
fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if b < a { (b, a) } else { (a, b) }
}

/// Whether the text before the cursor opens a block that the next line
/// belongs inside.
fn opens_block(before: &str) -> bool {
    let trimmed = before.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    // A line that is nothing but the opener is the plainest case, and it is
    // the one an empty `{` produces.
    matches!(trimmed.chars().last(), Some('{' | '(' | '[' | ':'))
}

/// Whether the block the line opens is closed again on the same line — the
/// empty `()` of a call, which a new line should not go inside.
fn closes_itself(before: &str, after: Option<char>) -> bool {
    let trimmed = before.trim_end();
    let Some(opener) = trimmed.chars().last() else {
        return false;
    };
    let closer = match opener {
        '{' => '}',
        '(' => ')',
        '[' => ']',
        _ => return false,
    };
    after == Some(closer)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path: Option<PathBuf> = None;
    let mut background_index = true;
    let mut bench_open: Option<PathBuf> = None;
    let mut force = false;
    let mut stdin_mode = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("zeno {VERSION}");
                return ExitCode::SUCCESS;
            }
            "--no-background-index" => background_index = false,
            // `-` is a file name here, not an option: it means the pipe.
            "-" => stdin_mode = true,
            "--force" => force = true,
            // Internal: the child `zeno -` starts so that the keyboard and
            // the document can come from different places.
            "--pipe-edit" => {
                let (Some(spool), Some(result)) = (args.get(i + 1), args.get(i + 2)) else {
                    eprintln!("zeno: --pipe-edit needs a spool and a result file");
                    return ExitCode::FAILURE;
                };
                return match edit_spooled(Path::new(spool), force, Some(Path::new(result))) {
                    Ok(_) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("zeno: {e}");
                        ExitCode::FAILURE
                    }
                };
            }
            "--bench-open" => {
                i += 1;
                match args.get(i) {
                    Some(p) => bench_open = Some(PathBuf::from(p.clone())),
                    None => {
                        eprintln!("zeno: --bench-open needs a file");
                        return ExitCode::FAILURE;
                    }
                }
            }
            other if other.starts_with('-') => {
                eprintln!("zeno: unknown option `{other}`");
                print_help();
                return ExitCode::FAILURE;
            }
            other => path = Some(PathBuf::from(other)),
        }
        i += 1;
    }

    // Diagnostic mode: open a file, paint one frame, report how long it took.
    // Used by zeno-bench to measure process launch to first frame.
    if let Some(bench_path) = bench_open {
        return bench_open_frame(&bench_path);
    }

    if stdin_mode {
        return match run_stdin(force) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("zeno -: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let Some(path) = path else {
        eprintln!("zeno: no file given");
        print_help();
        return ExitCode::FAILURE;
    };

    match run(&path, background_index, force) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("zeno: {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("zeno {VERSION} — a fast, non-modal terminal text editor");
    println!();
    println!("USAGE:");
    println!("    zeno [OPTIONS] <FILE>");
    println!("    zeno [OPTIONS] -            read stdin, write the buffer to stdout on exit");
    println!();
    println!("OPTIONS:");
    println!("        --no-background-index   do not scan the whole file in the background");
    println!("        --force                 open a binary file as raw text");
    println!("        --bench-open <FILE>     open a file, paint one frame, print the time, exit");
    println!("    -h, --help                 print this help");
    println!("    -V, --version              print the version");
}

/// Opens a file, paints one frame into an off-screen buffer and prints the
/// elapsed time in milliseconds.
fn bench_open_frame(path: &Path) -> ExitCode {
    let start = std::time::Instant::now();
    let result = (|| -> Result<f64, String> {
        let doc = Doc::open(path).map_err(|e| e.to_string())?;
        let area = ratatui::layout::Rect::new(0, 0, 120, 40);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let state = ViewState::default();
        let view = View {
            doc: &doc,
            state: &state,
            highlight: None,
        };
        view.render(area, &mut buf);
        Ok(start.elapsed().as_secs_f64() * 1000.0)
    })();
    match result {
        Ok(elapsed) => {
            println!("{elapsed:.3}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("zeno: {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// What the editor should do when it is done.
fn run(path: &Path, background_index: bool, force: bool) -> Result<(), String> {
    let doc = Doc::open(path).map_err(|e| e.to_string())?;
    if doc.looks_binary(8192) && !force {
        return Err("refusing to open a binary file (--force opens it)".into());
    }
    if background_index {
        let _ = doc.spawn_indexer();
    }

    let mut editor = Editor::new(doc, path.to_path_buf());
    editor.apply_settings(&zeno_cli::config::load());
    editor.run()
}

/// `zeno -`: read a document from stdin, edit it, write it back to stdout.
///
/// The editor works on a real file — the whole model is built on mapping one
/// — so the pipe is spooled to a temporary file first. The printed text is
/// what the editor holds at the end, not what came in: an edit that was saved
/// and one that was not both leave the buffer as the answer (spec §4.1).
///
/// A pipe cannot also carry keystrokes, so when stdin is not a terminal the
/// editing session runs in a child process whose standard input and output
/// are the terminal itself; the parent waits and prints the buffer the child
/// left for it. When stdin already is the terminal it is edited here, and
/// there is nothing to re-open.
fn run_stdin(force: bool) -> Result<(), String> {
    use std::io::IsTerminal;

    let mut buffer = Vec::new();
    io::stdin()
        .read_to_end(&mut buffer)
        .map_err(|e| format!("reading stdin: {e}"))?;
    let directory = std::env::temp_dir();
    let pid = std::process::id();
    let spool = directory.join(format!("zeno-stdin-{pid}.txt"));
    fs::write(&spool, &buffer).map_err(|e| format!("spooling stdin: {e}"))?;

    let edited = if io::stdin().is_terminal() {
        edit_spooled(&spool, force, None)
    } else {
        let result = directory.join(format!("zeno-stdin-{pid}.out"));
        let _ = fs::remove_file(&result);
        edit_on_terminal(&spool, &result, force)
            .and_then(|()| fs::read(&result).map_err(|e| format!("reading the buffer back: {e}")))
    };
    let _ = fs::remove_file(&spool);
    let text = edited?;

    let mut out = io::stdout();
    out.write_all(&text)
        .and_then(|()| out.flush())
        .map_err(|e| format!("writing stdout: {e}"))
}

/// Edits a spooled document, writing the buffer to `result` when given.
///
/// Returns the buffer too, so the in-process case needs no file to read back.
fn edit_spooled(path: &Path, force: bool, result: Option<&Path>) -> Result<Vec<u8>, String> {
    let doc = Doc::open(path).map_err(|e| e.to_string())?;
    if doc.looks_binary(8192) && !force {
        return Err("refusing to edit a binary stream (--force opens it)".into());
    }
    let mut editor = Editor::new(doc, path.to_path_buf());
    // The spool file is a stand-in, not the user's file: writing it back is
    // pointless, and saving it must never look like a save of the document.
    editor.pipe_out = true;
    editor.apply_settings(&zeno_cli::config::load());
    editor.run()?;
    let text = editor.text();
    if let Some(result) = result {
        fs::write(result, &text).map_err(|e| format!("writing the buffer back: {e}"))?;
    }
    Ok(text.into_bytes())
}

/// Runs the editing session in a child that owns the terminal.
///
/// The child is this same binary with an internal flag, so there is one
/// editor and not two; only its standard streams differ.
fn edit_on_terminal(spool: &Path, result: &Path, force: bool) -> Result<(), String> {
    let (input, output) = terminal_streams()?;
    let output_copy = output.try_clone().map_err(|e| e.to_string())?;
    let program = std::env::current_exe().map_err(|e| format!("finding the editor: {e}"))?;
    let mut command = std::process::Command::new(program);
    command
        .arg("--pipe-edit")
        .arg(spool)
        .arg(result)
        .stdin(input)
        .stdout(output)
        .stderr(output_copy);
    if force {
        command.arg("--force");
    }
    let status = command
        .status()
        .map_err(|e| format!("starting the editor: {e}"))?;
    if !status.success() {
        return Err(format!("the editing session ended with {status}"));
    }
    Ok(())
}

/// A readable and a writable handle on the terminal itself.
///
/// Standard input is the pipe here, which is why the terminal has to be
/// opened by name: `CONIN$`/`CONOUT$` on Windows, `/dev/tty` elsewhere.
fn terminal_streams() -> Result<(std::fs::File, std::fs::File), String> {
    #[cfg(windows)]
    {
        let input =
            fs::File::open("CONIN$").map_err(|e| format!("opening the console for input: {e}"))?;
        let output = fs::File::options()
            .write(true)
            .open("CONOUT$")
            .map_err(|e| format!("opening the console for output: {e}"))?;
        Ok((input, output))
    }
    #[cfg(not(windows))]
    {
        let input = fs::File::open("/dev/tty")
            .map_err(|e| format!("opening the terminal for input: {e}"))?;
        let output = fs::File::options()
            .write(true)
            .open("/dev/tty")
            .map_err(|e| format!("opening the terminal for output: {e}"))?;
        Ok((input, output))
    }
}

/// Which end of the document to try when a search finds nothing.
///
/// Cycling through the matches wraps around: a search that reaches the end
/// of the file starts again at the top, and one that reaches the top starts
/// again at the end.
#[derive(Debug, Clone, Copy)]
enum Wrap {
    Start,
    End,
}

/// A search in progress.
///
/// The pattern is compiled once and used twice: the renderer matches it
/// against the visible lines to draw the highlight, and background scans walk
/// the document to count the matches and to find the next one.
struct Search {
    /// The compiled pattern.
    pattern: Pattern,
    /// A scan counting every match in the document.
    count: Option<SearchJob>,
    /// The document version the count was started against.
    count_version: u64,
    /// How many matches the document has, once the count scan has finished.
    total: Option<u64>,
    /// A scan looking for the next match.
    job: Option<SearchJob>,
    /// The document version the navigation scan was started against.
    job_version: u64,
    /// The end of the document to try if the scan finds nothing.
    wrap: Option<Wrap>,
    /// Whether Enter has already jumped to the first match for this pattern.
    cycled: bool,
}

struct Editor {
    doc: Doc,
    path: PathBuf,
    view: ViewState,
    dirty: bool,
    message: String,
    quit: bool,
    /// Height of the text area, learned from the last draw.
    viewport_height_hint: u16,
    /// The leader sequence being typed, if any.
    seq: Sequences,
    /// The internal clipboard. `Leader y`, `Ctrl+C` and `Ctrl+X` fill it;
    /// `Ctrl+V` pastes it.
    clipboard: String,
    /// An escape sequence for the system clipboard, written after the next
    /// frame so it cannot confuse the terminal mid-draw.
    osc: Option<String>,
    /// The question being asked, if any. While this is open it owns the
    /// keyboard: no chord fires and no sequence starts.
    prompt: Option<Prompt>,
    /// The search in progress, if any. Its pattern is what the viewport
    /// highlights.
    search: Option<Search>,
    /// The syntax highlighter. It colours the viewport in the background.
    highlight: Highlighter,
    /// Named spots the user has remembered (`Leader m a` … `Leader ' a`).
    marks: std::collections::HashMap<char, Pos>,
    /// The pattern a replace is about to look for, while it is asking what to
    /// put in its place.
    pending_replace: Option<Pattern>,
    /// Whether an opening bracket or quote brings its partner with it.
    auto_pair: bool,
    /// The panel drawn over the viewport, if one is open.
    overlay: Option<Overlay>,
    /// Whether the buffer belongs to a pipe and is printed when the editor
    /// exits, rather than being a file with a name (`zeno -`).
    pipe_out: bool,
    /// What one indent level is written as, decided at startup from
    /// `.editorconfig`, the file's own content, and `zeno.toml`, in that
    /// order (spec §4.4).
    indent: IndentStyle,
    /// Whether the file is written on its own after a moment of quiet.
    autosave: bool,
    /// When the last key was handled, for the autosave timer.
    last_key: std::time::Instant,
    /// The leader and the chords `zeno.toml` added, on top of the defaults.
    keys: Keymap,
}

/// A panel that covers the document while it is open.
#[derive(Debug, Clone)]
enum Overlay {
    /// Every binding, and the operator grammar under it.
    Help { offset: usize },
    /// The command palette, with its query.
    Palette(Palette),
}

impl Editor {
    fn new(doc: Doc, path: PathBuf) -> Self {
        // The highlighter is started first: it works in the background, and
        // the editor paints plain text until the colours arrive.
        let highlight = Highlighter::start(&path);
        Self {
            doc,
            path,
            view: ViewState::default(),
            dirty: false,
            message: String::new(),
            quit: false,
            viewport_height_hint: 24,
            seq: Sequences::new(),
            clipboard: String::new(),
            osc: None,
            prompt: None,
            search: None,
            highlight,
            marks: std::collections::HashMap::new(),
            pending_replace: None,
            auto_pair: true,
            overlay: None,
            pipe_out: false,
            indent: indent::DEFAULT,
            autosave: false,
            last_key: std::time::Instant::now(),
            keys: Keymap::default(),
        }
    }

    /// Applies what `zeno.toml` asked for.
    ///
    /// Nothing here can fail: every setting has a default, so a file that
    /// says only `tab_width = 2` changes only that (spec §6).
    fn apply_settings(&mut self, config: &Config) {
        let settings = &config.settings;
        self.view.word_wrap = settings.word_wrap;
        self.view.show_line_numbers = settings.line_numbers;
        self.view.show_relative_numbers = settings.relative_line_numbers;
        self.indent = indent::resolve(&self.path, &self.doc, settings);
        self.auto_pair = settings.auto_pair;
        self.autosave = settings.autosave;
        if settings.theme.as_deref() != self.highlight.theme() {
            self.highlight = Highlighter::start_with_theme(&self.path, settings.theme.clone());
        }
        self.keys = Keymap::from_config(&config.keys);
    }

    /// Writes the file after a moment of quiet, when autosave is on.
    ///
    /// The wait is what makes it affordable: writing on every keystroke would
    /// cost the whole file per character. A piped buffer has no file worth
    /// keeping up to date, so it is never autosaved.
    fn autosave_if_idle(&mut self) {
        if !self.autosave || !self.dirty || self.pipe_out {
            return;
        }
        if self.last_key.elapsed() < Duration::from_millis(600) {
            return;
        }
        self.save();
    }

    /// The whole document, as it would be written to disk.
    ///
    /// Reading it costs the whole index, which is why it is only asked for
    /// when the buffer is about to be printed (`zeno -`).
    fn text(&mut self) -> String {
        self.doc.index_fully();
        let last = self.doc.line_count().saturating_sub(1);
        let mut text = self
            .doc
            .text_in_range(Pos::new(0, 0), Pos::new(last, self.line_len(last)));
        // The line model does not keep a trailing newline as a line, so the
        // one property the save path preserves has to be put back here.
        if self.doc.ends_with_newline() {
            text.push('\n');
        }
        text
    }

    fn run(&mut self) -> Result<(), String> {
        enable_raw_mode().map_err(|e| e.to_string())?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).map_err(|e| e.to_string())?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

        let result = self.event_loop(&mut terminal);

        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
        terminal.show_cursor().ok();

        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<(), String> {
        self.draw(terminal).map_err(|e| e.to_string())?;
        self.flush_system_clipboard().map_err(|e| e.to_string())?;
        while !self.quit {
            if !event::poll(Duration::from_millis(30)).map_err(|e| e.to_string())? {
                // Nothing happened: the background index may have advanced,
                // and the file may have been quiet long enough to be written.
                self.autosave_if_idle();
                self.draw(terminal).map_err(|e| e.to_string())?;
                continue;
            }
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    self.last_key = std::time::Instant::now();
                    self.handle_key(key);
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
            self.draw(terminal).map_err(|e| e.to_string())?;
            self.flush_system_clipboard().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Writes a pending OSC 52 sequence, if a copy produced one.
    ///
    /// Written straight to stdout, which is the same handle the backend
    /// draws through, and only between frames so it cannot land in the middle
    /// of one. A terminal that ignores OSC 52 simply drops it.
    fn flush_system_clipboard(&mut self) -> io::Result<()> {
        if let Some(sequence) = self.osc.take() {
            let mut out = io::stdout();
            out.write_all(sequence.as_bytes())?;
            out.flush()?;
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // An open prompt owns every key before the leader does: while the
        // editor is asking a question, a keystroke is an answer.
        if self.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        // A panel owns every key while it is open, for the same reason.
        if let Some(overlay) = self.overlay.take() {
            self.handle_overlay_key(overlay, key);
            return;
        }

        // A pending leader sequence owns the next key, whatever it is.
        if !self.seq.is_idle() {
            self.feed_sequence(key);
            return;
        }

        if self.keys.is_leader(&key) {
            self.seq.start();
            self.message =
                "Actions: [s] Save  [q] Quit  [f] Find  [p] Palette  [?] Help  [w] Wrap".into();
            return;
        }

        // A chord `zeno.toml` added is looked at before the built-in ones, so
        // a config file can move a command without unbinding where it was.
        if let Some(command) = self.keys.command_for(&key) {
            self.command(command);
            return;
        }

        if let KeyCode::F(1) = key.code {
            self.open_palette();
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if alt && !ctrl {
            // `Alt+Z` toggles wrapping, the way it does in VSCode. Matched on
            // the key rather than the character, for the reason in ADR 0010.
            if let KeyCode::Char('z' | 'Z') = key.code {
                self.toggle_word_wrap();
                return;
            }
        }
        if ctrl {
            // `Ctrl+/` arrives as `/` from a console key, and as the control
            // byte it stands for from a VT-style terminal over a pty.
            if let KeyCode::Char('/' | '_' | '\u{1f}') = key.code {
                self.toggle_line_comment();
                return;
            }
            match chord_letter(key.code) {
                // `Ctrl+C` means copy, so it must not quit: `Leader q` quits.
                Some('a') => {
                    self.select_all();
                    return;
                }
                Some('c') => {
                    self.copy();
                    return;
                }
                Some('x') => {
                    self.cut();
                    return;
                }
                Some('s') => {
                    self.save();
                    return;
                }
                Some('v') => {
                    self.paste();
                    return;
                }
                Some('d') => {
                    self.duplicate_line();
                    return;
                }
                Some('f') => {
                    self.open_find();
                    return;
                }
                Some('h') => {
                    self.open_replace();
                    return;
                }
                Some('g') => {
                    self.open_prompt(Purpose::GoToLine);
                    return;
                }
                Some('w') => {
                    self.delete_word_before();
                    return;
                }
                Some('k') => {
                    self.delete_to_end_of_line();
                    return;
                }
                Some('e') => {
                    self.clear_selection();
                    self.set_cursor_col(usize::MAX);
                    return;
                }
                Some('l') => {
                    self.select_line();
                    return;
                }
                Some('p') => {
                    self.open_palette();
                    return;
                }
                Some('z') => {
                    self.undo();
                    return;
                }
                Some('y') => {
                    self.redo();
                    return;
                }
                Some('q') => {
                    if self.dirty {
                        self.prompt = Some(Prompt::choice(
                            Purpose::ChangedOnDisk,
                            &[('y', "Save"), ('n', "Discard")],
                        ));
                        self.message.clear();
                    } else {
                        self.quit = true;
                    }
                    return;
                }
                _ => {}
            }
        }

        // Shift turns any movement into an extension of the selection.
        if shift {
            match key.code {
                KeyCode::Left => {
                    self.begin_selection();
                    self.move_cursor(-1, 0);
                    return;
                }
                KeyCode::Right => {
                    self.begin_selection();
                    self.move_cursor(1, 0);
                    return;
                }
                KeyCode::Up => {
                    self.begin_selection();
                    self.move_cursor(0, -1);
                    return;
                }
                KeyCode::Down => {
                    self.begin_selection();
                    self.move_cursor(0, 1);
                    return;
                }
                KeyCode::Home => {
                    self.begin_selection();
                    self.set_cursor_col(0);
                    return;
                }
                KeyCode::End => {
                    self.begin_selection();
                    self.set_cursor_col(usize::MAX);
                    return;
                }
                KeyCode::PageUp => {
                    self.begin_selection();
                    self.page(-1);
                    return;
                }
                KeyCode::PageDown => {
                    self.begin_selection();
                    self.page(1);
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Home if ctrl => {
                self.clear_selection();
                self.goto(0);
            }
            KeyCode::End if ctrl => {
                self.clear_selection();
                self.goto_end();
            }
            KeyCode::Char(c) if !ctrl => self.insert_char(c),
            KeyCode::Enter => self.split_line(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Tab => self.insert_text(&self.indent.unit()),
            KeyCode::BackTab => self.unindent_here(),
            KeyCode::Left => {
                self.clear_selection();
                self.move_cursor(-1, 0);
            }
            KeyCode::Right => {
                self.clear_selection();
                self.move_cursor(1, 0);
            }
            KeyCode::Up => {
                self.clear_selection();
                self.move_cursor(0, -1);
            }
            KeyCode::Down => {
                self.clear_selection();
                self.move_cursor(0, 1);
            }
            KeyCode::Home => {
                self.clear_selection();
                self.set_cursor_col(0);
            }
            KeyCode::End => {
                self.clear_selection();
                self.set_cursor_col(usize::MAX);
            }
            KeyCode::PageUp => {
                self.clear_selection();
                self.page(-1);
            }
            KeyCode::PageDown => {
                self.clear_selection();
                self.page(1);
            }
            KeyCode::Esc => {
                self.message.clear();
                self.clear_selection();
                // While a search is highlighted, one Esc already closed the
                // prompt; a second Esc clears the highlight and returns to
                // ordinary editing.
                if self.search.is_some() {
                    self.clear_search();
                }
            }
            _ => {}
        }
    }

    /// Routes one key into the open prompt.
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let mut submit: Option<Answer> = None;
        let mut cancel = false;
        let mut escape_leaves_search = false;
        let mut search_enter = false;
        let mut edited = false;
        {
            let Some(prompt) = self.prompt.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Esc => {
                    if prompt.purpose() == Purpose::Search {
                        // The first Esc leaves the prompt open as a
                        // highlighted search, the way VSCode keeps the
                        // selection alive after closing the find box.
                        escape_leaves_search = true;
                    } else {
                        cancel = true;
                    }
                }
                KeyCode::Enter if prompt.is_text() => {
                    if prompt.purpose() == Purpose::Search {
                        search_enter = true;
                    } else {
                        submit = Some(Answer::Text(prompt.text().to_owned()));
                    }
                }
                KeyCode::Char(c) if !ctrl && !prompt.is_text() => {
                    if let Some(choice) = prompt.choose(c) {
                        submit = Some(Answer::Choice(choice));
                    }
                }
                KeyCode::Char(c) if !ctrl => {
                    prompt.insert(c);
                    edited = true;
                }
                KeyCode::Backspace => {
                    prompt.backspace();
                    edited = true;
                }
                KeyCode::Delete => {
                    prompt.delete();
                    edited = true;
                }
                KeyCode::Left => prompt.left(),
                KeyCode::Right => prompt.right(),
                KeyCode::Home => prompt.home(),
                KeyCode::End => prompt.end(),
                _ => {}
            }
        }

        if escape_leaves_search {
            // Keep the highlight the prompt just built, and close the
            // prompt so the document is editable again.
            self.prompt = None;
            self.message.clear();
            return;
        }
        if cancel {
            self.prompt = None;
            self.clear_search();
            self.message.clear();
            return;
        }
        if edited {
            // The pattern is compiled as it is typed, so the highlight in
            // the viewport follows the typing.
            self.refresh_search();
        }
        if search_enter {
            self.cycle_search();
            return;
        }
        if let Some(answer) = submit {
            let purpose = self.prompt.take().map(|prompt| prompt.purpose());
            if let Some(purpose) = purpose {
                self.answer(purpose, answer);
            }
        }
    }

    fn open_prompt(&mut self, purpose: Purpose) {
        self.prompt = Some(Prompt::input(purpose));
        self.message.clear();
    }

    /// Acts on the answer to a prompt.
    fn answer(&mut self, purpose: Purpose, answer: Answer) {
        match answer {
            Answer::Choice(c) => match purpose {
                Purpose::GoToLine => {
                    if let Ok(line) = c.to_string().parse::<u64>() {
                        let last = self.doc.line_count().saturating_sub(1);
                        self.goto(line.min(last));
                    } else {
                        self.message = "not a line number".into();
                    }
                }
                Purpose::ChangedOnDisk => match c {
                    'y' => {
                        self.save();
                    }
                    'n' => {
                        self.quit = true;
                        self.message = "not saved".into();
                    }
                    'r' => {
                        self.reload();
                    }
                    'o' => {
                        // overwrite: save the editor's version to disk
                        self.write_now();
                    }
                    'c' => {
                        // cancel: do nothing, keep unsaved changes
                        self.message = "not saved".into();
                    }
                    _ => {
                        self.message = format!("unexpected choice: {c}");
                    }
                },
                // A search or a replacement is answered with text, never
                // with a letter.
                Purpose::Search | Purpose::Replace | Purpose::ReplaceWith => {
                    self.message = "unexpected choice".into();
                }
            },
            Answer::Text(text) => match purpose {
                Purpose::GoToLine => {
                    if let Ok(line) = text.parse::<u64>() {
                        // Line numbers in prompts are 1-indexed (human-friendly)
                        let last = self.doc.line_count().saturating_sub(1);
                        self.goto(line.saturating_sub(1).min(last));
                    } else {
                        self.message = "not a line number".into();
                    }
                }
                Purpose::Replace => {
                    if text.is_empty() {
                        self.message = "nothing to replace".into();
                        return;
                    }
                    match Pattern::compile(&text) {
                        Ok(pattern) => {
                            // The pattern is remembered twice: as what the
                            // viewport highlights while the answer is typed,
                            // and as the thing this replace will look for.
                            self.set_pattern(pattern.clone());
                            self.pending_replace = Some(pattern);
                            self.prompt = Some(Prompt::input(Purpose::ReplaceWith));
                            self.message.clear();
                        }
                        Err(message) => self.message = message,
                    }
                }
                Purpose::ReplaceWith => {
                    let Some(pattern) = self.pending_replace.take() else {
                        self.message = "nothing to replace".into();
                        return;
                    };
                    self.replace_all(&pattern, &text);
                }
                Purpose::Search => {
                    self.step_search(&text);
                }
                _ => {
                    self.message = format!("unexpected text: {text}");
                }
            },
        }
    }

    /// `Ctrl+K`: delete from the caret to the end of the line.
    fn delete_to_end_of_line(&mut self) {
        let cursor = self.cursor();
        let end = motion::target(&self.doc, cursor, Motion::LineEnd);
        if end <= cursor {
            self.message = "already at the end of the line".into();
            return;
        }
        self.doc.edit(cursor, end, "");
        self.clear_selection();
        self.dirty = true;
        self.message.clear();
    }

    /// `Ctrl+W`: delete the word before the cursor.
    fn delete_word_before(&mut self) {
        let cursor = self.cursor();
        let line = self.line_text(cursor.line);
        let col = cursor.col;
        if col == 0 {
            self.message = "nothing to delete".into();
            return;
        }
        let text = line.as_str();
        // Find the start of the word by going backwards from the cursor
        let mut char_idx = col;
        // Go backwards past word characters (alphanumeric)
        while char_idx > 0 {
            let c = text[char_idx - 1..char_idx].chars().next().unwrap_or(' ');
            if !c.is_alphanumeric() {
                break;
            }
            char_idx -= 1;
        }
        // Skip past any whitespace before the word
        while char_idx > 0 {
            let c = text[char_idx - 1..char_idx].chars().next().unwrap_or(' ');
            if c.is_whitespace() {
                break;
            }
            char_idx -= 1;
        }
        // char_idx now points to the first character of the word (or 0)
        // Delete from char_idx to cursor position
        let start = Pos::new(cursor.line, char_idx);
        let end = cursor;
        if start >= end {
            self.message = "nothing to delete".into();
            return;
        }
        let new_cursor = self.doc.edit(start, end, "");
        // Update cursor position from the edit result
        self.view.cursor_line = new_cursor.line;
        self.view.cursor_col = new_cursor.col;
        self.clear_selection();
        self.dirty = true;
        self.message.clear();
    }

    /// `Ctrl+L`: select the line, and the next one on a second press.
    fn select_line(&mut self) {
        let line = self.view.cursor_line;
        let len = self.line_len(line);
        let already = self.selection() == Some((Pos::new(line, 0), Pos::new(line, len)));
        if already {
            let next = line + 1;
            if self.doc.line_text(next).is_some() {
                self.view.cursor_line = next;
                self.view.cursor_col = self.line_len(next);
                self.ensure_cursor_visible();
            }
            return;
        }
        self.view.anchor = Some(Pos::new(line, 0));
        self.view.cursor_col = len;
        self.ensure_cursor_visible();
        self.message.clear();
    }

    /// Routes one key into the open leader sequence.
    fn feed_sequence(&mut self, key: KeyEvent) {
        // Pressing the leader again keeps the sequence open; any other chord
        // abandons it. `Char('s')` with Control is `Ctrl+S`, not the `s`
        // mnemonic, and must not save behind the user's back.
        if self.keys.is_leader(&key) {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.seq.cancel();
            self.message = "cancelled".into();
            return;
        }

        let result = match key.code {
            KeyCode::Esc => {
                self.seq.cancel();
                self.message.clear();
                return;
            }
            KeyCode::Char(c) => self.seq.feed(c),
            _ => {
                self.seq.cancel();
                self.message.clear();
                return;
            }
        };
        match result {
            Step::Action(action) => {
                self.message.clear();
                self.perform(action);
            }
            Step::Pending(hint) => {
                if !hint.is_empty() {
                    self.message = hint.into();
                }
            }
            Step::Cancelled => self.message = "cancelled".into(),
        }
    }

    fn perform(&mut self, action: Action) {
        match action {
            Action::Command(command) => self.command(command),
            Action::Apply {
                operator,
                target,
                count,
            } => self.apply_operator(operator, target, count),
            Action::SetMark(name) => {
                self.marks.insert(name, self.cursor());
                let pos = self.cursor();
                self.message = format!("mark '{name}' at {}:{}", pos.line + 1, pos.col + 1);
            }
            Action::JumpToMark(name) => match self.marks.get(&name).copied() {
                Some(pos) => {
                    // The document may have lost lines since the mark was
                    // set; a mark past the end lands on the last line.
                    let last = self.doc.line_count().saturating_sub(1);
                    let line = pos.line.min(last);
                    let col = pos.col.min(self.line_len(line));
                    self.clear_selection();
                    self.move_to(Pos::new(line, col));
                    self.ensure_cursor_visible();
                    self.message.clear();
                }
                None => self.message = format!("no mark '{name}'"),
            },
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Save => self.save(),
            Command::Quit => {
                if self.dirty {
                    self.message = "unsaved changes — Ctrl+S saves, Leader Q discards".into();
                } else {
                    self.quit = true;
                }
            }
            Command::ForceQuit => self.quit = true,
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::ToggleGutter => {
                self.view.show_line_numbers = !self.view.show_line_numbers;
                self.message.clear();
            }
            Command::ToggleRelativeGutter => {
                self.view.show_relative_numbers = !self.view.show_relative_numbers;
                self.message.clear();
            }
            Command::ToggleWordWrap => self.toggle_word_wrap(),
            Command::IndentLine => {
                let line = self.view.cursor_line;
                let end = Pos::new(line, self.line_len(line));
                self.reindent(Pos::new(line, 0), end, true);
            }
            Command::UnindentLine => {
                let line = self.view.cursor_line;
                let end = Pos::new(line, self.line_len(line));
                self.reindent(Pos::new(line, 0), end, false);
            }
            Command::GoToLine => self.open_prompt(Purpose::GoToLine),
            Command::Find => self.open_find(),
            Command::Replace => self.open_replace(),
            Command::NextMatch => self.next_match(),
            Command::PreviousMatch => self.previous_match(),
            Command::ToggleLineComment => self.toggle_line_comment(),
            Command::Help => self.open_help(),
            Command::Palette => self.open_palette(),
        }
    }

    /// `Leader ?`: the help list, drawn over the viewport.
    fn open_help(&mut self) {
        self.overlay = Some(Overlay::Help { offset: 0 });
        self.message.clear();
    }

    /// `F1` and `Leader p`: every command, filtered by what is typed.
    fn open_palette(&mut self) {
        self.overlay = Some(Overlay::Palette(Palette::new()));
        self.message.clear();
    }

    /// Routes one key into an open panel.
    ///
    /// The panel owns the keyboard while it is open, the way the prompt does:
    /// a key that means `undo` in the document means `u` in the palette's
    /// query. Nothing edits the file until the panel is closed and a command
    /// is chosen.
    fn handle_overlay_key(&mut self, mut overlay: Overlay, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let height = self.viewport_height() as usize;
        let closes = match (&overlay, key.code) {
            (_, KeyCode::Esc) => true,
            // The help list is read, not edited: any of its own keys closes
            // it, so it never becomes a mode to escape from.
            (Overlay::Help { .. }, KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')) => {
                true
            }
            _ => false,
        };
        if closes {
            self.message.clear();
            return;
        }
        match &mut overlay {
            Overlay::Help { offset } => match key.code {
                KeyCode::Down => {
                    *offset = (*offset + 1).min(panel::help_height().saturating_sub(1))
                }
                KeyCode::Up => *offset = offset.saturating_sub(1),
                KeyCode::PageDown => {
                    *offset = (*offset + height).min(panel::help_height().saturating_sub(1));
                }
                KeyCode::PageUp => *offset = offset.saturating_sub(height),
                _ => {}
            },
            Overlay::Palette(palette) => match key.code {
                KeyCode::Enter => {
                    if let Some(entry) = palette.selected() {
                        self.message.clear();
                        self.command(entry.command);
                        return;
                    }
                }
                KeyCode::Down => palette.down(),
                KeyCode::Up => palette.up(),
                KeyCode::Backspace => palette.backspace(),
                KeyCode::Char(c) if !ctrl => palette.insert(c),
                _ => {}
            },
        }
        // An unknown key leaves the panel open rather than closing it: a
        // stray keystroke should not throw away what was typed.
        self.overlay = Some(overlay);
    }

    /// Resolves an operator against the range its target names, then acts on it.
    fn apply_operator(&mut self, operator: Operator, target: Target, count: u32) {
        let from = self.cursor();
        let (start, end) = match target {
            // A text object is a range in its own right: it is not measured
            // from the cursor, it contains it.
            Target::Object { object, around } => {
                match motion::object_range(&self.doc, from, object, around, count) {
                    Some(range) => range,
                    None => {
                        let where_ = if around { "around" } else { "inside" };
                        self.message = format!("no {} {} the cursor", object.label(), where_);
                        return;
                    }
                }
            }
            Target::Line => {
                let last = self.doc.line_count().saturating_sub(1);
                let end_line = from.line.saturating_add(count as u64);
                let to = if end_line > last {
                    Pos::new(last, self.line_len(last))
                } else {
                    Pos::new(end_line, 0)
                };
                ordered(from, to)
            }
            Target::Motion(motion) => {
                let mut to = from;
                for _ in 0..count.max(1) {
                    to = motion::target(&self.doc, to, motion);
                }
                ordered(from, to)
            }
        };

        // `v` names a range but changes nothing: it just moves the anchor.
        if operator == Operator::Select {
            self.view.anchor = Some(start);
            self.move_to(end);
            self.clamp_column();
            self.ensure_cursor_visible();
            self.message.clear();
            return;
        }

        if end <= start {
            self.message = format!("nothing to {} ({})", operator.label(), target_label(target));
            return;
        }
        if !self.range_is_affordable(start, end) {
            self.message = self.too_large(start, end);
            return;
        }

        let text = self.doc.text_in_range(start, end);
        let chars = text.chars().count();
        match operator {
            Operator::Yank => {
                self.osc = clipboard::osc52(&text);
                self.clipboard = text;
                self.message = format!("yanked {chars} chars");
            }
            Operator::Delete | Operator::Change => {
                // What is deleted is also what is on the clipboard, which is
                // what makes `Leader d` a cut rather than a loss.
                self.osc = clipboard::osc52(&text);
                self.doc.edit(start, end, "");
                self.move_to(start);
                self.clamp_column();
                self.ensure_cursor_visible();
                self.view.anchor = None;
                self.clipboard = text;
                self.dirty = true;
                let verb = match operator {
                    Operator::Change => "changed",
                    _ => "deleted",
                };
                self.message = format!("{verb} {chars} chars");
            }
            Operator::Select => unreachable!("handled before the match"),
            Operator::Indent => self.reindent(start, end, true),
            Operator::Unindent => self.reindent(start, end, false),
        }
    }

    /// Turns soft wrapping on or off.
    ///
    /// Wrapping is a layout choice, not an edit: the document does not move,
    /// the horizontal scroll is simply ignored while it is on, and off again
    /// as soon as it is off.
    fn toggle_word_wrap(&mut self) {
        self.view.word_wrap = !self.view.word_wrap;
        self.view.top_row = 0;
        self.message = if self.view.word_wrap {
            "word wrap on".into()
        } else {
            "word wrap off".into()
        };
        self.ensure_cursor_visible();
    }

    /// `Ctrl+/`: comments out the touched lines, or uncomments them.
    ///
    /// The rule is the one VSCode uses: if every line that carries text is
    /// already commented, the command uncomments; otherwise it comments the
    /// lines that are not. Blank lines are left alone either way, because a
    /// comment marker on an empty line is noise nobody asked for.
    ///
    /// The whole range is one [`Doc::edit`], so the command is one undo step
    /// however many lines it covers (spec §4.7).
    fn toggle_line_comment(&mut self) {
        let Some(prefix) = line_comment_prefix(&self.path) else {
            self.message = "this file type has no line comment".into();
            return;
        };
        let (from, to) = self.selection().unwrap_or((self.cursor(), self.cursor()));
        let first = from.line;
        let last = if to.col == 0 && to.line > first {
            to.line - 1
        } else {
            to.line
        };

        let range_start = Pos::new(first, 0);
        let range_end = Pos::new(last, self.line_len(last));
        let original = self.doc.text_in_range(range_start, range_end);
        let lines: Vec<&str> = original.split('\n').collect();
        let commented = lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .all(|line| line.trim_start().starts_with(prefix));

        let cursor_line = self.view.cursor_line;
        let mut changed = String::with_capacity(original.len() + lines.len() * (prefix.len() + 1));
        let mut cursor_delta: i64 = 0;
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                changed.push('\n');
            }
            let indent = line.len() - line.trim_start().len();
            let is_cursor_line = first + index as u64 == cursor_line;
            if commented {
                let rest = &line[indent..];
                let stripped = rest.strip_prefix(prefix).unwrap_or(rest);
                let stripped = stripped.strip_prefix(' ').unwrap_or(stripped);
                if is_cursor_line {
                    let removed = line.chars().count() - indent - stripped.chars().count();
                    cursor_delta = -(removed as i64);
                }
                changed.push_str(&line[..indent]);
                changed.push_str(stripped);
                continue;
            }
            if line.trim().is_empty() {
                changed.push_str(line);
                continue;
            }
            let marker = format!("{prefix} ");
            if is_cursor_line {
                let width = marker.chars().count();
                if self.view.cursor_col >= indent {
                    cursor_delta = width as i64;
                }
            }
            changed.push_str(&line[..indent]);
            changed.push_str(&marker);
            changed.push_str(&line[indent..]);
        }

        if changed == original {
            self.message = if commented {
                "nothing to uncomment".into()
            } else {
                "nothing to comment".into()
            };
            return;
        }
        self.doc.edit(range_start, range_end, &changed);
        self.dirty = true;
        let col = self.view.cursor_col as i64 + cursor_delta;
        self.view.cursor_col = col.max(0) as usize;
        self.clamp_column();
        let count = last - first + 1;
        let verb = if commented {
            "uncommented"
        } else {
            "commented"
        };
        self.message = format!(
            "{verb} {count} {}",
            if count == 1 { "line" } else { "lines" }
        );
    }

    /// Adds or removes one indent level on every line a range touches.
    ///
    /// The whole range is rewritten as a single [`Doc::edit`], so the
    /// command is one undo step however many lines it covers — the journal
    /// groups a command, not its lines (spec §4.7).
    ///
    /// The range is half-open, like every other: it stops before
    /// `(end.line, end.col)`, so a target at column 0 does not touch its own
    /// line, which is what makes `Leader > j` agree with `Leader d j` about
    /// which lines they name.
    fn reindent(&mut self, from: Pos, to: Pos, indent: bool) {
        let last_line = self.doc.line_count().saturating_sub(1);
        let (start, end) = if to < from { (to, from) } else { (from, to) };
        let first = start.line.min(last_line);
        let last = if end.col == 0 && end.line > first {
            (end.line - 1).min(last_line)
        } else {
            end.line.min(last_line)
        };

        let range_start = Pos::new(first, 0);
        let range_end = Pos::new(last, self.line_len(last));
        let original = self.doc.text_in_range(range_start, range_end);
        let cursor_line = self.view.cursor_line;
        let cursor_touched = (first..=last).contains(&cursor_line);

        let mut changed = String::with_capacity(original.len() + self.indent.width());
        let mut cursor_removed = 0usize;
        for (i, line) in original.split('\n').enumerate() {
            if i > 0 {
                changed.push('\n');
            }
            if indent {
                changed.push_str(&self.indent.unit());
                changed.push_str(line);
            } else {
                let (kept, removed) = indent::dedent(line, self.indent);
                if first + i as u64 == cursor_line {
                    cursor_removed = removed;
                }
                changed.push_str(kept);
            }
        }

        if changed == original {
            self.message = "nothing to unindent".into();
            return;
        }
        self.doc.edit(range_start, range_end, &changed);
        self.dirty = true;
        // The cursor stays on the character it was on: the text moved
        // under it, so it moves with the text.
        if cursor_touched {
            if indent {
                self.view.cursor_col += self.indent.width();
            } else {
                self.view.cursor_col = self.view.cursor_col.saturating_sub(cursor_removed);
            }
            self.clamp_column();
        }
        let count = last - first + 1;
        let verb = if indent { "indented" } else { "unindented" };
        self.message = format!(
            "{verb} {count} {}",
            if count == 1 { "line" } else { "lines" }
        );
    }

    fn paste(&mut self) {
        if self.clipboard.is_empty() {
            self.message = "clipboard is empty".into();
            return;
        }
        let text = self.clipboard.clone();
        self.insert_replacing_selection(&text);
    }

    // ----- selection -----------------------------------------------------

    /// The selected range, ordered and non-empty.
    fn selection(&self) -> Option<(Pos, Pos)> {
        let anchor = self.view.anchor?;
        let cursor = self.cursor();
        let (start, end) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };
        if start == end {
            None
        } else {
            Some((start, end))
        }
    }

    fn begin_selection(&mut self) {
        if self.view.anchor.is_none() {
            self.view.anchor = Some(self.cursor());
        }
    }

    fn clear_selection(&mut self) {
        self.view.anchor = None;
    }

    fn select_all(&mut self) {
        // Selecting everything needs to know where the end is, which is the
        // one question that requires the whole index.
        self.doc.index_fully();
        let last = self.doc.line_count().saturating_sub(1);
        self.view.anchor = Some(Pos::new(0, 0));
        self.view.cursor_line = last;
        self.view.cursor_col = self.line_len(last);
        self.ensure_cursor_visible();
        self.message.clear();
    }

    /// Whether an edit over this range finishes inside a frame budget.
    ///
    /// A range that crosses many lines is superlinear in the line model
    /// (performance finding 8), so an accidental `Ctrl+A` followed by one
    /// keystroke on a 40-million-line file would freeze the editor for
    /// minutes with no progress. It answers instead of hanging.
    fn range_is_affordable(&self, start: Pos, end: Pos) -> bool {
        end.line.saturating_sub(start.line) <= MAX_SPAN_LINES
    }

    fn too_large(&self, start: Pos, end: Pos) -> String {
        format!(
            "{} lines is beyond the current line model (ADR 0012)",
            end.line.saturating_sub(start.line) + 1
        )
    }

    fn copy(&mut self) {
        let Some((start, end)) = self.selection() else {
            self.message = "nothing selected".into();
            return;
        };
        if !self.range_is_affordable(start, end) {
            self.message = self.too_large(start, end);
            return;
        }
        let text = self.doc.text_in_range(start, end);
        let chars = text.chars().count();
        self.osc = clipboard::osc52(&text);
        self.clipboard = text;
        self.message = if self.osc.is_some() {
            format!("copied {chars} chars")
        } else {
            format!("copied {chars} chars (too large for the system clipboard)")
        };
    }

    fn cut(&mut self) {
        let Some((start, end)) = self.selection() else {
            self.message = "nothing selected".into();
            return;
        };
        if !self.range_is_affordable(start, end) {
            self.message = self.too_large(start, end);
            return;
        }
        let text = self.doc.text_in_range(start, end);
        let chars = text.chars().count();
        self.osc = clipboard::osc52(&text);
        self.doc.edit(start, end, "");
        self.clipboard = text;
        self.view.anchor = None;
        self.move_to(start);
        self.clamp_column();
        self.ensure_cursor_visible();
        self.dirty = true;
        self.message = format!("cut {chars} chars");
    }

    /// Deletes the selection if there is one, reporting whether it handled
    /// the keystroke.
    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        if !self.range_is_affordable(start, end) {
            self.message = self.too_large(start, end);
            return true;
        }
        self.doc.edit(start, end, "");
        self.view.anchor = None;
        self.move_to(start);
        self.clamp_column();
        self.ensure_cursor_visible();
        self.dirty = true;
        self.message.clear();
        true
    }

    // ----- editing -------------------------------------------------------

    fn line_text(&self, line: u64) -> String {
        self.doc.line_text(line).unwrap_or_default().into_owned()
    }

    fn line_len(&self, line: u64) -> usize {
        self.line_text(line).chars().count()
    }

    /// The cursor as a document position.
    fn cursor(&self) -> Pos {
        Pos::new(self.view.cursor_line, self.view.cursor_col)
    }

    fn move_to(&mut self, pos: Pos) {
        self.view.cursor_line = pos.line;
        self.view.cursor_col = pos.col;
    }

    // Every mutation below is one call to `Doc::edit`: a range replaced by a
    // string. Nothing else touches the document, which is why one journal
    // reverses all of them.

    /// Inserts text, replacing the selection if there is one.
    ///
    /// Replacing the range and inserting at a point are the same edit, so
    /// typing over a selection is one undo step rather than "delete, then
    /// type".
    fn insert_replacing_selection(&mut self, text: &str) {
        let (start, end) = match self.selection() {
            Some(range) => range,
            None => (self.cursor(), self.cursor()),
        };
        if !self.range_is_affordable(start, end) {
            self.message = self.too_large(start, end);
            return;
        }
        let end = self.doc.edit(start, end, text);
        self.view.anchor = None;
        self.move_to(end);
        self.ensure_cursor_visible();
        self.dirty = true;
        self.message.clear();
    }

    fn insert_char(&mut self, c: char) {
        // Typing the closing half of a pair that is already there steps over
        // it instead of doubling it, which is what the hands expect before
        // they have looked at what is next.
        if self.step_over(c) {
            return;
        }
        // A closer at the start of an indented line belongs one level out.
        self.close_block_indent(c);
        if let Some(close) = self.partner_for(c) {
            let start = self.cursor();
            self.insert_replacing_selection(&format!("{c}{close}"));
            // Back between the two, where the next character belongs.
            self.view.cursor_col = start.col + 1;
            self.clamp_column();
            return;
        }
        self.insert_replacing_selection(&c.to_string());
    }

    /// The character that would complete the pair `c` opens, if it is
    /// unambiguous that one is wanted (spec §4.9).
    ///
    /// Nothing is paired over a selection (typing replaces it), next to a word
    /// or a closer (there the user is editing text, not opening a block), or
    /// for a quote between two letters, which is an apostrophe.
    fn partner_for(&self, c: char) -> Option<char> {
        if !self.auto_pair || self.selection().is_some() {
            return None;
        }
        let close = match c {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '"' | '\'' | '`' => c,
            _ => return None,
        };
        let pos = self.cursor();
        let text = self.line_text(pos.line);
        let next = text.chars().nth(pos.col);
        let previous = pos.col.checked_sub(1).and_then(|col| text.chars().nth(col));
        if next.is_some_and(|n| n.is_alphanumeric() || n == '_') {
            return None;
        }
        if close == c
            && previous.is_some_and(|p| p.is_alphanumeric())
            && next.is_some_and(|n| n.is_alphanumeric())
        {
            return None;
        }
        Some(close)
    }

    /// Whether `c` is a closer that is already under the cursor.
    fn step_over(&mut self, c: char) -> bool {
        if !matches!(c, ')' | ']' | '}' | '"' | '\'' | '`') || self.selection().is_some() {
            return false;
        }
        let pos = self.cursor();
        if self.line_text(pos.line).chars().nth(pos.col) == Some(c) {
            self.view.cursor_col += 1;
            self.ensure_cursor_visible();
            return true;
        }
        false
    }

    fn insert_text(&mut self, text: &str) {
        self.insert_replacing_selection(text);
    }

    /// `Enter`, with the indentation a new line should start with.
    ///
    /// A new line keeps the indentation of the line it came from, and takes
    /// one more level when the line before it opens a block — which is what
    /// `{`, `(`, `[` and `:` say in every language this editor will meet
    /// (spec §4.4). It does not indent between a pair that is already there: a
    /// closer waiting on the next line is a block that stays where it is.
    fn split_line(&mut self) {
        let current = self.line_text(self.view.cursor_line);
        let before_cursor = current.chars().take(self.view.cursor_col).count();
        let before: String = current.chars().take(before_cursor).collect();
        let after: Option<char> = current.chars().nth(self.view.cursor_col);

        let base = indent::leading(&current).to_owned();
        let opens = opens_block(&before) && !closes_itself(&before, after);
        let extra = if opens {
            self.indent.unit()
        } else {
            String::new()
        };
        self.insert_replacing_selection(&format!("\n{base}{extra}"));
    }

    /// `Shift+Tab`: one level back, on every line the range touches.
    fn unindent_here(&mut self) {
        let (from, to) = self.selection().unwrap_or((self.cursor(), self.cursor()));
        self.reindent(from, to, false);
    }

    /// Removes one level of the cursor's line when a closer is typed at the
    /// start of it, so `}` lands level with the `{` it closes.
    fn close_block_indent(&mut self, c: char) -> bool {
        if !matches!(c, '}' | ')' | ']') {
            return false;
        }
        let text = self.line_text(self.view.cursor_line);
        let before: String = text.chars().take(self.view.cursor_col).collect();
        if !before.chars().all(char::is_whitespace) || before.is_empty() {
            return false;
        }
        let Some((kept, removed)) = Some(indent::dedent(&text, self.indent)) else {
            return false;
        };
        if removed == 0 {
            return false;
        }
        let line = self.view.cursor_line;
        let start = Pos::new(line, 0);
        let end = Pos::new(line, self.line_len(line));
        self.doc.edit(start, end, kept);
        self.view.cursor_col = self.view.cursor_col.saturating_sub(removed);
        self.dirty = true;
        true
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let pos = self.cursor();
        if pos.col > 0 {
            let text = self.line_text(pos.line);
            let before = text.chars().nth(pos.col - 1);
            let after = text.chars().nth(pos.col);
            // Backspacing an empty pair takes both halves: the pair was one
            // keystroke, so removing it is one.
            let empty_pair = matches!(
                (before, after),
                (Some('('), Some(')'))
                    | (Some('['), Some(']'))
                    | (Some('{'), Some('}'))
                    | (Some('"'), Some('"'))
                    | (Some('\''), Some('\''))
                    | (Some('`'), Some('`'))
            ) && self.auto_pair;
            let start = Pos::new(pos.line, pos.col - 1);
            if empty_pair {
                self.doc.edit(start, Pos::new(pos.line, pos.col + 1), "");
                self.move_to(start);
                self.dirty = true;
                return;
            }
            self.doc.edit(start, pos, "");
            self.move_to(start);
            self.dirty = true;
            return;
        }
        if pos.line == 0 {
            return;
        }
        // At the start of the line: join it onto the previous one.
        let start = Pos::new(pos.line - 1, self.line_len(pos.line - 1));
        self.doc.edit(start, pos, "");
        self.move_to(start);
        self.dirty = true;
    }

    fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let pos = self.cursor();
        if pos.col < self.line_len(pos.line) {
            self.doc.edit(pos, Pos::new(pos.line, pos.col + 1), "");
            self.dirty = true;
            return;
        }
        // `line_text` returns None only when the line really does not exist,
        // so this is correct even while the background index is still running.
        if self.doc.line_text(pos.line + 1).is_none() {
            return;
        }
        self.doc.edit(pos, Pos::new(pos.line + 1, 0), "");
        self.dirty = true;
    }

    fn duplicate_line(&mut self) {
        let line = self.view.cursor_line;
        let col = self.view.cursor_col;
        let text = self.line_text(line);
        let start = Pos::new(line, 0);
        self.doc.edit(start, start, &format!("{text}\n"));
        self.move_to(Pos::new(line + 1, col));
        self.dirty = true;
        self.message.clear();
    }

    fn undo(&mut self) {
        match self.doc.undo() {
            Some(pos) => {
                self.move_to(pos);
                self.clamp_column();
                self.dirty = true;
                self.message.clear();
            }
            None => self.message = "nothing to undo".into(),
        }
    }

    fn redo(&mut self) {
        match self.doc.redo() {
            Some(pos) => {
                self.move_to(pos);
                self.clamp_column();
                self.dirty = true;
                self.message.clear();
            }
            None => self.message = "nothing to redo".into(),
        }
    }

    // ----- movement ------------------------------------------------------

    fn move_cursor(&mut self, dx: i32, dy: i32) {
        // Wrapped, a vertical movement means one *screen* row, which is what
        // the eye follows; unwrapped, it means one line (spec §4.5).
        if dy != 0 && self.view.word_wrap {
            self.move_cursor_wrapped(dy);
            return;
        }
        if dy != 0 {
            let last = self.doc.line_count().saturating_sub(1);
            let target = (self.view.cursor_line as i64 + dy as i64).clamp(0, last as i64) as u64;
            self.view.cursor_line = target;
            self.clamp_column();
        }
        if dx != 0 {
            let text = self.line_text(self.view.cursor_line);
            let len = text.chars().count();
            let target = (self.view.cursor_col as i64 + dx as i64).clamp(0, len as i64) as usize;
            self.view.cursor_col = target;
        }
        self.ensure_cursor_visible();
    }

    /// One visual row up or down, keeping the column within the row.
    ///
    /// A row that ends early clamps to its end, which is the same rule the
    /// unwrapped move uses for a short line.
    fn move_cursor_wrapped(&mut self, dy: i32) {
        let width = self.view.width.max(1) as usize;
        let chars: Vec<char> = self.line_text(self.view.cursor_line).chars().collect();
        let (row, column) = render::wrapped_position(&chars, width, self.view.cursor_col);
        if dy < 0 {
            if row > 0 {
                let start = render::wrapped_row_start(&chars, width, row - 1);
                self.view.cursor_col = start + column;
            } else if self.view.cursor_line > 0 {
                let previous = self.view.cursor_line - 1;
                let previous_chars: Vec<char> = self.line_text(previous).chars().collect();
                let rows = render::wrapped_row_count(&previous_chars, width);
                let start = render::wrapped_row_start(&previous_chars, width, rows - 1);
                self.view.cursor_line = previous;
                let len = previous_chars.len();
                self.view.cursor_col = (start + column).min(len);
            }
        } else {
            let rows = render::wrapped_row_count(&chars, width);
            if row + 1 < rows {
                let start = render::wrapped_row_start(&chars, width, row + 1);
                self.view.cursor_col = (start + column).min(chars.len());
            } else if self.doc.line_text(self.view.cursor_line + 1).is_some() {
                self.view.cursor_line += 1;
                let next: Vec<char> = self.line_text(self.view.cursor_line).chars().collect();
                let start = render::wrapped_row_start(&next, width, 0);
                self.view.cursor_col = (start + column).min(next.len());
            }
        }
        self.ensure_cursor_visible();
    }

    fn set_cursor_col(&mut self, col: usize) {
        let text = self.line_text(self.view.cursor_line);
        let len = text.chars().count();
        self.view.cursor_col = col.min(len);
        self.ensure_cursor_visible();
    }

    fn clamp_column(&mut self) {
        let text = self.line_text(self.view.cursor_line);
        let len = text.chars().count();
        if self.view.cursor_col > len {
            self.view.cursor_col = len;
        }
    }

    fn goto(&mut self, line: u64) {
        self.view.cursor_line = line;
        self.view.top_line = line;
        self.clamp_column();
    }

    fn goto_end(&mut self) {
        // Knowing where the end is requires knowing every line, so this is
        // the one movement that pays for a full scan.
        self.doc.index_fully();
        let last = self.doc.line_count().saturating_sub(1);
        self.view.cursor_line = last;
        self.ensure_cursor_visible();
    }

    fn page(&mut self, direction: i32) {
        let height = self.viewport_height();
        let step = height.max(1) as i64 * direction as i64;
        let last = self.doc.line_count().saturating_sub(1) as i64;
        let target = (self.view.cursor_line as i64 + step).clamp(0, last) as u64;
        self.view.cursor_line = target;
        self.ensure_cursor_visible();
    }

    fn viewport_height(&self) -> u16 {
        self.viewport_height_hint.max(1)
    }

    fn ensure_cursor_visible(&mut self) {
        if self.view.word_wrap {
            self.ensure_cursor_visible_wrapped();
            return;
        }
        self.view.top_row = 0;
        let height = self.viewport_height() as u64;
        if self.view.cursor_line < self.view.top_line {
            self.view.top_line = self.view.cursor_line;
        } else if self.view.cursor_line >= self.view.top_line + height {
            self.view.top_line = self.view.cursor_line + 1 - height;
        }
        // The same sideways. A line longer than the screen scrolls under the
        // cursor, so the cursor is never stuck against the edge of it.
        let width = self.view.width as usize;
        if width == 0 {
            return;
        }
        let col = self.view.cursor_col;
        if col < self.view.left_col {
            self.view.left_col = col;
        } else if col - self.view.left_col >= width {
            self.view.left_col = col + 1 - width;
        }
    }

    /// Brings the cursor on screen when lines wrap.
    ///
    /// The unit is a screen row, not a line, because a wrapped line can be
    /// taller than the viewport: `top_row` says where inside the top line the
    /// viewport starts, and the check is always "how many rows is the cursor
    /// from there".
    fn ensure_cursor_visible_wrapped(&mut self) {
        let height = self.viewport_height() as usize;
        let width = self.view.width.max(1) as usize;
        let chars: Vec<char> = self.line_text(self.view.cursor_line).chars().collect();
        let (cursor_row, _) = render::wrapped_position(&chars, width, self.view.cursor_col);

        // Above the top of the viewport: scroll up to it.
        if self.view.cursor_line < self.view.top_line
            || (self.view.cursor_line == self.view.top_line && cursor_row < self.view.top_row)
        {
            self.view.top_line = self.view.cursor_line;
            self.view.top_row = cursor_row;
            return;
        }

        // A screen row per line is the best case, so a cursor this many lines
        // below the top cannot be on screen however the rows lay out.
        let rows_available = height.max(1) as u64;
        if self.view.cursor_line.saturating_sub(self.view.top_line) >= rows_available {
            self.view.top_line = self.view.cursor_line + 1 - rows_available;
            self.view.top_row = 0;
        }

        // Now walk the top of the viewport forward until the cursor's row
        // fits. The line gap is already under the height, so this is bounded
        // by the height of the screen rather than by the size of the file.
        let mut guard = 0usize;
        while self.wrapped_distance(width, cursor_row) >= height.max(1) {
            self.view.top_row += 1;
            let top_chars: Vec<char> = self.line_text(self.view.top_line).chars().collect();
            if self.view.top_row >= render::wrapped_row_count(&top_chars, width) {
                if self.view.top_line >= self.view.cursor_line {
                    // One line taller than the screen: the cursor cannot be
                    // brought all the way in, so show the line from its top.
                    self.view.top_row = 0;
                    break;
                }
                self.view.top_line += 1;
                self.view.top_row = 0;
            }
            guard += 1;
            if guard > height * 4 {
                self.view.top_line = self.view.cursor_line;
                self.view.top_row = 0;
                break;
            }
        }
    }

    /// How many screen rows the cursor is below the top of the viewport.
    fn wrapped_distance(&self, width: usize, cursor_row: usize) -> usize {
        let mut rows = 0usize;
        let mut line = self.view.top_line;
        while line < self.view.cursor_line {
            let chars: Vec<char> = self.line_text(line).chars().collect();
            rows += render::wrapped_row_count(&chars, width);
            line += 1;
        }
        // `top_row` is where inside `top_line` the viewport starts, so it
        // comes off the whole count, including when the cursor is on that
        // same line.
        (rows + cursor_row).saturating_sub(self.view.top_row)
    }

    // ----- searching -----------------------------------------------------

    /// `Ctrl+F` and `Leader f`: ask for a pattern.
    ///
    /// The prompt starts with the last pattern, so a second search is a
    /// small edit rather than a retype.
    fn open_find(&mut self) {
        let previous = self
            .search
            .as_ref()
            .map(|search| search.pattern.source().to_owned())
            .unwrap_or_default();
        if let Some(search) = self.search.as_mut() {
            search.cycled = false;
        }
        self.prompt = Some(Prompt::with_text(Purpose::Search, &previous));
        self.message.clear();
    }

    /// `Ctrl+H` and `Leader r`: ask what to look for, then what to put there.
    ///
    /// The pattern comes first because it is the answer with a history: the
    /// prompt starts with the last pattern, so replacing what was just found
    /// is one `Enter` rather than a retype.
    fn open_replace(&mut self) {
        let previous = self
            .search
            .as_ref()
            .map(|search| search.pattern.source().to_owned())
            .unwrap_or_default();
        self.pending_replace = None;
        self.prompt = Some(Prompt::with_text(Purpose::Replace, &previous));
        self.message.clear();
    }

    /// Replaces every match of `pattern` with `replacement`.
    ///
    /// The range is the selection if there is one and the whole file
    /// otherwise, and a range wider than one command may edit is refused with
    /// a message, like every other bulk command (ADR 0012). The whole range is
    /// rewritten as one [`Doc::edit`], so replace-all is one undo step.
    fn replace_all(&mut self, pattern: &Pattern, replacement: &str) {
        let (start, end) = match self.selection() {
            Some(range) => range,
            None => {
                // Replacing across a file needs to know where the file ends,
                // which is the one question that costs a full scan.
                self.doc.index_fully();
                let last = self.doc.line_count().saturating_sub(1);
                (Pos::new(0, 0), Pos::new(last, self.line_len(last)))
            }
        };
        if !self.range_is_affordable(start, end) {
            self.message = self.too_large(start, end);
            return;
        }
        let text = self.doc.text_in_range(start, end);
        let matches = pattern.find_in(&text);
        if matches.is_empty() {
            self.message = format!("no match for {}", pattern.source());
            return;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut replaced = String::with_capacity(text.len());
        let mut at = 0usize;
        for (from, to) in &matches {
            if *from < at {
                continue;
            }
            replaced.extend(&chars[at..*from]);
            replaced.push_str(replacement);
            at = *to;
        }
        replaced.extend(&chars[at..]);

        let count = matches.len();
        self.doc.edit(start, end, &replaced);
        self.view.anchor = None;
        self.dirty = true;
        self.move_to(start);
        self.clamp_column();
        self.ensure_cursor_visible();
        self.message = format!(
            "replaced {count} {}",
            if count == 1 { "match" } else { "matches" }
        );
    }

    /// Forgets the search and the highlight that goes with it.
    fn clear_search(&mut self) {
        self.search = None;
        self.view.highlight_pattern = None;
    }

    /// Recompiles the pattern being typed.
    ///
    /// The compiled pattern is what the viewport highlights, so the
    /// highlight follows the typing; the matches are counted in the
    /// background.
    fn refresh_search(&mut self) {
        let Some(prompt) = self.prompt.as_ref() else {
            return;
        };
        if !matches!(prompt.purpose(), Purpose::Search | Purpose::Replace) {
            return;
        }
        let source = prompt.text().to_owned();
        let compiled = Pattern::compile(&source);
        let note = compiled.as_ref().err().cloned();
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.set_note(note);
        }
        let Ok(pattern) = compiled else {
            // The last pattern that compiled keeps its highlight, so the
            // user can still see what they were looking for.
            return;
        };
        if source.is_empty() {
            self.clear_search();
            return;
        }
        self.set_pattern(pattern);
        self.start_count();
    }

    /// Records the pattern the viewport highlights.
    fn set_pattern(&mut self, pattern: Pattern) {
        let version = self.doc.version();
        self.view.highlight_pattern = Some(pattern.clone());
        let search = self.search.get_or_insert_with(|| Search {
            pattern: pattern.clone(),
            count: None,
            count_version: version,
            total: None,
            job: None,
            job_version: version,
            wrap: None,
            cycled: false,
        });
        search.pattern = pattern;
        search.total = None;
        search.count_version = version;
        search.cycled = false;
    }

    /// The document, as a background scan sees it.
    fn search_context(&self) -> Snapshot {
        self.doc.snapshot()
    }

    /// Starts a scan counting the matches in the whole document.
    ///
    /// The count is what tells the user whether the search found anything at
    /// all, and it is done in the background so that a huge file never
    /// freezes the editor.
    fn start_count(&mut self) {
        if self.search.is_none() {
            return;
        }
        let context = self.search_context();
        let Some(search) = self.search.as_mut() else {
            return;
        };
        if let Some(mut job) = search.count.take() {
            job.cancel();
        }
        let pattern = search.pattern.clone();
        search.count = Some(SearchJob::start(context, pattern, Scope::All));
    }

    /// Starts a scan looking for the next match.
    ///
    /// `wrap` says which end of the document to try if this scan finds
    /// nothing, so that cycling through the matches wraps around.
    fn start_scan(&mut self, scope: Scope, wrap: Option<Wrap>) {
        if self.search.is_none() {
            return;
        }
        let context = self.search_context();
        let version = self.doc.version();
        let Some(search) = self.search.as_mut() else {
            return;
        };
        if let Some(mut job) = search.job.take() {
            job.cancel();
        }
        let pattern = search.pattern.clone();
        search.job = Some(SearchJob::start(context, pattern, scope));
        search.job_version = version;
        search.wrap = wrap;
    }

    /// `Leader n`: the next match after the caret.
    fn next_match(&mut self) {
        if self.search.is_none() {
            // Nothing to move to yet, so ask for a pattern.
            self.open_find();
            return;
        }
        if let Some(search) = self.search.as_mut() {
            search.cycled = true;
        }
        let scope = Scope::After { pos: self.cursor() };
        self.start_scan(scope, Some(Wrap::Start));
    }

    /// `Leader N`: the previous match before the caret.
    fn previous_match(&mut self) {
        if self.search.is_none() {
            self.open_find();
            return;
        }
        let scope = Scope::Before { pos: self.cursor() };
        self.start_scan(scope, Some(Wrap::End));
    }

    /// `Enter` while inside a `Search` prompt: jump to the next match and
    /// keep the prompt open.
    fn cycle_search(&mut self) {
        let text = self
            .prompt
            .as_ref()
            .map(|prompt| prompt.text().to_owned())
            .unwrap_or_default();
        self.step_search(&text);
    }

    /// Advances the search along the document.
    ///
    /// The first Enter starts at or after the caret (`Scope::From`), so a
    /// match already under the cursor is confirmed. Subsequent Enters advance
    /// past the caret (`Scope::After`), cycling through the matches and
    /// wrapping around at the ends of the file.
    fn step_search(&mut self, text: &str) {
        if text.is_empty() {
            self.clear_search();
            return;
        }
        let ready = self
            .search
            .as_ref()
            .is_some_and(|search| search.pattern.source() == text);
        if !ready {
            self.message = Pattern::compile(text)
                .err()
                .unwrap_or_else(|| "invalid pattern".into());
            return;
        }
        self.message.clear();
        let cycled = self
            .search
            .as_ref()
            .map(|search| search.cycled)
            .unwrap_or(false);
        let scope = if cycled {
            Scope::After { pos: self.cursor() }
        } else {
            Scope::From { pos: self.cursor() }
        };
        if let Some(search) = self.search.as_mut() {
            search.cycled = true;
        }
        self.start_scan(scope, Some(Wrap::Start));
    }

    /// Applies whatever the background scans have reported.
    ///
    /// The scans run in their own threads; the editor only ever reads what
    /// they have already finished, so a frame is never held up by a search.
    fn poll_search(&mut self) {
        let Some(mut search) = self.search.take() else {
            return;
        };
        let version = self.doc.version();
        // The document moved under the scans, so whatever they were counting
        // or looking for is about a document that no longer exists.
        let moved = search.count_version != version;

        // The count.
        let mut counted = None;
        if let Some(job) = search.count.as_ref() {
            for event in job.drain() {
                if let SearchEvent::Finished { matches } = event {
                    counted = Some(matches);
                }
            }
        }
        if let Some(total) = counted {
            search.count = None;
            if !moved {
                search.total = Some(total);
            }
        }
        if moved {
            // The number is counted again against the document as it is now.
            search.total = None;
            search.count_version = version;
        }

        // The navigation scan.
        let mut found = None;
        let mut finished = false;
        if let Some(job) = search.job.as_ref() {
            for event in job.drain() {
                match event {
                    SearchEvent::Found(m) => found = Some(m),
                    SearchEvent::Finished { .. } => finished = true,
                    SearchEvent::Progress { .. } => {}
                }
            }
        }
        if finished {
            search.job = None;
        }

        let wrap = if found.is_none() && finished {
            // The scan ran out of document without finding anything, so the
            // other end is tried next.
            search.wrap.take()
        } else {
            None
        };
        let job_version = search.job_version;

        self.search = Some(search);

        if moved {
            self.start_count();
        }

        if let Some(m) = found {
            // A match found for a document that has since changed is a
            // match in a document that no longer exists.
            if job_version == version {
                self.goto_match(m);
                self.message.clear();
            }
        } else if let Some(wrap) = wrap {
            let scope = match wrap {
                Wrap::Start => Scope::From {
                    pos: Pos::new(0, 0),
                },
                Wrap::End => Scope::Last,
            };
            self.start_scan(scope, None);
        } else if finished {
            self.message = "no match".into();
        }
    }

    /// Asks for the window on screen to be coloured, and takes the colours
    /// that have arrived.
    ///
    /// The highlighter works in its own thread: the request is a message
    /// and the answer is taken when it turns up, so neither of them holds up
    /// a frame. The colours of a document that has changed since they were
    /// computed are dropped rather than believed.
    fn poll_highlight(&mut self, height: u16) {
        let first = self.view.top_line;
        let last = first + height as u64;
        self.highlight.request(&self.doc, first, last);
        self.highlight.poll(&self.doc);
    }

    /// Moves the caret to a match and scrolls it into view.
    fn goto_match(&mut self, m: MatchPos) {
        self.move_to(Pos::new(m.line, m.start));
        self.clamp_column();
        self.ensure_cursor_visible();
    }

    // ----- saving --------------------------------------------------------

    /// Saves, unless the file on disk is no longer the one that was mapped.
    ///
    /// Overwriting silently is how an editor eats someone else's work — and,
    /// because the mapping is read-only but not protected from truncation, it
    /// is also how it can read past the end of a file that shrank.
    fn save(&mut self) {
        if let Ok(current) = identity_of(&self.path)
            && current != self.doc.identity()
        {
            self.prompt = Some(Prompt::choice(Purpose::ChangedOnDisk, CHANGED_ON_DISK));
            self.message.clear();
            return;
        }
        self.write_now();
    }

    fn write_now(&mut self) {
        match self.write_to_disk() {
            Ok(()) => {
                self.dirty = false;
                // The rename replaced the file, so the identity that was
                // recorded at open no longer describes it.
                if let Ok(identity) = identity_of(&self.path) {
                    self.doc.set_identity(identity);
                }
                self.message = format!("wrote {}", self.path.display());
            }
            Err(e) => self.message = format!("save failed: {e}"),
        }
    }

    /// Replaces the document with what is on disk.
    ///
    /// This is not a merge. The overlay is dropped and so is the undo journal,
    /// because every position in it referred to the document that just went
    /// away; the message says so rather than leaving the user to discover it.
    fn reload(&mut self) {
        match Doc::open(&self.path) {
            Ok(doc) => {
                let _ = doc.spawn_indexer();
                self.doc = doc;
                self.view.anchor = None;
                self.view.cursor_line = 0;
                self.view.cursor_col = 0;
                self.view.top_line = 0;
                self.dirty = false;
                self.message =
                    "reloaded from disk — unsaved edits and undo history discarded".into();
            }
            Err(e) => self.message = format!("reload failed: {e}"),
        }
    }

    fn write_to_disk(&self) -> Result<(), String> {
        let ending = self.doc.line_ending();
        let trailing = self.doc.ends_with_newline();
        let count = self.doc.line_count();

        let dir = self.path.parent().filter(|p| !p.as_os_str().is_empty());
        let tmp_name = match self.path.file_name() {
            Some(name) => dir
                .unwrap_or_else(|| Path::new("."))
                .join(format!(".{}.zeno.tmp", name.to_string_lossy())),
            None => return Err("cannot save a file without a name".into()),
        };

        let mut file =
            fs::File::create(&tmp_name).map_err(|e| format!("{}: {e}", tmp_name.display()))?;
        let mut out = io::BufWriter::with_capacity(1 << 20, &mut file);
        for line in 0..count {
            let text = self.doc.line_text(line).unwrap_or_default();
            out.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            if line + 1 < count || trailing {
                out.write_all(ending.as_str().as_bytes())
                    .map_err(|e| e.to_string())?;
            }
        }
        out.flush().map_err(|e| e.to_string())?;
        drop(out);
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);

        fs::rename(&tmp_name, &self.path).map_err(|e| {
            let _ = fs::remove_file(&tmp_name);
            format!("{}: {e}", self.path.display())
        })?;
        Ok(())
    }

    // ----- drawing -------------------------------------------------------

    fn draw(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
        // Whatever the background scans have finished is applied before the
        // frame is painted, so a match that was found is shown where it is.
        self.poll_search();
        let size = terminal.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        let body = chunks[0];
        let status = chunks[1];

        self.viewport_height_hint = body.height;
        // The width the editor scrolls by is the width of the *text*, not the
        // width of the screen: the gutter takes some of it.
        let gutter = View {
            doc: &self.doc,
            state: &self.view,
            highlight: None,
        }
        .gutter_width();
        self.view.width = body.width.saturating_sub(gutter);
        // The window may have changed shape since the last move, so the
        // cursor is brought back into view before anything is painted.
        self.ensure_cursor_visible();
        // The colours of the window that is about to be painted are asked
        // for here, so that the frame is painted with the colours that
        // belong to the text on screen.
        self.poll_highlight(body.height);

        terminal.draw(|f| {
            // A panel covers the document rather than sharing the screen with
            // it: the list is the thing being read, so it gets the room.
            let cursor = match &self.overlay {
                Some(overlay) => {
                    let lines = match overlay {
                        Overlay::Help { offset } => {
                            panel::help_lines(*offset, body.height as usize)
                        }
                        Overlay::Palette(palette) => {
                            panel::palette_lines(palette, body.height as usize)
                        }
                    };
                    f.render_widget(Paragraph::new(Text::from(lines)), body);
                    None
                }
                None => {
                    let view = View {
                        doc: &self.doc,
                        state: &self.view,
                        highlight: Some(self.highlight.lines()),
                    };
                    let cursor = view.cursor_position(body);
                    f.render_widget(view, body);
                    cursor
                }
            };
            match &self.prompt {
                // The question replaces the status line, so the document area
                // keeps its size and nothing on screen moves.
                Some(prompt) => {
                    f.render_widget(prompt_line(prompt, self.search.as_ref()), status);
                    // A choice prompt has no caret: the answer is one letter.
                    if let Some(x) = prompt.cursor_column() {
                        f.set_cursor_position((x, status.y));
                    }
                }
                None => {
                    f.render_widget(self.status_line(), status);
                    if let Some((x, y)) = cursor {
                        f.set_cursor_position((x, y));
                    }
                }
            }
        })?;
        terminal.show_cursor()?;
        Ok(())
    }

    fn status_line(&self) -> Paragraph<'static> {
        let mut spans = Vec::new();
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        spans.push(Span::styled(
            format!(" {name}"),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        if self.dirty {
            spans.push(Span::styled(" [+]", Style::default().fg(Color::Yellow)));
        }
        spans.push(Span::raw("  "));
        spans.push(Span::raw(format!("Ln {}", self.view.cursor_line + 1)));
        spans.push(Span::raw(format!(", Col {}", self.view.cursor_col + 1)));
        // The language of the file, once the highlighter has worked out
        // what it is.
        if let Some(syntax) = self.highlight.syntax() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                syntax.to_string(),
                Style::default().fg(Color::Magenta),
            ));
        }
        spans.push(Span::raw("  "));
        spans.push(Span::raw("UTF-8"));
        spans.push(Span::raw("  "));
        // `line_count` is the *edited* count: it applies the overlay to the
        // index. Once the scan is finished it is exact, so it is what the
        // status bar should show in both cases.
        let lines = self.doc.line_count();
        if self.doc.is_indexing() {
            spans.push(Span::styled(
                format!("{lines} lines (indexing…)"),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(format!("{lines} lines")));
        }
        if let Some(search) = &self.search {
            spans.push(Span::raw("  "));
            let shown: String = search.pattern.source().chars().take(24).collect();
            spans.push(Span::styled(
                format!("Find: {shown}"),
                Style::default().fg(Color::Magenta),
            ));
            spans.push(Span::raw("  "));
            let count = match search.total {
                Some(1) => "1 match".to_string(),
                Some(total) => format!("{total} matches"),
                None => "counting…".to_string(),
            };
            spans.push(Span::styled(count, Style::default().fg(Color::Yellow)));
        }
        if !self.message.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                self.message.clone(),
                Style::default().fg(Color::Cyan),
            ));
        } else {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "F1: Menu  ^S: Save  ^Q: Exit  ^F: Find",
                Style::default().fg(Color::DarkGray),
            ));
        }
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(28, 28, 34)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(name: &str, contents: &[u8]) -> (Editor, PathBuf) {
        let dir = std::env::temp_dir().join("zeno-editor-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        let doc = Doc::open(&path).unwrap();
        let mut editor = Editor::new(doc, path.clone());
        editor.doc.index_fully();
        (editor, path)
    }

    fn line(editor: &Editor, n: u64) -> String {
        editor.doc.line_text(n).unwrap_or_default().into_owned()
    }

    #[test]
    fn status_line_shows_utf8_encoding() {
        let (e, _p) = editor_with("encoding.txt", b"hello\n");
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        Widget::render(e.status_line(), area, &mut buf);
        let mut text = String::new();
        for x in 0..area.width {
            text.push_str(buf[(x, 0)].symbol());
        }
        assert!(
            text.contains("UTF-8"),
            "status line must show encoding: {text}"
        );
    }

    #[test]
    fn status_line_shows_shortcut_hints_when_idle() {
        let (e, _p) = editor_with("hints.txt", b"hello\n");
        let area = Rect::new(0, 0, 100, 1);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        Widget::render(e.status_line(), area, &mut buf);
        let mut text = String::new();
        for x in 0..area.width {
            text.push_str(buf[(x, 0)].symbol());
        }
        assert!(
            text.contains("F1: Menu  ^S: Save  ^Q: Exit  ^F: Find"),
            "status line must show key hints when idle: {text}"
        );
    }

    #[test]
    fn inserts_a_character_at_the_cursor() {
        let (mut e, _p) = editor_with("insert.txt", b"abc\n");
        e.view.cursor_col = 1;
        e.insert_char('X');
        assert_eq!(line(&e, 0), "aXbc");
        assert_eq!(e.view.cursor_col, 2);
        assert!(e.dirty);
    }

    #[test]
    fn enter_splits_the_line() {
        let (mut e, _p) = editor_with("split.txt", b"abcd\n");
        e.view.cursor_col = 2;
        e.split_line();
        assert_eq!(line(&e, 0), "ab");
        assert_eq!(line(&e, 1), "cd");
        assert_eq!(e.view.cursor_line, 1);
        assert_eq!(e.view.cursor_col, 0);
        assert_eq!(e.doc.line_count(), 2);
    }

    #[test]
    fn backspace_mid_line_removes_the_previous_character() {
        let (mut e, _p) = editor_with("backspace.txt", b"abcd\n");
        e.view.cursor_col = 2;
        e.backspace();
        assert_eq!(line(&e, 0), "acd");
        assert_eq!(e.view.cursor_col, 1);
    }

    #[test]
    fn backspace_at_the_start_joins_with_the_line_above() {
        let (mut e, _p) = editor_with("join.txt", b"ab\ncd\n");
        e.view.cursor_line = 1;
        e.view.cursor_col = 0;
        e.backspace();
        assert_eq!(line(&e, 0), "abcd");
        assert_eq!(e.doc.line_count(), 1);
        assert_eq!(e.view.cursor_line, 0);
        assert_eq!(e.view.cursor_col, 2);
    }

    #[test]
    fn backspace_at_the_start_of_the_first_line_does_nothing() {
        let (mut e, _p) = editor_with("backspace-first.txt", b"abc\n");
        e.view.cursor_col = 0;
        e.backspace();
        assert_eq!(line(&e, 0), "abc");
    }

    #[test]
    fn delete_at_the_end_of_a_line_joins_with_the_next() {
        let (mut e, _p) = editor_with("delete-join.txt", b"ab\ncd\n");
        e.view.cursor_line = 0;
        e.view.cursor_col = 2;
        e.delete_forward();
        assert_eq!(line(&e, 0), "abcd");
        assert_eq!(e.doc.line_count(), 1);
    }

    #[test]
    fn delete_on_the_last_line_does_nothing() {
        let (mut e, _p) = editor_with("delete-last.txt", b"ab\n");
        e.view.cursor_line = 0;
        e.view.cursor_col = 2;
        e.delete_forward();
        assert_eq!(line(&e, 0), "ab");
    }

    #[test]
    fn duplicate_line_copies_the_text() {
        let (mut e, _p) = editor_with("duplicate.txt", b"ab\ncd\n");
        e.view.cursor_line = 0;
        e.duplicate_line();
        assert_eq!(line(&e, 0), "ab");
        assert_eq!(line(&e, 1), "ab");
        assert_eq!(line(&e, 2), "cd");
        assert_eq!(e.view.cursor_line, 1);
    }

    #[test]
    fn movement_clamps_to_the_document() {
        let (mut e, _p) = editor_with("move.txt", b"ab\ncd\n");
        e.view.cursor_line = 0;
        e.move_cursor(0, -1);
        assert_eq!(e.view.cursor_line, 0);
        e.move_cursor(0, 1);
        e.move_cursor(0, 1);
        e.move_cursor(0, 1);
        assert_eq!(e.view.cursor_line, 1);
        e.view.cursor_col = 0;
        e.move_cursor(-1, 0);
        assert_eq!(e.view.cursor_col, 0);
    }

    #[test]
    fn saving_writes_the_document_back() {
        let (mut e, path) = editor_with("save.txt", b"one\ntwo\n");
        e.view.cursor_line = 1;
        e.view.cursor_col = 0;
        e.insert_char('X');
        e.save();
        assert!(!e.dirty);
        assert_eq!(std::fs::read(&path).unwrap(), b"one\nXtwo\n");
    }

    #[test]
    fn saving_a_new_file_creates_it_on_disk() {
        let dir = std::env::temp_dir().join("zeno-editor-tests");
        let path = dir.join("new_file_save.txt");
        let _ = std::fs::remove_file(&path);
        let doc = Doc::open(&path).expect("open new file");
        let mut e = Editor::new(doc, path.clone());
        e.insert_char('H');
        e.insert_char('i');
        e.save();
        assert!(!e.dirty);
        assert_eq!(std::fs::read(&path).unwrap(), b"Hi");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_preserves_crlf_and_the_missing_final_newline() {
        let (mut e, path) = editor_with("save-crlf.txt", b"one\r\ntwo");
        e.view.cursor_line = 0;
        e.insert_char('X');
        e.save();
        assert_eq!(std::fs::read(&path).unwrap(), b"Xone\r\ntwo");
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let (mut e, path) = editor_with("save-tmp.txt", b"one\n");
        e.save();
        // Only this test's own temporary file matters. The directory is
        // shared with the other saving tests, which run in parallel and may
        // legitimately be mid-write, so a directory-wide scan is racy.
        let tmp = path.parent().unwrap().join(".save-tmp.txt.zeno.tmp");
        assert!(!tmp.exists(), "{} was left behind", tmp.display());
        assert_eq!(std::fs::read(&path).unwrap(), b"one\n");
    }

    #[test]
    fn the_cursor_stays_inside_the_line() {
        let (mut e, _p) = editor_with("clamp.txt", b"ab\n");
        e.set_cursor_col(usize::MAX);
        assert_eq!(e.view.cursor_col, 2);
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn a_sequence_of_keys_produces_the_expected_document() {
        let (mut e, _p) = editor_with(
            "sequence.txt",
            b"fn main() {\n    x();\n}\n\n// c\nlet y = 1;\n",
        );
        for k in [
            key(KeyCode::Char('X')),
            key(KeyCode::Char('Y')),
            key(KeyCode::Char('Z')),
            key(KeyCode::Down),
            key(KeyCode::Down),
            key(KeyCode::End),
            key(KeyCode::Enter),
            key(KeyCode::Char('N')),
            key(KeyCode::Char('E')),
            key(KeyCode::Char('W')),
        ] {
            e.handle_key(k);
        }
        assert_eq!(line(&e, 0), "XYZfn main() {");
        assert_eq!(line(&e, 1), "    x();");
        assert_eq!(line(&e, 2), "}");
        assert_eq!(line(&e, 3), "NEW");
        assert_eq!(line(&e, 4), "");
        assert_eq!(line(&e, 5), "// c");
        assert_eq!(line(&e, 6), "let y = 1;");
        assert_eq!(e.view.cursor_line, 3);
        assert_eq!(e.view.cursor_col, 3);
    }

    fn chord(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn chords_are_identified_by_their_key_not_their_character() {
        assert_eq!(chord_letter(KeyCode::Char('s')), Some('s'));
        assert_eq!(chord_letter(KeyCode::Char('S')), Some('s'));
        // A bare control byte, as a VT-style terminal delivers Ctrl+S.
        assert_eq!(chord_letter(KeyCode::Char('\u{13}')), Some('s'));
        assert_eq!(chord_letter(KeyCode::Char('\u{04}')), Some('d'));
        assert_eq!(chord_letter(KeyCode::Char('/')), None);
        assert_eq!(chord_letter(KeyCode::Down), None);
    }

    /// The whole chord layer used to be dead on any non-Latin keyboard
    /// layout, because the layout decides which character a chord carries.
    #[test]
    fn a_control_character_saves_through_the_key_handler() {
        let (mut e, path) = editor_with("chord-save.txt", b"ab\n");
        e.insert_char('X');
        assert!(e.dirty);
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        assert!(!e.dirty, "Ctrl+S must save: {}", e.message);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Xab\n");
    }

    #[test]
    fn a_character_that_is_not_a_chord_letter_does_not_save() {
        let (mut e, _p) = editor_with("chord-slash.txt", b"ab\n");
        e.insert_char('X');
        e.handle_key(chord(KeyCode::Char('/')));
        assert!(e.dirty, "Ctrl+/ is a toggle, not a save");
    }

    #[test]
    fn a_control_chord_duplicates_the_line() {
        let (mut e, _p) = editor_with("chord-dup.txt", b"ab\ncd\n");
        e.handle_key(chord(KeyCode::Char('\u{04}')));
        assert_eq!(line(&e, 0), "ab");
        assert_eq!(line(&e, 1), "ab");
        assert_eq!(line(&e, 2), "cd");
    }

    #[test]
    fn ctrl_z_undoes_a_typing_run_and_ctrl_y_redoes_it() {
        let (mut e, _p) = editor_with("undo-run.txt", b"ab\n");
        for c in ['x', 'y', 'z'] {
            e.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(line(&e, 0), "xyzab");
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert_eq!(line(&e, 0), "ab", "one undo must remove the whole run");
        assert_eq!(e.view.cursor_line, 0);
        e.handle_key(chord(KeyCode::Char('\u{19}')));
        assert_eq!(line(&e, 0), "xyzab");
    }

    #[test]
    fn undo_restores_a_line_that_was_joined() {
        let (mut e, _p) = editor_with("undo-join.txt", b"ab\ncd\n");
        e.view.cursor_line = 1;
        e.view.cursor_col = 0;
        e.backspace();
        assert_eq!(e.doc.line_count(), 1);
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert_eq!(line(&e, 0), "ab");
        assert_eq!(line(&e, 1), "cd");
        assert_eq!(e.doc.line_count(), 2);
    }

    #[test]
    fn undo_restores_a_split_line() {
        let (mut e, _p) = editor_with("undo-split.txt", b"abcd\n");
        e.view.cursor_col = 2;
        e.split_line();
        assert_eq!(e.doc.line_count(), 2);
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert_eq!(line(&e, 0), "abcd");
        assert_eq!(e.doc.line_count(), 1);
        assert_eq!(e.view.cursor_col, 2);
    }

    #[test]
    fn undo_with_nothing_to_undo_says_so() {
        let (mut e, _p) = editor_with("undo-empty.txt", b"ab\n");
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert_eq!(e.message, "nothing to undo");
        assert!(!e.dirty, "a failed undo must not mark the file dirty");
    }

    #[test]
    fn a_new_edit_after_undo_discards_the_redo_stack() {
        let (mut e, _p) = editor_with("undo-branch.txt", b"ab\n");
        e.insert_char('x');
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert!(e.doc.can_redo());
        e.insert_char('y');
        assert!(!e.doc.can_redo());
        assert_eq!(line(&e, 0), "yab");
    }

    // ----- leader sequences ----------------------------------------------

    /// Sends the leader key, then each character of `keys` as a plain key.
    ///
    /// Mnemonics are typed unmodified; a chord is not a mnemonic.
    fn sequence(e: &mut Editor, keys: &str) {
        e.handle_key(chord(KeyCode::Char(' ')));
        for c in keys.chars() {
            e.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn a_bare_control_space_opens_a_sequence() {
        let (mut e, _p) = editor_with("leader-bare.txt", b"ab\n");
        // A NUL byte, which is what a terminal sends for Ctrl+Space.
        let leader = KeyEvent {
            code: KeyCode::Char('\0'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(Keymap::default().is_leader(&leader));
        e.handle_key(leader);
        assert!(!e.seq.is_idle());
        assert_eq!(
            e.message,
            "Actions: [s] Save  [q] Quit  [f] Find  [p] Palette  [?] Help  [w] Wrap"
        );
    }

    #[test]
    fn a_shifted_at_sign_is_not_the_leader() {
        let (mut e, _p) = editor_with("leader-at.txt", b"ab\n");
        let at = KeyEvent {
            code: KeyCode::Char('@'),
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        e.handle_key(at);
        assert!(e.seq.is_idle());
        assert_eq!(line(&e, 0), "@ab");
    }

    #[test]
    fn leader_d_w_deletes_a_word() {
        let (mut e, _p) = editor_with("leader-dw.txt", b"alpha beta\n");
        sequence(&mut e, "dw");
        assert_eq!(line(&e, 0), "beta");
        assert_eq!(e.view.cursor_col, 0);
        assert!(e.dirty);
        assert_eq!(e.clipboard, "alpha ", "a delete also fills the clipboard");
    }

    #[test]
    fn leader_2_d_w_deletes_two_words() {
        let (mut e, _p) = editor_with("leader-count.txt", b"one two three\n");
        sequence(&mut e, "2dw");
        assert_eq!(line(&e, 0), "three");
    }

    #[test]
    fn leader_d_d_deletes_the_whole_line() {
        let (mut e, _p) = editor_with("leader-dd.txt", b"one\ntwo\nthree\n");
        sequence(&mut e, "dd");
        assert_eq!(e.doc.line_count(), 2);
        assert_eq!(line(&e, 0), "two");
        assert_eq!(line(&e, 1), "three");
        assert_eq!(e.clipboard, "one\n");
    }

    #[test]
    fn leader_d_dollar_deletes_to_the_end_of_the_line() {
        let (mut e, _p) = editor_with("leader-dend.txt", b"abcdef\nnext\n");
        e.view.cursor_col = 2;
        sequence(&mut e, "d$");
        assert_eq!(line(&e, 0), "ab");
        assert_eq!(line(&e, 1), "next");
    }

    #[test]
    fn a_deletion_through_the_leader_is_one_undo_step() {
        let (mut e, _p) = editor_with("leader-undo.txt", b"alpha beta\n");
        sequence(&mut e, "dw");
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert_eq!(line(&e, 0), "alpha beta");
        assert_eq!(
            e.view.cursor_col, 0,
            "undo returns to where the range began"
        );
    }

    #[test]
    fn yanking_then_pasting_copies_a_range() {
        let (mut e, _p) = editor_with("leader-yank.txt", b"alpha beta\n");
        sequence(&mut e, "yw");
        assert_eq!(e.clipboard, "alpha ");
        assert!(!e.dirty, "yank must not change the file");
        e.handle_key(chord(KeyCode::Char('\u{16}')));
        assert_eq!(line(&e, 0), "alpha alpha beta");
    }

    #[test]
    fn leader_u_undoes_and_leader_shift_u_redoes() {
        let (mut e, _p) = editor_with("leader-undo-key.txt", b"ab\n");
        e.insert_char('x');
        sequence(&mut e, "u");
        assert_eq!(line(&e, 0), "ab");
        sequence(&mut e, "U");
        assert_eq!(line(&e, 0), "xab");
    }

    #[test]
    fn escape_abandons_a_sequence_and_the_next_key_types_normally() {
        let (mut e, _p) = editor_with("leader-esc.txt", b"ab\n");
        sequence(&mut e, "d");
        assert_eq!(e.message, "delete: motion?");
        e.handle_key(key(KeyCode::Esc));
        assert!(e.seq.is_idle());
        e.handle_key(key(KeyCode::Char('w')));
        assert_eq!(line(&e, 0), "wab");
    }

    #[test]
    fn leader_comma_toggles_the_gutter() {
        let (mut e, _p) = editor_with("leader-gutter.txt", b"ab\n");
        assert!(e.view.show_line_numbers);
        sequence(&mut e, ",");
        assert!(!e.view.show_line_numbers);
        sequence(&mut e, ",");
        assert!(e.view.show_line_numbers);
    }

    #[test]
    fn a_mark_remembers_a_spot_and_returns_to_it() {
        let (mut e, _p) = editor_with("mark.txt", b"one\ntwo\nthree\n");
        e.view.cursor_line = 2;
        e.view.cursor_col = 1;
        sequence(&mut e, "ma");
        assert!(e.marks.contains_key(&'a'), "the mark is stored");

        // Walk away, then come back.
        e.move_to(Pos::new(0, 0));
        sequence(&mut e, "'a");
        assert_eq!(e.view.cursor_line, 2, "back on the marked line");
        assert_eq!(e.view.cursor_col, 1, "at the marked column");
    }

    #[test]
    fn jumping_to_an_unset_mark_says_so_and_stays_put() {
        let (mut e, _p) = editor_with("mark-none.txt", b"one\n");
        sequence(&mut e, "'x");
        assert_eq!(e.message, "no mark 'x'");
        assert_eq!(e.view.cursor_line, 0, "the cursor did not move");
    }

    #[test]
    fn leader_bracket_indents_and_unindents_the_line() {
        let (mut e, _p) = editor_with("indent.txt", b"ab\ncd\n");
        e.view.cursor_col = 1;
        sequence(&mut e, "]");
        assert_eq!(line(&e, 0), "    ab");
        assert_eq!(e.view.cursor_col, 5, "the cursor moved with the text");
        assert!(e.dirty);

        sequence(&mut e, "[");
        assert_eq!(line(&e, 0), "ab", "one level comes off");
        assert_eq!(e.view.cursor_col, 1);
    }

    #[test]
    fn unindent_takes_only_what_was_there() {
        let (mut e, _p) = editor_with("indent-partial.txt", b"  ab\n");
        sequence(&mut e, "[");
        assert_eq!(line(&e, 0), "ab", "two spaces, not four");
        sequence(&mut e, "[");
        assert_eq!(line(&e, 0), "ab", "already at the margin");
        assert_eq!(e.message, "nothing to unindent");
        assert_eq!(e.doc.history().undo_depth(), 1, "the no-op added no edit");
    }

    #[test]
    fn unindent_removes_a_leading_tab_too() {
        let (mut e, _p) = editor_with("indent-tab.txt", b"\tab\n");
        sequence(&mut e, "[");
        assert_eq!(line(&e, 0), "ab");
    }

    #[test]
    fn the_indent_operator_covers_the_range_and_is_one_undo_step() {
        let (mut e, _p) = editor_with("indent-op.txt", b"ab\ncd\nef\n");
        // From column 1, `j` lands on (1,1), so the range reaches into
        // line 1 and both lines are touched by it.
        e.view.cursor_col = 1;
        sequence(&mut e, ">j");
        assert_eq!(line(&e, 0), "    ab");
        assert_eq!(line(&e, 1), "    cd");
        assert_eq!(line(&e, 2), "ef", "the target's line past its column");
        assert_eq!(e.doc.history().undo_depth(), 1, "one command, one undo");
        e.undo();
        assert_eq!(line(&e, 0), "ab");
        assert_eq!(line(&e, 1), "cd");
    }

    /// Indent must agree with delete about which lines a range names — one
    /// key means the same thing in every sequence (spec §2).
    #[test]
    fn the_indent_operator_skips_the_line_the_target_lands_on() {
        let (mut e, _p) = editor_with("indent-boundary.txt", b"ab\ncd\n");
        // From column 0, `j` lands on (1,0): half-open, so the range is
        // exactly line 0 — the same lines `Leader d j` would delete.
        sequence(&mut e, ">j");
        assert_eq!(line(&e, 0), "    ab");
        assert_eq!(line(&e, 1), "cd", "the endpoint line is untouched");
    }

    #[test]
    fn repeated_indent_targets_the_line() {
        let (mut e, _p) = editor_with("indent-repeat.txt", b"ab\ncd\n");
        sequence(&mut e, ">>");
        assert_eq!(line(&e, 0), "    ab");
        assert_eq!(line(&e, 1), "cd");
    }

    #[test]
    fn ctrl_slash_comments_and_uncomments_the_line() {
        let (mut e, _p) = editor_with("comment.rs", b"let x = 1;\n");
        e.handle_key(chord(KeyCode::Char('/')));
        assert_eq!(line(&e, 0), "// let x = 1;");
        assert_eq!(e.message, "commented 1 line");
        assert_eq!(
            e.view.cursor_col, 3,
            "the cursor moves with the text it is on"
        );
        e.handle_key(chord(KeyCode::Char('/')));
        assert_eq!(line(&e, 0), "let x = 1;");
        assert_eq!(e.view.cursor_col, 0);
    }

    #[test]
    fn commenting_keeps_the_indentation_and_skips_blank_lines() {
        let (mut e, _p) = editor_with("comment-indent.py", b"  x = 1\n\ny = 2\n");
        e.view.anchor = Some(Pos::new(0, 0));
        // A cursor mid-line brings its line into the range; a range that ends
        // at column 0 stops before that line, the same as every operator.
        e.view.cursor_line = 2;
        e.view.cursor_col = 3;
        e.handle_key(chord(KeyCode::Char('/')));
        assert_eq!(line(&e, 0), "  # x = 1");
        assert_eq!(line(&e, 1), "", "a blank line is not given a marker");
        assert_eq!(line(&e, 2), "# y = 2");
        assert_eq!(e.doc.history().undo_depth(), 1, "one command, one undo");
    }

    #[test]
    fn a_partly_commented_range_is_commented_rather_than_uncommented() {
        let (mut e, _p) = editor_with("comment-partly.go", b"// one\ntwo\n");
        e.view.anchor = Some(Pos::new(0, 0));
        e.view.cursor_line = 1;
        e.view.cursor_col = 3;
        e.handle_key(chord(KeyCode::Char('/')));
        assert_eq!(line(&e, 0), "// // one");
        assert_eq!(line(&e, 1), "// two");
    }

    #[test]
    fn a_file_type_without_a_line_comment_says_so() {
        let (mut e, _p) = editor_with("comment.md", b"# heading\n");
        e.handle_key(chord(KeyCode::Char('/')));
        assert_eq!(e.message, "this file type has no line comment");
        assert_eq!(line(&e, 0), "# heading");
    }

    #[test]
    fn leader_w_toggles_word_wrap() {
        let (mut e, _p) = editor_with("wrap-toggle.txt", b"alpha beta\n");
        assert!(!e.view.word_wrap, "off by default (spec 4.5)");
        sequence(&mut e, "w");
        assert!(e.view.word_wrap);
        assert_eq!(e.message, "word wrap on");
        sequence(&mut e, "w");
        assert!(!e.view.word_wrap);
    }

    #[test]
    fn alt_z_toggles_word_wrap_without_typing_a_z() {
        let (mut e, _p) = editor_with("wrap-alt.txt", b"alpha beta\n");
        e.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT));
        assert!(e.view.word_wrap);
        assert_eq!(line(&e, 0), "alpha beta", "nothing was typed");
    }

    #[test]
    fn a_wrapped_vertical_move_stays_on_the_same_line() {
        let (mut e, _p) = editor_with("wrap-move.txt", b"alpha beta gamma delta\nsecond\n");
        e.view.word_wrap = true;
        e.view.width = 10;
        e.viewport_height_hint = 5;
        e.move_cursor(0, 1);
        assert_eq!(e.view.cursor_line, 0, "the next *row*, not the next line");
        assert_eq!(e.view.cursor_col, 10);
        e.move_cursor(0, -1);
        assert_eq!(e.view.cursor_col, 0);
    }

    #[test]
    fn a_cursor_deep_in_a_wrapped_line_scrolls_the_viewport() {
        let (mut e, _p) = editor_with("wrap-scroll.txt", b"alpha beta gamma delta\nsecond\n");
        e.view.word_wrap = true;
        e.view.width = 10;
        e.viewport_height_hint = 2;
        e.view.cursor_col = 17;
        e.ensure_cursor_visible();
        assert_eq!(e.view.top_line, 0, "the line itself is still the top");
        assert_eq!(e.view.top_row, 1, "the viewport starts one row down");
    }

    #[test]
    fn a_text_object_deletes_what_it_names() {
        let (mut e, _p) = editor_with("object-word.txt", b"one two three\n");
        e.view.cursor_col = 5;
        sequence(&mut e, "diw");
        assert_eq!(line(&e, 0), "one  three", "the word, not the gap around it");
    }

    #[test]
    fn an_object_taken_around_a_word_takes_the_gap_too() {
        let (mut e, _p) = editor_with("object-around.txt", b"one two three\n");
        e.view.cursor_col = 5;
        sequence(&mut e, "daw");
        assert_eq!(line(&e, 0), "one three");
    }

    #[test]
    fn a_text_object_can_be_selected_instead_of_deleted() {
        let (mut e, _p) = editor_with("object-select.txt", b"f(alpha)\n");
        e.view.cursor_col = 4;
        sequence(&mut e, "vi(");
        assert_eq!(e.selection(), Some((Pos::new(0, 2), Pos::new(0, 7))));
        assert_eq!(line(&e, 0), "f(alpha)", "selecting changed nothing");
    }

    #[test]
    fn an_object_that_is_not_there_says_so() {
        let (mut e, _p) = editor_with("object-none.txt", b"plain text\n");
        sequence(&mut e, "di(");
        assert_eq!(e.message, "no parentheses inside the cursor");
        assert_eq!(line(&e, 0), "plain text");
    }

    #[test]
    fn a_mark_past_the_end_of_a_shrunk_document_lands_inside_it() {
        let (mut e, _p) = editor_with("mark-shrunk.txt", b"one\ntwo\nthree\n");
        e.view.cursor_line = 2;
        sequence(&mut e, "ma");
        // Delete the marked line out from under the mark: the range spans
        // the boundary so the line really disappears rather than emptying.
        e.doc.edit(Pos::new(1, 3), Pos::new(2, 5), "");
        assert_eq!(e.doc.line_count(), 2, "the marked line is gone");
        sequence(&mut e, "'a");
        assert_eq!(e.view.cursor_line, 1, "clamped to the last line");
    }

    #[test]
    fn ctrl_c_does_not_quit() {
        let (mut e, _p) = editor_with("ctrl-c.txt", b"ab\n");
        e.handle_key(chord(KeyCode::Char('\u{03}')));
        assert!(!e.quit, "Ctrl+C is copy; Leader q quits");
        assert_eq!(e.message, "nothing selected");
    }

    #[test]
    fn leader_q_refuses_to_drop_unsaved_changes() {
        let (mut e, _p) = editor_with("leader-q.txt", b"ab\n");
        e.insert_char('x');
        sequence(&mut e, "q");
        assert!(!e.quit);
        assert!(
            e.message.contains("unsaved changes") && e.message.contains("Ctrl+S saves"),
            "{}",
            e.message
        );
        sequence(&mut e, "Q");
        assert!(e.quit);
    }

    #[test]
    fn leader_q_quits_a_clean_buffer() {
        let (mut e, _p) = editor_with("leader-q-clean.txt", b"ab\n");
        sequence(&mut e, "q");
        assert!(e.quit);
    }

    #[test]
    fn a_chord_inside_a_sequence_cancels_it_instead_of_acting_as_a_mnemonic() {
        let (mut e, _p) = editor_with("leader-chord.txt", b"ab\n");
        e.insert_char('x');
        sequence(&mut e, "d");
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        assert_eq!(e.message, "cancelled");
        assert!(e.dirty, "Ctrl+S inside a sequence must not save");
        assert!(e.seq.is_idle());
        assert_eq!(line(&e, 0), "xab");
    }

    #[test]
    fn pressing_the_leader_again_keeps_the_sequence_open() {
        let (mut e, _p) = editor_with("leader-again.txt", b"ab\n");
        sequence(&mut e, "d");
        e.handle_key(chord(KeyCode::Char(' ')));
        assert_eq!(
            e.message, "delete: motion?",
            "the operator is still pending"
        );
        e.handle_key(key(KeyCode::Char('w')));
        assert_eq!(line(&e, 0), "", "the word the operator was waiting for");
        assert_eq!(e.message, "deleted 2 chars");
    }

    // ----- changed on disk -----------------------------------------------

    /// Replaces the file the way another program would: write a new file, then
    /// rename it over the old one. (On Windows a mapped file cannot be opened
    /// for writing at all, so a direct overwrite is not even possible here.)
    fn rewrite(path: &std::path::Path, contents: &[u8]) {
        let replacement = path.with_extension("external");
        std::fs::write(&replacement, contents).unwrap();
        std::fs::rename(&replacement, path).unwrap();
    }

    #[test]
    fn saving_refuses_when_the_file_changed_on_disk() {
        let (mut e, path) = editor_with("changed-guard.txt", b"original\n");
        e.insert_char('X');
        rewrite(&path, b"someone else\n");
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        assert!(e.prompt.is_some(), "the editor must ask before overwriting");
        assert!(e.dirty);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "someone else\n",
            "the other writer's file must survive"
        );
    }

    #[test]
    fn choosing_overwrite_saves_the_editor_version() {
        let (mut e, path) = editor_with("changed-overwrite.txt", b"original\n");
        e.insert_char('X');
        rewrite(&path, b"someone else\n");
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        e.handle_key(key(KeyCode::Char('o')));
        assert!(!e.dirty);
        assert!(e.prompt.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Xoriginal\n");
    }

    #[test]
    fn choosing_reload_takes_the_disk_version_and_drops_the_journal() {
        let (mut e, path) = editor_with("changed-reload.txt", b"original\n");
        e.insert_char('X');
        assert!(e.doc.can_undo());
        rewrite(&path, b"theirs\n");
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        e.handle_key(key(KeyCode::Char('r')));
        assert_eq!(line(&e, 0), "theirs");
        assert!(!e.dirty);
        assert!(!e.doc.can_undo(), "the journal described the old document");
        assert!(e.message.contains("discarded"), "{}", e.message);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");
    }

    #[test]
    fn choosing_cancel_leaves_everything_alone() {
        let (mut e, path) = editor_with("changed-cancel.txt", b"original\n");
        e.insert_char('X');
        rewrite(&path, b"theirs\n");
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        e.handle_key(key(KeyCode::Char('c')));
        assert_eq!(e.message, "not saved");
        assert!(e.dirty, "the editor keeps the unsaved work");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");
    }

    #[test]
    fn a_choice_ignores_a_key_that_is_not_an_option() {
        let (mut e, path) = editor_with("changed-unknown.txt", b"original\n");
        e.insert_char('X');
        rewrite(&path, b"theirs\n");
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        e.handle_key(key(KeyCode::Char('z')));
        assert!(e.prompt.is_some(), "an unknown key is not an answer");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");
    }

    #[test]
    fn a_save_that_replaced_the_file_does_not_ask_again() {
        let (mut e, path) = editor_with("changed-resave.txt", b"ab\n");
        e.insert_char('X');
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        assert!(e.prompt.is_none(), "nothing changed, so nothing to ask");
        e.insert_char('Y');
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        assert!(
            e.prompt.is_none(),
            "the identity must be refreshed by the save that replaced the file"
        );
        // The caret stayed after the X, so the Y lands beside it.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "XYab\n");
    }

    // ----- prompt line ---------------------------------------------------

    fn prompt_text(e: &Editor) -> Option<String> {
        e.prompt.as_ref().map(|p| p.text().to_owned())
    }

    fn type_into_prompt(e: &mut Editor, text: &str) {
        for c in text.chars() {
            e.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn ctrl_g_asks_for_a_line_and_enter_jumps() {
        let (mut e, _p) = editor_with("goto.txt", b"one\ntwo\nthree\nfour\n");
        e.handle_key(chord(KeyCode::Char('\u{07}')));
        assert!(e.prompt.is_some(), "Ctrl+G must open the prompt");
        type_into_prompt(&mut e, "3");
        assert_eq!(prompt_text(&e).as_deref(), Some("3"));
        assert_eq!(line(&e, 0), "one", "typing a number must not edit the file");
        e.handle_key(key(KeyCode::Enter));
        assert!(e.prompt.is_none(), "Enter closes the prompt");
        assert_eq!(e.view.cursor_line, 2);
        assert_eq!(e.view.cursor_col, 0);
        assert!(!e.dirty);
    }

    #[test]
    fn the_prompt_edits_its_own_text() {
        let (mut e, _p) = editor_with("goto-edit.txt", b"a\nb\n");
        e.handle_key(chord(KeyCode::Char('\u{07}')));
        type_into_prompt(&mut e, "12");
        e.handle_key(key(KeyCode::Backspace));
        assert_eq!(prompt_text(&e).as_deref(), Some("1"));
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(e.view.cursor_line, 0, "line 1 is the first line");
    }

    #[test]
    fn escape_closes_the_prompt_without_moving() {
        let (mut e, _p) = editor_with("goto-esc.txt", b"one\ntwo\n");
        e.handle_key(chord(KeyCode::Char('\u{07}')));
        type_into_prompt(&mut e, "2");
        e.handle_key(key(KeyCode::Esc));
        assert!(e.prompt.is_none());
        assert_eq!(e.view.cursor_line, 0);
    }

    #[test]
    fn a_chord_does_not_fire_while_the_prompt_is_open() {
        let (mut e, _p) = editor_with("goto-chord.txt", b"ab\n");
        e.insert_char('x');
        e.handle_key(chord(KeyCode::Char('\u{07}')));
        e.handle_key(chord(KeyCode::Char('\u{13}')));
        assert!(e.prompt.is_some(), "Ctrl+S must not close the prompt");
        assert!(e.dirty, "Ctrl+S must not save while a question is open");
    }

    #[test]
    fn a_line_past_the_end_lands_on_the_last_line() {
        let (mut e, _p) = editor_with("goto-clamp.txt", b"one\ntwo\nthree\n");
        e.handle_key(chord(KeyCode::Char('\u{07}')));
        type_into_prompt(&mut e, "999");
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(e.view.cursor_line, 2);
    }

    #[test]
    fn a_prompt_that_is_not_a_number_says_so() {
        let (mut e, _p) = editor_with("goto-garbage.txt", b"one\n");
        e.handle_key(chord(KeyCode::Char('\u{07}')));
        type_into_prompt(&mut e, "abc");
        e.handle_key(key(KeyCode::Enter));
        assert!(e.message.contains("not a line number"), "{}", e.message);
        assert_eq!(e.view.cursor_line, 0);
    }

    #[test]
    fn leader_g_opens_the_same_prompt() {
        let (mut e, _p) = editor_with("goto-leader.txt", b"one\ntwo\n");
        sequence(&mut e, "g");
        assert!(e.prompt.is_some());
        assert_eq!(prompt_text(&e).as_deref(), Some(""));
    }

    #[test]
    fn ctrl_w_deletes_the_word_before_the_caret() {
        let (mut e, _p) = editor_with("ctrl-w.txt", b"alpha beta\n");
        e.view.cursor_col = 10;
        e.handle_key(chord(KeyCode::Char('\u{17}')));
        assert_eq!(line(&e, 0), "alpha ");
        assert_eq!(e.view.cursor_col, 6);
        assert!(e.dirty);
    }

    #[test]
    fn ctrl_k_deletes_to_the_end_of_the_line() {
        let (mut e, _p) = editor_with("ctrl-k.txt", b"alpha beta\nsecond\n");
        e.view.cursor_col = 5;
        e.handle_key(chord(KeyCode::Char('\u{0b}')));
        assert_eq!(line(&e, 0), "alpha");
        assert_eq!(line(&e, 1), "second", "only the current line is touched");
    }

    #[test]
    fn ctrl_e_moves_to_the_end_of_the_line() {
        let (mut e, _p) = editor_with("ctrl-e.txt", b"alpha beta\n");
        e.handle_key(chord(KeyCode::Char('\u{05}')));
        assert_eq!(e.view.cursor_col, 10);
        assert!(!e.dirty);
    }

    #[test]
    fn ctrl_l_selects_the_line_and_the_next_one_on_repeat() {
        let (mut e, _p) = editor_with("ctrl-l.txt", b"one\ntwo\nthree\n");
        e.handle_key(chord(KeyCode::Char('\u{0c}')));
        assert_eq!(e.selection(), Some((Pos::new(0, 0), Pos::new(0, 3))));
        e.handle_key(chord(KeyCode::Char('\u{0c}')));
        assert_eq!(
            e.selection(),
            Some((Pos::new(0, 0), Pos::new(1, 3))),
            "the selection grows by a line"
        );
        assert_eq!(line(&e, 0), "one");
    }

    // ----- selection -----------------------------------------------------

    fn shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    /// Selects `[from, to)` on one line by driving real shift+arrow keys.
    fn select(e: &mut Editor, from: usize, to: usize) {
        e.view.cursor_line = 0;
        e.view.cursor_col = from;
        for _ in from..to {
            e.handle_key(shift_key(KeyCode::Right));
        }
    }

    #[test]
    fn shift_right_starts_and_extends_a_selection() {
        let (mut e, _p) = editor_with("sel-start.txt", b"abcdef\n");
        e.handle_key(shift_key(KeyCode::Right));
        assert_eq!(e.view.anchor, Some(Pos::new(0, 0)));
        assert_eq!(e.view.cursor_col, 1);
        e.handle_key(shift_key(KeyCode::Right));
        assert_eq!(e.view.anchor, Some(Pos::new(0, 0)), "the anchor stays put");
        assert_eq!(e.selection(), Some((Pos::new(0, 0), Pos::new(0, 2))));
    }

    #[test]
    fn a_plain_arrow_clears_the_selection() {
        let (mut e, _p) = editor_with("sel-clear.txt", b"abcdef\n");
        select(&mut e, 1, 3);
        assert!(e.selection().is_some());
        e.handle_key(key(KeyCode::Left));
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn escape_clears_the_selection() {
        let (mut e, _p) = editor_with("sel-esc.txt", b"abcdef\n");
        select(&mut e, 1, 3);
        e.handle_key(key(KeyCode::Esc));
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn typing_replaces_the_selection_in_one_undo_step() {
        let (mut e, _p) = editor_with("sel-type.txt", b"abcdef\n");
        select(&mut e, 1, 3);
        e.handle_key(key(KeyCode::Char('X')));
        assert_eq!(line(&e, 0), "aXdef");
        assert_eq!(e.selection(), None, "typing consumes the selection");
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert_eq!(line(&e, 0), "abcdef", "one undo restores the replaced text");
    }

    #[test]
    fn backspace_deletes_the_selection() {
        let (mut e, _p) = editor_with("sel-backspace.txt", b"alpha beta\n");
        select(&mut e, 0, 6);
        e.handle_key(key(KeyCode::Backspace));
        assert_eq!(line(&e, 0), "beta");
        assert_eq!(e.view.cursor_col, 0);
        assert!(e.dirty);
    }

    #[test]
    fn delete_deletes_the_selection() {
        let (mut e, _p) = editor_with("sel-delete.txt", b"alpha beta\n");
        select(&mut e, 5, 10);
        e.handle_key(key(KeyCode::Delete));
        assert_eq!(line(&e, 0), "alpha");
    }

    #[test]
    fn ctrl_a_selects_the_whole_document() {
        let (mut e, _p) = editor_with("sel-all.txt", b"ab\ncd\n");
        e.handle_key(chord(KeyCode::Char('\u{1}')));
        assert_eq!(e.view.anchor, Some(Pos::new(0, 0)));
        assert_eq!(e.view.cursor_line, 1);
        assert_eq!(e.view.cursor_col, 2, "the end of the last line");
    }

    #[test]
    fn ctrl_c_copies_without_editing_and_queues_the_system_clipboard() {
        let (mut e, _p) = editor_with("sel-copy.txt", b"abcdef\n");
        select(&mut e, 1, 4);
        e.handle_key(chord(KeyCode::Char('\u{03}')));
        assert_eq!(e.clipboard, "bcd");
        assert_eq!(line(&e, 0), "abcdef", "copy must not edit");
        assert!(!e.dirty);
        assert!(
            e.osc.as_deref() == Some("\x1b]52;c;YmNk\x1b\\"),
            "the copy must queue an OSC 52 sequence: {:?}",
            e.osc
        );
        assert_eq!(e.message, "copied 3 chars");
    }

    #[test]
    fn ctrl_x_cuts_the_selection_and_undo_brings_it_back() {
        let (mut e, _p) = editor_with("sel-cut.txt", b"abcdef\n");
        select(&mut e, 1, 4);
        e.handle_key(chord(KeyCode::Char('\u{18}')));
        assert_eq!(line(&e, 0), "aef");
        assert_eq!(e.clipboard, "bcd");
        e.handle_key(chord(KeyCode::Char('\u{1a}')));
        assert_eq!(line(&e, 0), "abcdef");
    }

    #[test]
    fn ctrl_v_pastes_over_the_selection() {
        let (mut e, _p) = editor_with("sel-paste.txt", b"abcdef\n");
        select(&mut e, 1, 4);
        e.handle_key(chord(KeyCode::Char('\u{03}')));
        e.handle_key(key(KeyCode::End));
        e.handle_key(chord(KeyCode::Char('\u{16}')));
        // Copying clears nothing, so pasting at the end appends.
        assert_eq!(line(&e, 0), "abcdefbcd");

        let (mut e, _p) = editor_with("sel-paste-over.txt", b"abcdef\n");
        select(&mut e, 1, 4);
        e.handle_key(chord(KeyCode::Char('\u{03}')));
        e.handle_key(chord(KeyCode::Char('\u{16}')));
        assert_eq!(
            line(&e, 0),
            "abcdef",
            "pasting over its own text is a no-op"
        );
    }

    #[test]
    fn leader_v_w_selects_a_word() {
        let (mut e, _p) = editor_with("sel-leader.txt", b"alpha beta\n");
        sequence(&mut e, "vw");
        assert_eq!(e.view.anchor, Some(Pos::new(0, 0)));
        assert_eq!(e.view.cursor_col, 6);
        assert_eq!(line(&e, 0), "alpha beta", "selecting must not edit");
    }

    #[test]
    fn a_selection_wider_than_the_line_model_can_edit_is_refused() {
        let mut contents = String::new();
        for i in 0..20_001 {
            contents.push_str(&format!("line {i}\n"));
        }
        let (mut e, _p) = editor_with("sel-huge.txt", contents.as_bytes());
        e.view.anchor = Some(Pos::new(0, 0));
        e.view.cursor_line = 20_000;
        e.view.cursor_col = 0;
        e.handle_key(key(KeyCode::Char('X')));
        assert!(
            e.message.contains("beyond the current line model"),
            "{}",
            e.message
        );
        assert_eq!(
            line(&e, 0),
            "line 0",
            "the refused edit must change nothing"
        );
        assert!(!e.dirty);
    }

    #[test]
    fn a_sequence_swallows_the_key_that_does_not_belong_to_it() {
        let (mut e, _p) = editor_with("leader-typo.txt", b"ab\n");
        sequence(&mut e, "dz");
        assert_eq!(e.message, "cancelled");
        assert_eq!(line(&e, 0), "ab", "a cancelled sequence must not edit");
    }

    // ----- searching -----------------------------------------------------

    /// Waits for the background scans to report what they found.
    fn settle(e: &mut Editor) {
        for _ in 0..500 {
            e.poll_search();
            let running = e
                .search
                .as_ref()
                .is_some_and(|search| search.job.is_some() || search.count.is_some());
            if !running {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn ctrl_f_asks_for_a_pattern() {
        let (mut e, _p) = editor_with("find-open.txt", b"one\ntwo\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        assert!(e.prompt.is_some(), "Ctrl+F must open the prompt");
        assert_eq!(e.prompt.as_ref().unwrap().purpose(), Purpose::Search);
        assert_eq!(prompt_text(&e).as_deref(), Some(""));
    }

    #[test]
    fn leader_f_opens_the_same_prompt() {
        let (mut e, _p) = editor_with("find-leader.txt", b"one\n");
        sequence(&mut e, "f");
        assert!(e.prompt.is_some());
        assert_eq!(e.prompt.as_ref().unwrap().purpose(), Purpose::Search);
    }

    #[test]
    fn typing_a_pattern_highlights_the_matches_in_the_viewport() {
        let (mut e, _p) = editor_with("find-highlight.txt", b"alpha\nbeta\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "beta");
        let pattern = e
            .view
            .highlight_pattern
            .as_ref()
            .expect("the typed pattern is what the viewport highlights");
        assert_eq!(pattern.source(), "beta");
        let view = View {
            doc: &e.doc,
            state: &e.view,
            highlight: None,
        };
        assert_eq!(
            view.match_ranges(1, 20),
            vec![(0, 4)],
            "the match on the second line"
        );
        assert!(
            view.match_ranges(0, 20).is_empty(),
            "there is no match on the first line"
        );
    }

    #[test]
    fn enter_jumps_to_the_first_match() {
        let (mut e, _p) = editor_with("find-jump.txt", b"one\ntwo\nneedle\nthree\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "needle");
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert!(e.prompt.is_some(), "Enter must keep the find prompt open");
        assert_eq!(e.view.cursor_line, 2);
        assert_eq!(e.view.cursor_col, 0);
    }

    #[test]
    fn enter_cycles_the_matches_and_leaves_the_prompt_open() {
        let (mut e, _p) = editor_with("find-cycle.txt", b"hit\nx\nhit\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");

        // First Enter: moves to the first match and leaves the prompt open.
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert!(e.prompt.is_some(), "Enter must keep the find prompt open");
        assert_eq!(e.prompt.as_ref().unwrap().purpose(), Purpose::Search);
        assert_eq!(e.view.cursor_line, 0, "the first match");

        // Second Enter: advances past the current match while prompt remains open.
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert!(
            e.prompt.is_some(),
            "prompt must remain open after second Enter"
        );
        assert_eq!(e.view.cursor_line, 2, "the second match");

        // Third Enter: wraps around to the start of the file.
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert!(e.prompt.is_some(), "prompt must remain open after wrapping");
        assert_eq!(e.view.cursor_line, 0, "wrapped around to the first match");
    }

    #[test]
    fn find_first_enter_on_match_at_caret_keeps_match_and_subsequent_enter_advances() {
        let (mut e, _p) = editor_with("find-at-caret.txt", b"hit\nx\nhit\n");
        // Caret starts at (0, 0), which is exactly where the first match starts.
        assert_eq!(e.view.cursor_line, 0);
        assert_eq!(e.view.cursor_col, 0);

        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");

        // First Enter: should use Scope::From and keep the match at the caret.
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert_eq!(e.view.cursor_line, 0);
        assert_eq!(e.view.cursor_col, 0);

        // Second Enter: should use Scope::After and advance past the match at the caret.
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert_eq!(e.view.cursor_line, 2);
    }

    #[test]
    fn find_esc_closes_prompt_and_keeps_highlight_second_esc_clears_highlight() {
        let (mut e, _p) = editor_with("find-two-esc.txt", b"hit\nx\nhit\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);

        assert!(e.prompt.is_some());
        assert!(e.view.highlight_pattern.is_some());
        assert!(e.search.is_some());

        // First Esc: closes the prompt, keeps the highlight
        e.handle_key(key(KeyCode::Esc));
        assert!(e.prompt.is_none(), "first Esc closes the prompt");
        assert!(
            e.view.highlight_pattern.is_some(),
            "first Esc keeps the highlights"
        );
        assert!(e.search.is_some(), "search state remains active");

        // Second Esc (in ordinary editing): clears the highlight and search state
        e.handle_key(key(KeyCode::Esc));
        assert!(e.prompt.is_none());
        assert!(
            e.view.highlight_pattern.is_none(),
            "second Esc clears the highlights"
        );
        assert!(e.search.is_none(), "second Esc clears the search");
    }

    #[test]
    fn leader_n_walks_to_the_next_match_and_wraps_around() {
        let (mut e, _p) = editor_with("find-next.txt", b"hit\nx\nhit\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert_eq!(e.view.cursor_line, 0, "the first match");
        e.handle_key(key(KeyCode::Esc));
        sequence(&mut e, "n");
        settle(&mut e);
        assert_eq!(e.view.cursor_line, 2, "the next match");
        sequence(&mut e, "n");
        settle(&mut e);
        assert_eq!(e.view.cursor_line, 0, "the search wraps around to the top");
    }

    #[test]
    fn leader_shift_n_walks_back_to_the_previous_match() {
        let (mut e, _p) = editor_with("find-prev.txt", b"hit\nx\nhit\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert_eq!(e.view.cursor_line, 0);
        e.handle_key(key(KeyCode::Esc));
        sequence(&mut e, "N");
        settle(&mut e);
        assert_eq!(
            e.view.cursor_line, 2,
            "there is no match above, so the search wraps around to the last one"
        );
    }

    #[test]
    fn an_invalid_pattern_is_reported_and_moves_nothing() {
        let (mut e, _p) = editor_with("find-invalid.txt", b"one\ntwo\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "a(");
        assert!(
            e.prompt.as_ref().unwrap().note().is_some(),
            "the prompt says why the pattern is not good"
        );
        e.handle_key(key(KeyCode::Enter));
        assert!(e.message.contains("invalid pattern"), "{}", e.message);
        assert_eq!(e.view.cursor_line, 0, "an invalid pattern moves nothing");
    }

    #[test]
    fn the_matches_are_counted_in_the_background() {
        let (mut e, _p) = editor_with("find-count.txt", b"hit\nx\nhit\nhit\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");
        settle(&mut e);
        assert_eq!(e.search.as_ref().unwrap().total, Some(3));
    }

    #[test]
    fn a_search_that_finds_nothing_says_so() {
        let (mut e, _p) = editor_with("find-none.txt", b"one\ntwo\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "zzz");
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert_eq!(e.message, "no match");
        assert_eq!(e.view.cursor_line, 0);
    }

    #[test]
    fn a_second_search_starts_with_the_last_pattern() {
        let (mut e, _p) = editor_with("find-again.txt", b"hit\nx\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        e.handle_key(key(KeyCode::Esc));
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        assert_eq!(prompt_text(&e).as_deref(), Some("hit"));
    }

    #[test]
    fn tab_writes_one_level_in_the_files_own_style() {
        let (mut e, _p) = editor_with("indent-tab-key.txt", b"one\n");
        e.indent = IndentStyle::Tabs;
        e.handle_key(key(KeyCode::Tab));
        assert_eq!(line(&e, 0), "\tone");

        let (mut e, _p) = editor_with("indent-space-key.txt", b"one\n");
        e.indent = IndentStyle::Spaces(2);
        e.handle_key(key(KeyCode::Tab));
        assert_eq!(line(&e, 0), "  one");
    }

    #[test]
    fn shift_tab_unindents_the_line_and_moves_the_cursor_with_it() {
        let (mut e, _p) = editor_with("indent-shift-tab.txt", b"    one\n");
        e.indent = IndentStyle::Spaces(4);
        e.view.cursor_col = 5;
        e.handle_key(key(KeyCode::BackTab));
        assert_eq!(line(&e, 0), "one");
        assert_eq!(e.view.cursor_col, 1, "the text moved under the cursor");
    }

    #[test]
    fn shift_tab_unindents_every_line_a_selection_touches() {
        let (mut e, _p) = editor_with("indent-shift-tab-selection.txt", b"    one\n    two\n");
        e.indent = IndentStyle::Spaces(4);
        e.view.anchor = Some(Pos::new(0, 0));
        e.view.cursor_line = 1;
        e.view.cursor_col = 3;
        e.handle_key(key(KeyCode::BackTab));
        assert_eq!(line(&e, 0), "one");
        assert_eq!(line(&e, 1), "two");
    }

    #[test]
    fn enter_keeps_the_indentation_and_opens_a_block() {
        let (mut e, _p) = editor_with("enter-indent.rs", b"fn main() {\n");
        e.indent = IndentStyle::Spaces(4);
        e.view.cursor_col = 11;
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(line(&e, 1), "    ", "one level in from the line above");
        e.handle_key(key(KeyCode::Char('x')));
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(line(&e, 2), "    ", "a plain line keeps its own level");
    }

    #[test]
    fn enter_inside_an_empty_pair_does_not_indent() {
        let (mut e, _p) = editor_with("enter-pair.txt", b"call()\n");
        e.indent = IndentStyle::Spaces(4);
        e.view.cursor_col = 5;
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(line(&e, 0), "call(");
        assert_eq!(line(&e, 1), ")", "the closer stays where it is");
    }

    #[test]
    fn a_closer_at_the_start_of_a_line_takes_a_level_back() {
        let (mut e, _p) = editor_with("closer-dedent.rs", b"fn main() {\n    \n");
        e.indent = IndentStyle::Spaces(4);
        e.view.cursor_line = 1;
        e.view.cursor_col = 4;
        e.insert_char('}');
        assert_eq!(line(&e, 1), "}");
    }

    #[test]
    fn a_tab_indented_file_is_edited_with_tabs() {
        let (mut e, _p) = editor_with("detect-tabs.rs", b"fn f() {\n\tif x {\n\t\ty();\n\t}\n}\n");
        e.apply_settings(&Config::default());
        assert_eq!(e.indent, IndentStyle::Tabs);
    }

    #[test]
    fn editorconfig_outranks_the_files_own_content() {
        let dir = std::env::temp_dir().join("zeno-editorconfig-tests");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".editorconfig"),
            "root = true\n[*.py]\nindent_style = space\nindent_size = 2\n",
        )
        .unwrap();
        let path = dir.join("script.py");
        // The file itself is indented four ways; the config says two.
        std::fs::write(&path, b"if x:\n    one()\n").unwrap();
        let doc = Doc::open(&path).unwrap();
        let mut editor = Editor::new(doc, path);
        editor.apply_settings(&Config::default());
        assert_eq!(editor.indent, IndentStyle::Spaces(2));
    }

    #[test]
    fn settings_reach_the_editor() {
        let (mut e, _p) = editor_with("settings.txt", b"ab\n");
        let settings = zeno_cli::config::parse(
            "[settings]\nword_wrap = true\nline_numbers = false\ntab_width = 2\nauto_pair = false\ntheme = \"Solarized (dark)\"\n",
        );
        let config = Config {
            settings,
            ..Config::default()
        };
        e.apply_settings(&config);
        assert!(e.view.word_wrap);
        assert!(!e.view.show_line_numbers);
        assert_eq!(e.highlight.theme(), Some("Solarized (dark)"));
        assert_eq!(
            e.indent,
            IndentStyle::Spaces(2),
            "a file with no style of its own"
        );
        e.insert_char('(');
        assert_eq!(line(&e, 0), "(ab", "pairing was turned off");
    }

    #[test]
    fn a_chord_from_the_config_runs_its_command() {
        let (mut e, _p) = editor_with("binding.toml", b"one two\n");
        let config = zeno_cli::config::parse_config(
            "[keys]\nleader = \"ctrl+b\"\n\n[[keys.bindings]]\nkeys = \"ctrl+shift+w\"\ncommand = \"toggle_word_wrap\"\nsave_as = \"nonsense\"\n",
        );
        e.apply_settings(&config);
        assert!(!e.view.word_wrap);
        e.handle_key(KeyEvent::new(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert!(e.view.word_wrap, "the configured chord ran its command");
        // The bound chord swallowed the key: nothing was typed.
        assert_eq!(line(&e, 0), "one two");
    }

    #[test]
    fn a_rebound_leader_opens_a_sequence() {
        let (mut e, _p) = editor_with("leader-rebind.txt", b"one\ntwo\n");
        e.apply_settings(&zeno_cli::config::parse_config(
            "[keys]\nleader = \"ctrl+b\"\n",
        ));
        e.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert!(!e.seq.is_idle(), "Ctrl+B opened a sequence");
        e.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE));
        assert!(
            !e.view.show_line_numbers,
            "Leader , still toggles the gutter"
        );
    }

    #[test]
    fn a_leader_that_was_rebound_is_an_ordinary_key_again() {
        let (mut e, _p) = editor_with("leader-rebind-old.txt", b"ab\n");
        e.apply_settings(&zeno_cli::config::parse_config(
            "[keys]\nleader = \"ctrl+b\"\n",
        ));
        // Ctrl+Space is no longer the leader, so it types nothing and starts
        // no sequence.
        e.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
        assert!(e.seq.is_idle());
        assert_eq!(line(&e, 0), "ab");
    }

    #[test]
    fn autosave_writes_the_file_after_a_quiet_moment() {
        let (mut e, path) = editor_with("autosave.txt", b"ab\n");
        e.autosave = true;
        e.insert_char('X');
        // Not quiet yet: the key was just pressed.
        e.autosave_if_idle();
        assert!(e.dirty, "it does not write mid-typing");
        e.last_key = std::time::Instant::now() - Duration::from_millis(1000);
        e.autosave_if_idle();
        assert!(!e.dirty, "it writes once the typing stops");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Xab\n");
    }

    #[test]
    fn the_buffer_prints_back_the_text_it_was_given() {
        let (mut e, _p) = editor_with("text-body.txt", b"a\nb\n");
        assert_eq!(e.text(), "a\nb\n", "a final newline stays a final newline");
        let (mut e, _p) = editor_with("text-no-newline.txt", b"a\nb");
        assert_eq!(e.text(), "a\nb");
    }

    #[test]
    fn the_buffer_prints_what_was_edited() {
        let (mut e, _p) = editor_with("text-edited.txt", b"one\n");
        e.insert_char('X');
        assert_eq!(e.text(), "Xone\n");
    }

    #[test]
    fn leader_question_opens_help_and_esc_closes_it() {
        let (mut e, _p) = editor_with("help.txt", b"ab\n");
        sequence(&mut e, "?");
        assert!(
            matches!(e.overlay, Some(Overlay::Help { .. })),
            "Leader ? opens the help list"
        );
        e.handle_key(key(KeyCode::Down));
        match e.overlay {
            Some(Overlay::Help { offset }) => assert_eq!(offset, 1, "it scrolls"),
            _ => panic!("the help list is still open"),
        }
        e.handle_key(key(KeyCode::Esc));
        assert!(e.overlay.is_none());
        assert_eq!(line(&e, 0), "ab", "reading help edits nothing");
    }

    #[test]
    fn f1_opens_the_palette_and_enter_runs_the_chosen_command() {
        let (mut e, _p) = editor_with("palette.txt", b"ab\n");
        assert!(!e.view.word_wrap);
        e.handle_key(key(KeyCode::F(1)));
        assert!(matches!(e.overlay, Some(Overlay::Palette(_))));
        type_into_prompt(&mut e, "wrap");
        e.handle_key(key(KeyCode::Enter));
        assert!(e.view.word_wrap, "running the command took effect");
        assert!(e.overlay.is_none(), "and the palette closed");
    }

    #[test]
    fn ctrl_p_opens_the_palette() {
        let (mut e, _p) = editor_with("palette-ctrl-p.txt", b"ab\n");
        e.handle_key(chord(KeyCode::Char('p')));
        assert!(matches!(e.overlay, Some(Overlay::Palette(_))));
    }

    #[test]
    fn the_palette_query_does_not_reach_the_document() {
        let (mut e, _p) = editor_with("palette-keys.txt", b"ab\n");
        e.handle_key(key(KeyCode::F(1)));
        type_into_prompt(&mut e, "save");
        assert_eq!(line(&e, 0), "ab", "the letters went to the query");
        assert!(!e.dirty);
    }

    #[test]
    fn escape_closes_the_palette_without_running_anything() {
        let (mut e, _p) = editor_with("palette-esc.txt", b"ab\n");
        e.handle_key(key(KeyCode::F(1)));
        type_into_prompt(&mut e, "quit");
        e.handle_key(key(KeyCode::Esc));
        assert!(e.overlay.is_none());
        assert!(!e.quit, "the command under the cursor did not run");
    }

    #[test]
    fn a_palette_command_that_needs_an_answer_asks_for_it() {
        let (mut e, _p) = editor_with("palette-prompt.txt", b"one\ntwo\n");
        e.handle_key(key(KeyCode::F(1)));
        type_into_prompt(&mut e, "go to line");
        e.handle_key(key(KeyCode::Enter));
        assert!(e.overlay.is_none(), "the palette closed");
        assert!(e.prompt.is_some(), "and the line prompt took over");
    }

    #[test]
    fn the_palette_arrow_keys_move_the_selection() {
        let (mut e, _p) = editor_with("palette-arrows.txt", b"ab\n");
        e.handle_key(key(KeyCode::F(1)));
        e.handle_key(key(KeyCode::Down));
        match &e.overlay {
            Some(Overlay::Palette(palette)) => {
                assert_eq!(palette.selection(), 1);
                assert_eq!(palette.selected().map(|entry| entry.label), Some("Quit"));
            }
            _ => panic!("the palette is open"),
        }
    }

    #[test]
    fn an_opening_bracket_brings_its_partner() {
        let (mut e, _p) = editor_with("pair.txt", b"\n");
        e.insert_char('(');
        assert_eq!(line(&e, 0), "()");
        assert_eq!(e.view.cursor_col, 1, "the caret sits between them");
    }

    #[test]
    fn typing_the_closer_that_is_already_there_steps_over_it() {
        let (mut e, _p) = editor_with("pair-step.txt", b"()\n");
        e.view.cursor_col = 1;
        e.insert_char(')');
        assert_eq!(line(&e, 0), "()", "it is not doubled");
        assert_eq!(e.view.cursor_col, 2);
    }

    #[test]
    fn a_bracket_next_to_text_is_not_paired() {
        let (mut e, _p) = editor_with("pair-text.txt", b"foo\n");
        e.view.cursor_col = 0;
        e.insert_char('(');
        assert_eq!(line(&e, 0), "(foo", "editing text, not opening a block");
    }

    #[test]
    fn an_apostrophe_between_letters_is_not_paired() {
        let (mut e, _p) = editor_with("pair-apostrophe.txt", b"dont\n");
        e.view.cursor_col = 2;
        let mut editor = e;
        editor.insert_char('\'');
        assert_eq!(line(&editor, 0), "do'nt");
        assert_eq!(editor.view.cursor_col, 3);
    }

    #[test]
    fn a_pair_is_not_put_around_a_selection() {
        let (mut e, _p) = editor_with("pair-selection.txt", b"abc\n");
        e.view.anchor = Some(Pos::new(0, 0));
        e.view.cursor_col = 3;
        e.insert_char('(');
        assert_eq!(line(&e, 0), "(", "typing over a selection replaces it");
    }

    #[test]
    fn backspacing_an_empty_pair_removes_both_halves() {
        let (mut e, _p) = editor_with("pair-backspace.txt", b"\n");
        e.insert_char('(');
        assert_eq!(line(&e, 0), "()");
        e.backspace();
        assert_eq!(line(&e, 0), "");
        assert_eq!(e.view.cursor_col, 0);
    }

    #[test]
    fn ctrl_h_replaces_every_match_in_the_file() {
        let (mut e, _p) = editor_with("replace.txt", b"hit x hit\ny hit\n");
        e.handle_key(chord(KeyCode::Char('h')));
        type_into_prompt(&mut e, "hit");
        assert_eq!(prompt_text(&e).as_deref(), Some("hit"));
        assert_eq!(line(&e, 0), "hit x hit", "typing the pattern edits nothing");
        e.handle_key(key(KeyCode::Enter));
        assert!(e.prompt.is_some(), "it asks what to put there");
        type_into_prompt(&mut e, "HIT");
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(line(&e, 0), "HIT x HIT");
        assert_eq!(line(&e, 1), "y HIT");
        assert_eq!(e.message, "replaced 3 matches");
        assert_eq!(e.doc.history().undo_depth(), 1, "one command, one undo");
        e.undo();
        assert_eq!(line(&e, 0), "hit x hit");
    }

    #[test]
    fn a_replacement_may_be_empty() {
        let (mut e, _p) = editor_with("replace-empty.txt", b"a-b-c\n");
        sequence(&mut e, "r");
        type_into_prompt(&mut e, "-");
        e.handle_key(key(KeyCode::Enter));
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(line(&e, 0), "abc");
        assert_eq!(e.message, "replaced 2 matches");
    }

    #[test]
    fn a_replacement_with_nothing_to_find_says_so() {
        let (mut e, _p) = editor_with("replace-none.txt", b"abc\n");
        sequence(&mut e, "r");
        type_into_prompt(&mut e, "zzz");
        e.handle_key(key(KeyCode::Enter));
        // It still asks for the replacement: whether the pattern matches is
        // answered by the scan, not by the prompt.
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(e.message, "no match for zzz");
        assert_eq!(line(&e, 0), "abc");
    }

    #[test]
    fn a_replacement_that_does_not_compile_is_refused() {
        let (mut e, _p) = editor_with("replace-bad.txt", b"abc\n");
        e.handle_key(chord(KeyCode::Char('h')));
        type_into_prompt(&mut e, "a(");
        e.handle_key(key(KeyCode::Enter));
        assert!(
            e.prompt.is_none(),
            "a pattern that does not compile is not replaced"
        );
        assert!(e.message.contains("invalid pattern"), "{}", e.message);
        assert_eq!(line(&e, 0), "abc");
    }

    #[test]
    fn a_replacement_takes_the_selection_only_when_there_is_one() {
        let (mut e, _p) = editor_with("replace-selection.txt", b"hit\nmiddle\nhit\n");
        e.view.anchor = Some(Pos::new(0, 0));
        e.view.cursor_line = 0;
        e.view.cursor_col = 3;
        sequence(&mut e, "r");
        type_into_prompt(&mut e, "hit");
        e.handle_key(key(KeyCode::Enter));
        type_into_prompt(&mut e, "HIT");
        e.handle_key(key(KeyCode::Enter));
        assert_eq!(line(&e, 0), "HIT");
        assert_eq!(line(&e, 2), "hit", "the last line was outside the range");
    }

    #[test]
    fn a_count_for_a_document_that_changed_is_counted_again() {
        let (mut e, _p) = editor_with("find-stale.txt", b"hit\nx\nhit\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");
        e.handle_key(key(KeyCode::Enter));
        settle(&mut e);
        assert_eq!(e.search.as_ref().unwrap().total, Some(2));
        // The document changes, so the number was about a document that no
        // longer exists, and it is counted again.
        e.insert_text("hit");
        settle(&mut e);
        assert_eq!(e.search.as_ref().unwrap().total, Some(3));
    }

    #[test]
    fn emptying_the_pattern_clears_the_highlight() {
        let (mut e, _p) = editor_with("find-clear.txt", b"hit\nx\n");
        e.handle_key(chord(KeyCode::Char('\u{06}')));
        type_into_prompt(&mut e, "hit");
        assert!(e.view.highlight_pattern.is_some());
        for _ in 0..3 {
            e.handle_key(key(KeyCode::Backspace));
        }
        assert!(
            e.view.highlight_pattern.is_none(),
            "an empty pattern highlights nothing"
        );
    }

    #[test]
    fn the_viewport_scrolls_sideways_to_follow_the_cursor() {
        let (mut e, _p) = editor_with("scroll-x.txt", b"0123456789abcdefghij\n");
        e.view.width = 10;
        for _ in 0..10 {
            e.handle_key(key(KeyCode::Right));
        }
        assert_eq!(e.view.cursor_col, 10);
        assert_eq!(
            e.view.left_col, 1,
            "the window scrolls to keep the cursor in it"
        );
        e.handle_key(key(KeyCode::Right));
        assert_eq!(e.view.cursor_col, 11);
        assert_eq!(e.view.left_col, 2);
    }

    #[test]
    fn moving_back_scrolls_the_window_back() {
        let (mut e, _p) = editor_with("scroll-back.txt", b"0123456789abcdefghij\n");
        e.view.width = 10;
        for _ in 0..12 {
            e.handle_key(key(KeyCode::Right));
        }
        assert_eq!(e.view.left_col, 3);
        for _ in 0..12 {
            e.handle_key(key(KeyCode::Left));
        }
        assert_eq!(e.view.cursor_col, 0);
        assert_eq!(
            e.view.left_col, 0,
            "the window scrolls back to the first column"
        );
    }

    #[test]
    fn end_scrolls_the_window_to_the_end_of_the_line() {
        let (mut e, _p) = editor_with("scroll-end.txt", b"0123456789abcdefghij\n");
        e.view.width = 10;
        e.handle_key(key(KeyCode::End));
        assert_eq!(e.view.cursor_col, 20);
        assert_eq!(e.view.left_col, 11);
        e.handle_key(key(KeyCode::Home));
        assert_eq!(e.view.cursor_col, 0);
        assert_eq!(e.view.left_col, 0);
    }
}
