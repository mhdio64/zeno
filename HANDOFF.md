# zeno — handoff

Everything the next agent needs to pick this project up without re-deriving it.
Read this file first, then `CONTEXT.md`, `docs/spec.md`, and `docs/keymap.md`.
There is no `README.md` and no `AGENTS.md`; this file is the primary entry point.

---

## 0. State of the working tree — read before touching anything

- **Repository root**: `c:\Users\main\Documents\Projects\zeno`. Branch: `master`.
- **Working tree status**: Clean. All feature, optimization, security, and resilience work has been committed and verified.
- **Latest commits**:
  - `3f004ca`: *install panic hook and raw mode guard and redraw on window resize and handle thread spawn exhaustion*
  - `76de336`: *harden file saving against symlinks and preserve file permissions and validate open file types and escape git paths*
  - `1e6e690`: *optimize search matches and finder lowercase caching and completion word scanning*
  - `0589520`: *optimize viewport styled spans and cursor position and overlay locate and remove duplicate key code*
  - `b5f538b`: *increase dark modern theme contrast with black background and vibrant syntax colors*
  - `e84e791`: *add vs code dark modern theme and make it the default theme*
  - `1352e23`: *highlight matching bracket pairs in viewport and add toggle command with config option*
  - `9d8f4ed`: *surround selection with brackets or quotes when typing paired characters and update tests*
- **Test suite**: **511 tests passing cleanly** (295 library tests in `zeno_cli` + 216 editor tests in `zeno`, 0 failed, 0 ignored).
- **Quality gates**:
  - `cargo fmt -- --check`: Clean (no formatting diffs).
  - `cargo clippy --all-targets -- -D warnings`: Clean (0 warnings).
  - `cargo test`: 100% green.

---

## 1. What zeno is, and the rules it is built on

zeno is a lightning-fast, non-modal terminal text editor designed to replace `nano`: instant open, one file per invocation, full editing power, atomic save, and quit. The editor is judged by keystroke latency and launch responsiveness (first frame < 50 ms, keystroke latency < 16 ms, launch < 30 ms; spec §1, §7).

The architectural vocabulary is defined in `CONTEXT.md` and must be adhered to strictly in code, docs, and conversation: **quick editor, non-modal, command, operator, motion, text object, binding, leader key, instant open, prompt, viewport, non-goal**. Do not use "mode", "workspace", "dialog", "shortcut", or "window" for these concepts.

### Non-negotiable architectural decisions (ADRs 0001–0024):

| Rule / Principle | ADR Reference |
| :--- | :--- |
| No modes. One universal keymap, always. No normal/insert/visual separation. | ADR 0002 |
| Never rely on `Ctrl+Shift+<letter>` (indistinguishable on Windows/ConPTY). | ADR 0005 |
| A chord resolves strictly through the **physical key**, never the produced character. | ADR 0010 |
| Every single edit routes through one atomic primitive: `Doc::edit`, with an undo journal. | ADR 0011 |
| Motions name destinations; ranges are half-open `[start, end)`. | ADR 0012 |
| A selection is an optional anchor, not an editor mode. | ADR 0013 |
| The prompt (and any panel/picker) owns the entire keyboard while active. | ADR 0014 |
| The editable overlay always sits cleanly atop an immutable mmap buffer. | ADR 0004 |
| The input path never blocks: background workers + version guards, never a `.join()`. | ADR 0007, 0018 |
| Highlighting is drawn from the active pattern, not from an asynchronous scan result. | ADR 0016 |
| Exactly one horizontal offset governs the entire document viewport. | ADR 0017 |
| Soft word wrapping adds `top_row` offsets without altering document edit semantics. | ADR 0019 |
| Atomic save: format-preserving, identity-guarded, symlink-safe, permission-preserving. | ADR 0009, 0015 |
| Indentation is detected at startup; `.editorconfig` takes precedence over file content. | ADR 0021 |
| The leader key is configurable in `zeno.toml`; user chords shadow built-in bindings. | ADR 0022 |
| End-to-end integration tests drive a real PTY harness via `pyte`, not a mock terminal. | ADR 0023 |
| `zeno -` spools stdin into a temporary file and launches a child process to preserve tty input. | ADR 0020 |
| Enter cycles search matches while keeping prompt open; Esc clears in two stages. | ADR 0024 |

**Design non-goals for v1 (spec §10)** — do NOT casually add these without deliberate architectural approval: multiple tabs / window splits, multi-cursor editing, heavy LSP / diagnostics servers, hex viewer, plugin interpreter runtime, persistent undo across restarts, remote server collaboration.

---

## 2. Build, run, test

