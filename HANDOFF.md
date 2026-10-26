# zeno — handoff

Everything the next agent needs to pick this project up without re-deriving it.
Read this file first, then `CONTEXT.md`, `docs/spec.md` and `docs/keymap.md`.
There is no `README.md` and no `AGENTS.md`; this file is the entry point.

---

## 0. State of the working tree — read before touching anything

- Repository root: this directory (on this machine:
  `C:\Users\main\Desktop\freebuff-desktop\zino`). Branch: `master`.
- Last commit: `546e3c7 Colour the viewport in the background, and keep the parser's place`.
  Only five commits exist (`0dea786 initial`, `327cdca`, `2a3dd86`, `5383954`, `546e3c7`).
- **There is one large, uncommitted batch of work on top of that commit** —
  roughly 3,800 added lines across 15 modified files, plus 4 new modules,
  5 new ADRs, the whole `tests/` tree and `.github/`:

  Modified: `.gitignore`, `CONTEXT.md`, `docs/keymap.md`, `src/bin/zeno-bench.rs`,
  `src/clipboard.rs`, `src/doc/edit.rs`, `src/doc/mod.rs`, `src/highlight.rs`,
  `src/lib.rs`, `src/main.rs`, `src/motion.rs`, `src/prompt.rs`, `src/render.rs`,
  `src/search.rs`, `src/sequence.rs`.

  New: `src/config.rs`, `src/indent.rs`, `src/keys.rs`, `src/panel.rs`,
  `tests/e2e/{harness.py,test_zeno.py}`, `.github/workflows/ci.yml`,
  `docs/adr/0019…0023`.

  That batch is green (see §2) but uncommitted. **Commit it as a checkpoint
  before starting new work**, so a new change can be reviewed on its own. Do
  not stage unrelated files and do not rewrite the existing commits.

---

## 1. What zeno is, and the rules it is built on

zeno is a fast, non-modal terminal text editor meant to replace nano: one file
per invocation, instant open, real editing power, save, quit. The product is
judged by the time from launching the binary to the first accepted keystroke
(spec §1, §7).

The vocabulary is defined in `CONTEXT.md` and must be used consistently in code,
docs and conversation: **quick editor, non-modal, command, operator, motion,
text object, binding, leader key, instant open, prompt, viewport, non-goal**.
Do not write "mode", "workspace", "dialog", "shortcut" or "window" for those
concepts.

Non-negotiables, each with a decision record:

| Rule | Where |
| --- | --- |
| No modes. One keymap, always. No normal/insert/visual. | ADR 0002 |
| Never rely on `Ctrl+Shift+<letter>` (indistinguishable on Windows). | ADR 0005 |
| A chord is matched on the **physical key**, not the produced character. | ADR 0010 |
| Every edit goes through one primitive, `Doc::edit`, and is journaled. | ADR 0011 |
| Motions name destinations; ranges are half-open. | ADR 0012 |
| A selection is an optional anchor, not a mode. | ADR 0013 |
| The prompt (and a panel) owns the whole keyboard while open. | ADR 0014 |
| The editable overlay always sits above an immutable mmap. | ADR 0004 |
| The input path never blocks: workers + version guards, never a join. | ADR 0007, 0018 |
| Highlighting is drawn from the pattern, not from the scan's result. | ADR 0016 |
| One horizontal offset for the whole document. | ADR 0017 |
| Wrapping adds `top_row` and changes no edit semantics. | ADR 0019 |
| Save is atomic, format-preserving, and identity-guarded. | ADR 0009, 0015 |
| Indentation is resolved once at startup; `.editorconfig` wins. | ADR 0021 |
| The leader is a chord from `zeno.toml`; user bindings shadow built-ins. | ADR 0022 |
| The e2e tests drive a real pty, not a fake terminal. | ADR 0023 |
| `zeno -` runs a second process because a pipe cannot carry keys. | ADR 0020 |
| Enter cycles find matches; Esc leaves search in two stages. | ADR 0024 |

Design non-goals for v1 (spec §10) — do not quietly add them: tabs/multiple
files, multi-cursor, LSP/diagnostics, hex view, plugins, persistent undo,
remote editing, collaboration.

---

## 2. Build, run, test

Windows here, but the commands are the same everywhere (CI runs Linux, macOS and
Windows; see `.github/workflows/ci.yml`).

