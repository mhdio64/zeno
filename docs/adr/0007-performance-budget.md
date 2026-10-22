# A hard performance budget, enforced in CI

zeno is judged on three numbers. The editor's own work — mapping the file, discovering the lines it needs, painting the first frame — must take **under 50 ms for any file size**. A keystroke must reach the screen in **under 16 ms**. Starting the process must take **under 30 ms** before the editor has read a single byte.

Every design decision that trades latency for convenience is a decision to break this budget, so the budget is encoded as a benchmark suite that runs in CI against generated files of 1 MB, 100 MB and 1 GB, and against adversarial shapes such as a single enormous line, a file with no trailing newline, CRLF line endings, invalid UTF-8 and a BOM-marked UTF-16 file. The budget is the reason the file is mapped rather than loaded, the reason highlighting runs in the background, and the reason rendering is damage-based; those reasons are recorded so that nobody quietly trades them away.

## Amendment, after measurement

The benchmark suite exists (`zeno-bench`, `src/bin/zeno-bench.rs`) and the budget was measured against it. The first run showed that the original wording was measuring the wrong thing, and it is amended here.

**The 50 ms budget applies to the editor's own work, not to the operating system's first access of the file.** On Windows with Defender real-time protection enabled, the first `CreateFile` of a file costs 69 ms (1 MiB) to 391 ms (1 GiB), all of it an antivirus scan charged to the opener. The mapping itself costs 0.0 ms, and faulting the first mebibyte into memory costs 0.4 ms. The second open of the same file costs 0.3 ms. Quoting a single "cold start" number without saying whether it includes the operating system's per-file check would be dishonest, so the suite reports the two separately and the budget is stated against the part the editor controls.

**Process launch is measured separately.** Starting the process costs 10–30 ms on Windows before `main` does anything, which is most of the original 30 ms budget. The empty file is no longer treated as a special case, because an empty file is not faster to launch.

**The budget is met with room to spare.** On a 1 GiB file with 39 million lines, the first painted frame takes 0.5 ms and a keystroke takes 0.1 ms.

Two things the measurement changed:

- **The editable overlay must not be a whole-line model.** Editing a 16 MiB single-line file costs 6.6 ms per keystroke against 0.1 ms for ordinary text, because an edit rewrites the whole line. A piece table or rope makes an edit cost O(edit size). This is the measurement that justifies ADR 0001's choice of ropey over a line-based buffer.
- **The line-number gutter cannot be sized from the line count.** The line count is unknown until the whole file is scanned — about two seconds for 1 GiB — and a gutter sized from it would widen under the user's eyes. It is sized from the size of the file instead, which is always sufficient and never changes.

The full numbers, including what was *not* measured, are in `docs/performance.md`.
