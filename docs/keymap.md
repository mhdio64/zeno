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
| `Shift+Tab` | Unindent line |
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

With wrapping on (`Alt+Z`, `Leader w`) the window is the line's width instead,
and the horizontal offset is ignored. A wrapped line carries its number only
on its first screen row; the rows under it are the same line and their gutter
is blank. `Up` and `Down` move one *screen row*, so a long line is walked row
by row, and a row that ends early clamps the way a short line does.

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

Replace (`Ctrl+H`, `Leader r`) asks twice: first for the pattern, then for what
to put in its place. It replaces every match in the selection, and every match
in the file when there is no selection; a file too big to edit as one range is
refused with the same explanation a wide delete gives. The pattern is
highlighted while it is typed, and the whole replacement is one edit, so one
`Ctrl+Z` reverses it.

Find asks for a pattern in the same prompt. The pattern is compiled as it is
typed, so the matches in the viewport are highlighted while it is being
written, and the prompt line says how many matches there are once the
background scan has counted them. A pattern that does not compile says so on
the prompt line and keeps the last pattern that did. `Enter` jumps to the
first match at or after the caret; `Leader n` and `Leader N` cycle through
the matches and wrap around at the ends of the file.

Esc closes the find prompt and keeps the highlighted matches on screen so you
can edit the file while looking at them; a second Esc clears the highlight and
returns to ordinary editing. Replace behaves the same way: the first Esc keeps
the current pattern's highlights, and the second Esc clears them.

Reloading after an external change discards the buffer's unsaved edits *and* its
undo history, because that history describes the document that was replaced. The
message says so (ADR 0015).

## Panels

`Leader ?` opens the help list and `F1` or `Leader p` opens the command
palette. Both cover the document while they are open and own the keyboard for
the same reason the prompt does: in the palette every printable key is part of
the query, so no chord fires and no sequence starts. `Esc` closes either one
without running anything, `Enter` runs the highlighted command, the up and
down arrows move the highlight, and in the help list they scroll. Both are
built from one table, so a command reachable from the palette is one the help
list also shows.

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

Landmark motions — `}`, `%` — are accepted everywhere, but the *range* they
name is checked before anything is edited: one wider than 10,000 lines is
refused with an explanation, the same rule that stops a stray `Ctrl+A` from
turning one keystroke into a minute of work (ADR 0012, and "Finding 8" in
[performance.md](./performance.md)). The two motions that name a whole-file
range by definition, `gg` and `G`, are not accepted at all: there is no reach
at which they could ever be allowed to edit.

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

`zeno.toml`, read once at startup.

| Platform | Path |
| --- | --- |
| Linux, macOS | `$XDG_CONFIG_HOME/zeno/zeno.toml`, else `~/.config/zeno/zeno.toml` |
| Windows | `%APPDATA%\zeno\zeno.toml` |

Only the `[settings]` table is read today. A missing file, a missing key and a
value that cannot be understood all leave the default in place: a typo in a
config file is not a reason for an editor not to start.

```toml
[settings]
word_wrap = false          # Alt+Z / Leader w at startup
line_numbers = true
relative_line_numbers = false
tab_width = 4              # one indent level, 1–16
auto_pair = true           # (", [", {", " and ' complete themselves
autosave = false           # write the file after a moment of quiet

[keys]
leader = "ctrl+space"      # what opens a sequence

[[keys.bindings]]
keys = "ctrl+shift+w"      # written the way it is said
command = "toggle_word_wrap"
```

`[keys]` is read as well. The leader is one chord, and each
`[[keys.bindings]]` entry adds another: `keys` is parsed as `ctrl+s`,
`alt+z`, `f1`, `ctrl+space`, and `command` is a name from the table this file
is built from — `save`, `quit`, `force_quit`, `undo`, `redo`, `find`,
`replace`, `next_match`, `previous_match`, `go_to_line`, `help`, `palette`,
`indent_line`, `unindent_line`, `toggle_line_comment`, and the three
`toggle_*` view switches. Spaces, dashes and underscores are interchangeable
in a name: `go to line`, `go-to-line` and `go_to_line` are one command.

A user binding is looked at before the built-in chords, so a config file can
move a command without having to unbind where it was. A chord or command that
cannot be read is a line the editor ignores: a typo in a config file is not a
reason for an editor not to start.

The directory next to `zeno.toml` also holds `syntaxes/`, whose
`.sublime-syntax` files are picked up automatically and win over the bundled
definitions for the extensions they claim (spec §4.3). A `themes/` directory
is reserved for later; the theme is fixed for now.

Indentation is not asked for in `zeno.toml` beyond `tab_width`: the style is
detected from the file itself, and `.editorconfig` in the file's directory
tree outranks that detection (spec §4.4).

A filename with no line-comment syntax (`Ctrl+/`) has none to insert, and the
message line says so rather than inventing a block comment. Comment prefixes
are chosen by extension: `//` in C-like files, `#` in shell, Python, TOML and
YAML, `--` in SQL and Lua, `;` in Lisp and assembly.
