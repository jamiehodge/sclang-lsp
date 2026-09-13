//! Byte offsets to LSP positions and back.
//!
//! The syntax tree speaks byte offsets. LSP speaks line/character, where
//! "character" is a UTF-16 code unit unless the client agrees otherwise. Every
//! position that crosses the wire goes through here, because getting it wrong
//! is invisible in ASCII and corrupts every range the moment a user types a
//! `°` or an emoji into a comment.

use lsp_types::{Position, Range};

/// How a client counts the `character` field of a position.
///
/// UTF-16 is the protocol default and the only encoding a client is required
/// to support. UTF-8 is negotiated when offered, because it makes this module
/// a no-op on the hot path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}

impl PositionEncoding {
    /// How many units `c` occupies under this encoding.
    fn width(self, c: char) -> u32 {
        match self {
            PositionEncoding::Utf8 => c.len_utf8() as u32,
            PositionEncoding::Utf16 => c.len_utf16() as u32,
            PositionEncoding::Utf32 => 1,
        }
    }
}

/// Line start offsets for one document revision.
///
/// Lines are split on `\n` only; a `\r` is ordinary content belonging to the
/// line it sits on. That matches what editors send and what every other
/// language server assumes.
#[derive(Debug, Clone, Default)]
pub struct LineIndex {
    line_starts: Vec<u32>,
    len: u32,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        LineIndex {
            line_starts,
            len: text.len() as u32,
        }
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Byte range of a line, excluding its terminator.
    fn line_bounds(&self, text: &str, line: u32) -> (u32, u32) {
        let start = match self.line_starts.get(line as usize) {
            Some(s) => *s,
            None => return (self.len, self.len),
        };
        let end = match self.line_starts.get(line as usize + 1) {
            // Step back over the `\n`, and over a `\r` if the file has CRLF
            // endings, so the two directions agree on where a line ends.
            Some(next) => {
                let mut e = next - 1;
                if e > start && text.as_bytes().get(e as usize - 1) == Some(&b'\r') {
                    e -= 1;
                }
                e
            }
            None => self.len,
        };
        (start, end)
    }

    /// LSP position to byte offset.
    ///
    /// Clamps rather than fails: a client may legitimately send a position one
    /// past the end of a line, and an out-of-date position is better served as
    /// the nearest valid offset than as an error.
    pub fn offset(&self, text: &str, pos: Position, enc: PositionEncoding) -> u32 {
        let (start, end) = self.line_bounds(text, pos.line);
        if pos.character == 0 {
            return start;
        }
        let line = &text[start as usize..end as usize];

        let mut units = 0u32;
        for (byte_offset, c) in line.char_indices() {
            if units >= pos.character {
                return start + byte_offset as u32;
            }
            units += enc.width(c);
        }
        end
    }

    /// The line an offset falls on.
    ///
    /// No encoding needed: how a client counts characters changes the column
    /// and never the line, which is why folding ranges can be answered without
    /// negotiating anything.
    pub fn line(&self, offset: u32) -> u32 {
        let offset = offset.min(self.len);
        // The line whose start is the last one at or before `offset`.
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(next) => next - 1,
        };
        line as u32
    }

    /// Byte offset to LSP position.
    pub fn position(&self, text: &str, offset: u32, enc: PositionEncoding) -> Position {
        let offset = offset.min(self.len);
        let line = self.line(offset);

        let (start, end) = self.line_bounds(text, line);
        let stop = offset.min(end);
        let character = if enc == PositionEncoding::Utf8 {
            stop - start
        } else {
            text[start as usize..stop as usize]
                .chars()
                .map(|c| enc.width(c))
                .sum()
        };

        Position { line, character }
    }

    pub fn range(&self, text: &str, range: std::ops::Range<u32>, enc: PositionEncoding) -> Range {
        Range {
            start: self.position(text, range.start, enc),
            end: self.position(text, range.end, enc),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(text: &str, enc: PositionEncoding) {
        let index = LineIndex::new(text);
        // Every char boundary must survive offset -> position -> offset.
        for (offset, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            let offset = offset as u32;
            let pos = index.position(text, offset, enc);
            assert_eq!(
                index.offset(text, pos, enc),
                offset,
                "roundtrip failed at {offset} in {text:?} ({enc:?})"
            );
        }
    }

    #[test]
    fn ascii_roundtrips() {
        for enc in [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
        ] {
            roundtrip("Foo {\n  bar { ^1 }\n}\n", enc);
        }
    }

    #[test]
    fn non_ascii_roundtrips() {
        // A BMP char is 2 bytes / 1 UTF-16 unit; an emoji is 4 bytes / 2 units.
        for enc in [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
        ] {
            roundtrip("// °ø\nFoo { }\n// 🎛 knob\nBar { }", enc);
        }
    }

    #[test]
    fn utf16_counts_surrogate_pairs() {
        let text = "// 🎛\nx";
        let index = LineIndex::new(text);
        // The emoji is 4 bytes but 2 UTF-16 units, so the end of line 0 is
        // character 5 in UTF-16 and byte 7 in UTF-8.
        let end_of_line = index.position(text, 7, PositionEncoding::Utf16);
        assert_eq!(end_of_line, Position::new(0, 5));
        let as_utf8 = index.position(text, 7, PositionEncoding::Utf8);
        assert_eq!(as_utf8, Position::new(0, 7));
    }

    #[test]
    fn crlf_line_ends_agree() {
        let text = "a\r\nbb\r\n";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 3);
        // Position past the `a` is character 1, and must not land on the `\r`.
        assert_eq!(
            index.position(text, 1, PositionEncoding::Utf16),
            Position::new(0, 1)
        );
        assert_eq!(
            index.offset(text, Position::new(1, 2), PositionEncoding::Utf16),
            5
        );
    }

    #[test]
    fn clamps_out_of_range_positions() {
        let text = "abc\n";
        let index = LineIndex::new(text);
        // Far past the end of the line, and past the end of the file.
        assert_eq!(
            index.offset(text, Position::new(0, 99), PositionEncoding::Utf16),
            3
        );
        assert_eq!(
            index.offset(text, Position::new(99, 0), PositionEncoding::Utf16),
            4
        );
    }

    #[test]
    fn empty_document() {
        let index = LineIndex::new("");
        assert_eq!(
            index.position("", 0, PositionEncoding::Utf16),
            Position::new(0, 0)
        );
        assert_eq!(
            index.offset("", Position::new(0, 0), PositionEncoding::Utf16),
            0
        );
    }
}
