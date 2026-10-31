//! Small text helpers shared by logging and session code. No dependencies.

use std::num::NonZeroUsize;

/// Maps text to the same text with secrets removed.
///
/// The real implementation lives in `kellu-security` (milestone M3) and is injected; until then
/// [`IdentityRedactor`] is used. Any `Fn(&str) -> String` is a redactor too.
pub trait Redactor: Send + Sync {
    /// Return `text` with secrets removed.
    fn redact(&self, text: &str) -> String;
}

impl<F> Redactor for F
where
    F: Fn(&str) -> String + Send + Sync,
{
    fn redact(&self, text: &str) -> String {
        self(text)
    }
}

/// Placeholder redactor that returns the text unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityRedactor;

impl Redactor for IdentityRedactor {
    fn redact(&self, text: &str) -> String {
        text.to_owned()
    }
}

/// Whether `c` must never reach a log, a session file or a terminal.
///
/// Covers control characters (Unicode `Cc`), the format characters (`Cf`: bidirectional
/// overrides, zero-width characters, tag characters), private-use code points, noncharacters
/// and the line and paragraph separators. Unassigned code points are not covered, since that
/// needs the full Unicode tables; revisit with a general-category crate if that matters.
fn is_unsafe(c: char) -> bool {
    if c.is_control() {
        return true;
    }
    let u = u32::from(c);
    matches!(
        u,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x2028..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
            | 0xE000..=0xF8FF
            | 0xFDD0..=0xFDEF
            | 0xF0000..=0xFFFFD
            | 0x100000..=0x10FFFD
    ) || u & 0xFFFE == 0xFFFE
}

/// Remove control and format characters, keeping only newline and tab.
///
/// This blocks terminal-escape injection (ESC sequences, carriage returns) and bidirectional
/// spoofing when logs are shown with `cat` or `less`.
#[must_use]
pub fn strip_control_chars(text: &str) -> String {
    text.chars()
        .filter(|&c| c == '\n' || c == '\t' || !is_unsafe(c))
        .collect()
}

/// Cut `text` to `limit` characters and say how much was dropped.
#[must_use]
pub fn truncate(text: &str, limit: NonZeroUsize) -> String {
    let total = text.chars().count();
    if total <= limit.get() {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit.get()).collect();
    format!("{head}... [truncated {} chars]", total - limit.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    #[test]
    fn identity_redactor_returns_text_unchanged() {
        assert_eq!(IdentityRedactor.redact("a secret"), "a secret");
    }

    #[test]
    fn closures_are_redactors() {
        let redactor = |text: &str| text.replace("hunter2", "***");
        assert_eq!(redactor.redact("pw hunter2"), "pw ***");
    }

    #[test]
    fn strip_control_chars_keeps_newline_and_tab() {
        assert_eq!(strip_control_chars("a\tb\nc"), "a\tb\nc");
    }

    #[test]
    fn strip_control_chars_removes_escape_sequences_and_carriage_returns() {
        assert_eq!(
            strip_control_chars("\u{1b}[31mred\u{1b}[0m\rx\0y"),
            "[31mred[0mxy"
        );
    }

    #[test]
    fn strip_control_chars_removes_bidi_override_and_zero_width() {
        assert_eq!(
            strip_control_chars("a\u{202e}b\u{200b}c\u{2066}d\u{feff}e"),
            "abcde"
        );
    }

    #[test]
    fn strip_control_chars_removes_tag_chars_separators_and_private_use() {
        assert_eq!(
            strip_control_chars("a\u{e0041}b\u{2028}c\u{e000}d\u{ffff}e"),
            "abcde"
        );
    }

    #[test]
    fn strip_control_chars_covers_the_format_character_blocks() {
        // Checked once against Unicode 15.1 (Python's `unicodedata`): every Cc, Cf and Co code
        // point is stripped. These are the edges of the blocks.
        for u in [
            0x00ADu32, 0x0600, 0x0605, 0x061C, 0x06DD, 0x070F, 0x0890, 0x08E2, 0x180E, 0x200B,
            0x200F, 0x202A, 0x202E, 0x2060, 0x2064, 0x2066, 0x206F, 0xFEFF, 0xFFF9, 0xFFFB,
            0x110BD, 0x110CD, 0x13430, 0x1343F, 0x1BCA0, 0x1BCA3, 0x1D173, 0x1D17A, 0xE0001,
            0xE0020, 0xE007F, 0xE000, 0xF8FF, 0xF0000, 0x10FFFD,
        ] {
            let c = char::from_u32(u).unwrap();
            assert_eq!(strip_control_chars(&format!("a{c}b")), "ab", "U+{u:X}");
        }
    }

    #[test]
    fn strip_control_chars_keeps_ordinary_unicode() {
        let text = "ñandú 日本語 🦀 café";
        assert_eq!(strip_control_chars(text), text);
    }

    #[test]
    fn truncate_leaves_short_text_alone() {
        assert_eq!(truncate("hello", limit(5)), "hello");
    }

    #[test]
    fn truncate_reports_dropped_characters() {
        assert_eq!(
            truncate("abcdefghij", limit(4)),
            "abcd... [truncated 6 chars]"
        );
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("ñññññ", limit(2)), "ññ... [truncated 3 chars]");
    }
}
