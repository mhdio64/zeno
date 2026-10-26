# Indentation is detected, and `.editorconfig` outranks the file

Spec §4.4 asks for four things at once: a style detected from the file's own
content, `.editorconfig` winning over that detection, rules that indent after
an opening bracket and dedent a closing one, and `Tab`/`Shift+Tab` that agree
with all of it. The trap is the phrase "language-aware": the obvious
implementation reaches for the syntax of the file, which means waiting on the
highlighter — a background worker that may not have answered yet, may answer
about a file that was never a language at all, and whose answer changes when
the first line changes. Indentation cannot wait for any of that, because
`Enter` is a keystroke and a keystroke has sixteen milliseconds.

The decision is that indentation is decided **once, at startup, from static
evidence**, in three steps:

1. `.editorconfig`, searched from the file's own directory upwards, reading
   `indent_style`, `indent_size` and `tab_width`. The nearest file wins, and a
   `[*]` section is the fallback for a path no section matches.
2. Failing that, the content: every line's leading whitespace is counted, and
   the majority says tabs or spaces. For spaces the width is the greatest
   common divisor of the indents the file actually uses, clamped to between two
   and eight — so a file indented 2-2-4 reads as two, and a file aligned by a
   single column does not read as a one-space indent, which nobody chose. A
   file with no indented line at all keeps the default.
3. Failing that, `tab_width` from `zeno.toml` and spaces, which is what a
   document with no style of its own should get.

What the user types is then derived from that one value: `Tab` inserts a
literal tab or the detected width of spaces, `Shift+Tab` takes away exactly
what a level is worth in this file, and `Leader <`/`Leader >` do the same over
a range. The first implementation inserted `tab_width` spaces everywhere,
which meant a tab-indented file lost its indentation style to the first edit —
two halves of one feature measured in different units.

`Enter` is where the "language-aware" part actually lives, and it is much
narrower than a syntax table: the new line copies the leading whitespace of
the line it split, and then, if the text to the left of the cursor ends in
`{`, `(`, `[` or `:` — an opening brace, or the colon Python and YAML end a
block header with — one more level is added. Not when the block already
closes itself on the same line: an empty `()` under a call is not somewhere a
new line belongs. Typing a closer at the start of an indented line takes the
corresponding level back, so `}` lands level with the `{` it closes. This is
a rule about the text of one line, not about the language it belongs to, so it
is one string test per keystroke and needs no parser.

The costs are honest. A file that mixes styles gets the majority, so a
three-tab file with one four-space line still indents with tabs — correct for
the file, wrong for that line. A language with no leading-whitespace signal
(the indentation is real but the file is one line long, say) falls through to
the default. And the bracket rule fires on a `{` inside a comment or a string;
it is not a parser, and pretending to be one would cost the first frame
(ADR 0007) to fix an edge case the user can see and correct with one more
keystroke.
