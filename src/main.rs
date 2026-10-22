//! zeno — a fast, non-modal terminal text editor.
//!
//! One file per run. The file is mapped into memory, the visible part is
//! indexed on demand, and the viewport is repainted after every keystroke.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Terminal;

use zeno_cli::clipboard;
use zeno_cli::doc::edit::Pos;
use zeno_cli::doc::{identity_of, Doc};
use zeno_cli::motion::{self, Motion};
use zeno_cli::prompt::{Prompt, Purpose};
use zeno_cli::render::{View, ViewState};
use zeno_cli::sequence::{Action, Command, Operator, Sequences, Step, Target};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TAB_WIDTH: usize = 4;

/// The most lines one command may edit at once.
///
/// Not a product limit — it is the point where the line-anchored model stops
/// being able to answer inside a frame (see `docs/performance.md`, finding 8).
/// Ten thousand lines costs about 20 ms to delete; a hundred thousand would
/// cost seconds.
const MAX_SPAN_LINES: u64 = 10_000;

/// The answers offered when the file changed on disk under the editor.
const CHANGED_ON_DISK: &[(char, &'static str)] = &[
    ('r', "reload"),
    ('o', "overwrite"),
    ('c', "cancel"),
];

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
        return layout_letter(c);
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

/// Whether this key is the leader.
///
/// A legacy terminal sends `Ctrl+Space` as `NUL`, which may reach the editor
/// as a control-space, as `@`, or as the NUL character itself — and on some
/// platforms the modifiers are lost along the way. All of those are the
/// leader; a plain `@` typed with Shift is not.
fn is_leader(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(' ') => ctrl,
        KeyCode::Char('@') => ctrl,
        // A terminal that keeps the NUL byte reports it as the NUL character,
        // or as no key at all, with the modifiers already dropped.
        KeyCode::Char('\0') | KeyCode::Null => true,
        _ => false,
    }
}

/// The prompt as a widget: the label and the text being typed.
fn prompt_line(prompt: &Prompt) -> Paragraph<'static> {
    let style = Style::default()
        .bg(Color::Rgb(28, 28, 34))
        .fg(Color::Yellow);
    Paragraph::new(Line::from(Span::styled(prompt.line(), style))).style(style)
}

