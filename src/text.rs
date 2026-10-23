use tower_lsp::lsp_types::{Position, Range, TextDocumentContentChangeEvent};

/// Byte offsets internally, UTF-16 code units at the protocol boundary.
#[derive(Clone, Debug)]
pub struct Text {
    pub source: String,
    lines: Vec<usize>,
}

impl Text {
    pub fn newline(&self) -> &'static str {
        if self.source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }

    pub fn text_line_count(&self) -> usize {
        self.lines.len()
    }
    pub fn new(source: String) -> Self {
        let mut lines = vec![0];
        lines.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Self { source, lines }
    }

    pub fn offset(&self, position: Position) -> Option<usize> {
        let start = *self.lines.get(position.line as usize)?;
        let end = self
            .lines
            .get(position.line as usize + 1)
            .copied()
            .map(|i| i - 1)
            .unwrap_or(self.source.len());
        let line = self.source[start..end]
            .strip_suffix('\r')
            .unwrap_or(&self.source[start..end]);
        let mut units = 0;
        for (i, ch) in line.char_indices() {
            if units == position.character {
                return Some(start + i);
            }
            units += ch.len_utf16() as u32;
            if units > position.character {
                return None;
            }
        }
        (units == position.character).then_some(start + line.len())
    }

    /// LSP query positions clamp oversized columns to the line end. Editing
    /// notifications keep using strict `offset` to validate entire batches.
    pub(crate) fn clamped_offset(&self, position: Position) -> Option<usize> {
        if let Some(offset) = self.offset(position) {
            return Some(offset);
        }
        let start = *self.lines.get(position.line as usize)?;
        let line = self.source[start..].split('\n').next()?;
        let line = line.strip_suffix('\r').unwrap_or(line);
        (position.character as usize > line.encode_utf16().count()).then_some(start + line.len())
    }

    pub fn position(&self, offset: usize) -> Position {
        let mut offset = offset.min(self.source.len());
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.lines.partition_point(|&start| start <= offset) - 1;
        let units = self.source[self.lines[line]..offset].encode_utf16().count();
        Position::new(line as u32, units as u32)
    }

    pub fn range(&self, start: usize, end: usize) -> Range {
        Range::new(self.position(start), self.position(end))
    }

    /// Changes in one notification are sequential; reject the entire batch on invalid ranges.
    pub fn changed(&self, changes: Vec<TextDocumentContentChangeEvent>) -> Option<Self> {
        let mut next = self.clone();
        for change in changes {
            if let Some(range) = change.range {
                let start = next.offset(range.start)?;
                let end = next.offset(range.end)?;
                if start > end {
                    return None;
                }
                next.source.replace_range(start..end, &change.text);
                next = Self::new(next.source);
            } else {
                next = Self::new(change.text);
            }
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_crlf_and_surrogate_boundaries() {
        let text = Text::new("中😀a\r\n文\n".into());
        assert_eq!(text.offset(Position::new(0, 3)), Some(7));
        assert_eq!(text.offset(Position::new(0, 2)), None);
        assert_eq!(text.offset(Position::new(0, 5)), None);
        assert_eq!(text.position(7), Position::new(0, 3));
        assert_eq!(text.position(10), Position::new(1, 0));
        assert_eq!(text.offset(Position::new(2, 0)), Some(14));
        assert_eq!(text.offset(Position::new(3, 0)), None);
    }

    #[test]
    fn query_columns_clamp_without_accepting_invalid_incremental_edits() {
        let text = Text::new("中😀a\r\n文\n".into());
        assert_eq!(text.clamped_offset(Position::new(0, 999)), Some(8));
        assert_eq!(text.clamped_offset(Position::new(1, 999)), Some(13));
        assert_eq!(text.clamped_offset(Position::new(2, 999)), Some(14));
        assert_eq!(text.clamped_offset(Position::new(0, 2)), None);
        assert_eq!(text.clamped_offset(Position::new(3, 0)), None);
        assert!(text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 999), Position::new(0, 1000))),
                range_length: None,
                text: "replacement".into(),
            }])
            .is_none());
    }

    #[test]
    fn changes_use_updated_positions() {
        let text = Text::new("😀abc".into());
        let change = |start, end, value: &str| TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(0, start), Position::new(0, end))),
            range_length: None,
            text: value.into(),
        };
        let result = text
            .changed(vec![change(2, 3, "中文"), change(4, 5, "!")])
            .unwrap();
        assert_eq!(result.source, "😀中文!c");
        assert!(text.changed(vec![change(1, 2, "x")]).is_none());
        assert_eq!(text.source, "😀abc");
    }
}
