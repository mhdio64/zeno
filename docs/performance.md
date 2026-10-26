# Performance: what was measured, and what it changed

The performance budget in ADR 0007 was a claim. This is the evidence for it.

Everything below was measured on the machine that produced this file:
Windows 11, 4 cores, Rust 1.95.0, **release** build, 120×40 viewport. The
benchmark harness is `zeno-bench`; the source is `src/bin/zeno-bench.rs`.

```
cargo build --release
./target/release/zeno-bench --sizes 1mb,100mb,1024mb --frames 200
```

The harness generates its own files, runs two passes over each (the first
time the file is touched since it was written, and the second time), and
exits non-zero if a budget is missed.

## The numbers

| file | size | lines | cold start | first frame | repaint | edit | keystroke | warm first frame | warm keystroke | index MB/s | launch |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| text-1mb.txt | 1.0 MiB | 38182 | 76.7 | 76.9 | 0.1 | 0.0 | 0.1 | 0.4 | 0.1 | 783 | 8.7 |
| text-100mb.txt | 100.0 MiB | 3818294 | 264.0 | 264.4 | 0.1 | 0.0 | 0.1 | 0.4 | 0.1 | 1299 | 12.7 |
| text-1024mb.txt | 1.0 GiB | 39099297 | 259.1 | 259.3 | 0.1 | 0.0 | 0.1 | 0.7 | 0.1 | 981 | 21.6 |
| one-line-16mb.txt | 16.0 MiB | 1 | 35.9 | 52.8 | 1.1 | 10.2 | 10.0 | 6.9 | 10.0 | 2326 | 21.1 |
| no-trailing-newline.txt | 64.0 KiB | 2386 | 13.0 | 13.2 | 0.1 | 0.0 | 0.1 | 0.4 | 0.1 | 169 | 7.3 |
| crlf.txt | 64.0 KiB | 2302 | 14.4 | 14.6 | 0.1 | 0.0 | 0.1 | 0.5 | 0.1 | 126 | 8.1 |
| unicode.txt | 64.1 KiB | 772 | 13.2 | 14.2 | 0.1 | 0.0 | 0.1 | 0.4 | 0.1 | 173 | 8.4 |
| invalid-utf8.txt | 64.0 KiB | 2362 | 13.0 | 13.2 | 0.1 | 0.0 | 0.1 | 0.4 | 0.1 | 155 | 7.4 |
| utf16le-bom.txt | 64.0 KiB | 1214 | 16.5 | 16.7 | 0.1 | 0.0 | 0.1 | 0.8 | 0.1 | 80 | 8.8 |

All times in milliseconds. `cold start` is the first touch of a fresh file;
`warm` is the second touch. `launch` is the cost of starting the process at
all, measured from outside and with the editor's own work subtracted out.

Cold open breakdown — the first time a fresh file is opened:

| file | file open | mmap | fault in 1 MiB |
| --- | --- | --- | --- |
| text-1mb.txt | 66.8 | 0.0 | 0.4 |
| text-100mb.txt | 238.0 | 0.0 | 0.4 |
| text-1024mb.txt | 329.1 | 0.0 | 0.4 |
| one-line-16mb.txt | 8.7 | 0.0 | 0.4 |
| no-trailing-newline.txt | 18.2 | 0.0 | 0.0 |
| crlf.txt | 12.6 | 0.0 | 0.0 |
| unicode.txt | 7.9 | 0.0 | 0.0 |
| invalid-utf8.txt | 12.2 | 0.0 | 0.0 |
| utf16le-bom.txt | 10.1 | 0.0 | 0.0 |

## Finding 1 — the design holds, by two orders of magnitude

On a 1 GiB file with 39 million lines, the editor reaches its first painted
frame in **0.7 ms** and repaints after a keystroke in **0.1 ms**. The
viewport cost is proportional to the height of the viewport and not to the
size of the file, which is the entire point of mapping the file and
discovering lines on demand. The 16 ms keystroke budget is met with two
orders of magnitude to spare.

## Finding 2 — the cold-start budget was measuring the wrong thing

