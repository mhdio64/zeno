//! Curated themes and the interactive theme picker.
//!
//! Provides a lean selection of the most popular themes, plus a high-contrast
//! theme designed for low-vision and accessibility, and an interactive panel
//! to switch themes live.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// One theme available in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeItem {
    /// Identifier passed to syntect / Highlighter
    pub id: &'static str,
    /// Human-readable name
    pub label: &'static str,
    /// Short description
    pub description: &'static str,
}

/// The curated theme list: popular dark, light, and high-contrast accessibility themes.
pub const CURATED_THEMES: &[ThemeItem] = &[
    ThemeItem {
        id: "Dark Modern",
        label: "Dark Modern",
        description: "VS Code default dark",
    },
    ThemeItem {
        id: "base16-ocean.dark",
        label: "Ocean Dark",
        description: "Dark slate",
    },
    ThemeItem {
        id: "Solarized (dark)",
        label: "Solarized Dark",
        description: "Classic warm teal",
    },
    ThemeItem {
        id: "Monokai",
        label: "Monokai",
        description: "Vibrant high-contrast dark",
    },
    ThemeItem {
        id: "Solarized (light)",
        label: "Solarized Light",
        description: "Clean warm light",
    },
    ThemeItem {
        id: "High Contrast",
        label: "High Contrast",
        description: "Accessibility / low-vision (WCAG AAA)",
    },
];

/// The cohesive color scheme for editor chrome, gutter, cursor line, and document body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColors {
    /// Document canvas background color
    pub bg: Color,
    /// Default plain text foreground color
    pub fg: Color,
    /// Background color of the active cursor line
    pub line_highlight: Color,
    /// Background color of selected text
    pub selection_bg: Color,
    /// Foreground color of selected text
    pub selection_fg: Color,
    /// Line number gutter text color
    pub gutter_fg: Color,
    /// Line number gutter background color
    pub gutter_bg: Color,
    /// Status bar background color
    pub status_bg: Color,
    /// Status bar foreground color
    pub status_fg: Color,
    /// Whether this is a dark theme (for contrast decisions)
    pub is_dark: bool,
}

impl ThemeColors {
    pub const DARK_MODERN: Self = Self {
        bg: Color::Rgb(24, 24, 24),
        fg: Color::Rgb(224, 224, 224),
        line_highlight: Color::Rgb(34, 34, 34),
        selection_bg: Color::Rgb(38, 79, 120),
        selection_fg: Color::White,
        gutter_fg: Color::Rgb(125, 130, 140),
        gutter_bg: Color::Rgb(24, 24, 24),
        status_bg: Color::Rgb(16, 16, 16),
        status_fg: Color::Rgb(215, 215, 215),
        is_dark: true,
    };

    pub const OCEAN_DARK: Self = Self {
        bg: Color::Rgb(43, 48, 59),
        fg: Color::Rgb(192, 197, 206),
        line_highlight: Color::Rgb(52, 55, 62),
        selection_bg: Color::White,
        selection_fg: Color::Black,
        gutter_fg: Color::DarkGray,
        gutter_bg: Color::Rgb(43, 48, 59),
        status_bg: Color::Rgb(28, 28, 34),
        status_fg: Color::Rgb(192, 197, 206),
        is_dark: true,
    };

    pub const SOLARIZED_DARK: Self = Self {
        bg: Color::Rgb(0, 43, 54),
        fg: Color::Rgb(131, 148, 150),
        line_highlight: Color::Rgb(7, 54, 66),
        selection_bg: Color::White,
        selection_fg: Color::Black,
        gutter_fg: Color::Rgb(88, 110, 117),
        gutter_bg: Color::Rgb(0, 43, 54),
        status_bg: Color::Rgb(7, 54, 66),
        status_fg: Color::Rgb(147, 161, 161),
        is_dark: true,
    };

    pub const MONOKAI: Self = Self {
        bg: Color::Rgb(39, 40, 34),
        fg: Color::Rgb(248, 248, 242),
        line_highlight: Color::Rgb(62, 61, 50),
        selection_bg: Color::White,
        selection_fg: Color::Black,
        gutter_fg: Color::Rgb(117, 113, 94),
        gutter_bg: Color::Rgb(39, 40, 34),
        status_bg: Color::Rgb(30, 31, 28),
        status_fg: Color::Rgb(248, 248, 242),
        is_dark: true,
    };

