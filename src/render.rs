//! Viewport rendering.
//!//! Only the lines that fit on screen are ever touched. The cost of a frame
//! is proportional to the height of the viewport, never to the size of the
//! file — that is the claim the performance budget rests on, and it is
//! measurable here.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget};

use std::collections::HashMap;

use crate::complete::is_word_char;
use crate::doc::Doc;
use crate::doc::edit::Pos;
use crate::git::{DiffSign, GitGutterStyle};
use crate::highlight::Highlighted;
use crate::search::Pattern;
use crate::theme::ThemeColors;

/// The visible state of the editor.
#[derive(Debug, Clone)]
pub struct ViewState {
    /// First line shown in the viewport.
    pub top_line: u64,
    /// First *visual* row of `top_line` shown, when wrapping is on.
    ///
    /// A wrapped line can be taller than the screen, so a line alone cannot
    /// say where the viewport starts. Unwrapped, this is always zero.
    pub top_row: usize,
    /// Whether long lines are wrapped at the right edge instead of scrolled.
    ///
    /// Off by default: a scroll is cheaper than laying a line out, and an
    /// editor whose first job is to feel instant should not pay for a
    /// feature nobody asked for until they ask (spec §4.5).
    pub word_wrap: bool,
    /// Line the cursor is on.
    pub cursor_line: u64,
    /// Column the cursor is at, counted in characters.
    pub cursor_col: usize,
    /// Whether the line-number gutter is drawn.
    pub show_line_numbers: bool,
    /// Whether the gutter shows distances from the cursor instead of
    /// absolute line numbers (the cursor's own line still shows absolute).
    pub show_relative_numbers: bool,
    /// Width of the viewport in columns, excluding the gutter.
    pub width: u16,
    /// First column of text shown in the viewport.
    ///
    /// One offset for the whole document, the way `top_line` is one offset
    /// for the whole document vertically: every line starts at the same
    /// column on screen, so a column means the same thing on every line.
    /// A line shorter than the offset is simply blank to its left, which is
    /// what every editor with a horizontal scroll does.
    pub left_col: usize,
    /// Where the current selection started, if there is one.
    ///
    /// A selection is one optional position, not a mode and not a second
    /// cursor: the range is always between this anchor and the cursor, so a
    /// motion that already knows where it is going can extend a selection
    /// for free.
    pub anchor: Option<Pos>,
    /// Optional pattern for search highlight rendering.
    pub highlight_pattern: Option<Pattern>,
    /// Whether the visual scrollbar is drawn at the right edge.
    pub show_scrollbar: bool,
    /// Whether git diff signs are shown in the gutter.
    pub show_git_diff: bool,
    /// Visual style of the diff signs in the gutter.
    pub git_gutter_style: GitGutterStyle,
    /// Whether occurrences of the word under cursor are highlighted in the viewport.
    pub highlight_word: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 80,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        }
    }
}

/// Renders the document viewport.
pub struct View<'a> {
    pub doc: &'a Doc,
    pub state: &'a ViewState,
    /// The colours of the lines that have been coloured, if any.
    ///
    /// The highlighter works in the background, so this is empty until the
    /// colours arrive, and stays empty for a file with no syntax.
    pub highlight: Option<&'a Highlighted>,
    /// Git diff signs indicating additions, modifications, and deletions.
    pub diff_signs: Option<&'a HashMap<usize, DiffSign>>,
    /// The active theme's colors for editor canvas, cursor line, gutter, and chrome.
    pub theme: ThemeColors,
}

impl<'a> View<'a> {
    /// Creates a view with default theme colors and no syntax highlight.
    pub fn new(doc: &'a Doc, state: &'a ViewState) -> Self {
        Self {
            doc,
            state,
            highlight: None,
            diff_signs: None,
            theme: ThemeColors::default(),
        }
    }

    /// Width of the line-number gutter, in columns.
    ///
    /// Stable for the whole session: it is derived from the size of the file
    /// rather than from the number of lines, which is not known until the
    /// background scan finishes.
    pub fn gutter_width(&self) -> u16 {
        if !self.state.show_line_numbers {
            return 0;
        }
        (self.doc.gutter_digits() + 1).max(4)
    }

    /// Whether the visual scrollbar should be rendered.
    ///
    /// The scrollbar is drawn when enabled in settings, when the viewport has
    /// enough room to display text alongside it, and when the document is
    /// taller than the screen.
    pub fn show_scrollbar(&self, height: u16, width: u16) -> bool {
        if !self.state.show_scrollbar || width <= self.gutter_width() + 2 || height <= 1 {
            return false;
        }
        let total = self
            .doc
            .total_lines()
            .unwrap_or_else(|| self.doc.line_count())
            .max(self.state.top_line + height as u64);
        total > height as u64
    }

    /// Width of the visual scrollbar, in columns (1 if visible, 0 if hidden).
    pub fn scrollbar_width(&self, height: u16, width: u16) -> u16 {
        if self.show_scrollbar(height, width) {
            1
        } else {
            0
        }
    }

