//! `zeno.toml`: the settings and key bindings read at startup.
//!
//! The file is small by design — `key = value` lines under `[settings]` and
//! `[keys]` — so it is read by a parser written for it rather than by a
//! general TOML library. That keeps the first frame free of a dependency and
//! of tables the editor would ignore anyway (performance budget, ADR 0007).
//!
//! Nothing here can fail the editor. A missing file is the defaults, and a
//! line that cannot be understood is skipped: a typo in a config file should
//! not be the reason an editor will not start.

use std::path::{Path, PathBuf};

/// The settings a file can change, with the values the editor uses when it
/// says nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Soft-wrap long lines. Off: a scroll is cheaper than a layout.
    pub word_wrap: bool,
    /// Draw the line-number gutter.
    pub line_numbers: bool,
    /// Count the gutter from the cursor instead of from the top of the file.
    pub relative_line_numbers: bool,
    /// Columns one indent level is worth.
    pub tab_width: usize,
    /// Complete `(` `[` `{` and quotes as they are typed.
    pub auto_pair: bool,
    /// Write the file when the editor has been idle for a moment.
    pub autosave: bool,
    /// The syntax highlighting theme name. `None` uses the default dark theme.
    pub theme: Option<String>,
    /// Draw the visual scrollbar on the right edge.
    pub scrollbar: bool,
    /// Automatically trim trailing spaces and tabs from lines upon save.
    pub trim_trailing_whitespace: bool,
    /// Whether git diff gutter signs are shown.
    pub git_diff: bool,
    /// Visual style of git diff gutter symbols ("bar" or "ascii").
    pub git_gutter_style: String,
    /// Whether occurrences of the word under cursor are highlighted in the viewport.
    pub highlight_word: bool,
    /// Whether matching bracket pairs are highlighted in the viewport.
    pub highlight_bracket: bool,
    /// Whether Zen (distraction-free) mode is enabled.
    pub zen_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            word_wrap: false,
            line_numbers: true,
            relative_line_numbers: false,
            tab_width: 4,
            auto_pair: true,
            autosave: false,
            theme: None,
            scrollbar: true,
            trim_trailing_whitespace: false,
            git_diff: true,
            git_gutter_style: "bar".to_string(),
            highlight_word: true,
            highlight_bracket: true,
            zen_mode: false,
        }
    }
}

/// One `[[keys.bindings]]` entry, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The keys, as written: `ctrl+s`, `alt+z`, `f1`.
    pub keys: String,
    /// The command, as written: `save`. Bound to a [`crate::sequence::Command`]
    /// by the keymap, which is the only place that knows what a name means.
    pub command: String,
}

/// The `[keys]` section: the leader and whatever chords the file adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyConfig {
    /// The leader key, as written. `None` keeps `Ctrl+Space`.
    pub leader: Option<String>,
    pub bindings: Vec<Binding>,
}

/// Everything a config file says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub settings: Settings,
    pub keys: KeyConfig,
}

/// Where the config file lives, if the platform says.
///
/// Windows keeps it under `%APPDATA%`; everything else follows the XDG rule
/// with `~/.config` as the fallback (spec §6). A custom path can be supplied
/// via the `ZENO_CONFIG_PATH` environment variable.
pub fn config_path() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("ZENO_CONFIG_PATH") {
        return Some(PathBuf::from(custom));
    }
    Some(config_dir()?.join("zeno.toml"))
}

/// The directory the config lives in, which also holds `syntaxes/` and
/// `themes/` (spec §6, ADR 0008).
pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    Some(base.join("zeno"))
}

/// Where the user's own `.sublime-syntax` files are read from.
pub fn syntaxes_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("syntaxes"))
}

/// Where the user's own `.tmTheme` files are read from.
pub fn themes_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("themes"))
}

/// Reads the config, or the defaults when there is no file.
///
/// A file that cannot be read at all is also the defaults: the reason to
/// start is the document, not the configuration.
pub fn load() -> Config {
    match config_path() {
        Some(path) => from_file(&path),
        None => Config::default(),
    }
}

/// Reads the config from one file.
pub fn from_file(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_config(&text),
        Err(_) => Config::default(),
    }
}

/// Reads the settings out of the text of a config file.
pub fn parse(text: &str) -> Settings {
    parse_config(text).settings
}

