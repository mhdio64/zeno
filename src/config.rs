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
/// with `~/.config` as the fallback (spec §6).
pub fn config_path() -> Option<PathBuf> {
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
"#,
        );
        assert!(settings.word_wrap);
        assert!(!settings.line_numbers);
        assert!(settings.relative_line_numbers);
        assert_eq!(settings.tab_width, 2);
        assert!(!settings.auto_pair);
        assert!(settings.autosave);
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
theme = "zeno-dark"
tab_width = 3   # trailing
"#,
        );
        assert_eq!(settings.tab_width, 3);
        assert_eq!(settings, Settings::default().with_tab_width(3));
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

    impl Settings {
        fn with_tab_width(mut self, width: usize) -> Self {
            self.tab_width = width;
            self
        }
    }
}
