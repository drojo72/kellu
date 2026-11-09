//! Positions: LSP counts lines from 0 and columns in UTF-16 code units; Rust strings are UTF-8.

/// The byte offset of (`line`, `character`), or `None` if the document has fewer lines. A column
/// past the end of the line is clamped to the line end, as the protocol asks.
#[must_use]
pub fn offset_at(text: &str, line: u64, character: u64) -> Option<usize> {
    let mut start = 0_usize;
    for _ in 0..line {
        start += text[start..].find('\n')? + 1;
    }
    let rest = &text[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    let line_text = rest[..end].strip_suffix('\r').unwrap_or(&rest[..end]);
    let mut units = 0_u64;
    for (index, c) in line_text.char_indices() {
        if units >= character {
            return Some(start + index);
        }
        units += c.len_utf16() as u64;
    }
    Some(start + line_text.len())
}

/// The (`line`, `character`) of a byte offset that lies on a character boundary.
#[must_use]
pub fn position_at(text: &str, offset: usize) -> (u64, u64) {
    let head = &text[..offset.min(text.len())];
    let line = head.matches('\n').count() as u64;
    let line_start = head.rfind('\n').map_or(0, |i| i + 1);
    let character = head[line_start..]
        .chars()
        .map(|c| c.len_utf16() as u64)
        .sum();
    (line, character)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The byte range of the identifier touching `offset` (empty if there is none).
#[must_use]
pub fn word_at(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(offset, |(i, _)| i);
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(i, _)| offset + i);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_count_utf16_units() {
        let text = "a\u{1f600}b\nsecond\r\nthird";
        // The emoji is two UTF-16 units and four UTF-8 bytes.
        assert_eq!(offset_at(text, 0, 0), Some(0));
        assert_eq!(offset_at(text, 0, 1), Some(1));
        assert_eq!(offset_at(text, 0, 3), Some(5));
        assert_eq!(offset_at(text, 0, 4), Some(6));
        assert_eq!(
            offset_at(text, 1, 99),
            Some(13),
            "past the end clamps before the CR"
        );
        assert_eq!(offset_at(text, 2, 2), Some(text.len() - 3));
        assert_eq!(offset_at(text, 3, 0), None);
    }

    #[test]
    fn the_middle_of_a_surrogate_pair_lands_on_a_boundary() {
        let text = "\u{1f600}x";
        let offset = offset_at(text, 0, 1).unwrap();
        assert!(text.is_char_boundary(offset));
    }

    #[test]
    fn positions_round_trip() {
        let text = "ab\n\u{1f600}cd\nz";
        for (line, character) in [(0, 0), (0, 2), (1, 0), (1, 2), (1, 4), (2, 1)] {
            let offset = offset_at(text, line, character).unwrap();
            assert_eq!(position_at(text, offset), (line, character));
        }
    }

    #[test]
    fn words_are_found_around_the_cursor() {
        let text = "let foo_bar = baz;";
        assert_eq!(&text[word_at(text, 6).0..word_at(text, 6).1], "foo_bar");
        assert_eq!(
            &text[word_at(text, 4).0..word_at(text, 4).1],
            "foo_bar",
            "at the start"
        );
        assert_eq!(
            &text[word_at(text, 11).0..word_at(text, 11).1],
            "foo_bar",
            "at the end"
        );
        let (s, e) = word_at(text, 12);
        assert_eq!(s, e, "between spaces there is no word");
        assert_eq!(word_at("", 0), (0, 0));
        assert_eq!(
            &"\u{e9}t\u{e9}"[word_at("\u{e9}t\u{e9}", 2).0..word_at("\u{e9}t\u{e9}", 2).1],
            "\u{e9}t\u{e9}"
        );
    }
}