    /// The selected columns of `line`, as a half-open range.
    ///
    /// Half-open for the same reason operators are: the cursor sits *between*
    /// characters, so a range that ends at the cursor must not paint the
    /// character under it. A selection that ends at column 0 of a later line
    /// therefore paints nothing on that line, which is what stops `Shift+Down`
    /// from highlighting the first character of the line below.
    pub fn selection_span(&self, line: u64) -> Option<(usize, usize)> {
        let anchor = self.state.anchor?;
        let cursor = Pos::new(self.state.cursor_line, self.state.cursor_col);
        let (start, end) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };
        if start == end || line < start.line || line > end.line {
            return None;
        }
        let len = self.doc.line_text(line).map_or(0, |t| t.chars().count());
        let from = if line == start.line { start.col } else { 0 };
        let to = if line == end.line { end.col } else { len };
        if to <= from {
            return None;
        }
        Some((from, to))
    }

    /// The columns of `line` that the search pattern matches, as half-open
    /// character ranges *within the visible window*.
    ///
    /// Only the part of the line that can be seen is looked at, plus a short
    /// look-around on either side: a file with one enormous line must not
    /// cost more to draw than a file with ten thousand short ones. A match
    /// that starts more than `MATCH_LOOKBEHIND` characters before the left
    /// edge is therefore not found, which is the price of that bound.
    pub fn match_ranges(&self, line: u64, painted: usize) -> Vec<(usize, usize)> {
        let Some(text) = self.doc.line_text(line) else {
            return Vec::new();
        };
        let left = self.state.left_col;
        let behind = left.saturating_sub(MATCH_LOOKBEHIND);
        let head = head_of(&text, behind);
        self.match_ranges_in(head, left - behind, painted)
    }

    /// The matches in `head`, as ranges within the visible window.
    ///
    /// `head` is the text of the line starting `offset` characters before the
    /// left edge, so a match that reaches into the window from the left is
    /// still found.
    fn match_ranges_in(&self, head: &str, offset: usize, painted: usize) -> Vec<(usize, usize)> {
        let Some(pattern) = self.state.highlight_pattern.as_ref() else {
            return Vec::new();
        };
        if painted == 0 {
            return Vec::new();
        }
        let scanned = take_chars(head, offset + painted + MATCH_LOOKAHEAD);
        pattern
            .find_in(scanned)
            .into_iter()
            .map(|(start, end)| (start.saturating_sub(offset), end.saturating_sub(offset)))
            .filter(|(start, end)| *start < painted && *end > *start)
            .map(|(start, end)| (start.min(painted), end.min(painted)))
            .collect()
    }

    /// Finds the word under the cursor at `(line, col)` if any.
    pub fn word_under_cursor(&self) -> Option<String> {
        word_under_cursor(self.doc, self.state.cursor_line, self.state.cursor_col)
    }

    /// The coloured runs of `line` that fall inside the window.
    ///
    /// The spans are in the line's own columns; the window shows a slice of
    /// them, so they are moved into the window's own columns here.
    fn syntax_spans(&self, line: u64, left: usize, painted: usize) -> Vec<(usize, usize, Style)> {
        let Some(highlight) = self.highlight else {
            return Vec::new();
        };
        let Some(spans) = highlight.get(&line) else {
            return Vec::new();
        };
        let mut inside = Vec::new();
        for span in spans {
            let start = span.start.max(left);
            let end = span.end.min(left + painted);
            if end <= start {
                continue;
            }
            inside.push((start - left, end - left, span.style));
        }
        inside
    }

    /// Paints one line, in runs of equal style.
    ///
    /// The line is at most one screen wide, so deciding the style of every
    /// character and merging the runs afterwards is cheaper than working out
    /// where the selection and the search matches intersect.
    fn styled_spans(
        &self,
        text: &str,
        base: Style,
        syntax: &[(usize, usize, Style)],
        word_matches: &[(usize, usize)],
        selection: Option<(usize, usize)>,
        matches: &[(usize, usize)],
    ) -> Vec<Span<'static>> {
        let chars: Vec<char> = text.chars().collect();
        let mut styles = vec![base; chars.len()];
        // The syntax says what the text means. It is painted first, because
        // the search match and the selection say what the user is looking
        // at, and those win.
        for (start, end, style) in syntax {
            let start = (*start).min(chars.len());
            let end = (*end).clamp(start, chars.len());
            for slot in &mut styles[start..end] {
                *slot = (*slot).patch(*style);
            }
        }
        // Word under cursor matches: subtle background + underline
        let word_style = self.word_match_style();
        for (start, end) in word_matches {
            let start = (*start).min(chars.len());
            let end = (*end).clamp(start, chars.len());
            for slot in &mut styles[start..end] {
                *slot = (*slot).patch(word_style);
            }
        }
        for (start, end) in matches {
            let start = (*start).min(chars.len());
            let end = (*end).clamp(start, chars.len());
            for slot in &mut styles[start..end] {
                *slot = self.match_style();
            }
        }
        // The selection wins over the search match: it is what the user has
        // just made, and one highlight is easier to read than two.
        if let Some((from, to)) = selection {
            let from = from.min(chars.len());
            let to = to.clamp(from, chars.len());
            for slot in &mut styles[from..to] {
                *slot = self.selection_style();
            }
        }

        let (display_chars, display_styles) = if crate::persian::has_rtl_chars(&chars) {
            let layout = crate::persian::layout_chars(&chars);
            let reordered_styles: Vec<Style> = layout
                .vis_to_log
                .iter()
                .map(|&log_idx| styles[log_idx])
                .collect();
            (layout.chars, reordered_styles)
        } else {
            (chars, styles)
        };

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut start = 0;
        for col in 1..=display_chars.len() {
            if col == display_chars.len() || display_styles[col] != display_styles[start] {
                spans.push(Span::styled(
                    display_chars[start..col].iter().collect::<String>(),
                    display_styles[start],
                ));
                start = col;
            }
        }
        if spans.is_empty() {
            spans.push(Span::styled(String::new(), base));
        }
        spans
    }

    /// Screen position of the cursor, as `(x, y)` relative to `area`.
    ///
    /// `None` when the cursor is not on screen: the viewport scrolls to
    /// follow it, so this only happens in the frame between a resize and
    /// the scroll that answers it.
    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let gutter = self.gutter_width();
        let scrollbar = self.scrollbar_width(area.height, area.width);
        let body_width = area.width.saturating_sub(gutter).saturating_sub(scrollbar);
        if body_width == 0 {
            return None;
        }
        if self.state.word_wrap {
            return self.wrapped_cursor_position(area, gutter, body_width);
        }
        let row = self.state.cursor_line.checked_sub(self.state.top_line)?;
        if row >= area.height as u64 {
            return None;
        }
        // `cursor_col` is the character column: the editor keeps it inside
        // the line, and walking the line to count it again would cost a
        // keystroke's worth of work on a line with a million characters.
        let col = if let Some(text) = self.doc.line_text(self.state.cursor_line) {
            if crate::persian::has_rtl(&text) {
                crate::persian::logical_to_visual(&text, self.state.cursor_col)
            } else {
                self.state.cursor_col
            }
        } else {
            self.state.cursor_col
        };
        let left = self.state.left_col;
        if col < left || col - left >= body_width as usize {
            return None;
        }
        let x = area.x + gutter + (col - left) as u16;
        Some((x, area.y + row as u16))
    }

    /// Where the cursor is when the viewport wraps.
    ///
    /// The row is counted from the top of the viewport, so a line taller than
    /// the screen is laid out only as far as it is needed.
    fn wrapped_cursor_position(
        &self,
        area: Rect,
        gutter: u16,
        body_width: u16,
    ) -> Option<(u16, u16)> {
        let width = body_width as usize;
        let mut row = 0usize;
        let mut line = self.state.top_line;
        while line < self.state.cursor_line {
            row += wrapped_row_count(&self.chars_of(line), width);
            line += 1;
        }
        let chars = self.chars_of(self.state.cursor_line);
        let (within, col) = wrapped_position(&chars, width, self.state.cursor_col);
        row += within;
        let row = row.checked_sub(self.state.top_row)?;
        if row >= area.height as usize {
            return None;
        }
        Some((area.x + gutter + col as u16, area.y + row as u16))
    }

    /// The characters of one line.
    ///
    /// Wrapping has to know where the spaces are, so it needs the line in
    /// hand; every caller that pays this is bounded by the viewport height.
    fn chars_of(&self, line: u64) -> Vec<char> {
        self.doc
            .line_text(line)
            .map(|text| text.chars().collect())
            .unwrap_or_default()
    }

    /// The rows that fill `height` screen rows, from the top of the viewport.
    ///
    /// The second half of each entry says whether the row starts a line: only
    /// that row carries a line number, because the next one is still the same
    /// line.
    fn layout_rows(&self, width: usize, height: u16) -> Vec<(Row, bool)> {
        let capacity = height as usize;
        let mut rows: Vec<(Row, bool)> = Vec::with_capacity(capacity);
        if !self.state.word_wrap {
            for offset in 0..height as u64 {
                rows.push((
                    Row {
                        line: self.state.top_line + offset,
                        start: self.state.left_col,
                        end: self.state.left_col + width,
                    },
                    true,
                ));
            }
            return rows;
        }
        let mut line = self.state.top_line;
        let mut skip = self.state.top_row;
        while rows.len() < capacity {
            if self.doc.line_text(line).is_none() {
                break;
            }
            let chars = self.chars_of(line);
            let starts = wrap_segments(&chars, width);
            for (index, start) in starts.iter().enumerate() {
                if index < skip {
                    continue;
                }
                let end = starts.get(index + 1).copied().unwrap_or(chars.len());
                rows.push((
                    Row {
                        line,
                        start: *start,
                        end,
                    },
                    index == 0,
                ));
                if rows.len() == capacity {
                    break;
                }
            }
            line += 1;
            skip = 0;
        }
        rows
    }
}