```bash
cargo build --release            # target/release/zeno.exe  (the e2e tests need this)
cargo run --release -- file.txt  # edit a file
cargo test                       # 205 library tests + 137 editor tests, all green
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```

End-to-end tests (spawn a real pty, render the screen through `pyte`):

```bash
python -m pip install pyte            # plus pywinpty on Windows
python -m unittest discover -s tests/e2e -v
```

Current status on this machine: **13 e2e tests, `OK (skipped=4)`** — the four
that need `Alt` or the NUL byte (`Ctrl+Space`) are skipped on Windows because
ConPTY's input parser cannot carry them through a byte pipe (ADR 0023).

Manual probing, which is how UI bugs actually get diagnosed here:

```bash
python tools/tui_probe.py file.txt "hello" "\r" "\x1b"   # keys in, screen text out
python tools/pty_capture.py target/release/zeno.exe "x"  # raw bytes the editor wrote
```

`TUI_PROBE_DELAY` / `PTY_DELAY` tune the wait per keystroke. Both tools need a
release build and `pywinpty`.

---

## 3. Repo map

```
src/lib.rs            crate `zeno_cli` — the testable core
  doc/mod.rs          Doc: mmap + line index + overlay + journal + version + identity
  doc/index.rs        progressive line index; background scanner
  doc/overlay.rs      line-level edit segments over the mapped file
  doc/edit.rs         Pos, the one edit primitive, History/journal
  render.rs           View widget + ViewState, gutter, wrapping, match_ranges
  motion.rs           pure cursor destinations (move targets, text objects)
  search.rs           Pattern (regex), SearchJob (thread), Scope, Wrap, events
  highlight.rs        syntect worker; viewport colours, checkpoints
  prompt.rs           the prompt line: Purpose, text vs choice, caret, note
  panel.rs            help list + command palette, one shared table
  sequence.rs         leader grammar: steps in, Action out
  keys.rs             Chord parsing, default keymap, config-supplied bindings
  config.rs           zeno.toml parser ([settings], [keys])
  indent.rs           IndentStyle: .editorconfig → content → default
  clipboard.rs        internal + OSC 52 write path

src/main.rs           binary `zeno`: the Editor, event loop, rendering, prompts
src/bin/zeno-bench.rs binary `zeno-bench`: performance harness
tests/e2e/            pty harness + 13 screen-level tests
tools/                tui_probe.py, pty_capture.py (manual drivers)
docs/spec.md          the product spec — the source of truth for scope
docs/keymap.md        the user-facing keymap and behaviour contract
docs/performance.md   measured numbers, findings 1–11, reproduction steps
docs/adr/0001…0024    one decision record per irreversible choice
```

`src/main.rs` (4,520 lines) holds the editor because the editor is one state
machine: it is the only place that knows about the terminal. Everything that can
be tested without a terminal lives in the library.

### The `Editor` struct (`src/main.rs`)

`doc`, `path`, `view: ViewState`, `dirty`, `message`, `quit`,
`viewport_height_hint`, `seq: Sequences`, `clipboard: String`, `osc: Option<String>`,
`prompt: Option<Prompt>`, `search: Option<Search>`, `highlight: Highlighter`,
`marks: HashMap<char, Pos>`, `pending_replace: Option<Pattern>`, `auto_pair`,
`overlay: Option<Overlay>` (`Help { offset }` or `Palette`), `pipe_out`, `indent`,
`autosave`, `last_key: Instant`, `keys: Keymap`.

### Key routing order (`Editor::handle_key`) — this order is load-bearing

1. `prompt.is_some()` → `handle_prompt_key` (every key is an answer).
2. `overlay.take()` → `handle_overlay_key` (help/palette).
3. `!seq.is_idle()` → `feed_sequence` (a pending leader owns the next key).
4. `keys.is_leader(&key)` → start a sequence, message `Leader…`.
5. `keys.command_for(&key)` → a **config-supplied** chord (shadows built-ins).
6. `F1` → palette.
7. `Alt+Z` → word wrap (matched on the key, not the character).
8. `Ctrl` chords via `chord_letter` (`Ctrl+/` special-cased: console `/` vs VT
   control byte) — `a c x s v d f h g w k e l z y q`.
