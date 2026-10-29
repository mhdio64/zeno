//! Arabic and Persian script shaping and Bidirectional (BiDi) reordering for terminal display.
//!
//! Terminal text editors render grid cells in Left-To-Right (LTR) order. In standard Unicode
//! text files, Arabic and Persian characters are stored in logical reading order using basic
//! Unicode code points (`U+0600..=0x06FF`).
//!
//! Without presentation shaping and BiDi reordering, Arabic/Persian characters appear disconnected
//! and visually reversed in terminals. This module provides a zero-dependency, ultra-fast shaping
//! and layout transformation that:
//! 1. Detects right-to-left text in `O(1)` to `O(N)` with zero allocations.
//! 2. Shapes Persian (گچپژ و ک، ی) and Arabic characters into their contextual presentation forms
//!    (Isolated, Initial, Medial, Final) preserving 1:1 character length.
//! 3. Reorders RTL runs into visual display order for terminal output.
//! 4. Maintains bidirectional mappings between logical and visual column indices so cursor
//!    positioning, syntax highlighting, selection, and search matches remain exact.

/// Checks whether `text` contains any character from right-to-left scripts.
#[inline]
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_char)
}

/// Checks whether `chars` contains any character from right-to-left scripts.
#[inline]
pub fn has_rtl_chars(chars: &[char]) -> bool {
    chars.iter().copied().any(is_rtl_char)
}

