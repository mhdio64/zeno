# zeno — design specification

## 1. What it is

zeno is a non-modal terminal text editor built to replace nano. It opens a file instantly, gives the user real editing power, saves, and quits. One file at a time. No workspace, no tabs, no sessions.

The product is judged by one number: the time from launching the binary to accepting the first keystroke. Everything below is subordinate to that.

## 2. Product identity

- **Non-modal.** One editing model. The same key means the same thing everywhere. There is no normal/insert/visual mode, and none will be added. (ADR 0002)
- **Quick editor.** One file per invocation. Passing more than one path is an error.
- **Vim's power, VSCode's ergonomics.** Editing power arrives as non-modal commands — motions, text objects, operator+motion composition. Ergonomics arrive as VSCode-compatible chords.
- **No lag, ever.** See §7. (ADR 0007)

## 3. Platform targets

Priority order: Linux, macOS, Windows. All three are first-class, with the same bindings and the same behaviour everywhere.

Supported and tested terminals:

| OS | Terminals |
| --- | --- |
| Linux | GNOME Terminal, kitty |
| macOS | Terminal.app, Ghostty, iTerm2 |
| Windows | Windows Terminal, PowerShell |

No default binding may rely on distinguishing `Ctrl+Shift+<letter>` from `Ctrl+<letter>`; legacy terminal input encodes them identically and crossterm cannot use the Kitty keyboard protocol on Windows. (ADR 0005) The leader key is `Ctrl+Space` by default and is rebindable — on macOS it collides with the system's input-source shortcut, so users with more than one input source must rebind it.

## 4. Features (v1)

### 4.1 Opening and saving

- `zeno <path>` opens a file. `zeno -` reads from stdin and writes to stdout, like `nano -`.
- Files up to at least 1 GB open without degradation. Beyond that the editor degrades to plain text rather than blocking.
- Binary files — NUL bytes, high control-byte density — are detected and refused with a clear message. `--force` opens them as raw text.
- Saving is atomic: a temporary file is written in the target's directory and renamed over the original. Permission bits, encoding, BOM, line endings and final-newline presence are preserved. (ADR 0009)
- If the file changed on disk while open, the editor says so and offers to reload. Autosave is an option.

### 4.2 Screen

- Gutter with line numbers. `Leader ,` toggles them; `Leader .` toggles relative line numbers.
- Status line: file name, modified marker, cursor position, language, encoding.
- Message line for transient messages, prompts and search input.
- No minimap. No mouse.

### 4.3 Syntax highlighting

- syntect's bundled Sublime syntaxes: every language and markup format it ships, baked into the binary. No download, no startup delay. (ADR 0003)
- Highlighting is computed in the background and is cancelable. It never occupies the input path.
- Lines longer than 16 KB are not highlighted — a deliberate performance guard, following `bat`.
- Extra `.sublime-syntax` files in the config directory are picked up automatically.
- Unknown file types fall back to plain text. Detection order: user override, filename or path pattern, extension, first-line shebang, plain text.

### 4.4 Indentation

- Indent style (tabs or spaces) and width are detected from the file's content.
- `.editorconfig` in the file's directory tree takes precedence over detection.
- Language-aware rules: opening a block after `{`, `(` or `:` indents; closing `}` dedents.
- `Tab` inserts according to the detected style; `Shift+Tab` unindents.
- User settings override both.

### 4.5 Wrapping

- Off by default; `Alt+Z` or `Leader w` toggles. Soft wrap at word boundaries.
- With wrap on, the cursor moves by visual row, like VSCode.
- Long lines remain horizontally scrollable.

### 4.6 Search and replace

- `Ctrl+F` finds, `Ctrl+H` replaces. Incremental, with live highlighting in the viewport.
- Rust `regex` syntax. An invalid pattern reports an error rather than failing silently.
- Search runs over the whole file in the background, with progress and cancel, so a 1 GB file never freezes the UI.
- `Leader n` and `Leader N` cycle matches.