```bash
cargo build --release            # Produces target/release/zeno.exe
cargo run --release -- file.txt  # Launch editor on a target file
cargo test                       # 511 tests (295 lib + 216 bin) — all pass
cargo fmt -- --check             # House style check
cargo clippy --all-targets -- -D warnings  # Zero warnings policy
```

### End-to-end PTY tests (`tests/e2e`):
```bash
python -m pip install pyte pywinpty
python -m unittest discover -s tests/e2e -v
```

### Manual terminal driver tools:
```bash
python tools/tui_probe.py file.txt "hello" "\r" "\x1b"   # Inject keystrokes, capture screen
python tools/pty_capture.py target/release/zeno.exe "x"  # Capture raw ANSI escape bytes
```

---

## 3. Repository map

```
src/
  lib.rs               Crate `zeno_cli` — the pure testable library core
    clipboard.rs       Internal clipboard engine + OSC 52 terminal copy integration
    complete.rs        Buffer-aware word completion with zero-allocation token iterator
    config.rs          zeno.toml parser & serializer ([settings], [keys]), in-place update
    doc/
      mod.rs           Doc: mmap + line index + overlay + journal + atomic safe tempfiles
      index.rs         Progressive line indexer; background scanner
      overlay.rs       Line-level edit segments over mmap with O(1) early bounds
      edit.rs          Pos coordinate, Doc::edit primitive, History/undo-redo journal
    finder.rs          Fuzzy file finder with cached lowercase bytes & recent file boost
    git.rs             Git gutter diff tracking, hunk navigation, option-injection safe (--)
    highlight.rs       Syntect syntax worker; viewport checkpoints; thread fallback
    indent.rs          IndentStyle: .editorconfig -> content heuristic -> default
    keys.rs            Chord parsing, physical key layout translation, user keymaps
    motion.rs          Pure cursor destinations (line, word, paragraph, brackets, text objects)
    panel.rs           Help list, command palette, recent files picker, theme picker
    persian.rs         Persian & Arabic unicode shaping (Forms-B) & visual BiDi reordering
    prompt.rs          Prompt line widget: Purpose, text vs choice, caret, status note
    recent.rs          LRU recent file tracking across editor sessions
    render.rs          View widget, gutter, word wrap, bracket matching, zero-alloc spans
    search.rs          Regex pattern, background SearchJob workers, O(1) ASCII column map
    sequence.rs        Leader grammar: state machine feeding chords to Actions
    theme.rs           Built-in curated palettes + VS Code Dark Modern high contrast + .tmTheme

  main.rs              Binary `zeno`: Terminal event loop, RawModeGuard, panic hook, layout
  bin/zeno-bench.rs    Binary `zeno-bench`: Synthetic performance harness (first frame, key latency)
tests/e2e/             PTY test harness driving real pseudo-terminals
docs/                  spec.md, keymap.md, performance.md, and ADRs 0001 through 0024
```

### Key routing pipeline (`Editor::handle_key` in `src/main.rs`) — strict order:
1. `self.prompt.is_some()` → `handle_prompt_key` (active prompt owns all keystrokes).
2. `self.overlay.take()` → `handle_overlay_key` (palette / help / recent files / theme picker).
3. `!self.seq.is_idle()` → `feed_sequence` (leader sequence pending).
4. `self.keys.is_leader(&key)` → initiate leader sequence, display `Leader…`.
5. `self.keys.command_for(&key)` → user custom chord from `zeno.toml` (shadows built-ins).
6. `F1` / `F11` → Palette (`F1`), Zen mode toggle (`F11`).
7. `Alt+Z` → Soft word wrap toggle.
8. `Ctrl+Space` → Word autocompletion cycling (`complete_word`).
9. `Ctrl` chords via physical key normalization (`Ctrl+S`, `Ctrl+F`, `Ctrl+H`, `Ctrl+Z`, `Ctrl+P`, etc.).
10. `Shift` + motion → Start or extend text selection anchor.
11. Printable characters with active selection: auto-surround if delimiter `(`, `[`, `{`, `"`, `'`, etc., else replace selection.
12. Plain typing, navigation keys (`Enter`, `Backspace`, `Delete`, `Tab`, arrows, `Home`/`End`, `PageUp`/`PageDown`, `Esc`).

---

## 4. Feature inventory & contracts