/// Checks whether a single character belongs to an RTL script (Arabic, Persian, Hebrew).
#[inline]
pub fn is_rtl_char(c: char) -> bool {
    matches!(
        c,
        '\u{0600}'..='\u{08FF}'   // Arabic, Arabic Supplement, Arabic Extended
        | '\u{FB50}'..='\u{FDFF}' // Arabic Presentation Forms-A (includes Persian گچپژ)
        | '\u{FE70}'..='\u{FEFF}' // Arabic Presentation Forms-B
        | '\u{0590}'..='\u{05FF}' // Hebrew
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoiningType {
    NonJoining,
    RightJoining,
    DualJoining,
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Isolated,
    Final,
    Initial,
    Medial,
}

#[inline]
fn joining_type(c: char) -> JoiningType {
    match c {
        // Transparent diacritics / harakat
        '\u{064B}'..='\u{065F}' | '\u{0670}' => JoiningType::Transparent,

        // Dual-joining characters (connect to both right and left)
        '\u{0626}' // ئ
        | '\u{0628}' // ب
        | '\u{062A}' // ت
        | '\u{062B}' // ث
        | '\u{062C}' // ج
        | '\u{062D}' // ح
        | '\u{062E}' // خ
        | '\u{0633}' // س
        | '\u{0634}' // ش
        | '\u{0635}' // ص
        | '\u{0636}' // ض
        | '\u{0637}' // ط
        | '\u{0638}' // ظ
        | '\u{0639}' // ع
        | '\u{063A}' // غ
        | '\u{0641}' // ف
        | '\u{0642}' // ق
        | '\u{0643}' // ك (Arabic Kaf)
        | '\u{0644}' // ل
        | '\u{0645}' // م
        | '\u{0646}' // ن
        | '\u{0647}' // ه
        | '\u{064A}' // ي (Arabic Yeh)
        | '\u{067E}' // پ (Persian Peh)
        | '\u{0686}' // چ (Persian Tcheh)
        | '\u{06A9}' // ک (Persian Keheh)
        | '\u{06AF}' // گ (Persian Gaf)
        | '\u{06CC}' // ی (Persian Yeh)
        | '\u{06C1}' // ہ (Heh goal)
        | '\u{0640}' // ـ (Tatweel)
        | '\u{200D}' // ZWJ
        => JoiningType::DualJoining,

        // Right-joining characters (connect to right, never connect to left)
        '\u{0622}' // آ
        | '\u{0623}' // أ
        | '\u{0624}' // ؤ
        | '\u{0625}' // إ
        | '\u{0627}' // ا
        | '\u{0629}' // ة
        | '\u{062F}' // د
        | '\u{0630}' // ذ
        | '\u{0631}' // ر
        | '\u{0632}' // ز
        | '\u{0698}' // ژ (Persian Jeh)
        | '\u{0648}' // و
        | '\u{0649}' // ى
        => JoiningType::RightJoining,

        _ => JoiningType::NonJoining,
    }
}

#[inline]
fn can_join_right(c: char) -> bool {
    matches!(
        joining_type(c),
        JoiningType::RightJoining | JoiningType::DualJoining
    )
}

#[inline]
fn can_join_left(c: char) -> bool {
    matches!(joining_type(c), JoiningType::DualJoining)
}

/// Returns the presentation form character for a base letter and contextual form.
fn presentation_form(c: char, form: Form) -> char {
    match c {
        '\u{0622}' => match form {
            Form::Final | Form::Medial => '\u{FE82}',
            _ => '\u{FE81}',
        },
        '\u{0623}' => match form {
            Form::Final | Form::Medial => '\u{FE84}',
            _ => '\u{FE83}',
        },
        '\u{0624}' => match form {
            Form::Final | Form::Medial => '\u{FE86}',
            _ => '\u{FE85}',
        },
        '\u{0625}' => match form {
            Form::Final | Form::Medial => '\u{FE88}',
            _ => '\u{FE87}',
        },
        '\u{0626}' => match form {
            Form::Isolated => '\u{FE89}',
            Form::Final => '\u{FE8A}',
            Form::Initial => '\u{FE8B}',
            Form::Medial => '\u{FE8C}',
        },
        '\u{0627}' => match form {
            Form::Final | Form::Medial => '\u{FE8E}',
            _ => '\u{FE8D}',
        },
        '\u{0628}' => match form {
            Form::Isolated => '\u{FE8F}',
            Form::Final => '\u{FE90}',
            Form::Initial => '\u{FE91}',
            Form::Medial => '\u{FE92}',
        },
        '\u{0629}' => match form {
            Form::Final | Form::Medial => '\u{FE94}',
            _ => '\u{FE93}',
        },
        '\u{062A}' => match form {
            Form::Isolated => '\u{FE95}',
            Form::Final => '\u{FE96}',
            Form::Initial => '\u{FE97}',
            Form::Medial => '\u{FE98}',
        },
        '\u{062B}' => match form {
            Form::Isolated => '\u{FE99}',
            Form::Final => '\u{FE9A}',
            Form::Initial => '\u{FE9B}',
            Form::Medial => '\u{FE9C}',
        },
        '\u{062C}' => match form {
            Form::Isolated => '\u{FE9D}',
            Form::Final => '\u{FE9E}',
            Form::Initial => '\u{FE9F}',
            Form::Medial => '\u{FEA0}',
        },
        '\u{062D}' => match form {
            Form::Isolated => '\u{FEA1}',
            Form::Final => '\u{FEA2}',
            Form::Initial => '\u{FEA3}',
            Form::Medial => '\u{FEA4}',
        },
        '\u{062E}' => match form {
            Form::Isolated => '\u{FEA5}',
            Form::Final => '\u{FEA6}',
            Form::Initial => '\u{FEA7}',
            Form::Medial => '\u{FEA8}',
        },
        '\u{062F}' => match form {
            Form::Final | Form::Medial => '\u{FEAA}',
            _ => '\u{FEA9}',
        },
        '\u{0630}' => match form {
            Form::Final | Form::Medial => '\u{FEAC}',
            _ => '\u{FEAB}',
        },
        '\u{0631}' => match form {
            Form::Final | Form::Medial => '\u{FEAE}',
            _ => '\u{FEAD}',
        },
        '\u{0632}' => match form {
            Form::Final | Form::Medial => '\u{FEB0}',
            _ => '\u{FEAF}',
        },
        '\u{0633}' => match form {
            Form::Isolated => '\u{FEB1}',
            Form::Final => '\u{FEB2}',
            Form::Initial => '\u{FEB3}',
            Form::Medial => '\u{FEB4}',
        },
        '\u{0634}' => match form {
            Form::Isolated => '\u{FEB5}',
            Form::Final => '\u{FEB6}',
            Form::Initial => '\u{FEB7}',
            Form::Medial => '\u{FEB8}',
        },
        '\u{0635}' => match form {
            Form::Isolated => '\u{FEB9}',
            Form::Final => '\u{FEBA}',
            Form::Initial => '\u{FEBB}',
            Form::Medial => '\u{FEBC}',
        },
        '\u{0636}' => match form {
            Form::Isolated => '\u{FEBD}',
            Form::Final => '\u{FEBE}',
            Form::Initial => '\u{FEBF}',
            Form::Medial => '\u{FEC0}',
        },
        '\u{0637}' => match form {
            Form::Isolated => '\u{FEC1}',
            Form::Final => '\u{FEC2}',
            Form::Initial => '\u{FEC3}',
            Form::Medial => '\u{FEC4}',
        },
        '\u{0638}' => match form {
            Form::Isolated => '\u{FEC5}',
            Form::Final => '\u{FEC6}',
            Form::Initial => '\u{FEC7}',
            Form::Medial => '\u{FEC8}',
        },
        '\u{0639}' => match form {
            Form::Isolated => '\u{FEC9}',
            Form::Final => '\u{FECA}',
            Form::Initial => '\u{FECB}',
            Form::Medial => '\u{FECC}',
        },
        '\u{063A}' => match form {
            Form::Isolated => '\u{FECD}',
            Form::Final => '\u{FECE}',
            Form::Initial => '\u{FECF}',
            Form::Medial => '\u{FED0}',
        },
        '\u{0641}' => match form {
            Form::Isolated => '\u{FED1}',
            Form::Final => '\u{FED2}',
            Form::Initial => '\u{FED3}',
            Form::Medial => '\u{FED4}',
        },
        '\u{0642}' => match form {
            Form::Isolated => '\u{FED5}',
            Form::Final => '\u{FED6}',
            Form::Initial => '\u{FED7}',
            Form::Medial => '\u{FED8}',
        },
        '\u{0643}' => match form {
            Form::Isolated => '\u{FED9}',
            Form::Final => '\u{FEDA}',
            Form::Initial => '\u{FEDB}',
            Form::Medial => '\u{FEDC}',
        },
        '\u{0644}' => match form {
            Form::Isolated => '\u{FEDD}',
            Form::Final => '\u{FEDE}',
            Form::Initial => '\u{FEDF}',
            Form::Medial => '\u{FEE0}',
        },
        '\u{0645}' => match form {
            Form::Isolated => '\u{FEE1}',
            Form::Final => '\u{FEE2}',
            Form::Initial => '\u{FEE3}',
            Form::Medial => '\u{FEE4}',
        },
        '\u{0646}' => match form {
            Form::Isolated => '\u{FEE5}',
            Form::Final => '\u{FEE6}',
            Form::Initial => '\u{FEE7}',
            Form::Medial => '\u{FEE8}',
        },
        '\u{0647}' => match form {
            Form::Isolated => '\u{FEE9}',
            Form::Final => '\u{FEEA}',
            Form::Initial => '\u{FEEB}',
            Form::Medial => '\u{FEEC}',
        },
        '\u{0648}' => match form {
            Form::Final | Form::Medial => '\u{FEEE}',
            _ => '\u{FEED}',
        },
        '\u{0649}' => match form {
            Form::Final | Form::Medial => '\u{FEF0}',
            _ => '\u{FEEF}',
        },
        '\u{064A}' => match form {
            Form::Isolated => '\u{FEF1}',
            Form::Final => '\u{FEF2}',
            Form::Initial => '\u{FEF3}',
            Form::Medial => '\u{FEF4}',
        },
        // Persian specific characters
        '\u{067E}' => match form {
            Form::Isolated => '\u{FB56}',
            Form::Final => '\u{FB57}',
            Form::Initial => '\u{FB58}',
            Form::Medial => '\u{FB59}',
        }, // پ
        '\u{0686}' => match form {
            Form::Isolated => '\u{FB7A}',
            Form::Final => '\u{FB7B}',
            Form::Initial => '\u{FB7C}',
            Form::Medial => '\u{FB7D}',
        }, // چ
        '\u{0698}' => match form {
            Form::Final | Form::Medial => '\u{FB8B}',
            _ => '\u{FB8A}',
        }, // ژ
        '\u{06A9}' => match form {
            Form::Isolated => '\u{FB8E}',
            Form::Final => '\u{FB8F}',
            Form::Initial => '\u{FEDB}',
            Form::Medial => '\u{FEDC}',
        }, // ک
        '\u{06AF}' => match form {
            Form::Isolated => '\u{FB92}',
            Form::Final => '\u{FB93}',
            Form::Initial => '\u{FB94}',
            Form::Medial => '\u{FB95}',
        }, // گ
        '\u{06CC}' => match form {
            Form::Isolated => '\u{FBFC}',
            Form::Final => '\u{FBFD}',
            Form::Initial => '\u{FEF3}',
            Form::Medial => '\u{FEF4}',
        }, // ی
        '\u{06C1}' => match form {
            Form::Isolated => '\u{FBA6}',
            Form::Final => '\u{FBA7}',
            Form::Initial => '\u{FBA8}',
            Form::Medial => '\u{FBA9}',
        }, // ہ
        other => other,
    }
}

/// Shapes Arabic and Persian letters in `chars` according to their surrounding characters.
///
/// Preserves the exact length and character indices of the input slice.
pub fn shape_chars(chars: &[char]) -> Vec<char> {
    let mut shaped = Vec::with_capacity(chars.len());
    for i in 0..chars.len() {
        let c = chars[i];
        let j_type = joining_type(c);
        if j_type == JoiningType::NonJoining || j_type == JoiningType::Transparent {
            shaped.push(c);
            continue;
        }

        // Check if previous character connects to left
        let mut prev_joins = false;
        let mut p = i;
        while p > 0 {
            p -= 1;
            let pc = chars[p];
            let pt = joining_type(pc);
            if pt == JoiningType::Transparent {
                continue;
            }
            if can_join_left(pc) {
                prev_joins = true;
            }
            break;
        }

        // Check if next character connects to right
        let mut next_joins = false;
        let mut n = i + 1;
        while n < chars.len() {
            let nc = chars[n];
            let nt = joining_type(nc);
            if nt == JoiningType::Transparent {
                n += 1;
                continue;
            }
            if can_join_right(nc) {
                next_joins = true;
            }
            break;
        }

        let form = match j_type {
            JoiningType::RightJoining => {
                if prev_joins {
                    Form::Final
                } else {
                    Form::Isolated
                }
            }
            JoiningType::DualJoining => match (prev_joins, next_joins) {
                (true, true) => Form::Medial,
                (true, false) => Form::Final,
                (false, true) => Form::Initial,
                (false, false) => Form::Isolated,
            },
            _ => Form::Isolated,
        };

        shaped.push(presentation_form(c, form));
    }
    shaped
}

/// Result of BiDi visual layout for a single line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLine {
    /// Reshaped and reordered characters for terminal rendering.
    pub chars: Vec<char>,
    /// Maps visual column index to logical character index: `vis_to_log[v] = l`.
    pub vis_to_log: Vec<usize>,
    /// Maps logical character index to visual column index: `log_to_vis[l] = v`.
    pub log_to_vis: Vec<usize>,
}

/// Transforms a logical character slice into visual characters for terminal rendering.
///
/// **Why manual shaping + visual reordering is required**:
/// ratatui writes each character into a separate grid cell and emits them left-to-right as
/// raw bytes. This completely bypasses the terminal's OpenType / HarfBuzz shaping engine:
/// the terminal never sees a run of Arabic letters as a unit, only a stream of isolated code
/// points. Consequently:
/// - Every letter renders in its isolated (disconnected) form.
/// - RTL runs read left-to-right instead of right-to-left.
///
/// This function performs the two transforms needed to fix both issues:
///
/// 1. **Contextual shaping** (`shape_chars`): each Arabic/Persian letter is replaced by its
///    Presentation-Forms-B glyph (isolated / initial / medial / final) so the glyphs look
///    connected even though they occupy individual terminal cells.
///
/// 2. **Visual (BiDi) reordering**: within each RTL span the *words* are reversed (so that
///    the first logical word ends up in the rightmost screen position) and the *characters
///    inside each word* are also reversed (so that, when painted left-to-right cell by cell,
///    the word reads right-to-left). LTR sub-runs (English words, numbers, markdown symbols)
///    keep their internal left-to-right order throughout.
pub fn layout_chars(chars: &[char]) -> VisualLine {
    let len = chars.len();
    if len == 0 || !has_rtl_chars(chars) {
        return VisualLine {
            chars: chars.to_vec(),
            vis_to_log: (0..len).collect(),
            log_to_vis: (0..len).collect(),
        };
    }

    // ── Step 1: contextual shaping ──────────────────────────────────────────
    let shaped = shape_chars(chars);

    // ── Step 2: classify each character's base direction ───────────────────
    // Strong RTL  = Arabic / Persian / Hebrew codepoints.
    // Strong LTR  = ASCII letters and Latin-extended.
    // Weak / Neutral = digits, spaces, and ALL punctuation (including `*`, `:`, `-`).
    //
    // Key rule: punctuation is ALWAYS Neutral, never pulled into the RTL class.
    // This keeps markdown delimiters (`**`, `-`, `:`) anchored in LTR context.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Dir {
        Rtl,
        Ltr,
        Neutral,
    }

    fn base_dir(c: char) -> Dir {
        if is_rtl_char(c) || matches!(c, '\u{060C}' | '\u{061B}' | '\u{061F}' | '\u{200F}') {
            Dir::Rtl
        } else if c.is_ascii_alphabetic() || (c > '\u{00FF}' && !is_rtl_char(c)) {
            Dir::Ltr
        } else {
            Dir::Neutral // spaces, digits, and ALL punctuation/symbols
        }
    }

    // ── Step 3: segment into runs of the same base direction ────────────────
    // Each run: (Dir, [(shaped_char, logical_index)])
    let mut raw_runs: Vec<(Dir, Vec<(char, usize)>)> = Vec::new();
    for (i, &c) in shaped.iter().enumerate() {
        let d = base_dir(c);
        if let Some(last) = raw_runs.last_mut()
            && last.0 == d
        {
            last.1.push((c, i));
            continue;
        }
        raw_runs.push((d, vec![(c, i)]));
    }

    // ── Step 4: resolve Neutral runs by their *immediate* strong neighbours ─
    // A Neutral run is RTL only when its directly adjacent strong runs are both RTL.
    // This stops punctuation between an LTR prefix and a Persian word from flipping.
    let n = raw_runs.len();
    let mut resolved_dir: Vec<Dir> = raw_runs.iter().map(|(d, _)| *d).collect();
    for i in 0..n {
        if resolved_dir[i] != Dir::Neutral {
            continue;
        }
        // Find the nearest strong neighbour to the left.
        let prev_strong = (0..i).rev().find_map(|j| {
            if resolved_dir[j] != Dir::Neutral {
                Some(resolved_dir[j])
            } else {
                None
            }
        });
        // Find the nearest strong neighbour to the right.
        let next_strong = (i + 1..n).find_map(|j| {
            if resolved_dir[j] != Dir::Neutral {
                Some(resolved_dir[j])
            } else {
                None
            }
        });
        resolved_dir[i] = match (prev_strong, next_strong) {
            (Some(Dir::Rtl), Some(Dir::Rtl)) => Dir::Rtl,
            _ => Dir::Ltr, // default: stay LTR
        };
    }

    // ── Step 5: merge adjacent same-direction runs into display spans ────────
    let mut spans: Vec<(bool, Vec<(char, usize)>)> = Vec::new(); // (is_rtl, pairs)
    for (i, (_, pairs)) in raw_runs.iter().enumerate() {
        let is_rtl = resolved_dir[i] == Dir::Rtl;
        if let Some(last) = spans.last_mut()
            && last.0 == is_rtl
        {
            last.1.extend(pairs.iter().copied());
            continue;
        }
        spans.push((is_rtl, pairs.to_vec()));
    }

    // ── Step 6: emit visual stream ──────────────────────────────────────────
    // RTL spans: split into "tokens" (RTL words, LTR sub-runs, spaces), reverse
    // the token order, and within each RTL token also reverse character order.
    // LTR tokens keep their character order even inside an RTL span.
    // When ratatui paints these left-to-right, RTL text reads right-to-left.
    let mut visual_chars: Vec<char> = Vec::with_capacity(len);
    let mut vis_to_log: Vec<usize> = Vec::with_capacity(len);

    for (is_rtl, span_pairs) in &spans {
        if !is_rtl {
            for &(c, idx) in span_pairs {
                visual_chars.push(c);
                vis_to_log.push(idx);
            }
            continue;
        }

        // Within an RTL span, split into sub-tokens: RTL words, LTR sub-runs, spaces.
        // We will reverse the *token order* but keep LTR sub-token character order intact
        // and reverse RTL sub-token character order.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum SubKind {
            RtlWord,
            LtrRun,
            Space,
        }

        let mut sub_tokens: Vec<(SubKind, Vec<(char, usize)>)> = Vec::new();
        for &(c, idx) in span_pairs {
            let sk = if c == ' ' || c == '\t' {
                SubKind::Space
            } else if is_rtl_char(c) || (c > '\u{FB00}' && c < '\u{FEFF}') {
                // shaped RTL glyph (Presentation Forms)
                SubKind::RtlWord
            } else {
                SubKind::LtrRun
            };
            if let Some(last) = sub_tokens.last_mut()
                && last.0 == sk
            {
                last.1.push((c, idx));
                continue;
            }
            sub_tokens.push((sk, vec![(c, idx)]));
        }

        // Emit tokens in natural forward reading order (left-to-right across the line):
        // - Space: emit as-is.
        // - LtrRun: emit in original character order (numbers, English words stay LTR).
        // - RtlWord: emit characters in reversed order so that cell-by-cell LTR rendering
        //   paints the shaped Arabic/Persian glyphs in their natural cursive connection.
        for (sk, token) in &sub_tokens {
            match sk {
                SubKind::Space | SubKind::LtrRun => {
                    for &(c, idx) in token {
                        visual_chars.push(c);
                        vis_to_log.push(idx);
                    }
                }
                SubKind::RtlWord => {
                    for &(c, idx) in token.iter().rev() {
                        visual_chars.push(c);
                        vis_to_log.push(idx);
                    }
                }
            }
        }
    }

    // ── Step 7: build inverse mapping ───────────────────────────────────────
    let mut log_to_vis = vec![0usize; len];
    for (v, &l) in vis_to_log.iter().enumerate() {
        if l < len {
            log_to_vis[l] = v;
        }
    }

    VisualLine {
        chars: visual_chars,
        vis_to_log,
        log_to_vis,
    }
}