### 4.7 Undo

Grouped like VSCode: a run of typing is one step, and so is each command. `Ctrl+Z` and `Ctrl+Y`.

### 4.8 Clipboard

Internal clipboard plus the system clipboard through OSC 52, so copy and paste work over SSH.

### 4.9 Auto-pairing

Brackets and quotes are paired when the surrounding text makes it unambiguous, and are not when it does not. Configurable.

## 5. Commands and bindings

The full default map is in [keymap.md](./keymap.md).

Three layers, in precedence order: plain keys, VSCode-compatible chords, leader sequences. The leader is `Ctrl+Space`; a sequence is `Ctrl+Space` followed by a mnemonic. Sequences are chords, not modes.

Leader sequences are two-part: an operator — `d` delete, `c` change, `y` yank, `v` select, `>` and `<` reindent — followed by a motion or a text object. `Leader d i (` deletes the text between the parentheses. A run of digits sets a repeat count.

## 6. Configuration

- `zeno.toml`, TOML, read once at startup.
- `$XDG_CONFIG_HOME/zeno/zeno.toml`, else `~/.config/zeno/zeno.toml` on Linux and macOS, `%APPDATA%\zeno\zeno.toml` on Windows.
- The directory also holds `syntaxes/` and `themes/`. (ADR 0008)

## 7. Performance budget

| Measurement | Budget |
| --- | --- |
| Editor work to first painted frame, any file size | < 50 ms |
| Keystroke to painted frame | < 16 ms |
| Process launch, before the editor reads anything | < 30 ms |

The first row covers the editor's own work — mapping the file, discovering the lines it needs, painting the frame. It does **not** cover the operating system's per-file first-access check, which on Windows with Defender real-time protection enabled costs 69–391 ms for a first open and 0.3 ms for every open after that. The benchmark reports the two separately, because a single "cold start" number would hide which one it means. (ADR 0007)

Enforced by a benchmark suite in CI, run against generated files of 1 MB, 100 MB and 1 GB, and against adversarial shapes: one enormous line, a file with no trailing newline, CRLF line endings, invalid UTF-8, and BOM-marked UTF-16.

Measured on a 1 GiB file with 39 million lines: first painted frame 0.5 ms, keystroke 0.1 ms. The one input that misses the shape of the budget is a file with enormous lines, where a keystroke costs 6.6 ms — the reason the editable overlay must be a rope or piece table and not a whole-line buffer. Full numbers, and what was not measured, are in `docs/performance.md`.

## 8. Architecture

- **Rust**, on ratatui for rendering, crossterm for terminal I/O, ropey for the editable overlay, syntect for highlighting. (ADR 0001)
- **The file is mapped, not loaded.** The original bytes stay immutable and mapped; edits are an overlay above them. Line offsets are discovered progressively in the background. (ADR 0004)
- **Rendering is damage-based.** Only changed cells are written to the terminal.
- **The input path never blocks.** Highlighting, indexing and search run on background workers and are cancelable; a result that arrives after the document changed is discarded.
- **Unicode.** Byte offsets, code points, grapheme clusters and display columns are distinct units. The cursor moves by grapheme; layout is computed by display width.

## 9. Testing

1. Unit tests on the core: buffer, edit operations, commands, layout, config parsing.
2. Golden snapshot tests of the rendered terminal output.
3. End-to-end tests driving a real PTY.

## 10. Non-goals for v1

Tabs and multiple files, multi-cursor, LSP and diagnostics, hex view, a plugin system, persistent undo across sessions, remote editing, collaborative editing.

## 11. Distribution

A single native binary per platform: GitHub Releases, Homebrew, Scoop/Chocolatey, and crates.io. The crate is published as `zeno-cli`; the binary is `zeno`, because the name `zeno` is already taken on crates.io by an unrelated rasterization library. (ADR 0006)