9. `Shift` + movement → extend the selection.
10. Plain keys: printing, `Enter`, `Backspace`, `Delete`, `Tab`, arrows,
    `Home`/`End`, `PageUp`/`PageDown`, `Esc`.

`Esc` in ordinary editing: clear the message and the selection, and if a search
is live, clear it (`clear_search`). That is the second half of the two-press Esc
described in §7.

### Frame (`Editor::draw`)

`poll_search()` → `terminal.size()` → layout (body + one status row) → measure
the gutter and set `view.width` → `ensure_cursor_visible()` →
`poll_highlight(body.height)` → paint panel *or* document → then either the
prompt line (with its own caret) or the status line (with the document caret).
The status line shows name, `[+]`, `Ln`/`Col`, syntax name once known, line
count (`indexing…` while scanning), the live `Find:` pattern with its match
count, then the message.

### Data model

- The file is memory-mapped and never mutated. `Doc` holds `Arc<Mmap>`,
  `Arc<LineIndex>`, an `Overlay` of `LineEdit` segments, a `History`, and a
  `version: u64` bumped on every edit.
- `LineIndex` discovers line starts progressively; a background thread scans
  ahead; `index_fully()` is the one call that costs a full scan and is only made
  when it is unavoidable (replace-all with no selection, writing a piped buffer).
- `Doc::snapshot()` is what a worker is given, so a worker never touches a live
  `Doc`.
- Every worker result is stamped with the `version` it was computed against
  (`search.count_version`, `search.job_version`) and is discarded if the
  document moved. Reproduce the reasoning from ADR 0018 before changing it.

### Search (`search.rs` + `Editor`)

- `Pattern::compile(&str)` → `Result<Pattern, String>`; the error string is what
  the prompt's `note` shows.
- Two background scans, distinct jobs: **count** (`Scope::All`, feeds
  "N matches") and **jump** (`Scope::From/After/Before` + `Wrap`, moves the
  cursor). The **viewport scan** is foreground and cheap: `View::match_ranges`
  colours the visible lines from the compiled pattern alone (ADR 0016).
- `poll_search()` is called once per frame from `draw`: it drains the jobs,
  restarts a stale count, applies a jump (`move_to` + `clamp_column` +
  `ensure_cursor_visible`), and `take`s/restores `self.search` around all of it.
- `refresh_search()` is called for every keystroke in a `Search`/`Replace`
  prompt: compile, set the note on error, keep the last good pattern on error,
  clear on empty, otherwise `set_pattern` + `start_count`.
- `next_match` / `previous_match` (`Leader n` / `Leader N`) open the prompt when
  there is no search; otherwise they start a wrap-aware scan.

---

## 4. Feature status against the spec

Implemented and tested:

- Open a file instantly; atomic save preserving permissions, line endings, BOM,
  final-newline presence; identity guard when the file changed on disk
  (reload / overwrite / cancel, ADR 0015).
- Binary detection with `--force`; `zeno -` (spool + child process, ADR 0020);
  `--no-background-index`, `--bench-open`, `--help`, `--version`.
- Gutter, absolute and relative numbers (`Leader ,` / `Leader .`), status line.
- syntect highlighting in a worker, 16 KB line guard, user `syntaxes/`.
- Indentation detection, `.editorconfig`, bracket/colon indent on `Enter`,
  closer dedent, `Tab` / `Shift+Tab`, `Leader <` / `Leader >` (ADR 0021).
- Word wrap (`Alt+Z`, `Leader w`) with visual-row movement and `top_row`.
- Horizontal scrolling with one offset (ADR 0017).
- Find: live highlight while typing, match count, invalid-pattern note, `Enter`
  jump, `Leader n`/`Leader N` cycling with wrap, Esc leaving the highlight on
  screen (see §7 for the change in flight).
- Replace: pattern then replacement, selection or whole file, one undo step,
  affordability guard.
- Undo/redo grouped by command; cut/copy/paste with OSC 52 (write-only);
  auto-pairing; marks (`Leader m <mark>`, `Leader ' <mark>`); comment toggle;
  duplicate line; select line; delete to end of line / word back.
- Help list (`Leader ?`) and command palette (`F1`, `Leader p`) from one table.
- `zeno.toml` `[settings]` and `[keys]` (leader + extra chords, names normalized
  so `go to line` = `go-to-line` = `go_to_line`); autosave after 600 ms of quiet.
