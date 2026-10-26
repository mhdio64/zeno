//! Indentation: what one level is worth, and where that answer comes from.
//!
//! Three sources, in this order (spec §4.4):
//!
//! 1. `.editorconfig` in the file's directory tree — an explicit statement of
//!    intent that outranks anything guessed.
//! 2. The file's own content — a file indented with tabs should be edited with
//!    tabs, whatever the settings say.
//! 3. `zeno.toml`, or the built-in four spaces.
//!
//! The answer is one value, [`IndentStyle`], decided once at startup and used
//! by every command that adds or removes indentation: `Tab`, `Leader ]`,
//! `Leader [`, `Shift+Tab`, and the extra level an opening brace starts on the
//! next line. One value is what stops the editor from writing two spaces here
//! and a tab there.

use std::path::{Path, PathBuf};

use crate::config::Settings;
use crate::doc::Doc;

/// How one indent level is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    /// A real tab character.
    Tabs,
    /// This many spaces.
    Spaces(usize),
}

impl IndentStyle {
    /// The text one level is made of.
    pub fn unit(self) -> String {
        match self {
            IndentStyle::Tabs => "\t".to_owned(),
            IndentStyle::Spaces(width) => " ".repeat(width),
        }
    }

    /// How many columns one level is worth on screen.
    pub fn width(self) -> usize {
        match self {
            IndentStyle::Tabs => 1,
            IndentStyle::Spaces(width) => width,
        }
    }

    pub fn is_tabs(self) -> bool {
        matches!(self, IndentStyle::Tabs)
    }
}

/// The indentation a file uses when nothing says otherwise.
pub const DEFAULT: IndentStyle = IndentStyle::Spaces(4);

/// The leading whitespace of a line.
pub fn leading(line: &str) -> &str {
    let end = line
        .char_indices()
        .find(|(_, c)| *c != ' ' && *c != '\t')
        .map_or(line.len(), |(at, _)| at);
    &line[..end]
}

/// One indent level back off the front of a line.
///
/// Returns the shortened line and how many characters went, which is what
/// moves a cursor that was sitting in the removed run.
pub fn dedent(line: &str, style: IndentStyle) -> (&str, usize) {
    if let Some(rest) = line.strip_prefix('\t') {
        return (rest, 1);
    }
    let width = match style {
        // A tab-indented line that starts with spaces is either mixed or
        // copied; taking one level's worth of columns is the least surprising
        // thing to do with it.
        IndentStyle::Tabs => DEFAULT.width(),
        IndentStyle::Spaces(width) => width,
    };
    let spaces = leading(line).chars().count().min(width);
    (&line[spaces..], spaces)
}

/// Decides the file's indentation from its content.
///
/// Tabs win when any line is indented with a tab and none with spaces; spaces
/// win otherwise, and their width is the greatest common measure of the
/// indents the file actually uses, which is what makes a file indented 2-2-4
/// read as two. `None` when the file has no indented line at all.
pub fn detect<'a>(lines: impl Iterator<Item = &'a str>) -> Option<IndentStyle> {
    let mut tabs = 0usize;
    let mut spaces: Vec<usize> = Vec::new();
    for line in lines {
        let indent = leading(line);
        if indent.is_empty() || line.trim().is_empty() {
            continue;
        }
        if indent.contains('\t') {
            tabs += 1;
        } else {
            spaces.push(indent.len());
        }
    }
    if tabs == 0 && spaces.is_empty() {
        return None;
    }
    if tabs > spaces.len() {
        return Some(IndentStyle::Tabs);
    }
    if spaces.is_empty() {
        return Some(IndentStyle::Tabs);
    }
    Some(IndentStyle::Spaces(space_width(&spaces)))
}

