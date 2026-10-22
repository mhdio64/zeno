# A motion names a destination; a sequence names a range

The editor's editing power is supposed to come from operator-plus-motion sequences, and the keymap has described them since the first draft while the code could only move a cursor. Building them in that shape would have meant writing every motion twice: once to move the cursor, once to compute a range. So a motion is now a **pure function** — `target(doc, from, motion) -> Pos` — that answers where a command would go and changes nothing. Moving the cursor is "compute the target, then move there"; an operator is "compute the target, then edit `from..target`". One function, two masters.

The second decision is that a destination is a **boundary**, not a character. `LineEnd` and `EndOfWord` land one past the last character rather than on it, so every operator range is `[from, to)` and no motion needs an "inclusive" flag. This is the same reasoning as the single edit primitive in ADR 0011: one uniform shape means no command has to know about the exceptions.

The third is that the leader layer is a **sequencer**, not a mode: `src/sequence.rs` holds a three-state machine (idle, leader, operator) that takes characters and returns an `Action`. It is testable without a terminal, it cannot leak state into editing, and the editor's key handler only has to ask it what happened. Nothing about editing changes while a sequence is pending, which is what the glossary means by non-modal.

Two consequences were accepted deliberately:

- **The sequence set is fenced.** The sequencer accepts only motions that stay near the cursor, and cancels on `G`, `gg`, `}` and `%`. A large range delete is quadratic in the line-anchored overlay (performance finding 8), so exposing the motions that name whole-file ranges would put a 52 ms operation — and a much worse one on a bigger file — behind a two-key chord. The fence is a line of code with a comment, not a promise anyone has to remember.
- **`Ctrl+C` had to stop quitting.** It quit before, even with unsaved changes, which contradicted the keymap (`Ctrl+C` is copy, `Leader q` quits) and silently threw work away. Quitting is now `Leader q`, which refuses while the buffer is dirty unless `Leader Q` says to discard, and `Ctrl+C` is copy — a message saying there is nothing selected until ADR 0013 gave selection somewhere to live. A key that destroys unsaved work should not also be the one users press out of habit.

The open question this leaves is the one the fence points at: the overlay's per-line cost. Until the model changes, operators are a near-cursor feature; afterwards the same motions will name any range without a code change, because the range itself was never the problem.