### 4.1. Core Editing & Navigation
- **Instant File Open**: Files are mapped into memory using `memmap2`. Unsaved edits reside in an in-memory `Overlay`. Opening huge files is instant regardless of file size.
- **Surround Selection**: Selecting any text and typing an opening or closing pair character (`(`, `)`, `[`, `]`, `{`, `}`, `"`, `'`, `` ` ``, `<`, `>`) encloses the selection in the corresponding pair in a single atomic undo step, retaining cursor direction.
- **Matching Bracket Highlight**: Viewport scans matching pairs `()`, `[]`, `{}` under the cursor bidirectionally and highlights them in bold cyan (`ThemeColors::DARK_MODERN`). Configurable via `highlight_matching_bracket = true/false` and toggleable in the palette.
- **Word Under Cursor Highlight**: Automatically highlights all visible occurrences of the identifier under the cursor across the viewport. Configurable via `highlight_word_under_cursor`.
- **Persian / Arabic RTL BiDi Support**: Native Unicode contextual shaping (Arabic Presentation Forms-B: initial, medial, final, isolated forms for Persian and Arabic glyphs like گ، چ، پ، ژ، ک، ی) combined with directional visual reordering. Punctuation and markdown symbols remain in forward position while RTL words are displayed correctly in terminal viewports.
- **Word Wrap (`Alt+Z`, `Leader w`)**: Soft visual line wrapping with full cursor up/down navigation across wrapped segments without inserting real newlines.
- **Buffer Word Completion (`Ctrl+Space`)**: Fast buffer-scanning autocompletion extracting word tokens, ranking by distance from cursor, cycling dynamically.

### 4.2. File Management & Git Integration
- **File Finder (`Leader f` / `Ctrl+P`)**: Subsequence fuzzy matching across workspace files with cached lowercase ASCII acceleration and recent file boost ranking.
- **Recent Files (`Leader b` / `Ctrl+Tab`)**: Fast buffer switcher tracking recently opened files with cursor position memory.
- **Git Diff Gutter & Hunk Jumping**: Live gutter indicators for modified (`~`), added (`+`), and deleted (`_`) lines relative to Git index. Navigate hunks with `[c` and `]c`.
- **Zen Mode (`F11`, `Leader Z`)**: Hides the gutter and status bar for a clean, distraction-free editing buffer.
- **Trailing Whitespace Trimming**: Optional auto-trim on save (`trim_trailing_whitespace = true` in `zeno.toml`) and manual command in the palette.

### 4.3. Curated Themes & Modern High-Contrast Palette
- **Default Theme**: VS Code Dark Modern 2026 (`ThemeColors::DARK_MODERN`), engineered with a deep black background (`#000000`) and high-contrast, vibrant syntax highlights for keywords (`#569cd6`), types (`#4ec9b0`), strings (`#ce9178`), functions (`#dcdcaa`), and comments (`#6a9955`).
- **Curated Theme Picker (`Leader T`)**: Switch on-the-fly between VS Code Dark Modern, Monokai, Solarized Dark, Solarized Light, GitHub Dark, and High Contrast. Saves selection automatically to `zeno.toml`.

---

## 5. Security invariants & audit guarantees

Hardened against symlink races, permission drops, and injection vulnerabilities:

1. **Atomic Save with Permission Preservation (CWE-732 & CWE-59)**:
   - When saving an existing file, `Editor::save_to` reads `fs::metadata(&target).permissions()` and applies them directly to the temporary file before atomic renaming.
   - Preserves POSIX file modes (`0644`, `0755`, etc.) across atomic overwrites.
   - If the target file is a symlink, `canonicalize()` resolves the target first, ensuring the link itself is not replaced with a regular file or redirected maliciously.
2. **Predictable Temporary File Sanitization (CWE-377)**:
   - File creation in `/tmp` (e.g. for piped buffers or empty documents) uses atomic `OpenOptions::new().create_new(true)` with high-resolution nanosecond timestamps + PID nonces, completely eliminating symlink pre-creation race conditions.
3. **Safe File-Type Validation**:
   - `Doc::open` validates `metadata.is_file()`. Attempting to open directories, FIFOs, unix domain sockets, or device nodes is safely rejected with an informative error rather than blocking the editor process.
4. **Git Command Argument & Option Injection Guards (CWE-88)**:
   - All Git CLI subprocesses invoke commands with the explicit `--` pathspec delimiter (e.g., `git ls-files -- <path>`, `git diff -U0 -- <path>`), preventing files named `--help` or `-o` from being interpreted as CLI flags.
5. **Git Path Traversal Rejection**:
   - `relative_git_path` strictly rejects paths containing `..` or attempting to escape the repository working tree root.

---

## 6. Resilience & crash-proofing architecture

Engineered to guarantee the terminal is never left in a corrupted or unusable state:

1. **RAII Raw Mode Guard (`RawModeGuard`)**:
   - An explicit RAII guard encapsulates `crossterm::terminal::enable_raw_mode()` and alternate screen activation.
   - When dropped under any condition (normal exit, error return, or panic unwind), it deterministically disables raw mode, leaves the alternate screen, and restores the cursor.
2. **Global Panic Hook (`install_panic_hook`)**:
   - At launch, `install_panic_hook()` sets a custom panic hook.
   - If an unhandled panic occurs anywhere in the process, the hook immediately resets the terminal, shows the cursor, prints human-readable crash diagnostics to `stderr`, and exits cleanly.
3. **Instant Terminal Resize Handling (`Event::Resize`)**:
   - The event loop reacts immediately to terminal window resizing by updating layout geometry and triggering an instant redraw without waiting for an additional keypress.
4. **Broken Pipe Resilience**:
   - Spooled stdin processing (`zeno -`) explicitly handles `io::ErrorKind::BrokenPipe` without panicking when downstream consumers close the pipe early.
5. **System Thread Exhaustion Tolerance**:
   - Spawning background worker threads (syntect highlighting, search regex jobs) uses `thread::Builder::spawn().ok()` fallbacks. If the host OS refuses new threads, zeno continues editing in the foreground without crashing.
6. **Defensive Math Invariants**:
   - Status line percentage calculation uses `saturating_mul` to prevent integer overflow on large line counts.
   - Scrollbar thumb math clamps `total.max(1)` to prevent division-by-zero on empty views.

---

## 7. Performance & zero-allocation hot paths

The keystroke event path must complete in microseconds to meet the <16 ms 60 FPS frame deadline:

1. **ASCII Viewport Rendering Fast Path**:
   - `styled_spans` detects whether a rendered line contains purely ASCII characters. For ASCII text, it slices string slices directly by byte index, avoiding `Vec<char>` allocations and per-span heap strings.
2. **Line Wrap Cursor Search Fast Path**:
   - `wrapped_cursor_position` performs an $O(1)$ bounds check: if the cursor is on a different line than the row being computed, it returns immediately rather than performing expensive wrap calculations.
3. **Overlay Fast Guard**:
   - `Overlay::locate` checks if the requested line index is lower than the first edited segment, returning `Segment::Original` in $O(1)$ time without searching the interval map.
4. **Finder Lowercase Precomputation**:
   - `FileFinder` precomputes lowercase byte representations once during directory discovery, saving ~2,000 heap string allocations on every single typed keystroke. Subsequence checking runs directly against ASCII byte slices.
5. **Completion Token Streaming**:
   - Autocompletion scans buffer lines using a zero-allocation `Words<'a>` iterator. Words are tested against a `HashSet<&str>` before allocating candidate `String` buffers.
6. **Search Column Byte Mapping**:
   - Regex matches on ASCII lines resolve character columns in $O(1)$ by direct byte offset, bypassing UTF-8 char counting.

---

## 8. Conventions & development rules

- **Test first, in the owning file**: Unit tests belong in a `#[cfg(test)] mod tests` at the bottom of each module.
- **Golden tests for rendering**: Use `render::draw(doc, state, highlight) -> Buffer` to test screen outputs without real hardware.
- **Keep e2e tests focused**: PTY tests in `tests/e2e/test_zeno.py` are reserved for behavior that requires a real pseudo-terminal (such as alternate screen switching and ConPTY byte translation).
- **Update documentation in the same commit**: Any change to keybindings or behavior must update `docs/keymap.md`. Any major architectural decision requires a new record in `docs/adr/`.
- **House style for commit messages**: Imperative mood, clauses joined with `and`, no trailing punctuation, no `feat:` or `fix:` prefixes (e.g. `highlight matching bracket pairs in viewport and add toggle command with config option`).
- **Zero latency budget**: Never invoke `.join()` on a background worker thread from the main key event loop. If an asynchronous result is not ready, render the current frame without it.

---

## 9. Next recommended roadmap

Future extensions that fit cleanly within zeno's architecture:

1. **Persistent Session State**: Optionally record last-visited cursor positions across editor sessions in `~/.config/zeno/session.json`.
2. **Expanded File Picker Filters**: Add `.gitignore` awareness to the file finder worker.
3. **Language Server Protocol (LSP) Sidecar**: If diagnostics are ever introduced, implement as a strictly non-blocking external worker over JSON-RPC stdin/stdout without stalling the render thread.

---

## 10. Starter prompt for the next agent

```text
You are continuing development on zeno, a high-performance, non-modal terminal text editor written in Rust.
Start by reading HANDOFF.md, CONTEXT.md, docs/spec.md, and docs/keymap.md.
All non-negotiable architectural rules in HANDOFF.md §1 and ADRs 0001–0024 are binding.

Verify the baseline before making any changes:
1. Run `cargo test` (expect 511 tests passing: 295 lib + 216 bin).
2. Run `cargo fmt -- --check` and `cargo clippy --all-targets -- -D warnings` (must be clean).
3. Check `git status` to ensure you start from a clean tree.

When implementing changes:
- Write unit tests first in the owning module.
- Never block the key handling loop; all background work uses versioned snapshots.
- Follow the imperative commit message convention joined by 'and'.
- Update docs/keymap.md and add an ADR if keyboard behavior or architecture changes.
```
