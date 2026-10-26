# User-Friendly Shortcuts and Hints Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the zeno editor intuitive and welcoming for users accustomed to modern editors (VSCode, micro, nano) by adding a direct `Ctrl+P` chord, displaying persistent key hints on the status bar, providing a rich cheat-sheet message when the leader key is pressed, and modernizing terminology in panels and messages.

**Architecture:** 
1. Map `Ctrl+P` in `Editor::handle_key` to `open_palette()` directly alongside `F1`.
2. When the editor is idle (`self.message.is_empty()`), render subtle key hints (`F1: Menu  ^S: Save  ^Q: Exit  ^F: Find`) in the status line.
3. When the leader key is triggered, set `self.message` to an informative cheatsheet (`Actions: [s] Save  [q] Quit  [f] Find  [p] Palette  [?] Help  [w] Wrap`) rather than bare `Leader…`.
4. Update `src/panel.rs` and `docs/keymap.md` to document `Ctrl+P` and clarify Vim-centric terms like `yank` -> `copy (yank)`.

**Tech Stack:** Rust, Ratatui, Crossterm.

**Spec:** `docs/spec.md`, `docs/keymap.md`, ADR 0002, ADR 0022.

## Global Constraints
- Preserve non-modal editing model (ADR 0002).
- Chords resolve through the physical key (ADR 0010).
- Maintain 100% test coverage and house commit style (imperative sentence, clauses joined with `and`).
- All existing 357 unit tests and 13 e2e tests must continue to pass.

## Review Focus
1. `Ctrl+P` must work seamlessly in any document without interfering with text insertion.
2. Status bar key hints must only appear when `self.message.is_empty()` and must not overflow or displace search counts or coordinates.
3. The leader cheatsheet message must clear cleanly when a command completes or cancels with Escape.
4. Dirty exit warning message must refer to standard `Ctrl+S` instead of Vim-like `Leader s`.
5. Keymap documentation in `docs/keymap.md` must stay synchronized with actual bindings.

---

### Task 1: Add `Ctrl+P` Built-In Chord for Command Palette

**Files:**
- Modify: `src/main.rs:757-825`
- Modify: `src/panel.rs:109-112`
- Modify: `docs/keymap.md:48`
- Test: `src/main.rs` (in `mod tests`)

- [ ] **Step 1.1**: Write a failing test in `src/main.rs` verifying that pressing `Ctrl+P` opens the command palette.
- [ ] **Step 1.2**: Run `cargo test ctrl_p` to confirm failure.
- [ ] **Step 1.3**: Add `Some('p') => { self.open_palette(); return; }` to `chord_letter` handling in `Editor::handle_key` in `src/main.rs`.
- [ ] **Step 1.4**: Update `keys` in `src/panel.rs` for `Command::Palette` to `"ctrl+p / f1 / leader p"`.
- [ ] **Step 1.5**: Update `docs/keymap.md` table to list `Ctrl+P` / `F1`.
- [ ] **Step 1.6**: Run `cargo test` and verify tests pass.
- [ ] **Step 1.7**: Commit changes with message `Support Ctrl+P for command palette, and document the binding`.

---

### Task 2: Friendly Leader Cheatsheet and Plain Exit Message

**Files:**
- Modify: `src/main.rs:721-725`, `src/main.rs:1275`
- Test: `src/main.rs` (in `mod tests`)

- [ ] **Step 2.1**: Write unit tests verifying that pressing the leader key sets a friendly actions cheatsheet and dirty exit message uses `Ctrl+S`.
- [ ] **Step 2.2**: Run `cargo test leader` to observe failure.
- [ ] **Step 2.3**: In `src/main.rs`, update `self.message` on leader press to `"Actions: [s] Save  [q] Quit  [f] Find  [p] Palette  [?] Help  [w] Wrap"`.
- [ ] **Step 2.4**: In `src/main.rs`, update `Command::Quit` dirty message from `"unsaved changes — Leader s saves, Leader Q discards"` to `"unsaved changes — Ctrl+S saves, Leader Q discards"`.
- [ ] **Step 2.5**: Run `cargo test` and verify all tests pass.
- [ ] **Step 2.6**: Commit changes with message `Show an action cheatsheet on leader key, and clarify unsaved changes message`.

---

### Task 3: Status Bar Shortcut Hints

**Files:**
- Modify: `src/main.rs:2836-2843`
- Test: `src/main.rs` (in `mod tests`)

- [ ] **Step 3.1**: Write a unit test asserting that `status_line()` includes `"F1: Menu  ^S: Save  ^Q: Exit  ^F: Find"` when `self.message.is_empty()`.
- [ ] **Step 3.2**: Run `cargo test status_line` to observe failure.
- [ ] **Step 3.3**: In `Editor::status_line()`, when `self.message.is_empty()`, push a styled DarkGray hint span `"F1: Menu  ^S: Save  ^Q: Exit  ^F: Find"`.
- [ ] **Step 3.4**: Run `cargo test` and verify all tests pass.
- [ ] **Step 3.5**: Commit changes with message `Display shortcut hints in status bar when idle`.

---

### Task 4: Clarify Terminology in Help Panel

**Files:**
- Modify: `src/panel.rs:117-129`
- Test: `src/panel.rs` (in `mod tests`)

- [ ] **Step 4.1**: Update `REFERENCE` in `src/panel.rs` to clarify `"y copy (yank)"`.
- [ ] **Step 4.2**: Run `cargo test panel` to verify.
- [ ] **Step 4.3**: Commit changes with message `Clarify copy in help panel reference`.

---

### Task 5: Full Verification and E2E Testing

**Files:**
- Verification only

- [ ] **Step 5.1**: Run `cargo test` (all unit and integration tests).
- [ ] **Step 5.2**: Run `cargo fmt -- --check`.
- [ ] **Step 5.3**: Run `cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5.4**: Run `cargo build --release`.
- [ ] **Step 5.5**: Run `python -m unittest discover -s tests/e2e -v`.