/// The width a set of space indents implies.
fn space_width(indents: &[usize]) -> usize {
    let mut width = indents[0];
    for indent in indents {
        width = gcd(width, *indent);
    }
    // A greatest common measure of one means the file is not consistently
    // indented (a one-space alignment, say), and a width of one is not an
    // indent anybody chose. Two is the narrowest that reads as a level.
    width.clamp(2, 8)
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Reads `.editorconfig` files from the file's directory up to the root and
/// returns what they say about indentation for this file.
pub fn from_editorconfig(path: &Path) -> Option<IndentStyle> {
    let file = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut directories: Vec<PathBuf> = Vec::new();
    let mut at = file.parent();
    while let Some(directory) = at {
        directories.push(directory.to_path_buf());
        at = directory.parent();
    }
    // Farthest first, so a nearer file overrides a farther one.
    directories.reverse();

    let mut style = None;
    for directory in directories {
        let config = directory.join(".editorconfig");
        let Ok(text) = std::fs::read_to_string(&config) else {
            continue;
        };
        let relative = file.strip_prefix(&directory).unwrap_or(&file);
        if let Some(found) = editorconfig_style(&text, relative) {
            style = Some(found);
        }
        if has_root_property(&text) {
            break;
        }
    }
    style
}

/// What one `.editorconfig` says about `relative`, in its own terms.
fn editorconfig_style(text: &str, relative: &Path) -> Option<IndentStyle> {
    let relative = relative.to_string_lossy().replace('\\', "/");
    let mut style: Option<String> = None;
    let mut size: Option<usize> = None;
    let mut matching = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(pattern) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            matching = section_matches(pattern, &relative);
            continue;
        }
        if !matching {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().to_ascii_lowercase();
        match key.trim().to_ascii_lowercase().as_str() {
            "indent_style" => style = Some(value),
            "indent_size" => size = value.parse().ok(),
            "tab_width" if size.is_none() => {
                size = value.parse().ok();
            }
            _ => {}
        }
    }
    match style.as_deref() {
        Some("tab") => Some(IndentStyle::Tabs),
        Some("space") => Some(IndentStyle::Spaces(size.unwrap_or(4).clamp(1, 16))),
        // `indent_size` on its own says nothing about which character to use.
        _ => None,
    }
}

fn has_root_property(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim().to_ascii_lowercase();
        line.strip_prefix("root")
            .is_some_and(|rest| rest.trim().trim_start_matches('=').trim() == "true")
    })
}

