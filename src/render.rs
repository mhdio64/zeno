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

use crate::doc::edit::Pos;
use crate::doc::Doc;
use crate::search::Pattern;

/// The visible state of the editor.
#[derive(Debug, Clone)]
pub struct ViewState {
    /// First line shown in the viewport.
    pub top_line: u64,
    /// Line the cursor is on.
    pub cursor_line: u64,
    /// Column the cursor is at, counted in characters.
    pub cursor_col: usize,
    /// Whether the line-number gutter is drawn.
    pub show_line_numbers: bool,
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
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            top_line: 0,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: true,
            width: 80,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
        }
    }
}

/// Renders the document viewport.
pub struct View<'a> {
    pub doc: &'a Doc,
    pub state: &'a ViewState,
}

impl View<'_> {
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

    /// Paints one line, in runs of equal style.
    ///
    /// The line is at most one screen wide, so deciding the style of every
    /// character and merging the runs afterwards is cheaper than working out
    /// where the selection and the search matches intersect.
    fn styled_spans(
        &self,
        text: &str,
        base: Style,
        selection: Option<(usize, usize)>,
        matches: &[(usize, usize)],
    ) -> Vec<Span<'static>> {
        let chars: Vec<char> = text.chars().collect();
        let mut styles = vec![base; chars.len()];
        for (start, end) in matches {
            for col in *start..(*end).min(chars.len()) {
                styles[col] = match_style();
            }
        }
        // The selection wins over the search match: it is what the user has
        // just made, and one highlight is easier to read than two.
        if let Some((from, to)) = selection {
            for col in from..to.min(chars.len()) {
                styles[col] = selection_style();
            }
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut start = 0;
        for col in 1..=chars.len() {
            if col == chars.len() || styles[col] != styles[start] {
                spans.push(Span::styled(
                    chars[start..col].iter().collect::<String>(),
                    styles[start],
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
        let row = self.state.cursor_line.checked_sub(self.state.top_line)?;
        if row >= area.height as u64 {
            return None;
        }
        let gutter = self.gutter_width();
        let body_width = area.width.saturating_sub(gutter);
        // `cursor_col` is the character column: the editor keeps it inside
        // the line, and walking the line to count it again would cost a
        // keystroke's worth of work on a line with a million characters.
        let col = self.state.cursor_col;
        let left = self.state.left_col;
        if col < left || col - left >= body_width as usize {
            return None;
        }
        let x = area.x + gutter + (col - left) as u16;
        Some((x, area.y + row as u16))
    }
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

fn gutter_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn cursor_line_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

fn selection_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// The style a search match is painted in.
///
/// Not reverse video: the selection and the cursor line already use that,
/// and two reverse-video ranges on one line cannot be told apart.
fn match_style() -> Style {
    Style::default().fg(Color::Black).bg(Color::Yellow)
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
        let body_x = area.x + gutter;
        let body_width = area.width.saturating_sub(gutter);
        if body_width == 0 {
            return;
        }

        let mut gutter_lines: Vec<Line> = Vec::with_capacity(area.height as usize);
        let mut body_lines: Vec<Line> = Vec::with_capacity(area.height as usize);

        for row in 0..area.height as u64 {
            let line_no = self.state.top_line + row;
            let is_cursor = line_no == self.state.cursor_line;

            if self.state.show_line_numbers {
                let label = format!("{:>width$}", line_no + 1, width = gutter as usize - 1);
                let style = if is_cursor {
                    gutter_style().add_modifier(Modifier::REVERSED)
                } else {
                    gutter_style()
                };
                gutter_lines.push(Line::from(Span::styled(label, style)));
            }

            // Only the characters that can fit on screen are painted. A file
            // with one enormous line must not cost more to draw than a file
            // with ten thousand short ones.
            let text = self.doc.line_text(line_no).unwrap_or_default();
            let left = self.state.left_col;
            // The head of the line reaches back before the window, so that a
            // match crossing the left edge is still found.
            let behind = left.saturating_sub(MATCH_LOOKBEHIND);
            let head = head_of(&text, behind);
            let window = skip_chars(head, left - behind);
            let clipped: String = window.chars().take(body_width as usize).collect();
            // While a selection is showing, the cursor line loses its own
            // highlight: two reverse-video ranges on one line are impossible
            // to tell apart, and the selection is the more useful of the two.
            let base = if is_cursor && self.state.anchor.is_none() {
                cursor_line_style()
            } else {
                Style::default()
            };
            let painted = clipped.chars().count();
            // The selection is named in the line's own columns; the window
            // shows a slice of them.
            let selection = self.selection_span(line_no).map(|(from, to)| {
                let from = from.max(left).min(left + painted);
                let to = to.max(left).min(left + painted);
                (from - left, to - left)
            });
            let matches = self.match_ranges_in(head, left - behind, painted);
            body_lines.push(Line::from(self.styled_spans(
                &clipped,
                base,
                selection,
                &matches,
            )));
        }

        if self.state.show_line_numbers {
            let gutter_area = Rect {
                x: area.x,
                y: area.y,
                width: gutter,
                height: area.height,
            };
            Paragraph::new(Text::from(gutter_lines)).render(gutter_area, buf);
        }

        let body_area = Rect {
            x: body_x,
            y: area.y,
            width: body_width,
            height: area.height,
        };
        Paragraph::new(Text::from(body_lines)).render(body_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn doc_of(name: &str, contents: &[u8]) -> Doc {
        use std::io::Write;
        let dir = std::env::temp_dir().join("zeno-render-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        Doc::open(&path).unwrap()
    }

    fn frame_text(doc: &Doc, state: &ViewState, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View { doc, state };
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
    fn renders_gutter_and_text() {
        let doc = doc_of("gutter.txt", b"alpha\nbeta\n");
        let state = ViewState {
            top_line: 0,
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: true,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
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
            cursor_line: 2,
            cursor_col: 0,
            show_line_numbers: true,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
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
            cursor_line: 0,
            cursor_col: 4,
            show_line_numbers: true,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
        };
        let view = View { doc: &doc, state: &state };
        assert_eq!(view.selection_span(0), Some((1, 4)));
        assert_eq!(view.selection_span(1), None, "the other line is not selected");

        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View { doc: &doc, state: &state };
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let reversed = |x: u16| {
            buffer[(4 + x, 0)]
                .modifier
                .contains(Modifier::REVERSED)
        };
        assert!(!reversed(0), "'a' is outside the selection");
        assert!(reversed(1) && reversed(2) && reversed(3), "'bcd' is selected");
        assert!(!reversed(4), "'e' is past the selection");
    }

    #[test]
    fn a_selection_across_lines_covers_the_middle_lines_whole() {
        let doc = doc_of("selection-lines.txt", b"one\ntw\nthree\n");
        let state = ViewState {
            top_line: 0,
            cursor_line: 2,
            cursor_col: 2,
            show_line_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
        };
        let view = View { doc: &doc, state: &state };
        assert_eq!(view.selection_span(0), Some((1, 3)));
        assert_eq!(view.selection_span(1), Some((0, 2)));
        assert_eq!(view.selection_span(2), Some((0, 2)));
    }

    #[test]
    fn a_selection_ending_at_a_line_start_paints_nothing_on_it() {
        let doc = doc_of("selection-edge.txt", b"one\ntwo\n");
        let state = ViewState {
            top_line: 0,
            cursor_line: 1,
            cursor_col: 0,
            show_line_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 1)),
            highlight_pattern: None,
        };
        let view = View { doc: &doc, state: &state };
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
            cursor_line: 1,
            cursor_col: 0,
            show_line_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: Some(Pattern::compile("beta").expect("a valid pattern")),
        };
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View { doc: &doc, state: &state };
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // "alpha " is six characters, so the match starts at column 6.
        assert_eq!(buffer[(6, 0)].bg, Color::Yellow);
        assert_eq!(buffer[(9, 0)].bg, Color::Yellow);
        assert_eq!(
            buffer[(5, 0)].bg,
            Color::Reset,
            "the text before the match is not painted"
        );
    }

    #[test]
    fn the_selection_wins_over_the_search_match() {
        let doc = doc_of("highlight-selection.txt", b"alpha beta\n");
        let state = ViewState {
            top_line: 0,
            cursor_line: 0,
            cursor_col: 8,
            show_line_numbers: false,
            width: 20,
            left_col: 0,
            anchor: Some(Pos::new(0, 6)),
            highlight_pattern: Some(Pattern::compile("beta").expect("a valid pattern")),
        };
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let view = View { doc: &doc, state: &state };
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert!(
            buffer[(6, 0)].modifier.contains(Modifier::REVERSED),
            "the selection is painted reversed"
        );
        assert_eq!(
            buffer[(6, 0)].bg,
            Color::Reset,
            "and not in the match style"
        );
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
            cursor_line: 0,
            cursor_col: 0,
            show_line_numbers: false,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: Some(Pattern::compile("needle").expect("a valid pattern")),
        };
        let view = View { doc: &doc, state: &state };
        assert!(view.match_ranges(0, 20).is_empty());
    }

    #[test]
    fn cursor_position_follows_the_line() {
        let doc = doc_of("cursor.txt", b"alpha\nbeta\n");
        let state = ViewState {
            top_line: 0,
            cursor_line: 1,
            cursor_col: 2,
            show_line_numbers: true,
            width: 20,
            left_col: 0,
            anchor: None,
            highlight_pattern: None,
        };
        let view = View { doc: &doc, state: &state };
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
            cursor_line: 0,
            cursor_col,
            show_line_numbers: true,
            width: 20,
            left_col,
            anchor: None,
            highlight_pattern: None,
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
                let view = View { doc: &doc, state: &state };
                f.render_widget(view, f.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let reversed = |x: u16| {
            buffer[(4 + x, 0)]
                .modifier
                .contains(Modifier::REVERSED)
        };
        // The selection runs from column 12 to 18, and the window starts at
        // column 10, so it is painted five columns further right.
        assert!(!reversed(0), "column 10 is outside the selection");
        assert!(reversed(2) && reversed(7), "columns 12 to 18 are selected");
        assert!(!reversed(8), "column 18 is the cursor and is not painted");
    }

    #[test]
    fn a_match_that_reaches_into_the_window_from_the_left_is_found() {
        let doc = doc_of("scroll-match.txt", b"abcdefghijklmnopqrstuvwxyz\n");
        let mut state = scrolled(12, 0);
        state.highlight_pattern = Some(Pattern::compile("klm").expect("a valid pattern"));
        let view = View { doc: &doc, state: &state };
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
        let view = View { doc: &doc, state: &state };
        assert!(
            view.match_ranges(0, 20).is_empty(),
            "the match is further behind the edge than the scan looks"
        );
    }

    #[test]
    fn the_cursor_sits_at_its_own_column() {
        let doc = doc_of("cursor-scroll.txt", b"abcdefghijklmnopqrstuvwxyz\n");
        let state = scrolled(10, 15);
        let view = View { doc: &doc, state: &state };
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
        let view = View { doc: &doc, state: &state };
        let area = Rect::new(0, 0, 24, 2);
        assert!(view.cursor_position(area).is_none());
    }
}