/// One screen row: a line, and the column it starts at on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub line: u64,
    /// First character column of the line shown on this row.
    pub start: usize,
    /// One past the last column shown, so a wrapped row stops where its
    /// segment ends instead of running into the next one.
    pub end: usize,
}

/// The character columns a wrapped line is cut at: one entry per screen row.
///
/// A segment ends after the last space that fits, so words stay whole; a word
/// wider than the screen is broken, because there is nothing else to do about
/// it. A line that ends exactly at the right edge gets one empty row after it,
/// which is where the cursor sits when it is at the end of that line.
pub fn wrap_segments(chars: &[char], width: usize) -> Vec<usize> {
    let width = width.max(1);
    let mut starts = vec![0];
    let mut start = 0;
    while start < chars.len() {
        let end = wrap_end(chars, start, width);
        if end >= chars.len() {
            break;
        }
        starts.push(end);
        start = end;
    }
    if chars.len() - *starts.last().expect("always one segment") == width {
        starts.push(chars.len());
    }
    starts
}

/// Where one wrapped row of a line ends.
fn wrap_end(chars: &[char], start: usize, width: usize) -> usize {
    let limit = (start + width).min(chars.len());
    if limit >= chars.len() {
        return chars.len();
    }
    // A space right past the edge is the natural place to break: the row is
    // exactly one screen wide and ends with a whole word.
    if chars[limit] == ' ' {
        return limit;
    }
    // Break after the last space that fits, so a word is not cut in half.
    let mut i = limit;
    while i > start + 1 {
        i -= 1;
        if chars[i] == ' ' {
            return i + 1;
        }
    }
    limit
}

/// How many screen rows `chars` takes when wrapped at `width`.
pub fn wrapped_row_count(chars: &[char], width: usize) -> usize {
    wrap_segments(chars, width).len()
}

/// Which row of a wrapped line `col` falls on, and its column within that row.
pub fn wrapped_position(chars: &[char], width: usize, col: usize) -> (usize, usize) {
    let starts = wrap_segments(chars, width);
    let mut row = 0;
    for (i, start) in starts.iter().enumerate() {
        if *start <= col {
            row = i;
        } else {
            break;
        }
    }
    (row, col.saturating_sub(starts[row]))
}

/// Where a wrapped row starts, in the line's own columns.
pub fn wrapped_row_start(chars: &[char], width: usize, row: usize) -> usize {
    let starts = wrap_segments(chars, width);
    starts[row.min(starts.len() - 1)]
}

/// The text of a line from `behind` characters before the visible window.
///
/// The byte length is checked first: a line that cannot reach the window
/// costs nothing to skip, so a file of short lines costs nothing to draw
/// even when the viewport is scrolled to the right.
fn head_of(text: &str, behind: usize) -> &str {
    if text.len() < behind {
        return "";
    }
    skip_chars(text, behind)
}

/// The part of `text` that starts at character `col`.
///
/// The cost is the number of characters skipped, which is what a viewport
/// scrolled to the right of a long line has to pay.
fn skip_chars(text: &str, col: usize) -> &str {
    let mut rest = text;
    let mut left = col;
    while left > 0 {
        match rest.chars().next() {
            Some(c) => {
                rest = &rest[c.len_utf8()..];
                left -= 1;
            }
            None => return "",
        }
    }
    rest
}

/// The first `count` characters of `text`.
fn take_chars(text: &str, count: usize) -> &str {
    match text.char_indices().nth(count) {
        Some((byte, _)) => &text[..byte],
        None => text,
    }
}

impl View<'_> {
    /// The style of the line the cursor sits on.
    fn cursor_line_style(&self) -> Style {
        Style::default()
            .bg(self.theme.line_highlight)
            .fg(self.theme.fg)
    }

    /// The style selected text is painted in.
    fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.theme.selection_bg)
            .fg(self.theme.selection_fg)
    }

    /// The style a search match is painted in.
    fn match_style(&self) -> Style {
        if self.theme.is_dark {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(181, 137, 0))
        }
    }

    /// The style an occurrence of the word under cursor is painted in.
    fn word_match_style(&self) -> Style {
        Style::default()
            .bg(self.theme.word_match_bg())
            .add_modifier(Modifier::UNDERLINED)
    }
}

/// How far past the right edge the search pattern is still read.
///
/// A match that starts just before the edge and ends past it would
/// otherwise be missed, because the line is cut where the screen ends.
const MATCH_LOOKAHEAD: usize = 64;

/// How far before the left edge the search pattern is still read.
///
/// The mirror of [`MATCH_LOOKAHEAD`]: a match that starts just before the
/// edge and reaches into it would otherwise be missed, because the line is
/// cut where the screen begins.
const MATCH_LOOKBEHIND: usize = 64;