    pub const SOLARIZED_LIGHT: Self = Self {
        bg: Color::Rgb(253, 246, 227),
        fg: Color::Rgb(101, 123, 131),
        line_highlight: Color::Rgb(238, 232, 213),
        selection_bg: Color::Rgb(7, 54, 66),
        selection_fg: Color::Rgb(253, 246, 227),
        gutter_fg: Color::Rgb(147, 161, 161),
        gutter_bg: Color::Rgb(253, 246, 227),
        status_bg: Color::Rgb(238, 232, 213),
        status_fg: Color::Rgb(88, 110, 117),
        is_dark: false,
    };

    pub const HIGH_CONTRAST: Self = Self {
        bg: Color::Black,
        fg: Color::White,
        line_highlight: Color::Rgb(35, 35, 35),
        selection_bg: Color::White,
        selection_fg: Color::Black,
        gutter_fg: Color::Yellow,
        gutter_bg: Color::Black,
        status_bg: Color::Black,
        status_fg: Color::White,
        is_dark: true,
    };

    /// Resolves curated theme colors by name or ID, falling back to Dark Modern.
    pub fn for_theme(theme_name: Option<&str>) -> Self {
        match theme_name {
            Some(name)
                if name.eq_ignore_ascii_case("Dark Modern")
                    || name.eq_ignore_ascii_case("dark-modern")
                    || name.eq_ignore_ascii_case("Dark (Visual Studio)")
                    || name.eq_ignore_ascii_case("VS Code Dark")
                    || name.eq_ignore_ascii_case("vscode") =>
            {
                Self::DARK_MODERN
            }
            Some(name)
                if name.eq_ignore_ascii_case("base16-ocean.dark")
                    || name.eq_ignore_ascii_case("Ocean Dark")
                    || name.eq_ignore_ascii_case("ocean-dark") =>
            {
                Self::OCEAN_DARK
            }
            Some(name)
                if name.eq_ignore_ascii_case("Solarized (dark)")
                    || name.eq_ignore_ascii_case("solarized-dark")
                    || name.eq_ignore_ascii_case("Solarized Dark") =>
            {
                Self::SOLARIZED_DARK
            }
            Some(name)
                if name.eq_ignore_ascii_case("Monokai") || name.eq_ignore_ascii_case("monokai") =>
            {
                Self::MONOKAI
            }
            Some(name)
                if name.eq_ignore_ascii_case("Solarized (light)")
                    || name.eq_ignore_ascii_case("solarized-light")
                    || name.eq_ignore_ascii_case("Solarized Light") =>
            {
                Self::SOLARIZED_LIGHT
            }
            Some(name)
                if name.eq_ignore_ascii_case("High Contrast")
                    || name.eq_ignore_ascii_case("high-contrast") =>
            {
                Self::HIGH_CONTRAST
            }
            _ => Self::DARK_MODERN,
        }
    }

    /// Color for the scrollbar track line.
    pub fn scrollbar_track(&self) -> Color {
        self.gutter_fg
    }

    /// Color for the visual scrollbar thumb.
    pub fn scrollbar_thumb(&self) -> Color {
        if !self.is_dark {
            Color::Rgb(101, 123, 131)
        } else if self.bg == Color::Black && self.fg == Color::White {
            Color::White
        } else {
            self.fg
        }
    }

    /// Background color for occurrences of the word under the cursor.
    pub fn word_match_bg(&self) -> Color {
        if !self.is_dark {
            Color::Rgb(225, 220, 200)
        } else if self.bg == Color::Black && self.fg == Color::White {
            Color::Rgb(40, 40, 40)
        } else {
            Color::Rgb(65, 72, 85)
        }
    }

    /// Foreground color for matching bracket highlighting.
    pub fn matching_bracket_fg(&self) -> Color {
        if !self.is_dark {
            Color::Rgb(38, 139, 210)
        } else if self.bg == Color::Black && self.fg == Color::White {
            Color::Yellow
        } else {
            Color::Rgb(102, 217, 239)
        }
    }

    /// Background color for matching bracket highlighting.
    pub fn matching_bracket_bg(&self) -> Color {
        if !self.is_dark {
            Color::Rgb(215, 230, 245)
        } else if self.bg == Color::Black && self.fg == Color::White {
            Color::Rgb(50, 50, 50)
        } else {
            Color::Rgb(70, 80, 95)
        }
    }
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self::DARK_MODERN
    }
}

