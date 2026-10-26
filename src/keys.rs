//! The keymap: what a key press means, and how a config file describes one.
//!
//! The default keymap lives in this crate's code and in `docs/keymap.md`;
//! `zeno.toml` can change the leader key and add chords. A chord is written the
//! way a person says it — `ctrl+s`, `alt+z`, `f1`, `ctrl+space` — and is
//! matched against the *key*, not the character it produces, because a Persian
//! or Russian layout rewrites `Ctrl+S` into `Ctrl+س` and a binding matched by
//! character would be dead on those layouts while plain typing kept working
//! (ADR 0010).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::KeyConfig;
use crate::sequence::Command;

/// A key press to match against: a physical key and its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl Chord {
    /// Parses `ctrl+s`, `alt+shift+z`, `f1`, `ctrl+space`, `esc`.
    ///
    /// `None` for anything it cannot make sense of, which is how a typo in a
    /// config file becomes a line the editor ignores rather than one that
    /// breaks a binding nobody meant to change.
    pub fn parse(text: &str) -> Option<Self> {
        let lowered = text.trim().to_ascii_lowercase();
        if lowered.is_empty() {
            return None;
        }
        // `+` is itself a key, so a chord that ends in one is separated
        // first: `ctrl++` is control and the plus sign, not control and
        // nothing.
        let (key_text, prefix) = if lowered == "+" {
            ("+", "")
        } else if let Some(prefix) = lowered.strip_suffix("++") {
            ("+", prefix)
        } else {
            let (head, key) = lowered.rsplit_once('+').unwrap_or(("", lowered.as_str()));
            (key, head)
        };
        let mut modifiers = KeyModifiers::empty();
        if !prefix.is_empty() {
            for part in prefix.split('+') {
                match part {
                    "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                    "alt" | "option" => modifiers |= KeyModifiers::ALT,
                    "shift" => modifiers |= KeyModifiers::SHIFT,
                    // A word in front of the key that is not a modifier is a
                    // chord that cannot be meant.
                    _ => return None,
                }
            }
        }
        let code = parse_key(key_text)?;
        Some(Self { code, modifiers })
    }

    /// Whether a key event is this chord.
    ///
    /// Letters and digits go through the physical key so a layout cannot
    /// break them; everything else compares the key itself.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        let ctrl = self.modifiers.contains(KeyModifiers::CONTROL);
        let alt = self.modifiers.contains(KeyModifiers::ALT);
        let shift = self.modifiers.contains(KeyModifiers::SHIFT);
        if event.modifiers.contains(KeyModifiers::CONTROL) != ctrl
            || event.modifiers.contains(KeyModifiers::ALT) != alt
        {
            // A VT-style terminal delivers `Ctrl+S` as the bare control byte
            // `\u{13}` with its modifiers already gone; that is still Ctrl+S,
            // and anything with Alt is a different press entirely.
            if !ctrl || event.modifiers.contains(KeyModifiers::ALT) {
                return false;
            }
            let KeyCode::Char(want) = self.code else {
                return false;
            };
            let KeyCode::Char(got) = event.code else {
                return false;
            };
            // A printable character without its modifier is not the chord: a
            // plain `s` is typing, not `Ctrl+S`.
            if !want.is_ascii_alphabetic() || shift || got.is_alphanumeric() {
                return false;
            }
            return chord_letter(event.code) == Some(want.to_ascii_lowercase());
        }
        match self.code {
            KeyCode::Char(want) if want.is_ascii_alphanumeric() => {
                // Letters are compared through the physical key, and Shift is
                // compared separately: `Ctrl+Shift+S` is a different chord
                // from `Ctrl+S`, though both arrive as a letter.
                chord_letter(event.code) == Some(want.to_ascii_lowercase())
                    && event.modifiers.contains(KeyModifiers::SHIFT) == shift
            }
            KeyCode::Char(want) => event.code == KeyCode::Char(want),
            code => {
                // A shifted function key or arrow is a different key press;
                // for characters the modifier is already in the character.
                event.code == code && event.modifiers.contains(KeyModifiers::SHIFT) == shift
            }
        }
    }
}

fn parse_key(text: &str) -> Option<KeyCode> {
    let mut chars = text.chars();
    if let (Some(single), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(single));
    }
    Some(match text {
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "page_up" => KeyCode::PageUp,
        "pagedown" | "page_down" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        other => {
            let number: u8 = other.strip_prefix('f')?.parse().ok()?;
            match number {
                1..=12 => KeyCode::F(number),
                _ => return None,
            }
        }
    })
}