impl Widget for View<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let gutter = self.gutter_width();
        let scrollbar = self.scrollbar_width(area.height, area.width);
        let body_x = area.x + gutter;
        let body_width = area.width.saturating_sub(gutter).saturating_sub(scrollbar);
        if body_width == 0 {
            return;
        }

        let mut gutter_lines: Vec<Line> = Vec::with_capacity(area.height as usize);
        let mut body_lines: Vec<Line> = Vec::with_capacity(area.height as usize);

        let current_word = if self.state.highlight_word && self.state.anchor.is_none() {
            self.word_under_cursor()
        } else {
            None
        };

        for (row, starts_line) in self.layout_rows(body_width as usize, area.height) {
            let line_no = row.line;
            let is_cursor = line_no == self.state.cursor_line;

            if self.state.show_line_numbers {
                // A wrapped line carries its number only on its first row:
                // the rows under it are still the same line.
                let label = if !starts_line {
                    String::new()
                } else if self.state.show_relative_numbers && !is_cursor {
                    let distance = line_no.abs_diff(self.state.cursor_line);
                    format!("{:>width$}", distance, width = gutter as usize - 1)
                } else {
                    format!("{:>width$}", line_no + 1, width = gutter as usize - 1)
                };
                let style = if is_cursor {
                    Style::default()
                        .bg(self.theme.line_highlight)
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .bg(self.theme.gutter_bg)
                        .fg(self.theme.gutter_fg)
                };

                let sign_span = if !starts_line {
                    Span::styled(
                        "",
                        Style::default().bg(if is_cursor {
                            self.theme.line_highlight
                        } else {
                            self.theme.gutter_bg
                        }),
                    )
                } else if self.state.show_git_diff {
                    let bg = if is_cursor {
                        self.theme.line_highlight
                    } else {
                        self.theme.gutter_bg
                    };
                    if let Some(sign) = self.diff_signs.and_then(|m| m.get(&(line_no as usize))) {
                        let sym = sign.symbol(self.state.git_gutter_style);
                        let fg = sign.color();
                        Span::styled(sym, Style::default().fg(fg).bg(bg))
                    } else {
                        Span::styled(" ", Style::default().bg(bg))
                    }
                } else {
                    let bg = if is_cursor {
                        self.theme.line_highlight
                    } else {
                        self.theme.gutter_bg
                    };
                    Span::styled(" ", Style::default().bg(bg))
                };

                gutter_lines.push(Line::from(vec![Span::styled(label, style), sign_span]));
            }

            // Only the characters that can fit on screen are painted. A file
            // with one enormous line must not cost more to draw than a file
            // with ten thousand short ones.
            let text = self.doc.line_text(line_no).unwrap_or_default();
            let left = row.start;
            // The head of the line reaches back before the window, so that a
            // match crossing the left edge is still found.
            let behind = left.saturating_sub(MATCH_LOOKBEHIND);
            let head = head_of(&text, behind);
            let window = skip_chars(head, left - behind);
            let shown = (row.end - row.start).min(body_width as usize);
            let clipped: String = window.chars().take(shown).collect();
            let base = if is_cursor {
                self.cursor_line_style()
            } else {
                Style::default().bg(self.theme.bg).fg(self.theme.fg)
            };
            let painted = clipped.chars().count();
            // The selection is named in the line's own columns; the window
            // shows a slice of them.
            let selection = self.selection_span(line_no).map(|(from, to)| {
                let from = from.max(left).min(left + painted);
                let to = to.max(left).min(left + painted);
                (from - left, to - left)
            });
            let word_matches = if let Some(ref word) = current_word {
                find_word_occurrences(head, word, left - behind, painted)
            } else {
                Vec::new()
            };
            let matches = self.match_ranges_in(head, left - behind, painted);
            let syntax = self.syntax_spans(line_no, left, painted);
            body_lines.push(
                Line::from(self.styled_spans(
                    &clipped,
                    base,
                    &syntax,
                    &word_matches,
                    selection,
                    &matches,
                ))
                .style(base),
            );
        }

        if self.state.show_line_numbers {
            let gutter_area = Rect {
                x: area.x,
                y: area.y,
                width: gutter,
                height: area.height,
            };
            Paragraph::new(Text::from(gutter_lines))
                .style(Style::default().bg(self.theme.gutter_bg))
                .render(gutter_area, buf);
        }

        let body_area = Rect {
            x: body_x,
            y: area.y,
            width: body_width,
            height: area.height,
        };
        Paragraph::new(Text::from(body_lines))
            .style(Style::default().bg(self.theme.bg).fg(self.theme.fg))
            .render(body_area, buf);

        if scrollbar > 0 {
            let scrollbar_x = area.x + area.width - 1;
            let total = self
                .doc
                .total_lines()
                .unwrap_or_else(|| self.doc.line_count())
                .max(self.state.top_line + area.height as u64);
            let height = area.height as usize;
            let thumb_height =
                ((height as u64 * height as u64) / total).clamp(1, height as u64) as usize;
            let max_top = total.saturating_sub(area.height as u64);
            let track_room = height.saturating_sub(thumb_height);
            let dividend = (self.state.top_line * track_room as u64) + max_top / 2;
            let thumb_top = dividend
                .checked_div(max_top)
                .map(|t| (t as usize).min(track_room))
                .unwrap_or(0);
            let thumb_bottom = thumb_top + thumb_height;

            let track_style = Style::default()
                .fg(self.theme.scrollbar_track())
                .bg(self.theme.bg);
            let thumb_style = Style::default()
                .fg(self.theme.scrollbar_thumb())
                .bg(self.theme.bg);

            for row in 0..area.height {
                let y = area.y + row;
                let (sym, style) = if (row as usize) >= thumb_top && (row as usize) < thumb_bottom {
                    ("█", thumb_style)
                } else {
                    ("│", track_style)
                };
                if let Some(cell) = buf.cell_mut((scrollbar_x, y)) {
                    cell.set_symbol(sym);
                    cell.set_style(style);
                }
            }
        }
    }
}

/// Finds the word under the cursor at `(line, col)` if any.
pub fn word_under_cursor(doc: &Doc, line: u64, col: usize) -> Option<String> {
    let text = doc.line_text(line)?;
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let pos = if col < chars.len() && is_word_char(chars[col]) {
        col
    } else if col == chars.len() && col > 0 && is_word_char(chars[col - 1]) {
        col - 1
    } else {
        return None;
    };

    let mut start = pos;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = pos;
    while end < chars.len() && is_word_char(chars[end]) {
        end += 1;
    }

    if end <= start {
        return None;
    }
    let word: String = chars[start..end].iter().collect();
    if word.len() < 2 || word.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(word)
}