/// The theme picker overlay state: query, items, matches, and selection.
#[derive(Debug, Clone)]
pub struct ThemePicker {
    query: String,
    items: Vec<ThemeItem>,
    matches: Vec<usize>,
    selected: usize,
    active_theme: Option<String>,
}

impl Default for ThemePicker {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ThemePicker {
    pub fn new(active_theme: Option<&str>) -> Self {
        let items: Vec<ThemeItem> = CURATED_THEMES.to_vec();
        let matches = (0..items.len()).collect();

        let active_id = active_theme.unwrap_or("Dark Modern");
        let initial_selected = items
            .iter()
            .position(|item| {
                item.id.eq_ignore_ascii_case(active_id)
                    || item.label.eq_ignore_ascii_case(active_id)
            })
            .unwrap_or(0);

        Self {
            query: String::new(),
            items,
            matches,
            selected: initial_selected,
            active_theme: active_theme.map(str::to_owned),
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    pub fn items(&self) -> &[ThemeItem] {
        &self.items
    }

    pub fn selection(&self) -> usize {
        self.selected
    }

    pub fn selected_theme(&self) -> Option<&ThemeItem> {
        self.matches.get(self.selected).map(|&idx| &self.items[idx])
    }

    pub fn insert(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if self.selected + 1 < self.matches.len() {
            self.selected += 1;
        }
    }

    fn refilter(&mut self) {
        if self.query.is_empty() {
            self.matches = (0..self.items.len()).collect();
            self.selected = 0;
            return;
        }

        let query = self.query.to_lowercase();
        let mut scored: Vec<(u8, usize)> = Vec::new();

        for (idx, item) in self.items.iter().enumerate() {
            let label = item.label.to_lowercase();
            let desc = item.description.to_lowercase();
            let id = item.id.to_lowercase();

            if label.contains(&query) || id.contains(&query) {
                scored.push((0, idx));
            } else if desc.contains(&query) {
                scored.push((1, idx));
            }
        }

        scored.sort_by_key(|&(tier, idx)| (tier, idx));
        self.matches = scored.into_iter().map(|(_, idx)| idx).collect();
        self.selected = 0;
    }
}

fn heading() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn body() -> Style {
    Style::default()
}

fn selected_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Renders the theme picker panel into screen lines.
pub fn theme_lines(picker: &ThemePicker, height: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("select theme: ", heading()),
        Span::styled(picker.query().to_owned(), body()),
        Span::styled(
            format!(" ({}/{})", picker.matches().len(), picker.items().len()),
            Style::default().fg(Color::DarkGray),
        ),
    ])];

    if picker.matches().is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching themes",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    let room = height.saturating_sub(lines.len()).max(1);
    let first = picker.selection().saturating_sub(room - 1);
    let active_id = picker.active_theme.as_deref().unwrap_or("Dark Modern");

    for (offset, &idx) in picker.matches().iter().enumerate().skip(first).take(room) {
        let item = &picker.items()[idx];
        let is_selected = offset == picker.selection();
        let is_active =
            item.id.eq_ignore_ascii_case(active_id) || item.label.eq_ignore_ascii_case(active_id);
        let style = if is_selected {
            selected_style()
        } else {
            body()
        };

        let indicator = if is_selected { "> " } else { "  " };
        let active_indicator = if is_active { " (active)" } else { "" };

        let desc_style = if is_selected {
            style
        } else {
            Style::default().fg(Color::DarkGray)
        };

        lines.push(Line::from(vec![
            Span::styled(indicator, style),
            Span::styled(
                format!("{:<20}", format!("{}{active_indicator}", item.label)),
                if is_active && !is_selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    style
                },
            ),
            Span::styled(format!("  {}", item.description), desc_style),
        ]));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_themes_has_all_six_items() {
        assert_eq!(CURATED_THEMES.len(), 6);
        assert!(CURATED_THEMES.iter().any(|t| t.id == "Dark Modern"));
        assert!(CURATED_THEMES.iter().any(|t| t.id == "Monokai"));
        assert!(CURATED_THEMES.iter().any(|t| t.id == "High Contrast"));
        assert!(CURATED_THEMES.iter().any(|t| t.id == "Solarized (dark)"));
    }