/// Reads everything a config file says.
pub fn parse_config(text: &str) -> Config {
    let mut config = Config::default();
    let mut section = String::new();
    let mut pending: Option<Binding> = None;
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        // `[[keys.bindings]]` opens a new entry; `[settings]` and `[keys]`
        // select a table.
        if let Some(name) = line
            .strip_prefix("[[")
            .and_then(|rest| rest.strip_suffix("]]"))
        {
            if name.trim() == "keys.bindings" {
                if let Some(binding) = pending.take() {
                    config.keys.bindings.push(binding);
                }
                pending = Some(Binding {
                    keys: String::new(),
                    command: String::new(),
                });
                section = "keys.bindings".to_owned();
            }
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if let Some(binding) = pending.take() {
                config.keys.bindings.push(binding);
            }
            section = name.trim().to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match section.as_str() {
            "keys" => {
                if key == "leader" {
                    config.keys.leader = Some(value.to_owned());
                }
                continue;
            }
            "keys.bindings" => {
                let Some(binding) = pending.as_mut() else {
                    continue;
                };
                match key {
                    "keys" => binding.keys = value.to_owned(),
                    "command" => binding.command = value.to_owned(),
                    _ => {}
                }
                continue;
            }
            _ => {}
        }
        if section != "settings" {
            continue;
        }
        parse_setting(&mut config.settings, key, value);
    }
    if let Some(binding) = pending.take() {
        config.keys.bindings.push(binding);
    }
    config
}

fn parse_setting(settings: &mut Settings, key: &str, value: &str) {
    match key {
        "word_wrap" => set_bool(&mut settings.word_wrap, value),
        "line_numbers" => set_bool(&mut settings.line_numbers, value),
        "relative_line_numbers" => set_bool(&mut settings.relative_line_numbers, value),
        "auto_pair" => set_bool(&mut settings.auto_pair, value),
        "autosave" => set_bool(&mut settings.autosave, value),
        "scrollbar" => set_bool(&mut settings.scrollbar, value),
        "trim_trailing_whitespace" => set_bool(&mut settings.trim_trailing_whitespace, value),
        "git_diff" | "git_gutter" => set_bool(&mut settings.git_diff, value),
        "highlight_word" | "highlight_word_under_cursor" => {
            set_bool(&mut settings.highlight_word, value)
        }
        "highlight_bracket" | "highlight_matching_bracket" | "matching_bracket" => {
            set_bool(&mut settings.highlight_bracket, value)
        }
        "zen_mode" | "zen" | "distraction_free" => set_bool(&mut settings.zen_mode, value),
        "git_gutter_style" => {
            let trimmed = value.trim().trim_matches('"');
            if !trimmed.is_empty() {
                settings.git_gutter_style = trimmed.to_owned();
            }
        }
        "theme" => {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                settings.theme = Some(trimmed.to_owned());
            }
        }
        "tab_width" => {
            // A width of zero would make an indent invisible and an unindent
            // an infinite loop; one is the narrowest useful.
            if let Ok(width) = value.parse::<usize>() {
                settings.tab_width = width.clamp(1, 16);
            }
        }
        _ => {}
    }
}

fn set_bool(field: &mut bool, value: &str) {
    if let Some(parsed) = as_bool(value) {
        *field = parsed;
    }
}

fn as_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Removes a `#` comment, keeping one that is inside a quoted value.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (index, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..index],
            _ => {}
        }
    }
    line
}

/// Updates or creates the `theme` setting in a config text, preserving comments,
/// whitespace, and all other sections.
pub fn update_theme_in_text(text: &str, theme_name: &str) -> String {
    if text.trim().is_empty() {
        return format!("[settings]\ntheme = \"{theme_name}\"\n");
    }

    let is_crlf = text.contains("\r\n");
    let ending = if is_crlf { "\r\n" } else { "\n" };

    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut current_section = String::new();
    let mut settings_header_idx: Option<usize> = None;
    let mut theme_line_idx: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = strip_comment(line).trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section_name = trimmed.trim_start_matches('[').trim_end_matches(']').trim();
            current_section = section_name.to_owned();
            if current_section == "settings" && settings_header_idx.is_none() {
                settings_header_idx = Some(i);
            }
            continue;
        }

        if current_section == "settings"
            && let Some((k, _)) = trimmed.split_once('=')
            && k.trim() == "theme"
        {
            theme_line_idx = Some(i);
            break;
        }
    }

    if let Some(idx) = theme_line_idx {
        lines[idx] = format!("theme = \"{theme_name}\"");
    } else if let Some(hdr_idx) = settings_header_idx {
        lines.insert(hdr_idx + 1, format!("theme = \"{theme_name}\""));
    } else {
        if !lines.is_empty() && !lines.last().is_none_or(|l| l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push("[settings]".to_owned());
        lines.push(format!("theme = \"{theme_name}\""));
    }

    let mut result = lines.join(ending);
    result.push_str(ending);
    result
}