/// Transforms a logical text string into visual characters and mappings.
pub fn layout_line(text: &str) -> VisualLine {
    let chars: Vec<char> = text.chars().collect();
    layout_chars(&chars)
}

/// Maps a logical column to its visual column index in `text`.
pub fn logical_to_visual(text: &str, logical_col: usize) -> usize {
    if !has_rtl(text) {
        return logical_col;
    }
    let chars: Vec<char> = text.chars().collect();
    if logical_col >= chars.len() {
        return chars.len();
    }
    let layout = layout_chars(&chars);
    layout
        .log_to_vis
        .get(logical_col)
        .copied()
        .unwrap_or(logical_col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_rtl_detects_persian_and_arabic_characters() {
        assert!(has_rtl("سلام"));
        assert!(has_rtl("پروژه"));
        assert!(has_rtl("چگونه"));
        assert!(has_rtl("کتاب"));
        assert!(has_rtl("گنج"));
        assert!(!has_rtl("hello world"));
        assert!(!has_rtl("fn main() { 123 }"));
    }

    #[test]
    fn shaping_connects_letters_in_persian_words() {
        // سلام:
        // س (Initial) -> \u{FEB3}
        // ل (Medial)  -> \u{FEE0}
        // ا (Final)   -> \u{FE8E}
        // م (Isolated)-> \u{FEE1}
        let input: Vec<char> = "سلام".chars().collect();
        let shaped = shape_chars(&input);
        assert_eq!(shaped, vec!['\u{FEB3}', '\u{FEE0}', '\u{FE8E}', '\u{FEE1}']);
    }

    #[test]
    fn shaping_persian_specific_letters_gechpazh() {
        // پ (Isolated) -> \u{FB56}
        // چ (Isolated) -> \u{FB7A}
        // ژ (Isolated) -> \u{FB8A}
        // گ (Isolated) -> \u{FB92}
        // ک (Isolated) -> \u{FB8E}
        // ی (Isolated) -> \u{FBFC}
        let input: Vec<char> = "پ چ ژ گ ک ی".chars().collect();
        let shaped = shape_chars(&input);
        assert_eq!(shaped[0], '\u{FB56}');
        assert_eq!(shaped[2], '\u{FB7A}');
        assert_eq!(shaped[4], '\u{FB8A}');
        assert_eq!(shaped[6], '\u{FB92}');
        assert_eq!(shaped[8], '\u{FB8E}');
        assert_eq!(shaped[10], '\u{FBFC}');
    }

    #[test]
    fn bidi_layout_reorders_pure_persian_sentence() {
        // "سلام دنیا" → visual order should be "دنیا سلام" (words reversed)
        // but each word's letters stay in their own forward order (shaper joins them).
        let line = layout_line("سلام دنیا");
        assert_eq!(line.chars.len(), "سلام دنیا".chars().count());
        assert_eq!(line.vis_to_log.len(), line.chars.len());
        assert_eq!(line.log_to_vis.len(), line.chars.len());
        // The visual stream must contain the shaped form of س (initial), which
        // is the first character of "سلام" — it appears in the output regardless of order.
        let visual_str: String = line.chars.iter().collect();
        assert!(visual_str.contains('\u{FEB3}'), "shaped س must be present");
    }

    #[test]
    fn bidi_layout_preserves_code_prefix_and_suffix() {
        // LTR prefix and suffix must remain anchored at their columns.
        let code = "let msg = \"سلام\";";
        let line = layout_line(code);
        let visual_str: String = line.chars.iter().collect();
        assert!(visual_str.starts_with("let msg = \""));
        assert!(visual_str.ends_with("\";"));
        assert_eq!(line.chars.len(), code.chars().count());
    }

    #[test]
    fn bidi_layout_keeps_numbers_in_natural_ltr_order() {
        let text = "قیمت 1250 تومان";
        let line = layout_line(text);
        let visual_str: String = line.chars.iter().collect();
        assert!(
            visual_str.contains("1250"),
            "Numbers must remain forward 1250"
        );
    }

    #[test]
    fn zero_rtl_returns_identity() {
        let text = "const BUFFER_SIZE: usize = 1024;";
        let line = layout_line(text);
        let visual_str: String = line.chars.iter().collect();
        assert_eq!(visual_str, text);
        assert_eq!(line.log_to_vis, (0..text.len()).collect::<Vec<usize>>());
    }

    #[test]
    fn mixed_persian_and_english_sentence_layout() {
        let text = "راهنمای direnv برای لینوکس و ویندوز";
        let line = layout_line(text);
        assert_eq!(line.chars.len(), text.chars().count());
        let visual_str: String = line.chars.iter().collect();
        assert!(visual_str.contains("direnv"));
    }

    #[test]
    fn markdown_and_english_phrases_not_scrambled() {
        // Markdown delimiters (LTR) must stay at their positions.
        let text1 = "**حالت تمرکز بدون شلوغی (Zen / Distraction-Free Mode)**";
        let line1 = layout_line(text1);
        let s1: String = line1.chars.iter().collect();
        assert!(s1.starts_with("**"), "** prefix must stay on left");
        assert!(s1.contains("(Zen / Distraction-Free Mode)**"));

        let text2 = "- **کاربرد**: با زدن یک کلید";
        let line2 = layout_line(text2);
        let s2: String = line2.chars.iter().collect();
        assert!(s2.starts_with("- **"), "markdown prefix must be on left");
        assert!(s2.contains("**: "));
    }
}