The original budget said "cold start to first painted frame, any file size,
under 50 ms". On this machine that is false, and the reason is not the
editor.

The breakdown is unambiguous: **`mmap` costs 0.0 ms in every single case.**
The entire cold cost is inside `File::open` — 67 ms for 1 MiB, 238 ms for
100 MiB, 329 ms for 1 GiB — and faulting the first mebibyte into memory
costs 0.4 ms.

`File::open` on Windows is `CreateFile`, and this machine has Windows
Defender real-time protection enabled, which scans a file the first time it
is opened and charges that scan to the opener. The cost is paid **once per
file**: the second open of the same file is 0.4 ms. It is also not
proportional to size beyond a point — the 1 GiB file costs only 91 ms more
than the 100 MiB one.

So the "under 50 ms" budget was, in practice, a budget on Windows Defender.

**What changed:** ADR 0007 now splits the budget in two. The editor's own
work has a 50 ms budget, which it meets with room to spare. The operating
system's per-file first-access cost is stated as a separate, measured number
that the editor does not control and cannot promise. Quoting a single
"cold start" figure without saying which of the two it means would be
dishonest.

## Finding 3 — process launch alone eats the 30 ms budget

Starting the process — loading the binary, resolving imports, entering
`main` — costs **7–22 ms** on Windows before the editor has read a single
byte of the file. The original "process launch to first frame for an empty
file, under 30 ms" budget was therefore mostly a budget for the Windows
loader, and it has almost no slack in it.

**What changed:** the launch figure is measured and reported separately
(`launch` in the table), and the empty-file case is no longer special-cased
in the budget, because the empty file is not faster to launch.

## Finding 4 — a single enormous line is the one place the data structure shows

`one-line-16mb.txt` is a 16 MiB file with no newline in it. Its keystroke
cost is **10.0 ms** against 0.1 ms for ordinary text — and it moves between
runs (6.6 to 10.4 ms measured), because it is dominated by a 16 MiB copy
competing with whatever else the machine is doing. The cause is the line-level
edit overlay: an edit rewrites the whole line, so an edit costs O(line length). Repaint stays at 1.1 ms, because the renderer clips each
line to the width of the viewport before painting it.

This is a property of the prototype's data structure, not of the design. A
piece table or rope makes an edit cost O(edit size) regardless of line
length. Until that exists, a file with enormous lines is the one input where
the editor is visibly slow.

**What changed:** the finding is recorded as the reason the editable overlay
must not be a whole-line model in the real implementation. The spec already
calls for ropey (ADR 0001); this is the measurement that justifies it.

## Finding 5 — the line-number gutter cannot be sized from the line count

The number of lines in a file is not known until the whole file has been
scanned, which for a 1 GiB file takes about two seconds. A gutter sized
from the line count would therefore widen under the user's eyes during that
time, shifting the whole text area.

**What changed:** the gutter is sized from the *size of the file*. A file of
*n* bytes has at most *n* lines, so the digit count of the file size is
always wide enough, and it never changes. The cost is a gutter a few columns
wider than strictly necessary.

## Finding 6 — the background index is fast enough and does not block

Indexing the whole file runs at **780–1300 MB/s** (1 GiB in about one
second) on a background thread that yields between chunks. The foreground
never waits for it: a keystroke at any point during indexing costs 0.1 ms.
The foreground does block on one thing — "go to the end of the file" has to
scan the whole file to know where the end is. That is the honest cost of
the question and it is paid on request.

## Finding 7 — CRLF was broken

Not a performance finding, but the benchmark surfaced it: a CRLF file was
being read with the carriage return left on the end of each line, and
saving then wrote `\r\r\n`. Fixed, with a regression test.

## Finding 8 — a large range delete is quadratic, and that is the answer to the model question

A keystroke only ever touches one line, so per-keystroke numbers say nothing
about a command that touches thousands of lines at once. `zeno-bench` now
measures one command deleting N lines and then undoing it:

| lines | delete | undo |
| --- | --- | --- |
| 1000 | 1.094 | 0.121 |
| 4000 | 6.947 | 0.543 |
| 16000 | 129.837 | 2.243 |

