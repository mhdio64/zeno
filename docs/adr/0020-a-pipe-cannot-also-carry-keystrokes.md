# A pipe cannot also carry keystrokes

`zeno -` reads a document from standard input and writes the buffer to standard
output (spec §4.1). The first implementation did exactly that in one process,
and could not work: a terminal editor reads keys from standard input, and when
that stream is the document it is at end of file by the time the first frame is
painted. The editor started, painted, and had nothing to read.

The decision is that the document and the keyboard come from different places,
and that getting them there is the job of the process boundary rather than of
the event loop. `zeno -` spools standard input to a temporary file — the whole
model is built on mapping one (ADR 0004) — and then, if standard input is not a
terminal, starts **this same binary** with an internal `--pipe-edit` flag, whose
standard streams are the terminal opened by name: `CONIN$` and `CONOUT$` on
Windows, `/dev/tty` elsewhere. The child edits the spool and writes the buffer
to a result file; the parent waits, prints that file to its own standard output
and deletes both. When standard input already is a terminal there is nothing to
re-open, and the session runs in place.

Two alternatives were rejected. Replacing file descriptor 0 with the terminal
needs `dup2`, which is not in the standard library, and this crate has no
dependency it does not need (ADR 0001); adding one for `zeno -` would put a
library in the first-frame path to save a process. Reading keys from the
terminal by hand would mean parsing escape sequences ourselves — the one part
of the editor crossterm exists to own.

The cost is a second process, and it is only paid when the input is a pipe; the
interactive path — `zeno file` — is untouched. The buffer that comes out is what
the editor held at the end, saved or not, because a pipe has no file for a save
to mean anything to: the spool is a stand-in and is deleted with the answer.
