//! The system clipboard, through OSC 52.
//!
//! OSC 52 is the only clipboard a terminal editor can reach that also works
//! over SSH: the editor writes an escape sequence, and the terminal emulator
//! puts the text on the clipboard of the machine the user is *looking at*.
//! There is no read direction — no terminal answers an OSC 52 query — so the
//! clipboard is write-only and paste stays internal.
//!
//! The payload travels inside an escape sequence, which terminals and
//! multiplexers cap. tmux refuses a sequence longer than 74,994 bytes, so a
//! copy that encodes to more than that is kept internally and reported,
//! rather than sent and silently dropped by the terminal.

/// The largest base64 payload that terminals and tmux accept.
const MAX_PAYLOAD: usize = 74_000;

/// The escape sequence that puts `text` on the system clipboard, or `None`
/// when it is too large to send.
pub fn osc52(text: &str) -> Option<String> {
    let encoded = base64(text.as_bytes());
    if encoded.len() > MAX_PAYLOAD {
        return None;
    }
    Some(format!("\x1b]52;c;{encoded}\x1b\\"))
}

/// Standard base64 with padding.
///
/// Hand-written because it is twenty lines and the alternative is a
/// dependency compiled into a binary whose whole point is starting fast.
pub fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        // RFC 4648, section 10.
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_utf8_bytes() {
        assert_eq!(base64("سلام".as_bytes()), "2LPZhNin2YU=");
    }

    #[test]
    fn the_sequence_names_the_clipboard_and_ends_with_st() {
        let seq = osc52("hi").unwrap();
        assert_eq!(seq, "\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn a_payload_over_the_terminal_limit_is_refused() {
        let text = "x".repeat(MAX_PAYLOAD);
        assert!(
            osc52(&text).is_none(),
            "a payload past the cap must not be sent"
        );
        assert!(osc52("short text").is_some());
    }

    #[test]
    fn the_limit_is_measured_after_encoding() {
        // 54,000 bytes encode to 72,000 characters: under the cap.
        assert!(osc52(&"x".repeat(54_000)).is_some());
        // 60,000 bytes encode to 80,000 characters: over it.
        assert!(osc52(&"x".repeat(60_000)).is_none());
    }
}