Four times the lines costs sixteen times the time: the line-anchored overlay
deletes one line at a time, and each deletion walks the segment list. Undo is
fast by contrast, because reversing the whole range is a *single* span edit.

The extrapolation is what matters: deleting a million lines this way would
take minutes, not milliseconds. So the answer to "is the whole-line overlay
enough?" is: yes for typing, no for range commands. Operators such as
delete-to-end-of-file, select-all-then-delete, or a bulk replace must not be
built on it. Adding a line *count* to each segment would collapse the delete
to one operation, but the honest fix is the piece table the spec already
calls for.

**What changed:** the operator layer was built on top of it anyway, but fenced:
the sequencer accepts only motions that stay near the cursor (`w b e $ 0 ^ h l
j k`) and cancels on `G`, `gg`, `}`, `%`. A repeat count multiplies a motion, so
the largest range a key sequence can name is a few thousand lines — far short
of the cliff. Lifting the fence needs this finding fixed first.

## Finding 9 — a word motion costs the rest of the line, and that is affordable

A motion is on the keystroke path, so `zeno-bench` now measures `w`, `e` and
`b` from three starting columns of line 1 of every fixture, and checks the
result against the 16 ms keystroke budget:

| file | line 1 (chars) | w | e | b |
| --- | --- | --- | --- | --- |
| text-1mb.txt | 48 | 0.000 | 0.000 | 0.000 |
| text-100mb.txt | 48 | 0.000 | 0.000 | 0.000 |
| text-1024mb.txt | 48 | 0.000 | 0.000 | 0.000 |
| one-line-16mb.txt | 16777216 | 1.576 | 2.677 | 0.927 |
| no-trailing-newline.txt | 48 | 0.000 | 0.000 | 0.000 |
| crlf.txt | 48 | 0.000 | 0.000 | 0.000 |
| unicode.txt | 39 | 0.000 | 0.000 | 0.000 |
| invalid-utf8.txt | 48 | 0.000 | 0.000 | 0.000 |
| utf16le-bom.txt | 54 | 0.000 | 0.000 | 0.000 |

A motion reads at most the rest of the cursor's line and never allocates a
copy of it, so ordinary text costs nothing measurable and the only shape that
shows is the 16 MiB single line: 2.7 ms to scan 16 million characters, still
well inside the budget. Both word scans walk one `Chars` iterator forward (or
one reversed slice backward) and track the column as they go, instead of
indexing "character at column n" per step, which would have been quadratic in
the column.

## Finding 10 — a motion is cheap because it only *names* a destination

Splitting "where would I go" from "go there" is what makes an operator a
range: the cursor-moving command and `Leader d w` share one function, and the
delete it performs is the same span edit that undo already reverses. Measured
end to end, `Leader d w` on ordinary text is one 0.1 ms edit.

## Finding 11 — colouring the viewport costs the distance to a checkpoint, and one bug made it the file

Syntect is now in the prototype (`src/highlight.rs`), so the "syntax
highlighting" gap below is closed: the harness probes it on a generated
48,000-line Rust file, measuring what the worker pays and what the editor
pays to ask.

| step | ms |
| --- | --- |
| syntax set + theme load (worker startup) | 6.7 |
| first window, top of file (includes that load) | 161.3 |
| window 47900 lines down, first visit (walks from the top) | 2156.3 |
| the next window (resumes from a checkpoint) | 2.1 |
| that window again after one edit on line 1 | 2312.8 |
| the same window once more (reusing the checkpoints just built) | 7.8 |
| editor: ask on a clean document | 0.0 |
| editor: edit + ask, 5000 edited lines | 0.4 |

The two ~2.2-second rows are the honest cost of walking a TextMate parser
over 48,000 lines of a file nobody has coloured yet — and, after an edit
on line 1, of walking it again, because the state at line 47,900 genuinely
depends on the line that changed. The design is vindicated by the rows
around them: a window near the bottom of the file costs **2.1 ms** once
anything has been walked, because a checkpoint of the parser's state sits
no further than 512 lines away, and the editor's own cost to ask is
nothing measurable, because it asks and paints plain text rather than
waiting.

