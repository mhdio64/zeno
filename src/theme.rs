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
        id: "base16-ocean.dark",
        label: "Ocean Dark",
        description: "Default dark slate",
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

        let active_id = active_theme.unwrap_or("base16-ocean.dark");
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
    let active_id = picker
        .active_theme
        .as_deref()
        .unwrap_or("base16-ocean.dark");

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
    fn curated_themes_has_all_five_items() {
        assert_eq!(CURATED_THEMES.len(), 5);
        assert!(CURATED_THEMES.iter().any(|t| t.id == "Monokai"));
        assert!(CURATED_THEMES.iter().any(|t| t.id == "High Contrast"));
        assert!(CURATED_THEMES.iter().any(|t| t.id == "Solarized (dark)"));
    }

    #[test]
    fn typing_filters_themes() {
        let mut picker = ThemePicker::new(None);
        assert_eq!(picker.matches().len(), 5);

        for c in "mono".chars() {
            picker.insert(c);
        }
        assert_eq!(picker.matches().len(), 1);
        assert_eq!(picker.selected_theme().unwrap().id, "Monokai");

        picker.backspace();
        picker.backspace();
        picker.backspace();
        picker.backspace();
        assert_eq!(picker.matches().len(), 5);

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
        // Starts on Solarized Dark
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
        assert!(lines[0].to_string().contains("select theme:  (5/5)"));
        assert!(
            lines
                .iter()
                .any(|l| l.to_string().contains("Monokai (active)"))
        );
        assert!(
            lines
                .iter()
                .any(|l| l.to_string().contains("High Contrast"))
        );
    }
}