- Leader grammar: operators `d c y v > <`, motions, text objects, counts,
  `MAX_SPAN_LINES = 10_000` guard (`src/main.rs`) with an explanatory message.

Known gaps and inconsistencies (deliberately listed, not hidden):

1. ~~**Config is read once at startup**~~ (Fixed: spec §6 updated to "read once at startup", agreeing with keymap.md and ADR 0022).
2. ~~**`docs/keymap.md` contradicts itself** in one paragraph~~ (Fixed: both `[settings]` and `[keys]` are noted as read).
3. ~~**Mouse capture is enabled** (`EnableMouseCapture`)~~ (Fixed: capture removed in accordance with spec §4.2 "no mouse", restoring native terminal selection).
4. ~~**No encoding in the status line**~~ (Fixed: UTF-8 encoding label added to status line per spec §4.2).
5. **Themes are not implemented** while `keymap.md` calls `themes/` "reserved
   for later"; the theme is fixed.
6. ~~**The e2e find test is weaker than it looks**~~ (Fixed: tightened to assert gutter line 150 scrolled into view and prompt lifecycle).
7. `docs/performance.md` numbers come from an earlier machine state; regenerate
   with `cargo run --release --bin zeno-bench` before quoting them.

---

## 5. The work in flight, stated precisely

The user's last request (Persian, paraphrased): *"While I'm in find, pressing
Enter should keep cycling through the results and I should stay in find; find
should not close until I press Esc. Like VSCode."*

Current behaviour in the working tree, as implemented:

- `Ctrl+F` opens the prompt pre-filled with the last pattern.
- Typing recompiles the pattern live (`refresh_search`), so matches highlight and
  the count appears, and an invalid pattern sets a note without losing the last
  good highlight.
- **`Enter` submits and closes the prompt** (`handle_prompt_key` returns
  `Answer::Text`, and the caller does `self.prompt.take()`), then starts a jump
  scan with `Scope::From { pos: cursor }` + `Wrap::Start`.
- `Esc` inside a `Search` prompt sets `escape_leaves_search`: the prompt closes,
  the highlight stays (`prompt = None`, message cleared) — the VSCode-like first
  Esc.
- `Esc` in ordinary editing clears message + selection, and if `search.is_some()`
  calls `clear_search()` — the second Esc, back to fully plain.

What must change:

1. `Enter` in a **Search** prompt moves to the next match and **leaves the
   prompt open** (repeatable: Enter, Enter, Enter walks the matches, wrapping at
   the end). It must *not* go through the `Answer`/`prompt.take()` path.
2. The **first** Enter after the pattern changed starts at or after the caret
   (`Scope::From`) — otherwise the first Enter on a match already under the
   cursor does nothing. Subsequent Enters should advance past the current match
   (`Scope::After` with the caret at the match start, or track the end of the
   last match). Pick one rule, write it in the ADR, and assert both cases.
3. `Esc` keeps the two-press behaviour already implemented: first press closes
   the prompt and keeps the highlight; second press (ordinary editing) clears it.
4. `Replace` keeps its current behaviour (Esc cancels; the highlight of the
   pattern survives) unless the user asks otherwise.

Files in play: `src/main.rs` (`handle_prompt_key`, the `Purpose::Search` arm of
`answer`, `open_find`, `next_match`), `docs/keymap.md` §"The prompt line",
possibly a new ADR 0024 ("Enter cycles the matches and Esc ends the search").
Tests to update or add in `src/main.rs` (`#[cfg(test)] mod tests`):
`enter_jumps_to_the_first_match`, `leader_n_walks_to_the_next_match_and_wraps_around`,
plus a new one asserting the prompt is still `Some` after `Enter` and that two
Enters land on the second match. `settle(&mut e)` drains the search jobs; use it
after every Enter. Also tighten the e2e find test (§4, gap 6).

Watch out for: `poll_search` moves the cursor while the prompt is open — that is
fine and is what scrolls the match into view, because the terminal caret is
parked on the prompt line, not in the document. Do not add a second "is the
prompt open" special case to the view state; the prompt already owns the
keyboard.

---

## 6. How work is done here (conventions that are checked by CI or review)

