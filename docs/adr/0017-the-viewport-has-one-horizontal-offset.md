# The viewport has one horizontal offset

A line longer than the screen cannot be drawn in full, so the viewport has to
choose what to leave out. The decision here is that the viewport keeps *one*
horizontal offset for the whole document, the way it keeps one `top_line` for
the vertical axis: every line starts at the same column on screen, so a column
number means the same thing on every line. The alternative — remembering an
offset per line — would let each line scroll independently, which sounds
friendlier and is not: the column under the cursor would change from line to
line, and a search result in a narrow line would sit at a different screen
column than the same result in a wide one. One offset costs something, and the
cost is honest: a line shorter than the offset is blank to the left of it,
because its text is to the left of the window. That is what every editor with
a horizontal scroll does, and it is the only way "column 40" can mean one
thing.

The offset follows the cursor. The cursor is never drawn at the edge of the
screen, and it is never drawn at a position that does not correspond to its
own column: before the change, a cursor on a long line was pinned to the last
column of the terminal, which meant that the part of the line under it could
not be read at all. The viewport scrolls by the smallest amount that brings
the cursor back inside, which is the same rule the vertical axis already
followed.

Drawing a scrolled window has one cost worth naming. The search highlight is
matched against the visible text, and a match that starts before the left
edge and reaches into it has to be found, or the visible part of it goes
unpainted. So the scan reaches a little way back before the window as well as
a little way past its right edge — 64 characters either side. A match that
starts more than 64 characters before the left edge is not found, because the
scan has to stay bounded: an unbounded scan would make the cost of drawing a
frame depend on how far the viewport is scrolled, and the performance budget
says a frame costs what a screen of text costs. The same bound is why the
renderer checks the byte length of a line before walking it: a line too short
to reach the window is skipped without being read, so a file of short lines
costs nothing to draw even when the viewport is scrolled far to the right.