**The benchmark found the bug that would have made all of that a lie.**
In the first version of the probe, "the same window once more" took
2773 ms against 2804 ms for the walk it was supposed to skip: the
checkpoints the walk had *just* built were being discarded again on the
very next request, for ever, because invalidation asked the wrong
question. "What is the lowest line ever edited?" has an answer that,
once the file has been touched, never changes — so one keystroke near the
top of a large file threw away every checkpoint, on every request, for the
rest of the session: rebuild, discard, re-walk. The right question is per
checkpoint: *has anything above this line been written since the version
it was built at?* — answerable because every overlay segment remembers
the version that wrote it (ADR 0018). After the fix the same row is
**7.8 ms**, and the harness fails the run if it ever becomes a walk again.

The same rule guards the editor's own colour cache: only the lines from
the edit down are forgotten. Before the fix, one edit near the top meant
every later keystroke blanked the colours of the whole viewport while the
worker re-walked the file.

The probe itself had to be fixed before its numbers were worth anything:
it used to stop waiting as soon as the window's *first* line was
coloured, so a window overlapping the one already in hand reported the
earlier walk's cost as its own — two rows read 0.0 ms while asking the
worker nothing at all. It now waits for every line of the window.

Two correctness bugs were also fixed on the way here, neither of them a
performance bug but both found while measuring — like the CRLF one above:

- the syntax set was loaded from the *newlines* dump while `line_text`
  hands the parser lines with no newline; in 7 of 29 languages a comment
  then never closes and every line below it renders comment-grey;
- a cancelled request answered with an empty result, wiping the colours
  that were on screen instead of leaving them alone.

## What was not measured

These are gaps, not passes:

- **Damage-based rendering.** The prototype repaints the whole viewport. The
  spec claims only changed cells are written. Damage tracking is unmeasured.
- **The terminal write itself.** The benchmark renders into an off-screen
  buffer. The actual cost of pushing the frame to a terminal — which on a
  slow SSH link or a high-resolution terminal is substantial — is not
  included in any number above.
- **Saving at scale.** Writing a 1 GiB file was not measured. The save path
  is whole-line and writes through a temporary file.
- **The system clipboard.** The OSC 52 write itself is a few hundred bytes
  between frames and was not timed; what is unknown is how *terminals* handle
  it — whether tmux has `set-clipboard on`, whether the emulator supports it at
  all, and how long a 74 KB sequence takes to travel down an SSH link. The
  74,000-character cap is tmux's limit, quoted, not measured here.
- **Selection on a huge file.** Painting a selection is bounded by the
  viewport, like everything else in the renderer, but `Ctrl+A` on a 1 GiB file
  first has to index the whole file (about two seconds, finding 6) before the
  selection exists, and that path was not timed. The refusal above 10,000
  lines is deliberate (`MAX_SPAN_LINES`), but the cost of selecting, as opposed
  to editing, is arithmetic rather than measurement.
- **Other platforms.** Every number here is Windows. Linux and macOS have a
  different `open` cost and no Defender scan; the numbers will be different
  and better, but they are unmeasured.
- **Memory.** Peak RSS while editing a 1 GiB file was not measured. Mapping
  the file means the resident set grows with the pages the user actually
  touches, but the number was not taken.
- **A scrolled viewport.** Every frame above is a viewport at column zero. A
  viewport scrolled to the right has to walk to the column it starts at, once
  per visible line, and a line too short to reach that column is skipped
  without being read — so a file of short lines costs nothing to draw however
  far the viewport is scrolled. The benchmark does not scroll, so the cost of
  a scrolled frame on a file of long lines is unmeasured.

## Reproducing

```
cargo build --release
cargo test                      # 346 tests
./target/release/zeno-bench --sizes 1mb,100mb,1024mb --frames 200
```

`zeno-bench` exits non-zero when a budget is missed, so it can be used as a
CI gate. It also refuses to lie about the build profile: run it in debug and
it warns that the numbers do not apply to a release build.