- **Test first, in the owning file.** Unit tests live in a `#[cfg(test)] mod
  tests` at the bottom of the module. `src/main.rs` has helpers:
  `editor_with(name, bytes) -> (Editor, PathBuf)`, `key(KeyCode)`,
  `chord(KeyCode)` (control modifier), `sequence(&mut Editor, "d w")`,
  `type_into_prompt`, `prompt_text`, `line(&Editor, n)`, `settle`.
  `render.rs` has `draw(doc, state, highlight) -> Buffer` for golden-style screen
  assertions. `prompt.rs`, `search.rs`, `motion.rs`, `sequence.rs`, `indent.rs`,
  `keys.rs`, `config.rs` are pure and tested without a terminal.
- **Screen-level behaviour gets an e2e test** in `tests/e2e/test_zeno.py` only
  when a unit test cannot see it (alternate screen, pty encoding, `zeno -`).
  Keep them few and blunt; they are the slow, timing-sensitive layer.
- **A change to behaviour updates `docs/keymap.md` in the same change.** It is
  the user-facing contract, not commentary.
- **A decision gets an ADR** in `docs/adr/NNNN-slug.md`: numbered in order, the
  title a sentence ("The prompt owns the keyboard"), body in this shape —
  the tension, "The decision is…", "Two alternatives were rejected…", "The costs
  are…". Match the voice of 0019–0023: plain English, no bullet salad, no
  marketing.
- **Comments explain why, once.** The codebase's house style is prose above a
  function explaining the constraint it answers, and no comment where the code
  already says it.
- **Commit messages** are imperative sentences, often two clauses joined by
  `and` / a colon: `Give the viewport a horizontal axis, so a long line can be
  read`, `Make the search reachable: find, cycle, highlight`. No
  `feat:`/`fix:` prefixes.
- **Performance work is measured, not argued.** `docs/performance.md` records
  numbers with the shape of the machine; `zeno-bench` generates the data files
  under `.bench-data/` (git-ignored). The budget is spec §7: first frame < 50 ms,
  keystroke < 16 ms, launch < 30 ms.
- Never make the input path wait: no `.join()` on a worker in the key handler,
  no full-file scan on a keystroke. If an answer is not ready, paint without it.

---

## 7. First thirty minutes

1. `git status` and `git diff --stat` — confirm the uncommitted batch described
   in §0 and commit it as a checkpoint.
2. `cargo test` (expect 342 passing) and `cargo build --release`, then
   `python -m unittest discover -s tests/e2e -v` (expect `OK (skipped=4)` here).
3. Read `CONTEXT.md`, `docs/spec.md` §1–§7, `docs/keymap.md` (all of it), and
   ADR 0013, 0014, 0016, 0022, 0023.
4. Reproduce the current find behaviour by hand:
   `python tools/tui_probe.py some.txt "\x06" "pattern" "\r" "\x1b" "\x1b"` and
   watch the prompt line, the highlight, and the status-bar match count.
5. Only then start §5. Write the failing test first.

---

## 8. Starter prompt for the next agent

Paste this, together with this file, at the start of the new session:

> You are continuing work on **zeno**, a non-modal terminal text editor written
> in Rust. Start by reading `HANDOFF.md` in the project root, then `CONTEXT.md`,
> `docs/spec.md`, `docs/keymap.md`, and the ADRs it points at. Treat `HANDOFF.md`
> §1 (non-negotiables) and §6 (conventions) as binding.
>
> First, confirm the baseline: `cargo test`, `cargo build --release`, and
> `python -m unittest discover -s tests/e2e -v`. Report the counts you see
> before changing anything. The working tree already contains one large
> uncommitted batch of work — commit it as a checkpoint first, without
> rewriting existing commits, and keep your own change separate.
>
> Then do the task in `HANDOFF.md` §5: while the Find prompt is open, `Enter`
> must move to the next match and keep the prompt open, cycling through the
> results and wrapping at the end of the file; the first `Esc` closes the prompt
> but keeps the matches highlighted; a second `Esc` clears the highlight and
> returns to fully ordinary editing. This is the VSCode behaviour the user
> asked for. Write the failing test first, keep the change in `src/main.rs`,
> update `docs/keymap.md`, and add an ADR for the Enter/Esc rule.
>
> Report what you verified (unit tests, e2e tests, and the behaviour typed into
> a real screen), and do not claim a check you could not run.