/// Saves the chosen theme to a specific configuration file path.
pub fn save_theme_to(path: &Path, theme_name: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let current_text = std::fs::read_to_string(path).unwrap_or_default();
    let updated_text = update_theme_in_text(&current_text, theme_name);
    std::fs::write(path, updated_text)
}

/// Saves the chosen theme to the user's config file (`zeno.toml`), creating
/// the parent directory and file if needed.
pub fn save_theme(theme_name: &str) -> std::io::Result<()> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not determine config directory",
        )
    })?;
    save_theme_to(&path, theme_name)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_file_is_the_defaults() {
        let settings = Settings::default();
        assert!(!settings.word_wrap);
        assert!(settings.line_numbers);
        assert!(!settings.relative_line_numbers);
        assert_eq!(settings.tab_width, 4);
        assert!(settings.auto_pair);
        assert!(!settings.autosave);
        assert!(settings.scrollbar);
    }

    #[test]
    fn settings_are_read_from_the_settings_table() {
        let settings = parse(
            r#"
[settings]
word_wrap = true
line_numbers = false
relative_line_numbers = true
tab_width = 2
auto_pair = false
autosave = true
scrollbar = false
"#,
        );
        assert!(settings.word_wrap);
        assert!(!settings.line_numbers);
        assert!(settings.relative_line_numbers);
        assert_eq!(settings.tab_width, 2);
        assert!(!settings.auto_pair);
        assert!(settings.autosave);
        assert!(!settings.scrollbar);
    }

    #[test]
    fn only_the_settings_table_counts() {
        let settings = parse(
            r#"
[keys]
word_wrap = true

[settings]
tab_width = 8
"#,
        );
        assert!(!settings.word_wrap, "a value outside [settings] is not one");
        assert_eq!(settings.tab_width, 8);
    }

    #[test]
    fn comments_and_unknown_keys_are_skipped() {
        let settings = parse(
            r#"
# a leading comment
[settings]  # the table
extra_option = "zeno-dark"
tab_width = 3   # trailing
"#,
        );
        assert_eq!(settings.tab_width, 3);
        assert_eq!(settings, Settings::default().with_tab_width(3));
    }

    #[test]
    fn theme_setting_is_read() {
        let settings = parse(
            r#"
[settings]
theme = "Solarized (dark)"
"#,
        );
        assert_eq!(settings.theme.as_deref(), Some("Solarized (dark)"));
    }

    #[test]
    fn trim_trailing_whitespace_setting_is_read() {
        let settings = parse(
            r#"
[settings]
trim_trailing_whitespace = true
"#,
        );
        assert!(settings.trim_trailing_whitespace);
    }

    #[test]
    fn the_themes_dir_is_next_to_the_config() {
        if let Some(dir) = themes_dir() {
            assert!(
                dir.ends_with(Path::new("zeno").join("themes")),
                "{}",
                dir.display()
            );
        }
    }

    #[test]
    fn a_hash_inside_a_quoted_value_is_not_a_comment() {
        let settings = parse(
            r#"
[settings]
theme = "dark#1"
tab_width = 5
"#,
        );
        assert_eq!(settings.tab_width, 5);
    }

    #[test]
    fn nonsense_values_leave_the_default() {
        let settings = parse(
            r#"
[settings]
tab_width = "wide"
word_wrap = maybe
"#,
        );
        assert_eq!(settings.tab_width, 4);
        assert!(!settings.word_wrap);
    }

    #[test]
    fn an_impossible_tab_width_is_clamped() {
        let settings = parse("[settings]\ntab_width = 0\n");
        assert_eq!(settings.tab_width, 1, "an indent must be visible");
        let settings = parse("[settings]\ntab_width = 99\n");
        assert_eq!(settings.tab_width, 16);
    }

    #[test]
    fn a_missing_file_is_the_defaults() {
        let path = std::env::temp_dir().join("zeno-no-such-config.toml");
        let _ = std::fs::remove_file(&path);
        assert_eq!(from_file(&path), Config::default());
    }

    #[test]
    fn the_config_path_includes_the_directory() {
        // The base directory depends on the machine, but the tail is fixed.
        if let Some(path) = config_path() {
            assert!(path.ends_with("zeno/zeno.toml"), "{}", path.display());
        }
    }

    #[test]
    fn update_theme_in_empty_config() {
        let updated = update_theme_in_text("", "Monokai");
        let parsed = parse(&updated);
        assert_eq!(parsed.theme.as_deref(), Some("Monokai"));
    }

    #[test]
    fn update_theme_replaces_existing_theme_and_keeps_other_settings() {
        let original = r#"# user configuration
[settings]
line_numbers = true
theme = "base16-ocean.dark" # old theme
tab_width = 2
"#;
        let updated = update_theme_in_text(original, "Solarized (light)");
        assert!(updated.contains("theme = \"Solarized (light)\""));
        assert!(!updated.contains("base16-ocean.dark"));
        assert!(updated.contains("# user configuration"));
        assert!(updated.contains("line_numbers = true"));
        assert!(updated.contains("tab_width = 2"));

        let config = parse_config(&updated);
        assert_eq!(config.settings.theme.as_deref(), Some("Solarized (light)"));
        assert_eq!(config.settings.tab_width, 2);
        assert!(config.settings.line_numbers);
    }

    #[test]
    fn update_theme_inserts_into_settings_section_if_missing() {
        let original = r#"[settings]
tab_width = 8
"#;
        let updated = update_theme_in_text(original, "Monokai");
        assert!(updated.contains("theme = \"Monokai\""));
        assert!(updated.contains("tab_width = 8"));

        let config = parse_config(&updated);
        assert_eq!(config.settings.theme.as_deref(), Some("Monokai"));
        assert_eq!(config.settings.tab_width, 8);
    }

    #[test]
    fn update_theme_appends_settings_section_if_none_existed() {
        let original = r#"[keys]
leader = "ctrl+space"
"#;
        let updated = update_theme_in_text(original, "High Contrast");
        assert!(updated.contains("[settings]"));
        assert!(updated.contains("theme = \"High Contrast\""));
        assert!(updated.contains("leader = \"ctrl+space\""));

        let config = parse_config(&updated);
        assert_eq!(config.settings.theme.as_deref(), Some("High Contrast"));
        assert_eq!(config.keys.leader.as_deref(), Some("ctrl+space"));
    }

    #[test]
    fn save_theme_to_writes_and_roundtrips() {
        let dir = std::env::temp_dir().join("zeno-config-test");
        let path = dir.join("zeno.toml");
        let _ = std::fs::remove_file(&path);

        save_theme_to(&path, "Solarized (dark)").expect("write theme to succeed");
        let config = from_file(&path);
        assert_eq!(config.settings.theme.as_deref(), Some("Solarized (dark)"));

        // Update it again to Monokai
        save_theme_to(&path, "Monokai").expect("update theme to succeed");
        let config2 = from_file(&path);
        assert_eq!(config2.settings.theme.as_deref(), Some("Monokai"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn git_diff_and_gutter_style_settings_are_read() {
        let text = "[settings]\ngit_diff = false\ngit_gutter_style = \"ascii\"\n";
        let config = parse(text);
        assert!(!config.git_diff);
        assert_eq!(config.git_gutter_style, "ascii");
    }

    #[test]
    fn highlight_word_setting_is_read() {
        let text = "[settings]\nhighlight_word = false\n";
        let config = parse(text);
        assert!(!config.highlight_word);

        let text_alias = "[settings]\nhighlight_word_under_cursor = true\n";
        let config_alias = parse(text_alias);
        assert!(config_alias.highlight_word);
    }

    #[test]
    fn highlight_bracket_setting_is_read() {
        let text = "[settings]\nhighlight_bracket = false\n";
        let config = parse(text);
        assert!(!config.highlight_bracket);

        let text_alias = "[settings]\nhighlight_matching_bracket = true\n";
        let config_alias = parse(text_alias);
        assert!(config_alias.highlight_bracket);
    }

    #[test]
    fn zen_mode_setting_is_read() {
        let text = "[settings]\nzen_mode = true\n";
        let config = parse(text);
        assert!(config.zen_mode);

        let text_alias = "[settings]\ndistraction_free = true\n";
        let config_alias = parse(text_alias);
        assert!(config_alias.zen_mode);
    }

    impl Settings {
        fn with_tab_width(mut self, width: usize) -> Self {
            self.tab_width = width;
            self
        }
    }
}