    #[test]
    fn typing_filters_themes() {
        let mut picker = ThemePicker::new(None);
        assert_eq!(picker.matches().len(), 6);

        for c in "mono".chars() {
            picker.insert(c);
        }
        assert_eq!(picker.matches().len(), 1);
        assert_eq!(picker.selected_theme().unwrap().id, "Monokai");

        picker.backspace();
        picker.backspace();
        picker.backspace();
        picker.backspace();
        assert_eq!(picker.matches().len(), 6);

        for c in "high".chars() {
            picker.insert(c);
        }
        // "high" matches both "Monokai" ("Vibrant high-contrast dark") and "High Contrast"
        assert!(!picker.matches().is_empty());
        assert!(picker.selected_theme().is_some());
    }

    #[test]
    fn navigation_and_selection() {
        let mut picker = ThemePicker::new(Some("Solarized (dark)"));
        // Starts on Solarized Dark (index 2: Dark Modern = 0, Ocean Dark = 1, Solarized Dark = 2)
        assert_eq!(picker.selection(), 2);

        picker.up();
        assert_eq!(picker.selection(), 1);
        picker.up();
        assert_eq!(picker.selection(), 0);
        picker.up(); // Clamps at top
        assert_eq!(picker.selection(), 0);

        picker.down();
        assert_eq!(picker.selection(), 1);
        picker.down();
        assert_eq!(picker.selection(), 2);
    }

    #[test]
    fn theme_lines_renders_header_and_items() {
        let picker = ThemePicker::new(Some("Monokai"));
        let lines = theme_lines(&picker, 10);
        assert!(lines[0].to_string().contains("select theme:  (6/6)"));
        assert!(
            lines
                .iter()
                .any(|l| l.to_string().contains("Monokai (active)"))
        );
        assert!(lines.iter().any(|l| l.to_string().contains("Dark Modern")));
        assert!(
            lines
                .iter()
                .any(|l| l.to_string().contains("High Contrast"))
        );
    }

    #[test]
    fn theme_colors_resolve_curated_palettes() {
        let dark_modern = ThemeColors::for_theme(Some("Dark Modern"));
        assert_eq!(dark_modern, ThemeColors::DARK_MODERN);
        assert_eq!(dark_modern.bg, Color::Rgb(24, 24, 24));
        assert_eq!(dark_modern.fg, Color::Rgb(224, 224, 224));
        assert_eq!(dark_modern.line_highlight, Color::Rgb(34, 34, 34));
        assert_eq!(dark_modern.selection_bg, Color::Rgb(38, 79, 120));
        assert!(dark_modern.is_dark);

        let ocean = ThemeColors::for_theme(Some("base16-ocean.dark"));
        assert_eq!(ocean, ThemeColors::OCEAN_DARK);
        assert!(ocean.is_dark);

        let solarized_dark = ThemeColors::for_theme(Some("Solarized (dark)"));
        assert_eq!(solarized_dark, ThemeColors::SOLARIZED_DARK);
        assert_eq!(solarized_dark.bg, Color::Rgb(0, 43, 54));
        assert_eq!(solarized_dark.line_highlight, Color::Rgb(7, 54, 66));
        assert!(solarized_dark.is_dark);

        let monokai = ThemeColors::for_theme(Some("Monokai"));
        assert_eq!(monokai, ThemeColors::MONOKAI);
        assert_eq!(monokai.bg, Color::Rgb(39, 40, 34));
        assert_eq!(monokai.fg, Color::Rgb(248, 248, 242));
        assert_eq!(monokai.line_highlight, Color::Rgb(62, 61, 50));
        assert!(monokai.is_dark);

        let solarized_light = ThemeColors::for_theme(Some("Solarized (light)"));
        assert_eq!(solarized_light, ThemeColors::SOLARIZED_LIGHT);
        assert_eq!(solarized_light.bg, Color::Rgb(253, 246, 227));
        assert_eq!(solarized_light.fg, Color::Rgb(101, 123, 131));
        assert!(!solarized_light.is_dark);
        // Selection must contrast against light background
        assert_ne!(solarized_light.selection_bg, Color::White);
        assert_eq!(solarized_light.selection_bg, Color::Rgb(7, 54, 66));

        let hc = ThemeColors::for_theme(Some("High Contrast"));
        assert_eq!(hc, ThemeColors::HIGH_CONTRAST);
        assert_eq!(hc.bg, Color::Black);
        assert_eq!(hc.fg, Color::White);
        assert!(hc.is_dark);

        // Fallback
        assert_eq!(ThemeColors::for_theme(None), ThemeColors::DARK_MODERN);
        assert_eq!(
            ThemeColors::for_theme(Some("unknown")),
            ThemeColors::DARK_MODERN
        );
    }
}
