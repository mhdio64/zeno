# Wrapping moves the viewport by visual row

Soft wrap is off by default (spec §4.5), because the editor's first promise is
that a keystroke paints a frame, and unwrapped text needs no layout at all: a
row is a line, a column is a column, and a line wider than the screen is
scrolled by one offset for the whole document (ADR 0017). Wrapping is the one
feature that makes a screen row and a document line different things, so it is
also the one feature that can quietly break every "the cursor is where the eye
is" assumption the rest of the editor is built on.

The decision is that wrapping adds **one** piece of state and changes **no**
edit semantics. The piece of state is `top_row`: the first visual row of
`top_line` that the viewport shows. A line can be taller than the screen — a
million-character line wrapped at eighty columns is twelve thousand rows — so a
line number alone cannot say where the viewport starts, and the first
implementation that stored only `top_line` could not keep the cursor visible
once it moved a screenful within one line.

Everything else is derived, per frame, from the lines the viewport actually
shows. `wrap_segments` cuts a line at the last space that fits, so words stay
whole; a word wider than the screen is broken, because there is nothing else to
do about it. A row's text stops where its segment ends, not where the screen
does — the first version painted a full screen width from each row and so drew
the first characters of the next segment twice.

Three rules keep the two models honest with each other:

- **Only the first row of a line carries a number.** The gutter's job is to say
  which line a row belongs to, and a continuation row belongs to the line above
  it; repeating the number would say the opposite.
- **A vertical movement is one screen row.** `Up` and `Down` convert through
  `wrapped_position` to (row, column-within-row), move one row, and convert
  back, so a long line is walked at the pace the eye reads it. The column
  within the row is kept across the move, which is the same rule the unwrapped
  move uses for a column that a short line cannot hold.
- **Scrolling is measured in rows, not lines.** `ensure_cursor_visible` counts
  rows from the top of the viewport, so the loop that scrolls is bounded by the
  height of the screen rather than by the size of the file: one row per
  iteration, at most a screenful of them.

Wrapping never edits the document. The horizontal offset is ignored while it is
on and intact when it goes off, the anchor and the cursor keep their character
columns, and every operator, motion and text object works on the same positions
as before. Turning it on is a layout change and nothing else.

The layout cost is paid only for the lines on screen, so the first painted frame
of a gigabyte file is unaffected; and it is paid only while wrapping is on,
which is why the default is what it is.
