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

    /// Screen position of the cursor, as `(x, y)` relative to `area`.
    pub fn cursor_position(&self, area: Rect) -> Option<(u16, u16)> {
        let row = self.state.cursor_line.checked_sub(self.state.top_line)?;
        if row >= area.height as u64 {
            return None;
        }
        let text = self.doc.line_text(self.state.cursor_line)?;
        let col = text.chars().take(self.state.cursor_col).count();
        let x = area.x + self.gutter_width() + col as u16;
        let max_x = area.right().saturating_sub(1);
        Some((x.min(max_x), area.y + row as u16))
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
            let clipped: String = text.chars().take(body_width as usize).collect();
            // While a selection is showing, the cursor line loses its own
            // highlight: two reverse-video ranges on one line are impossible
            // to tell apart, and the selection is the more useful of the two.
            let base = if is_cursor && self.state.anchor.is_none() {
                cursor_line_style()
            } else {
                Style::default()
            };
            body_lines.push(Line::from(match self.selection_span(line_no) {
                None => vec![Span::styled(clipped, base)],
                Some((from, to)) => {
                    let painted = clipped.chars().count();
                    let from = from.min(painted);
                    let to = to.min(painted);
                    if to <= from {
                        vec![Span::styled(clipped, base)]
                    } else {
                        vec![
                            Span::styled(clipped.chars().take(from).collect::<String>(), base),
                            Span::styled(
                                clipped.chars().skip(from).take(to - from).collect::<String>(),
                                selection_style(),
                            ),
                            Span::styled(clipped.chars().skip(to).collect::<String>(), base),
                        ]
                    }
                }
            }));
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
    fn cursor_position_follows_the_line() {
        let doc = doc_of("cursor.txt", b"alpha\nbeta\n");
        let state = ViewState {
            top_line: 0,
            cursor_line: 1,
            cursor_col: 2,
            show_line_numbers: true,
            width: 20,
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
}