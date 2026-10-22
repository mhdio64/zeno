# Default keymap

Three layers, in order of precedence:

1. **Plain keys** — every printable character inserts itself. `Enter`, `Backspace`, `Delete`, `Tab`, arrows, `Home`/`End`, `PageUp`/`PageDown`, `Esc`.
2. **Chords** — a modifier plus one key, matching VSCode muscle memory wherever VSCode's chord is unambiguous.
3. **Leader sequences** — `Ctrl+Space`, then a mnemonic. This is where the editing power lives.

There are no modes. A leader sequence is a chord, not a mode: the editor returns to ordinary editing the moment the sequence completes or is abandoned with `Esc`.

## The leader key

`Ctrl+Space` by default, rebindable in `zeno.toml`.

Two caveats worth knowing before rebinding:

- On macOS, `Ctrl+Space` is a system shortcut for switching input source. It is intercepted before the terminal sees it when more than one input source is configured, so macOS users in that situation need a different leader key.
- Legacy terminal input encodes `Ctrl+Space` as `NUL`, the same byte as `Ctrl+@`.

## Layer 2 — chords

| Keys | Command |
| --- | --- |
| `Ctrl+S` | Save |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` | Redo |
| `Ctrl+X` `Ctrl+C` `Ctrl+V` | Cut / Copy / Paste |
| `Ctrl+A` | Select all |
| `Ctrl+F` | Find |
| `Ctrl+H` | Replace |
| `Ctrl+G` | Go to line |
| `Ctrl+K` | Delete to end of line |
| `Ctrl+W` | Delete word before cursor |
| `Ctrl+E` | End of line |
| `Ctrl+L` | Select line (expands on repeat) |
| `Ctrl+/` | Toggle line comment |
| `Ctrl+D` | Duplicate line |
| `Ctrl+←` `Ctrl+→` | Move by word |
| `Ctrl+↑` `Ctrl+↓` | Scroll viewport one line |
| `Ctrl+Home` `Ctrl+End` | Start / end of file |
| `Alt+←` `Alt+→` | Back / forward through jump history |
| `Alt+↑` `Alt+↓` | Move line up / down |
| `Alt+Shift+↑` `Alt+Shift+↓` | Copy line up / down |
| `Alt+Z` | Toggle word wrap |
| `Shift+arrows` | Extend selection |
| `Shift+Home` `Shift+End` | Extend to line start / end |
| `F1` | Command palette |

`Ctrl+Shift+<letter>` is deliberately absent: legacy terminal input encodes it identically to `Ctrl+<letter>`, and crossterm cannot use the Kitty keyboard protocol on Windows to tell them apart. Every command reachable only through `Ctrl+Shift+<letter>` in VSCode has a two-key route here.

A chord is matched by the **key**, not by the character the layout produces for it. On Windows the console translates `Ctrl+S` through the active keyboard layout, so on a Persian or Russian layout it arrives as `'س'` or `'ы'`; matching on the character would leave every binding in this table dead while plain typing kept working. See [ADR 0010](adr/0010-chords-resolve-through-the-physical-key.md).

## Selection

`Shift` plus any movement extends a selection; any movement without it drops the
selection. `Ctrl+A` selects the whole file, `Esc` drops the selection, and typing,
`Backspace` or `Delete` applies to the selected range — one edit, so one `Ctrl+Z`
reverses it. `Leader v` with a motion selects that range without editing anything.

The range is half-open, like an operator's: it runs from the anchor up to the
cursor and stops *before* the character under it, so `Shift+Down` never
highlights the first character of the line below.

`Ctrl+C`, `Ctrl+X` and `Leader y` fill the clipboard and send it to the system
clipboard through OSC 52, which works over SSH. OSC 52 has no read direction, so
`Ctrl+V` pastes what this editor copied, not what was copied in another program.
Terminals cap the OSC 52 payload (tmux: 74,994 bytes); a bigger copy stays
internal and says so.

A command that would edit or copy more than 10,000 lines is refused with an
explanation, because a range that wide is quadratic in the current line model
(ADR 0012, [performance.md](./performance.md)). Selecting it is fine; editing it
is not, until the model changes.

## Long lines

A line longer than the screen is scrolled, not cut. The viewport keeps one
horizontal offset for the whole document, the way it keeps one line offset
vertically, and it scrolls by the smallest amount that keeps the cursor on
screen. The cursor is therefore always at its own column and the text under
it is always readable, even on a line ten screens wide (ADR 0017).

A line shorter than the offset is blank to the left of the window: its text is
to the left of what is showing. `Home` brings the viewport back to the first
column. The status line counts columns from the start of the line, not from
the edge of the screen.

## The prompt line

Commands that need an argument ask for it in the message line, so the document
never moves while a question is open:

```
 Go to line: 500
