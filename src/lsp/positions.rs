use lsp_types::{Position, PositionEncodingKind, Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Encoding {
    Utf8,
    Utf16,
    Utf32,
}

impl Encoding {
    pub(super) fn negotiate(offered: Option<&[PositionEncodingKind]>) -> Self {
        offered
            .into_iter()
            .flatten()
            .find_map(|kind| match kind.as_str() {
                "utf-8" => Some(Self::Utf8),
                "utf-16" => Some(Self::Utf16),
                "utf-32" => Some(Self::Utf32),
                _ => None,
            })
            .unwrap_or(Self::Utf16)
    }

    pub(super) fn kind(self) -> PositionEncodingKind {
        match self {
            Self::Utf8 => PositionEncodingKind::UTF8,
            Self::Utf16 => PositionEncodingKind::UTF16,
            Self::Utf32 => PositionEncodingKind::UTF32,
        }
    }

    pub(super) fn range(self, source: &str, span: crate::Span) -> Range {
        let start = span.start.min(source.len());
        let end = span.end.min(source.len()).max(start);
        let bytes = source.as_bytes();
        let mut offset = 0;
        let mut line = 0_u32;
        let mut character = 0_u32;
        let mut start_position = None;
        let mut end_position = None;

        while start_position.is_none() || end_position.is_none() {
            let position = Position::new(line, character);
            if start_position.is_none() && start <= offset {
                start_position = Some(position);
            }
            if end_position.is_none() && end <= offset {
                end_position = Some(position);
            }
            if offset == bytes.len() {
                break;
            }

            if let Some(next) = crate::source::line_end(bytes, offset) {
                // Offsets within a physical line ending belong to its end, not
                // to the next line (notably the interior byte of CRLF).
                if start_position.is_none() && start < next {
                    start_position = Some(position);
                }
                if end_position.is_none() && end < next {
                    end_position = Some(position);
                }
                offset = next;
                line = line.saturating_add(1);
                character = 0;
                continue;
            }

            let ch = source[offset..]
                .chars()
                .next()
                .expect("offset is before EOF");
            let next = offset + ch.len_utf8();
            if start_position.is_none() && start < next {
                start_position = Some(position);
            }
            if end_position.is_none() && end < next {
                end_position = Some(position);
            }
            character = character.saturating_add(match self {
                Self::Utf8 => u32::try_from(ch.len_utf8()).unwrap_or(u32::MAX),
                Self::Utf16 => u32::try_from(ch.len_utf16()).unwrap_or(u32::MAX),
                Self::Utf32 => 1,
            });
            offset = next;
        }

        let eof_position = Position::new(line, character);
        Range::new(
            start_position.unwrap_or(eof_position),
            end_position.unwrap_or(eof_position),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Encoding;
    use crate::Span;
    use lsp_types::{Position, PositionEncodingKind, Range};

    fn range(encoding: Encoding, source: &str, start: usize, end: usize) -> Range {
        encoding.range(source, Span { start, end })
    }

    #[test]
    fn negotiation_uses_first_supported_offer_and_utf16_fallback() {
        let offered = [
            PositionEncodingKind::new("unsupported"),
            PositionEncodingKind::UTF8,
            PositionEncodingKind::UTF32,
        ];
        assert_eq!(Encoding::negotiate(Some(&offered)), Encoding::Utf8);
        assert_eq!(Encoding::negotiate(Some(&offered[2..])), Encoding::Utf32);
        assert_eq!(Encoding::negotiate(Some(&[])), Encoding::Utf16);
        assert_eq!(Encoding::negotiate(None), Encoding::Utf16);
        assert_eq!(Encoding::Utf16.kind(), PositionEncodingKind::UTF16);
    }

    #[test]
    fn counts_unicode_in_negotiated_units() {
        // A supplementary scalar occupies four UTF-8 bytes, two UTF-16 units,
        // and one UTF-32 unit; combining marks and CJK remain separate scalars.
        let source = "A😀e\u{301}界";
        let end = source.len();
        assert_eq!(
            range(Encoding::Utf8, source, end, end).start,
            Position::new(0, 11)
        );
        assert_eq!(
            range(Encoding::Utf16, source, end, end).start,
            Position::new(0, 6)
        );
        assert_eq!(
            range(Encoding::Utf32, source, end, end).start,
            Position::new(0, 5)
        );
    }
    #[test]
    fn tabs_count_as_one_protocol_unit() {
        assert_eq!(
            range(Encoding::Utf16, "a\tb", 3, 3).start,
            Position::new(0, 3),
        );
    }

    #[test]
    fn maps_bom_and_mixed_physical_line_endings() {
        let source = "\u{feff}a\r\nb\rc\nd";
        assert_eq!(
            range(Encoding::Utf16, source, 0, 0).start,
            Position::new(0, 0)
        );
        assert_eq!(
            range(Encoding::Utf16, source, 4, 4).start,
            Position::new(0, 2)
        );
        assert_eq!(
            range(Encoding::Utf16, source, 6, 6).start,
            Position::new(1, 0)
        );
        assert_eq!(
            range(Encoding::Utf16, source, 8, 8).start,
            Position::new(2, 0)
        );
        assert_eq!(
            range(Encoding::Utf16, source, source.len(), source.len()).start,
            Position::new(3, 1)
        );
    }

    #[test]
    fn eof_after_final_line_ending_is_on_next_line() {
        for source in ["x\n", "x\r", "x\r\n"] {
            assert_eq!(
                range(Encoding::Utf16, source, source.len(), source.len()).start,
                Position::new(1, 0),
            );
        }
    }

    #[test]
    fn clamps_invalid_offsets_and_line_ending_interiors() {
        let source = "a😀\r\nz";
        let span = range(Encoding::Utf16, source, 3, 4);
        assert_eq!(span, Range::new(Position::new(0, 1), Position::new(0, 1)));
        assert_eq!(
            range(Encoding::Utf16, source, 8, 99),
            Range::new(Position::new(1, 1), Position::new(1, 1))
        );
        assert_eq!(
            range(Encoding::Utf16, source, 6, 6).start,
            Position::new(0, 3),
        );
        assert_eq!(
            range(Encoding::Utf16, source, 99, 1),
            Range::new(Position::new(1, 1), Position::new(1, 1))
        );
    }
}