/// Whether an editorconfig section pattern applies to `relative`.
///
/// The subset that .editorconfig files actually use: `*` for anything inside a
/// name, `**` for anything including separators, `?` for one character, and
/// `{a,b}` for alternatives. A pattern with no separator applies to the file
/// name anywhere in the tree, which is what makes `[*.rs]` mean every Rust
/// file.
fn section_matches(pattern: &str, relative: &str) -> bool {
    let mut regex = String::from("(?i)^");
    if !pattern.contains('/') {
        regex.push_str("(?:.*/)?");
    }
    let mut rest = pattern;
    while let Some(c) = rest.chars().next() {
        match c {
            '*' => {
                if rest.starts_with("**") {
                    regex.push_str(".*");
                    rest = &rest[2..];
                } else {
                    regex.push_str("[^/]*");
                    rest = &rest[1..];
                }
                continue;
            }
            '?' => regex.push_str("[^/]"),
            '{' => {
                if let Some(end) = rest.find('}') {
                    let alternatives = &rest[1..end];
                    regex.push_str("(?:");
                    regex.push_str(&alternatives.replace(',', "|"));
                    regex.push(')');
                    rest = &rest[end + 1..];
                    continue;
                }
                regex.push('\\');
                regex.push('{');
            }
            c if "\\.+()|[]^$".contains(c) => {
                regex.push('\\');
                regex.push(c);
            }
            c => regex.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    regex.push('$');
    regex::Regex::new(&regex)
        .map(|regex| regex.is_match(relative))
        .unwrap_or(false)
}

/// The indentation to use for a file: editorconfig, then its content, then the
/// settings.
pub fn resolve(path: &Path, doc: &Doc, settings: &Settings) -> IndentStyle {
    if let Some(style) = from_editorconfig(path) {
        return style;
    }
    // Only the first screenful is read: a style is a property of how a file
    // is written, and the first lines are as representative as the last.
    const SAMPLE: u64 = 200;
    let sample: Vec<String> = (0..SAMPLE)
        .map_while(|line| doc.line_text(line).map(|text| text.into_owned()))
        .collect();
    if let Some(style) = detect(sample.iter().map(String::as_str)) {
        return style;
    }
    IndentStyle::Spaces(settings.tab_width.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_leading_whitespace_is_what_before_the_first_character() {
        assert_eq!(leading("    indented"), "    ");
        assert_eq!(leading("\t\ttabbed"), "\t\t");
        assert_eq!(leading("plain"), "");
        assert_eq!(leading("   "), "   ");
    }

    #[test]
    fn a_dedent_takes_one_level() {
        assert_eq!(dedent("    deep", IndentStyle::Spaces(4)), ("deep", 4));
        assert_eq!(dedent("      deep", IndentStyle::Spaces(4)), ("  deep", 4));
        assert_eq!(dedent("\tdeep", IndentStyle::Spaces(4)), ("deep", 1));
        assert_eq!(dedent("  short", IndentStyle::Spaces(4)), ("short", 2));
        assert_eq!(dedent("plain", IndentStyle::Spaces(4)), ("plain", 0));
    }

    #[test]
    fn spaces_are_detected_with_their_width() {
        let text = "fn main() {\n    one();\n    if x {\n        two();\n    }\n}\n";
        assert_eq!(
            detect(text.lines()),
            Some(IndentStyle::Spaces(4)),
            "the width is the greatest common measure of the indents"
        );
        let two = "if x:\n  one()\n  two()\n";
        assert_eq!(detect(two.lines()), Some(IndentStyle::Spaces(2)));
    }

    #[test]
    fn tabs_win_when_the_file_is_indented_with_tabs() {
        let text = "fn main() {\n\tone();\n\ttwo();\n}\n";
        assert_eq!(detect(text.lines()), Some(IndentStyle::Tabs));
    }

    #[test]
    fn a_file_with_no_indentation_has_no_style() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(detect(text.lines()), None);
    }

    #[test]
    fn an_editorconfig_says_what_it_means() {
        let text = "root = true\n\n[*]\nindent_style = space\nindent_size = 2\n\n[*.go]\nindent_style = tab\n";
        assert_eq!(
            editorconfig_style(text, Path::new("main.rs")),
            Some(IndentStyle::Spaces(2))
        );
        assert_eq!(
            editorconfig_style(text, Path::new("main.go")),
            Some(IndentStyle::Tabs)
        );
    }

    #[test]
    fn a_later_section_overrides_an_earlier_one() {
        let text = "[*]\nindent_style = space\nindent_size = 4\n\n[*.py]\nindent_size = 2\n";
        assert_eq!(
            editorconfig_style(text, Path::new("x.py")),
            Some(IndentStyle::Spaces(2))
        );
    }

    #[test]
    fn a_pattern_with_a_separator_matches_the_whole_path() {
        let text = "[src/**.rs]\nindent_style = tab\n";
        assert_eq!(
            editorconfig_style(text, Path::new("src/deep/in.rs")),
            Some(IndentStyle::Tabs)
        );
        assert_eq!(editorconfig_style(text, Path::new("tests/in.rs")), None);
    }

    #[test]
    fn a_brace_list_of_extensions_matches_any_of_them() {
        let text = "[*.{js,ts}]\nindent_style = space\nindent_size = 2\n";
        assert_eq!(
            editorconfig_style(text, Path::new("a.ts")),
            Some(IndentStyle::Spaces(2))
        );
        assert_eq!(
            editorconfig_style(text, Path::new("a.js")),
            Some(IndentStyle::Spaces(2))
        );
        assert_eq!(editorconfig_style(text, Path::new("a.rs")), None);
    }

    #[test]
    fn comments_and_unknown_properties_are_ignored() {
        let text = "# a comment\n[*]\nmax_line_length = 80\nindent_style = tab\n";
        assert_eq!(
            editorconfig_style(text, Path::new("a.rs")),
            Some(IndentStyle::Tabs)
        );
    }

    #[test]
    fn root_stops_the_search() {
        assert!(has_root_property("root = true\n[*]\n"));
        assert!(!has_root_property("[*]\nindent_size = 2\n"));
    }

    #[test]
    fn one_level_is_written_as_the_style_says() {
        assert_eq!(IndentStyle::Spaces(2).unit(), "  ");
        assert_eq!(IndentStyle::Tabs.unit(), "\t");
        assert_eq!(IndentStyle::Spaces(4).width(), 4);
    }
}