```

The prompt owns the keyboard while it is open. Printable keys are text,
`Backspace`/`Delete`/arrows edit the answer, `Enter` answers, `Esc` abandons. No
chord fires and no leader sequence starts: `Ctrl+S` mid-answer does not save.

| Keys | Command |
| --- | --- |
| `Ctrl+G` `Leader g` | Go to line (asks for the number) |
| `Ctrl+F` `Leader f` | Find (asks for the pattern) |
| `Ctrl+S` | Save; if the file changed on disk, asks `[r]eload [o]overwrite [c]cancel` |

A line number past the end jumps to the last line. `Ctrl+W`, `Ctrl+K` and
`Ctrl+E` need no prompt — they act on the range the caret is already in.

There are two kinds of question: one collects text (the line number, the
search pattern), the other collects one letter from a fixed set (the save
decision). A choice has no caret and is answered with the letter itself; `Esc`
abandons either kind.

Find asks for a pattern in the same prompt. The pattern is compiled as it is
typed, so the matches in the viewport are highlighted while it is being
written, and the prompt line says how many matches there are once the
background scan has counted them. A pattern that does not compile says so on
the prompt line and keeps the last pattern that did. `Enter` jumps to the
first match at or after the caret; `Leader n` and `Leader N` cycle through
the matches and wrap around at the ends of the file.

Reloading after an external change discards the buffer's unsaved edits *and* its
undo history, because that history describes the document that was replaced. The
message says so (ADR 0015).

## Layer 3 — leader sequences

### Standalone

| Sequence | Command |
| --- | --- |
| `Leader s` | Save |
| `Leader q` | Quit (prompts if there are unsaved changes) |
| `Leader Q` | Quit, discarding changes |
| `Leader f` | Find in file |
| `Leader r` | Replace in file |
| `Leader n` `Leader N` | Next / previous search match |
| `Leader g` | Go to line |
| `Leader u` `Leader U` | Undo / Redo |
| `Leader ,` | Toggle line numbers |
| `Leader .` | Toggle relative line numbers |
| `Leader w` | Toggle word wrap |
| `Leader ?` | Help overlay (all commands and their bindings) |
| `Leader p` | Command palette (alias of `F1`) |
| `Leader [` `Leader ]` | Unindent / indent line |
| `Leader m` `<mark>` | Set mark |
| `Leader '` `<mark>` | Jump to mark |

### Two-part: operator then motion

`Leader` `<operator>` `<motion>`

| Operator | Effect |
| --- | --- |
| `d` | Delete the motion's range |
| `c` | Change it: delete, then insert |
| `y` | Yank (copy) it |
| `v` | Select it |
| `>` `<` | Indent / unindent it |

Repeating the operator applies it to the whole line: `Leader d d` deletes the
line the cursor is on, terminator included, and `Leader y y` yanks it. A count
in front of it covers that many lines: `Leader 3 d d` deletes three.

A motion names a *boundary*, so an operator's range is always half-open:
`Leader d $` deletes to the end of the line and `Leader d e` deletes the word
up to and including its last character. There is no per-motion "inclusive"
case.

The motions listed below are the design. Until the editable overlay stops
paying a superlinear price for a large range (ADR 0012, and "Finding 8" in
[performance.md](./performance.md)), the file-wide ones — `gg`, `G`, `}`,
`%` — are not accepted as an operator's motion: the sequence cancels rather
than issuing a delete that would take seconds.

| Motion | Target |
| --- | --- |
| `h` `l` | One character left / right |
| `j` `k` | One line down / up |
| `w` `W` | Next word / next WORD |
| `b` `B` | Previous word / previous WORD |
| `e` `E` | End of word / end of WORD |
| `$` | End of line |
| `0` | First column of the line |
| `^` | First non-blank character of the line |
| `{` `}` | Previous / next paragraph |
| `(` `)` | Previous / next sentence |
| `gg` | First line of the file |
| `G` | Last line of the file |
| `%` | Matching bracket |

### Two-part: text object

`Leader` `i` `<object>` selects inside the object; `Leader` `a` `<object>` selects around it. Both can follow an operator: `Leader d i (` deletes the text between the parentheses.

| Object | Target |
| --- | --- |
| `w` `W` | A word / a WORD |
| `l` | A line |
| `p` | A paragraph |
| `s` | A sentence |
| `(` `)` | Inside parentheses |
| `[` `]` | Inside square brackets |
| `{` `}` | Inside braces |
| `"` `'` | Inside double / single quotes |
| `` ` `` | Inside backticks |
| `t` | A tag (HTML/XML) |

### Counts

A run of digits before the operator sets a repeat count: `Leader 3 d w` deletes three words.

## Configuration

`zeno.toml`, reloaded when it changes on disk.

| Platform | Path |
| --- | --- |
| Linux, macOS | `$XDG_CONFIG_HOME/zeno/zeno.toml`, else `~/.config/zeno/zeno.toml` |
| Windows | `%APPDATA%\zeno\zeno.toml` |

The same directory holds `syntaxes/` and `themes/`. Any `.sublime-syntax` file in `syntaxes/` is registered at startup; any theme file in `themes/` becomes selectable.

```toml
[keys]
leader = "ctrl+space"

[[keys.bindings]]
keys = "ctrl+s"
command = "save"

[[keys.bindings]]
keys = "leader d w"
command = "delete_word"

[settings]
word_wrap = false
line_numbers = true
relative_line_numbers = false
indent_style = "auto"
tab_width = 4
auto_pair = true
autosave = false
theme = "zeno-dark"
```