/// The keymap in force: one leader, and whatever chords the config added.
#[derive(Debug, Clone)]
pub struct Keymap {
    leader: Chord,
    chords: Vec<(Chord, Command)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            leader: Chord {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::CONTROL,
            },
            chords: Vec::new(),
        }
    }
}

impl Keymap {
    /// Builds the keymap a config file describes, on top of the defaults.
    pub fn from_config(config: &KeyConfig) -> Self {
        let mut keymap = Self::default();
        if let Some(text) = &config.leader
            && let Some(chord) = Chord::parse(text)
        {
            keymap.leader = chord;
        }
        for binding in &config.bindings {
            let Some(chord) = Chord::parse(&binding.keys) else {
                continue;
            };
            // A key with no modifier that types itself is a plain key, and
            // the first layer is where those live: binding `j` here would
            // stop `j` from ever being typed. Function keys are already
            // chords, so they are allowed through unmodified.
            if chord.modifiers.is_empty() && !matches!(chord.code, KeyCode::F(_)) {
                continue;
            }
            let Some(command) = Command::from_name(&binding.command) else {
                continue;
            };
            keymap.chords.push((chord, command));
        }
        keymap
    }

    pub fn leader(&self) -> Chord {
        self.leader
    }

    /// Whether this key opens a sequence.
    ///
    /// `Ctrl+Space` keeps its two terminal aliases — a terminal that drops the
    /// modifiers reports the NUL byte, and some report `Ctrl+@`, which is the
    /// same byte by another name — but only while the leader really is that
    /// key: a rebindable leader should not answer to the old one's aliases.
    pub fn is_leader(&self, key: &KeyEvent) -> bool {
        if self.leader.matches(key) {
            return true;
        }
        let default_leader = Chord {
            code: KeyCode::Char(' '),
            modifiers: KeyModifiers::CONTROL,
        };
        if self.leader != default_leader {
            return false;
        }
        match key.code {
            KeyCode::Char('@') => key.modifiers.contains(KeyModifiers::CONTROL),
            KeyCode::Char('\0') | KeyCode::Null => true,
            _ => false,
        }
    }

    /// The command a chord is bound to, if any.
    ///
    /// A user binding is looked at before the built-in chords, so a config file
    /// can move a command without having to unbind where it was.
    pub fn command_for(&self, key: &KeyEvent) -> Option<Command> {
        self.chords
            .iter()
            .find(|(chord, _)| chord.matches(key))
            .map(|(_, command)| *command)
    }
}