/// What an operator's target is called, for messages.
fn target_label(target: Target) -> &'static str {
    match target {
        Target::Line => "line",
        Target::Motion(motion) => motion.label(),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path: Option<PathBuf> = None;
    let mut background_index = true;
    let mut bench_open: Option<PathBuf> = None;

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

    let Some(path) = path else {
        eprintln!("zeno: no file given");
        print_help();
        return ExitCode::FAILURE;
    };

    match run(&path, background_index) {
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
    println!();
    println!("OPTIONS:");
    println!("        --no-background-index   do not scan the whole file in the background");
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
fn run(path: &Path, background_index: bool) -> Result<(), String> {
    let doc = Doc::open(path).map_err(|e| e.to_string())?;
    if doc.looks_binary(8192) {
        return Err("refusing to open a binary file".into());
    }
    if background_index {
        let _ = doc.spawn_indexer();
    }

    let mut editor = Editor::new(doc, path.to_path_buf());
    editor.run()
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
}

impl Editor {
    fn new(doc: Doc, path: PathBuf) -> Self {
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
        }
    }

    fn run(&mut self) -> Result<(), String> {
        enable_raw_mode().map_err(|e| e.to_string())?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(|e| e.to_string())?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

        let result = self.event_loop(&mut terminal);

        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).ok();
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
                // Nothing happened: the background index may have advanced.
                self.draw(terminal).map_err(|e| e.to_string())?;
                continue;
            }
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
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

        // A pending leader sequence owns the next key, whatever it is.
        if !self.seq.is_idle() {
            self.feed_sequence(key);
            return;
        }

        if is_leader(&key) {
            self.seq.start();
            self.message = "Leader…".into();
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if ctrl {
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
            KeyCode::Tab => self.insert_text(&" ".repeat(TAB_WIDTH)),
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
            }
            _ => {}
        }
    }

    /// Routes one key into the open prompt.
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let mut submit: Option<Answer> = None;
        let mut cancel = false;
        {
            let Some(prompt) = self.prompt.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Esc => cancel = true,
                KeyCode::Enter if prompt.is_text() => {
                    submit = Some(Answer::Text(prompt.text().to_owned()));
                }
                KeyCode::Char(c) if !ctrl && !prompt.is_text() => {
                    if let Some(choice) = prompt.choose(c) {
                        submit = Some(Answer::Choice(choice));
                    }
                }
                KeyCode::Char(c) if !ctrl => prompt.insert(c),
                KeyCode::Backspace => {
                    prompt.backspace();
                }
                KeyCode::Delete => {
                    prompt.delete();
                }
                KeyCode::Left => prompt.left(),
                KeyCode::Right => prompt.right(),
                KeyCode::Home => prompt.home(),
                KeyCode::End => prompt.end(),
                _ => {}
            }
        }

        if cancel {
            self.prompt = None;
            self.message.clear();
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
                        self.message = format!("unexpected choice: {c}").into();
                    }
                }
            }
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
                _ => {
                    self.message = format!("unexpected text: {text}").into();
                }
            }
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
        if is_leader(&key) {
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
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Save => self.save(),
            Command::Quit => {
                if self.dirty {
                    self.message = "unsaved changes — Leader s saves, Leader Q discards".into();
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
            Command::GoToLine => self.open_prompt(Purpose::GoToLine),
        }
    }

    /// Resolves an operator against the range its target names, then acts on it.
    fn apply_operator(&mut self, operator: Operator, target: Target, count: u32) {
        let from = self.cursor();
        let to = match target {
            Target::Line => {
                let last = self.doc.line_count().saturating_sub(1);
                let end_line = from.line.saturating_add(count as u64);
                if end_line > last {
                    Pos::new(last, self.line_len(last))
                } else {
                    Pos::new(end_line, 0)
                }
            }
            Target::Motion(motion) => {
                let mut to = from;
                for _ in 0..count.max(1) {
                    to = motion::target(&self.doc, to, motion);
                }
                to
            }
        };
        let (start, end) = if to < from { (to, from) } else { (from, to) };

        // `v` names a range but changes nothing: it just moves the anchor.
        if operator == Operator::Select {
            self.view.anchor = Some(from);
            self.move_to(to);
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
        }
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
        self.insert_replacing_selection(&c.to_string());
    }

    fn insert_text(&mut self, text: &str) {
        self.insert_replacing_selection(text);
    }

    fn split_line(&mut self) {
        self.insert_replacing_selection("\n");
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let pos = self.cursor();
        if pos.col > 0 {
            let start = Pos::new(pos.line, pos.col - 1);
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
        let height = self.viewport_height() as u64;
        if self.view.cursor_line < self.view.top_line {
            self.view.top_line = self.view.cursor_line;
        } else if self.view.cursor_line >= self.view.top_line + height {
            self.view.top_line = self.view.cursor_line + 1 - height;
        }
    }

    // ----- saving --------------------------------------------------------

    /// Saves, unless the file on disk is no longer the one that was mapped.
    ///
    /// Overwriting silently is how an editor eats someone else's work — and,
    /// because the mapping is read-only but not protected from truncation, it
    /// is also how it can read past the end of a file that shrank.
    fn save(&mut self) {
        if let Ok(current) = identity_of(&self.path) {
            if current != self.doc.identity() {
                self.prompt = Some(Prompt::choice(Purpose::ChangedOnDisk, CHANGED_ON_DISK));
                self.message.clear();
                return;
            }
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
                out.write_all(ending.as_str().as_bytes()).map_err(|e| e.to_string())?;
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

    fn draw(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<()> {
        let size = terminal.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        let body = chunks[0];
        let status = chunks[1];

        self.viewport_height_hint = body.height;
        self.view.width = body.width;

        terminal.draw(|f| {
            let view = View {
                doc: &self.doc,
                state: &self.view,
            };
            let cursor = view.cursor_position(body);
            f.render_widget(view, body);
            match &self.prompt {
                // The question replaces the status line, so the document area
                // keeps its size and nothing on screen moves.
                Some(prompt) => {
                    f.render_widget(prompt_line(prompt), status);
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
            spans.push(Span::styled(
                " [+]",
                Style::default().fg(Color::Yellow),
            ));
        }
        spans.push(Span::raw("  "));
        spans.push(Span::raw(format!("Ln {}", self.view.cursor_line + 1)));
        spans.push(Span::raw(format!(", Col {}", self.view.cursor_col + 1)));
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
        if !self.message.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                self.message.clone(),
                Style::default().fg(Color::Cyan),
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
        let (mut e, _p) = editor_with("sequence.txt", b"fn main() {\n    x();\n}\n\n// c\nlet y = 1;\n");
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
        assert!(is_leader(&leader));
        e.handle_key(leader);
        assert!(!e.seq.is_idle());
        assert_eq!(e.message, "Leader…");
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
        assert_eq!(e.view.cursor_col, 0, "undo returns to where the range began");
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
        assert!(e.message.contains("unsaved changes"), "{}", e.message);
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
        assert_eq!(e.message, "delete: motion?", "the operator is still pending");
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
        assert!(
            e.message.contains("not a line number"),
            "{}",
            e.message
        );
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
        assert_eq!(line(&e, 0), "abcdef", "pasting over its own text is a no-op");
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
        assert_eq!(line(&e, 0), "line 0", "the refused edit must change nothing");
        assert!(!e.dirty);
    }

    #[test]
    fn a_sequence_swallows_the_key_that_does_not_belong_to_it() {
        let (mut e, _p) = editor_with("leader-typo.txt", b"ab\n");
        sequence(&mut e, "dz");
        assert_eq!(e.message, "cancelled");
        assert_eq!(line(&e, 0), "ab", "a cancelled sequence must not edit");
    }
}
