# zeno

A fast, non-modal terminal text editor built to replace nano: open a file instantly, edit it with real editing power, save, and quit. It ships as a single native binary per operating system and opens files of any size without perceptible delay.

## Language

**Quick editor**:
The product's core identity — an editor opened for one file, edited, and closed; not a workspace you live in. Every design decision is judged against the time from launching the binary to accepting the first keystroke.
_Avoid_: workspace editor, IDE, session

**Non-modal**:
One editing model with no modes. The same key means the same thing wherever the cursor sits; there is no normal/insert/visual distinction and no mode to enter.
_Avoid_: modal, vim mode, insert mode, normal mode

**Command**:
A named, single-purpose action — move, select, delete, transform — performed on the user's behalf. Commands are the only way editing happens; no raw keystroke means something different in a different context.
_Avoid_: operation, action, macro

**Operator**:
The half of a two-part command sequence that says what should happen to a range — delete it, change it, yank it, select it, reindent it. The other half names the range.
_Avoid_: verb, modifier, action

**Motion**:
A command that moves the cursor without changing the text, and that can also name the range an operator acts on. A motion computes a boundary and changes nothing by itself, which is why one motion can both move the cursor and delimit a range.
_Avoid_: jump, navigation, cursor movement

**Text object**:
A named region of text — a word, a line, a bracketed span — that a command can act on without the user marking it out first.
_Avoid_: selection, region, range

**Binding**:
The key or key sequence that invokes a command. Bindings are independent of the text being edited and can be rebound by the user.
_Avoid_: shortcut, hotkey, keymap

**Leader key**:
The non-printable key that opens a command sequence. A sequence is a short chord followed by a mnemonic; it is not a mode, because the editor returns to ordinary editing the moment the sequence is finished or abandoned, and every key that does not belong to the sequence cancels it.
_Avoid_: prefix key, escape key, modifier

**Instant open**:
The promise that the first screen of a file is visible and editable without waiting for the file to be fully read, indexed, or highlighted. Measured from process launch to first painted frame.
_Avoid_: fast startup, lazy load

**Prompt**:
The line in the message area where the editor asks for an argument — a line number, a pattern, a path. It owns the keyboard while it is open: printable keys are text, `Enter` answers, `Esc` abandons, and no binding fires.
_Avoid_: dialog, modal, input box

**Viewport**:
The region of the document currently visible in the terminal. Only the viewport is painted; everything outside it is computed on demand.
_Avoid_: screen, window, view

**Non-goal**:
A capability deliberately excluded from the current release, named explicitly so that it is not quietly smuggled back in later.
_Avoid_: out of scope, deferred feature, nice to have