/// Finds all whole-word occurrences of `word` in `text` within `[offset..offset + painted]`.
pub fn find_word_occurrences(
    text: &str,
    word: &str,
    offset: usize,
    painted: usize,
) -> Vec<(usize, usize)> {
    if word.is_empty() || painted == 0 {
        return Vec::new();
    }
    let scanned = take_chars(text, offset + painted + word.len().max(MATCH_LOOKAHEAD) + 1);
    let chars: Vec<char> = scanned.chars().collect();
    let word_chars: Vec<char> = word.chars().collect();
    let word_len = word_chars.len();
    if chars.len() < word_len {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let limit = chars.len().saturating_sub(word_len);

    for i in 0..=limit {
        if chars[i..i + word_len] == word_chars[..] {
            let left_ok = i == 0 || !is_word_char(chars[i - 1]);
            let right_ok = (i + word_len) == chars.len() || !is_word_char(chars[i + word_len]);
            if left_ok && right_ok {
                let start = i.saturating_sub(offset);
                let end = (i + word_len).saturating_sub(offset);
                if start < painted && end > start {
                    ranges.push((start.min(painted), end.min(painted)));
                }
            }
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::{Highlighted, LineSpan};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn doc_of(name: &str, contents: &[u8]) -> Doc {
        use std::io::Write;
        let dir = std::env::temp_dir().join("zeno-render-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        let mut doc = Doc::open(&path).unwrap();
        doc.index_fully();
        doc
    }

    fn frame_text(doc: &Doc, state: &ViewState, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View {
                    doc,
                    state,
                    highlight: None,
                    diff_signs: None,
                    theme: ThemeColors::default(),
                };
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn relative_numbers_show_distance_from_the_cursor() {
        let doc = doc_of("relative.txt", b"alpha\nbeta\ngamma\ndelta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 2,
            cursor_col: 0,
            show_line_numbers: true,
            show_relative_numbers: true,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let text = frame_text(&doc, &state, 20, 4);
        let rows: Vec<&str> = text.split('\n').collect();
        // Line 0 (row 0): distance from cursor line 2 is 2.
        assert!(rows[0].trim_start().starts_with('2'), "{text}");
        // Line 1: distance 1.
        assert!(rows[1].trim_start().starts_with('1'), "{text}");
        // Line 2 is the cursor: absolute number, 1-based → 3.
        assert!(rows[2].trim_start().starts_with('3'), "{text}");
        // Line 3: distance 1.
        assert!(rows[3].trim_start().starts_with('1'), "{text}");
        // The absolute file line count (4) never appears as a label for a
        // non-cursor row — every non-cursor label is a distance.
        assert!(!rows[0].contains('4'), "{text}");
    }

    #[test]
    fn renders_gutter_and_text() {
        let doc = doc_of("gutter.txt", b"alpha\nbeta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let text = frame_text(&doc, &state, 20, 3);
        assert!(text.contains("alpha"), "{text}");
        assert!(text.contains("1"), "{text}");
    }

    #[test]
    fn top_line_scrolls_the_viewport() {
        let doc = doc_of("scroll.txt", b"one\ntwo\nthree\n");
        let state = ViewState {
            top_line: 2,
            top_row: 0,
            word_wrap: false,
            cursor_line: 2,
            cursor_col: 0,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let text = frame_text(&doc, &state, 20, 3);
        assert!(text.contains("three"), "{text}");
        assert!(!text.contains("one"), "{text}");
    }

    #[test]
    fn a_selection_paints_only_the_selected_columns() {
        let doc = doc_of("selection.txt", b"abcdef\nsecond\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 4,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let view = View::new(&doc, &state);
        assert_eq!(view.selection_span(0), Some((1, 4)));
        assert_eq!(
            view.selection_span(1),
            None,
            "the other line is not selected"
        );

        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View::new(&doc, &state);
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let is_selected =
            |x: u16| buffer[(4 + x, 0)].bg == Color::White && buffer[(4 + x, 0)].fg == Color::Black;
        assert!(!is_selected(0), "'a' is outside the selection");
        assert!(
            is_selected(1) && is_selected(2) && is_selected(3),
            "'bcd' is selected"
        );
        assert!(!is_selected(4), "'e' is past the selection");
    }

    #[test]
    fn a_selection_across_lines_covers_the_middle_lines_whole() {
        let doc = doc_of("selection-lines.txt", b"one\ntw\nthree\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 2,
            cursor_col: 2,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let view = View::new(&doc, &state);
        assert_eq!(view.selection_span(0), Some((1, 3)));
        assert_eq!(view.selection_span(1), Some((0, 2)));
        assert_eq!(view.selection_span(2), Some((0, 2)));
    }

    #[test]
    fn a_selection_ending_at_a_line_start_paints_nothing_on_it() {
        let doc = doc_of("selection-edge.txt", b"one\ntwo\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 1,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let view = View::new(&doc, &state);
        assert_eq!(view.selection_span(0), Some((1, 3)));
        assert_eq!(
            view.selection_span(1),
            None,
            "Shift+Down must not highlight the first character of the next line"
        );
    }

    #[test]
    fn a_search_match_is_painted_in_the_match_style() {
        let doc = doc_of("highlight.txt", b"alpha beta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 1,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: Some(Pattern::compile("beta").expect("a valid pattern")),
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View::new(&doc, &state);
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // "alpha " is six characters, so the match starts at column 6.
        assert_eq!(buffer[(6, 0)].bg, Color::Yellow);
        assert_eq!(buffer[(9, 0)].bg, Color::Yellow);
        assert_eq!(
            buffer[(5, 0)].bg,
            ThemeColors::default().bg,
            "the text before the match keeps the theme background"
        );
    }

    #[test]
    fn the_selection_wins_over_the_search_match() {
        let doc = doc_of("highlight-selection.txt", b"alpha beta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 8,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 6)),
            highlight_pattern: Some(Pattern::compile("beta").expect("a valid pattern")),
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View::new(&doc, &state);
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(6, 0)].bg,
            Color::White,
            "the selection is painted with a white background"
        );
        assert_eq!(buffer[(6, 0)].fg, Color::Black, "and black foreground");
        assert_eq!(
            buffer[(8, 0)].bg,
            Color::Yellow,
            "the rest of the match keeps the match style"
        );
    }

    #[test]
    fn a_match_past_the_right_edge_is_not_looked_for() {
        let doc = doc_of("highlight-edge.txt", b"x\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: Some(Pattern::compile("needle").expect("a valid pattern")),
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let view = View::new(&doc, &state);
        assert!(view.match_ranges(0, 20).is_empty());
    }

    #[test]
    fn cursor_position_follows_the_line() {
        let doc = doc_of("cursor.txt", b"alpha\nbeta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 1,
            cursor_col: 2,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let view = View::new(&doc, &state);
        let area = Rect::new(0, 0, 20, 3);
        let (x, y) = view.cursor_position(area).unwrap();
        assert_eq!(y, 1);
        // gutter (4 columns) + 2 characters into "beta"
        assert_eq!(x, 4 + 2);
    }

    /// A state with the window scrolled to `left_col`.
    fn scrolled(left_col: usize, cursor_col: usize) -> ViewState {
        ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 20,
            left_col,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        }
    }

    #[test]
    fn a_long_line_shows_the_columns_the_window_is_scrolled_to() {
        let doc = doc_of("long.txt", b"abcdefghijklmnopqrstuvwxyz0123456789\n");
        let state = scrolled(10, 0);
        let text = frame_text(&doc, &state, 24, 2);
        assert!(text.contains("klmnopqrstuvwxyz"), "{text}");
        assert!(!text.contains("abcdefghij"), "{text}");
    }

    #[test]
    fn a_line_shorter_than_the_window_is_blank() {
        let doc = doc_of("short.txt", b"abc\n");
        let state = scrolled(10, 0);
        let text = frame_text(&doc, &state, 24, 2);
        assert!(!text.contains("abc"), "{text}");
    }

    #[test]
    fn the_selection_is_painted_where_the_window_shows_it() {
        let doc = doc_of("scroll-selection.txt", b"abcdefghijklmnopqrstuvwxyz\n");
        let mut state = scrolled(10, 18);
        state.anchor = Some(Pos::new(0, 12));
        let backend = TestBackend::new(24, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View::new(&doc, &state);
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let is_selected =
            |x: u16| buffer[(4 + x, 0)].bg == Color::White && buffer[(4 + x, 0)].fg == Color::Black;
        // The selection runs from column 12 to 18, and the window starts at
        // column 10, so it is painted five columns further right.
        assert!(!is_selected(0), "column 10 is outside the selection");
        assert!(
            is_selected(2) && is_selected(7),
            "columns 12 to 18 are selected"
        );
        assert!(
            !is_selected(8),
            "column 18 is the cursor and is not painted"
        );
    }

    #[test]
    fn a_match_that_reaches_into_the_window_from_the_left_is_found() {
        let doc = doc_of("scroll-match.txt", b"abcdefghijklmnopqrstuvwxyz\n");
        let mut state = scrolled(12, 0);
        state.highlight_pattern = Some(Pattern::compile("klm").expect("a valid pattern"));
        let view = View::new(&doc, &state);
        // "klm" sits at columns 10 to 13 and the window starts at 12, so
        // only its last character is on screen.
        assert_eq!(view.match_ranges(0, 20), vec![(0, 1)]);
    }

    #[test]
    fn a_match_far_behind_the_left_edge_is_not_found() {
        let mut line = String::from("needle");
        line.push_str(&"x".repeat(200));
        let doc = doc_of("scroll-far.txt", format!("{line}\n").as_bytes());
        let mut state = scrolled(100, 0);
        state.highlight_pattern = Some(Pattern::compile("needle").expect("a valid pattern"));
        let view = View::new(&doc, &state);
        assert!(
            view.match_ranges(0, 20).is_empty(),
            "the match is further behind the edge than the scan looks"
        );
    }

    #[test]
    fn the_cursor_sits_at_its_own_column() {
        let doc = doc_of("cursor-scroll.txt", b"abcdefghijklmnopqrstuvwxyz\n");
        let state = scrolled(10, 15);
        let view = View::new(&doc, &state);
        let area = Rect::new(0, 0, 24, 2);
        let (x, y) = view.cursor_position(area).unwrap();
        assert_eq!(y, 0);
        // gutter (4 columns) + 5 columns into the window
        assert_eq!(x, 4 + 5);
    }

    #[test]
    fn a_cursor_outside_the_window_is_not_drawn() {
        let doc = doc_of("cursor-off.txt", b"abcdefghijklmnopqrstuvwxyz\n");
        let state = scrolled(10, 3);
        let view = View::new(&doc, &state);
        let area = Rect::new(0, 0, 24, 2);
        assert!(view.cursor_position(area).is_none());
    }

    /// The colours of a line, as the highlighter would report them.
    fn coloured(spans: &[(usize, usize, Color)]) -> Highlighted {
        let mut highlight = Highlighted::new();
        highlight.insert(
            0,
            spans
                .iter()
                .map(|(start, end, colour)| LineSpan {
                    start: *start,
                    end: *end,
                    style: Style::default().fg(*colour),
                })
                .collect(),
        );
        highlight
    }

    /// The frame of a view, drawn into a test terminal.
    fn draw(doc: &Doc, state: &ViewState, highlight: Option<&Highlighted>) -> Buffer {
        draw_with_theme(doc, state, highlight, ThemeColors::default())
    }

    fn draw_with_theme(
        doc: &Doc,
        state: &ViewState,
        highlight: Option<&Highlighted>,
        theme: ThemeColors,
    ) -> Buffer {
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View {
                    doc,
                    state,
                    highlight,
                    diff_signs: None,
                    theme,
                };
                f.render_widget(view, f.area());
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn wrap_state(width: u16, cursor_line: u64, cursor_col: usize, top_row: usize) -> ViewState {
        ViewState {
            top_line: 0,
            top_row,
            word_wrap: true,
            cursor_line,
            cursor_col,
            show_line_numbers: false,
            show_relative_numbers: false,
            width,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        }
    }

    #[test]
    fn a_wrapped_line_takes_more_than_one_row() {
        let doc = doc_of("wrap.txt", b"alpha beta gamma delta\nsecond\n");
        let state = wrap_state(10, 0, 0, 0);
        let text = frame_text(&doc, &state, 10, 5);
        let rows: Vec<&str> = text.split('\n').collect();
        assert_eq!(rows[0].trim_end(), "alpha beta");
        assert_eq!(rows[1].trim_end(), " gamma");
        assert_eq!(rows[2].trim_end(), "delta");
        assert_eq!(rows[3].trim_end(), "second", "the next line follows it");
    }

    #[test]
    fn wrap_keeps_words_whole_unless_one_is_wider_than_the_screen() {
        let doc = doc_of("wrap-long-word.txt", b"abcdefghijklmnop qr\n");
        let state = wrap_state(10, 0, 0, 0);
        let text = frame_text(&doc, &state, 10, 3);
        let rows: Vec<&str> = text.split('\n').collect();
        assert_eq!(rows[0], "abcdefghij");
        assert_eq!(rows[1].trim_end(), "klmnop qr");
    }

    #[test]
    fn a_wrapped_line_shows_its_number_once() {
        let doc = doc_of("wrap-gutter.txt", b"alpha beta gamma delta\nsecond\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: true,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 10,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let text = frame_text(&doc, &state, 16, 5);
        let rows: Vec<&str> = text.split('\n').collect();
        assert!(rows[0].trim_start().starts_with('1'), "{text}");
        assert!(
            rows[1].chars().take(4).all(|c| c == ' '),
            "the row under it is still the same line, so its gutter is blank: {text}"
        );
        assert!(rows[2].trim_start().starts_with('2'), "{text}");
    }

    #[test]
    fn the_cursor_sits_on_the_row_its_column_is_on() {
        let doc = doc_of("wrap-cursor.txt", b"alpha beta gamma\n");
        let state = wrap_state(10, 0, 12, 0);
        let view = View::new(&doc, &state);
        // Column 12 is on the second row, two columns in: the row starts at
        // column 10, which holds the space after `beta`.
        assert_eq!(view.cursor_position(Rect::new(0, 0, 10, 4)), Some((2, 1)));
    }

    #[test]
    fn the_cursor_at_the_end_of_a_full_row_moves_to_the_next_one() {
        let doc = doc_of("wrap-cursor-edge.txt", b"abcdefghij\n");
        let state = wrap_state(10, 0, 10, 0);
        let view = View::new(&doc, &state);
        assert_eq!(view.cursor_position(Rect::new(0, 0, 10, 4)), Some((0, 1)));
    }

    #[test]
    fn top_row_scrolls_within_a_wrapped_line() {
        let doc = doc_of("wrap-scroll.txt", b"alpha beta gamma delta\nsecond\n");
        let state = wrap_state(10, 1, 0, 2);
        let text = frame_text(&doc, &state, 10, 3);
        let rows: Vec<&str> = text.split('\n').collect();
        assert_eq!(
            rows[0].trim_end(),
            "delta",
            "the first two rows scrolled away"
        );
        assert_eq!(rows[1].trim_end(), "second");
    }

    #[test]
    fn the_cursor_position_is_measured_from_the_top_row() {
        let doc = doc_of("wrap-cursor-row.txt", b"alpha beta gamma delta\nsecond\n");
        let state = wrap_state(10, 1, 0, 2);
        let view = View::new(&doc, &state);
        assert_eq!(view.cursor_position(Rect::new(0, 0, 10, 4)), Some((0, 1)));
    }

    #[test]
    fn a_syntax_colour_reaches_the_screen() {
        let doc = doc_of("colours.txt", b"alpha beta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 1,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let highlight = coloured(&[(0, 5, Color::Red)]);
        let buffer = draw(&doc, &state, Some(&highlight));
        assert_eq!(buffer[(0, 0)].fg, Color::Red, "'alpha' is coloured");
        assert_eq!(
            buffer[(6, 0)].fg,
            ThemeColors::default().fg,
            "the text after the coloured run has default theme foreground"
        );
    }

    #[test]
    fn the_selection_wins_over_the_syntax_colour() {
        let doc = doc_of("colours-selection.txt", b"alpha beta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 4,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let highlight = coloured(&[(0, 5, Color::Red)]);
        let buffer = draw(&doc, &state, Some(&highlight));
        assert_eq!(
            buffer[(2, 0)].bg,
            Color::White,
            "the selection is painted with a white background"
        );
        assert_eq!(
            buffer[(2, 0)].fg,
            Color::Black,
            "and not in the colour of the syntax"
        );
    }

    #[test]
    fn the_search_match_wins_over_the_syntax_colour() {
        let doc = doc_of("colours-match.txt", b"alpha beta\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 1,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: Some(Pattern::compile("beta").expect("a valid pattern")),
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let highlight = coloured(&[(0, 5, Color::Red)]);
        let buffer = draw(&doc, &state, Some(&highlight));
        assert_eq!(
            buffer[(6, 0)].bg,
            Color::Yellow,
            "the match is painted in the match style"
        );
        assert_eq!(
            buffer[(6, 0)].fg,
            Color::Black,
            "and not in the colour of the syntax"
        );
    }

    #[test]
    fn a_colour_outside_the_window_is_not_painted() {
        let doc = doc_of("colours-scroll.txt", b"abcdefghijklmnopqrstuvwxyz\n");
        let mut state = scrolled(10, 0);
        state.show_line_numbers = false;
        // The coloured run is at the start of the line, which is off screen.
        let highlight = coloured(&[(0, 5, Color::Red)]);
        let buffer = draw(&doc, &state, Some(&highlight));
        assert_eq!(
            buffer[(0, 0)].fg,
            ThemeColors::default().fg,
            "the window does not show the coloured run"
        );
    }

    #[test]
    fn cursor_line_is_subtle_background_and_not_reversed() {
        let doc = doc_of("cursor-style.txt", b"hello world\nsecond line\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: false,
        };
        let buffer = draw(&doc, &state, None);
        assert_eq!(buffer[(0, 0)].bg, Color::Rgb(52, 55, 62));
        assert!(
            !buffer[(0, 0)].modifier.contains(Modifier::REVERSED),
            "the cursor line must not be reverse video"
        );
    }

    #[test]
    fn cursor_line_differs_from_selection() {
        let doc = doc_of("cursor-vs-selection.txt", b"abcdef\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 4,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: false,
        };
        let buffer = draw(&doc, &state, None);
        assert_eq!(buffer[(0, 0)].bg, Color::Rgb(52, 55, 62));
        assert_ne!(buffer[(0, 0)].bg, Color::White);
        assert_eq!(buffer[(2, 0)].bg, Color::White);
        assert_eq!(buffer[(2, 0)].fg, Color::Black);
    }

    #[test]
    fn gutter_cursor_line_is_bold_accent_not_reversed() {
        let doc = doc_of("gutter-cursor.txt", b"line 1\nline 2\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: true,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: false,
        };
        let buffer = draw(&doc, &state, None);
        assert!(!buffer[(2, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buffer[(2, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn monokai_renders_authentic_background_and_cursor_line() {
        let doc = doc_of("monokai-render.txt", b"let x = 42;\nlet y = 10;\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: false,
        };
        let buffer = draw_with_theme(&doc, &state, None, ThemeColors::MONOKAI);
        // Cursor line gets Monokai's #3e3d32
        assert_eq!(buffer[(0, 0)].bg, ThemeColors::MONOKAI.line_highlight);
        assert_eq!(buffer[(0, 0)].fg, ThemeColors::MONOKAI.fg);

        // Non-cursor line gets Monokai's #272822
        assert_eq!(buffer[(0, 1)].bg, ThemeColors::MONOKAI.bg);
        assert_eq!(buffer[(0, 1)].fg, ThemeColors::MONOKAI.fg);
    }

    #[test]
    fn solarized_light_renders_warm_cream_background_and_contrasting_selection() {
        let doc = doc_of("sol-light-render.txt", b"hello world\nsecond line\n");
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 5,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 0)),
            highlight_pattern: None,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
        };
        let buffer = draw_with_theme(&doc, &state, None, ThemeColors::SOLARIZED_LIGHT);
        // Selection is dark navy #073642 on light theme
        assert_eq!(buffer[(0, 0)].bg, ThemeColors::SOLARIZED_LIGHT.selection_bg);
        assert_eq!(buffer[(0, 0)].fg, ThemeColors::SOLARIZED_LIGHT.selection_fg);

        // Non-selected line 1 gets cream #fdf6e3 and dark text #657b83
        assert_eq!(buffer[(0, 1)].bg, ThemeColors::SOLARIZED_LIGHT.bg);
        assert_eq!(buffer[(0, 1)].fg, ThemeColors::SOLARIZED_LIGHT.fg);
    }

    #[test]
    fn scrollbar_hidden_when_content_fits_viewport() {
        let doc = doc_of("short.txt", b"line 1\nline 2\nline 3\n");
        let state = ViewState {
            top_line: 0,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
            ..ViewState::default()
        };
        let view = View::new(&doc, &state);
        assert!(!view.show_scrollbar(5, 20));
        assert_eq!(view.scrollbar_width(5, 20), 0);
    }

    #[test]
    fn scrollbar_renders_track_and_thumb_when_content_overflows() {
        let text = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let doc = doc_of("long.txt", text.as_bytes());
        let state = ViewState {
            top_line: 0,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
            show_line_numbers: false,
            ..ViewState::default()
        };
        let view = View::new(&doc, &state);
        assert!(view.show_scrollbar(5, 20));
        assert_eq!(view.scrollbar_width(5, 20), 1);

        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // At top_line = 0, row 0 on the rightmost column (19) has thumb '█'
        assert_eq!(buffer[(19, 0)].symbol(), "█");
        // Rows below the thumb have track '│'
        assert_eq!(buffer[(19, 4)].symbol(), "│");
    }

    #[test]
    fn scrollbar_thumb_moves_when_viewport_scrolls() {
        let text = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let doc = doc_of("scrolled.txt", text.as_bytes());
        let state = ViewState {
            top_line: 15,
            cursor_line: 15,
            show_scrollbar: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
            show_line_numbers: false,
            ..ViewState::default()
        };
        let view = View::new(&doc, &state);
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // At top_line = 15 (near bottom of 20 lines with height 5), row 4 has thumb '█'
        assert_eq!(buffer[(19, 4)].symbol(), "█");
        // Top row has track '│'
        assert_eq!(buffer[(19, 0)].symbol(), "│");
    }

    #[test]
    fn scrollbar_hidden_when_disabled_in_view_state() {
        let text = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let doc = doc_of("disabled.txt", text.as_bytes());
        let state = ViewState {
            top_line: 0,
            show_scrollbar: false,
            ..ViewState::default()
        };
        let view = View::new(&doc, &state);
        assert!(!view.show_scrollbar(5, 20));
        assert_eq!(view.scrollbar_width(5, 20), 0);
    }

    #[test]
    fn diff_signs_render_in_gutter() {
        let doc = doc_of("gutter-diff.txt", b"alpha\nbeta\ngamma\n");
        let mut signs = HashMap::new();
        signs.insert(0, DiffSign::Added);
        signs.insert(1, DiffSign::Modified);
        signs.insert(2, DiffSign::Deleted);

        let state = ViewState {
            top_line: 0,
            show_line_numbers: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: true,
            ..ViewState::default()
        };

        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View {
                    doc: &doc,
                    state: &state,
                    highlight: None,
                    diff_signs: Some(&signs),
                    theme: ThemeColors::default(),
                };
                f.render_widget(view, f.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // Column 3 is the gutter sign column (gutter_width is 4)
        assert_eq!(buffer[(3, 0)].symbol(), "▎");
        assert_eq!(buffer[(3, 0)].fg, Color::Green);
        assert_eq!(buffer[(3, 1)].symbol(), "▎");
        assert_eq!(buffer[(3, 1)].fg, Color::Yellow);
        assert_eq!(buffer[(3, 2)].symbol(), "▾");
        assert_eq!(buffer[(3, 2)].fg, Color::Red);
    }

    #[test]
    fn diff_signs_ascii_render_in_gutter() {
        let doc = doc_of("gutter-ascii.txt", b"alpha\nbeta\ngamma\n");
        let mut signs = HashMap::new();
        signs.insert(0, DiffSign::Added);
        signs.insert(1, DiffSign::Modified);
        signs.insert(2, DiffSign::Deleted);

        let state = ViewState {
            top_line: 0,
            show_line_numbers: true,
            show_git_diff: true,
            git_gutter_style: GitGutterStyle::Ascii,
            ..ViewState::default()
        };

        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View {
                    doc: &doc,
                    state: &state,
                    highlight: None,
                    diff_signs: Some(&signs),
                    theme: ThemeColors::default(),
                };
                f.render_widget(view, f.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(3, 0)].symbol(), "+");
        assert_eq!(buffer[(3, 0)].fg, Color::Green);
        assert_eq!(buffer[(3, 1)].symbol(), "~");
        assert_eq!(buffer[(3, 1)].fg, Color::Yellow);
        assert_eq!(buffer[(3, 2)].symbol(), "-");
        assert_eq!(buffer[(3, 2)].fg, Color::Red);
    }

    #[test]
    fn word_under_cursor_extraction() {
        let doc = doc_of("words.txt", b"let alpha_12 = beta + 42;\n");
        // 'let' (col 0..3)
        assert_eq!(word_under_cursor(&doc, 0, 0).as_deref(), Some("let"));
        assert_eq!(word_under_cursor(&doc, 0, 2).as_deref(), Some("let"));
        // space at col 3
        assert_eq!(word_under_cursor(&doc, 0, 3), None);
        // 'alpha_12' (col 4..12)
        assert_eq!(word_under_cursor(&doc, 0, 4).as_deref(), Some("alpha_12"));
        assert_eq!(word_under_cursor(&doc, 0, 10).as_deref(), Some("alpha_12"));
        // '=' at col 13
        assert_eq!(word_under_cursor(&doc, 0, 13), None);
        // '42' (pure number) at col 22
        assert_eq!(word_under_cursor(&doc, 0, 22), None);
        // ';' at col 24
        assert_eq!(word_under_cursor(&doc, 0, 24), None);

        // 1-letter word is ignored
        let doc_one = doc_of("one.txt", b"x = 1\n");
        assert_eq!(word_under_cursor(&doc_one, 0, 0), None);
    }

    #[test]
    fn find_word_occurrences_exact_boundary() {
        let text = "alpha alpha_beta beta_alpha alpha";
        let matches = find_word_occurrences(text, "alpha", 0, 35);
        // Only first "alpha" (0..5) and last "alpha" (28..33)
        assert_eq!(matches, vec![(0, 5), (28, 33)]);
    }

    #[test]
    fn word_under_cursor_highlights_in_rendered_buffer() {
        let doc = doc_of("hl.txt", b"hello world hello\n");
        let state = ViewState {
            top_line: 0,
            cursor_line: 0,
            cursor_col: 1, // inside first "hello"
            show_line_numbers: false,
            highlight_word: true,
            ..ViewState::default()
        };

        let backend = TestBackend::new(25, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View::new(&doc, &state);
                f.render_widget(view, f.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // Both "hello" occurrences (cols 0..5 and 12..17) should have UNDERLINED
        for col in 0..5 {
            assert!(
                buffer[(col, 0)].modifier.contains(Modifier::UNDERLINED),
                "col {col} should be underlined"
            );
        }
        for col in 12..17 {
            assert!(
                buffer[(col, 0)].modifier.contains(Modifier::UNDERLINED),
                "col {col} should be underlined"
            );
        }
        // "world" (cols 6..11) should NOT have UNDERLINED
        for col in 6..11 {
            assert!(
                !buffer[(col, 0)].modifier.contains(Modifier::UNDERLINED),
                "col {col} should not be underlined"
            );
        }

        // When highlight_word is false, no underline
        let state_off = ViewState {
            highlight_word: false,
            ..state.clone()
        };
        let backend_off = TestBackend::new(25, 1);
        let mut terminal_off = Terminal::new(backend_off).unwrap();
        terminal_off
            .draw(|f| {
                let view = View::new(&doc, &state_off);
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer_off = terminal_off.backend().buffer();
        for col in 0..5 {
            assert!(
                !buffer_off[(col, 0)].modifier.contains(Modifier::UNDERLINED),
                "col {col} should not be underlined when disabled"
            );
        }

        // When selection (anchor) is active, word highlight is disabled
        let state_sel = ViewState {
            anchor: Some(Pos { line: 0, col: 0 }),
            ..state.clone()
        };
        let backend_sel = TestBackend::new(25, 1);
        let mut terminal_sel = Terminal::new(backend_sel).unwrap();
        terminal_sel
            .draw(|f| {
                let view = View::new(&doc, &state_sel);
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer_sel = terminal_sel.backend().buffer();
        // col 12 (second "hello") is outside selection, should not be underlined
        assert!(!buffer_sel[(12, 0)].modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn persian_text_renders_shaped_and_reordered_in_terminal_buffer() {
        let doc = doc_of("fa.txt", "سلام\n".as_bytes());
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: false,
            show_git_diff: false,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: false,
        };
        let backend = TestBackend::new(20, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View::new(&doc, &state);
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // "سلام" is a single RTL word. After shaping + visual reorder the word's
        // characters are reversed so they paint left-to-right as right-to-left:
        // Logical:  س(FEB3 init)  ل(FEE0 med)  ا(FE8E fin)  م(FEE1 iso)
        // Visual:   م(FEE1)       ا(FE8E)       ل(FEE0)      س(FEB3)
        assert_eq!(buffer[(0, 0)].symbol(), "\u{FEE1}"); // م isolated
        assert_eq!(buffer[(1, 0)].symbol(), "\u{FE8E}"); // ا final
        assert_eq!(buffer[(2, 0)].symbol(), "\u{FEE0}"); // ل medial
        assert_eq!(buffer[(3, 0)].symbol(), "\u{FEB3}"); // س initial
    }

    #[test]
    fn cursor_position_on_persian_line_maps_to_visual_column() {
        let doc = doc_of("fa.txt", "سلام\n".as_bytes());
        let state = ViewState {
            top_line: 0,
            top_row: 0,
            word_wrap: false,
            cursor_line: 0,
            cursor_col: 0, // logical col 0 is 'س'
            show_line_numbers: false,
            show_relative_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
            show_scrollbar: false,
            show_git_diff: false,
            git_gutter_style: GitGutterStyle::Bar,
            highlight_word: false,
        };
        let view = View::new(&doc, &state);
        let pos = view.cursor_position(Rect::new(0, 0, 20, 2));
        // Logical 0 ('س') maps to visual column 3 (rightmost of the reversed word).
        assert_eq!(pos, Some((3, 0)));
    }
}
