# Enter cycles the matches and Esc ends the search

The prompt line was designed so that every question follows one grammar:
printable keys are text, `Enter` answers and closes the prompt, and `Esc`
abandons it (ADR 0014). For `GoToLine` and `ReplaceWith` that rule is natural,
because an answer is submitted once. For `Search` it was frustrating: pressing
`Enter` jumped to the first match and immediately closed the prompt, leaving the
user back in ordinary editing where walking to the next match required a leader
sequence (`Leader n`). In modern editors like VSCode, find is a cycling
interaction: typing previews matches, `Enter` steps through them, and the search
box stays open until dismissed.

The decision is that `Enter` inside a `Search` prompt **cycles through the
matches without closing the prompt**, and `Esc` leaves the search in two stages.

The cycling behaviour follows two precise rules:

- **The first `Enter` starts at or after the caret (`Scope::From`).** If the caret
  is already sitting at the start of a match when find is opened, that match is
  kept and confirmed rather than skipped. If the caret is before a match, it jumps
  to it.
- **Subsequent `Enter`s advance strictly past the match (`Scope::After`).** Once a
  match has been reached, the cursor sits at the match start. Subsequent `Enter`
  presses advance past the current match (`start > pos.col`), finding the next
  match on the same line or later lines, and wrapping around to the start of the
  file when the end is reached. Editing the pattern or reopening find resets this
  state, so the next `Enter` is again a first `Enter`.

The two-stage `Esc` behaviour completes the interaction:

- **The first `Esc` closes the prompt and leaves the matches highlighted.** The
  document regains keyboard ownership so the user can edit immediately while
  keeping the visual context of the search results on screen.
- **A second `Esc` in ordinary editing clears the search.** The highlight and
  search state are dropped, returning to completely plain editing.

Two alternatives were rejected. Closing the prompt on `Enter` and relying on
`Leader n` to cycle preserved the prompt rule at the cost of breaking the single
most common search reflex in terminal and desktop editors alike. Tracking the end
column of each match explicitly was also rejected: `goto_match` parks the caret
at `m.start`, and `Scope::After` already demands `start > pos.col`, so
`Scope::After { pos: self.cursor() }` advances past the match without inventing a
second coordinate system.

The cost is that `Search` is the one prompt where `Enter` is a repeating motion
rather than a terminating answer. Because the prompt owns the keyboard while it
is open (ADR 0014), leader sequences and document edits cannot fire until the
first `Esc` closes it.