/// The ASCII letter a `Ctrl` chord stands for.
///
/// The character a chord carries is whatever the active keyboard layout puts
/// there, so a letter chord is matched through the physical key and mapped
/// back to the ASCII letter it would have produced on a US layout.
pub fn chord_letter(code: KeyCode) -> Option<char> {
    let KeyCode::Char(c) = code else {
        return None;
    };
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_lowercase());
    }
    // A bare control character, as a VT-style terminal sends it.
    let n = c as u32;
    if (1..=26).contains(&n) {
        return char::from_u32(n + 0x60);
    }
    #[cfg(windows)]
    {
        layout_letter(c)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The ASCII letter typed by the key that produces `c` on the active layout.
#[cfg(windows)]
fn layout_letter(c: char) -> Option<char> {
    let mut buf = [0u16; 2];
    let encoded = c.encode_utf16(&mut buf);
    if encoded.len() != 1 {
        return None;
    }
    let vk = unsafe { VkKeyScanW(encoded[0]) };
    if vk == -1 {
        return None;
    }
    // The low byte is the virtual key; the high byte is the modifiers it
    // needs, which are not part of the question here.
    let virtual_key = (vk as u16 & 0xff) as u32;
    let letter = match virtual_key {
        0x41..=0x5A => char::from_u32(virtual_key)?,
        _ => return None,
    };
    Some(letter.to_ascii_lowercase())
}

#[cfg(windows)]
unsafe extern "system" {
    /// Maps a character to the virtual-key code of the key that types it on
    /// the active keyboard layout. Returns -1 when no key types it.
    fn VkKeyScanW(ch: u16) -> i16;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn a_chord_is_written_the_way_it_is_said() {
        assert_eq!(
            Chord::parse("ctrl+s"),
            Some(Chord {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL
            })
        );
        assert_eq!(
            Chord::parse("Ctrl+Space"),
            Some(Chord {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::CONTROL
            })
        );
        assert_eq!(
            Chord::parse("alt+shift+z"),
            Some(Chord {
                code: KeyCode::Char('z'),
                modifiers: KeyModifiers::ALT | KeyModifiers::SHIFT
            })
        );
        assert_eq!(
            Chord::parse("f1"),
            Some(Chord {
                code: KeyCode::F(1),
                modifiers: KeyModifiers::NONE
            })
        );
    }

    #[test]
    fn a_chord_that_cannot_be_read_is_none() {
        assert_eq!(Chord::parse(""), None);
        assert_eq!(Chord::parse("ctrl+"), None);
        assert_eq!(Chord::parse("hyper+x"), None);
        assert_eq!(Chord::parse("f99"), None);
    }

    #[test]
    fn a_letter_chord_matches_through_the_physical_key() {
        let chord = Chord::parse("ctrl+s").expect("a valid chord");
        assert!(chord.matches(&key(KeyCode::Char('s'), KeyModifiers::CONTROL)));
        assert!(chord.matches(&key(KeyCode::Char('S'), KeyModifiers::CONTROL)));
        // A bare control byte, as a VT-style terminal delivers Ctrl+S.
        assert!(chord.matches(&key(KeyCode::Char('\u{13}'), KeyModifiers::NONE)));
        assert!(!chord.matches(&key(KeyCode::Char('s'), KeyModifiers::NONE)));
        assert!(!chord.matches(&key(KeyCode::Char('d'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn the_leader_is_ctrl_space_and_keeps_its_aliases() {
        let keymap = Keymap::default();
        assert!(keymap.is_leader(&key(KeyCode::Char(' '), KeyModifiers::CONTROL)));
        assert!(keymap.is_leader(&key(KeyCode::Char('\0'), KeyModifiers::NONE)));
        assert!(keymap.is_leader(&key(KeyCode::Null, KeyModifiers::NONE)));
        assert!(!keymap.is_leader(&key(KeyCode::Char(' '), KeyModifiers::NONE)));
    }

    #[test]
    fn a_rebound_leader_does_not_answer_to_the_old_aliases() {
        let config = KeyConfig {
            leader: Some("ctrl+b".into()),
            bindings: Vec::new(),
        };
        let keymap = Keymap::from_config(&config);
        assert!(keymap.is_leader(&key(KeyCode::Char('b'), KeyModifiers::CONTROL)));
        assert!(
            !keymap.is_leader(&key(KeyCode::Char('\0'), KeyModifiers::NONE)),
            "Ctrl+Space is just a key again"
        );
    }

    #[test]
    fn a_binding_runs_its_command() {
        let config = KeyConfig {
            leader: None,
            bindings: vec![crate::config::Binding {
                keys: "ctrl+shift+w".into(),
                command: "toggle_word_wrap".into(),
            }],
        };
        let keymap = Keymap::from_config(&config);
        assert_eq!(
            keymap.command_for(&key(
                KeyCode::Char('w'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(Command::ToggleWordWrap)
        );
        assert_eq!(
            keymap.command_for(&key(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn a_binding_on_a_plain_key_is_ignored_so_typing_still_types() {
        let config = KeyConfig {
            leader: None,
            bindings: vec![
                crate::config::Binding {
                    keys: "j".into(),
                    command: "save".into(),
                },
                crate::config::Binding {
                    keys: "f2".into(),
                    command: "quit".into(),
                },
            ],
        };
        let keymap = Keymap::from_config(&config);
        assert_eq!(
            keymap.command_for(&key(KeyCode::Char('j'), KeyModifiers::NONE)),
            None,
            "a plain key is not up for binding: it inserts itself"
        );
        assert_eq!(
            keymap.command_for(&key(KeyCode::F(2), KeyModifiers::NONE)),
            Some(Command::Quit),
            "a function key with no modifier is still a chord"
        );
    }

    #[test]
    fn a_binding_with_a_typo_is_ignored_rather_than_fatal() {
        let config = KeyConfig {
            leader: Some("nonsense".into()),
            bindings: vec![
                crate::config::Binding {
                    keys: "ctrl+".into(),
                    command: "save".into(),
                },
                crate::config::Binding {
                    keys: "ctrl+k".into(),
                    command: "no such command".into(),
                },
            ],
        };
        let keymap = Keymap::from_config(&config);
        assert_eq!(
            keymap.leader(),
            Keymap::default().leader(),
            "a leader that cannot be read leaves the default"
        );
        assert_eq!(
            keymap.command_for(&key(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            None
        );
    }
}
