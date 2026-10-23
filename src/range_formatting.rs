//! Restrict a contextual formatter's result to complete changes inside a range.
use crate::{formatting, text::Text};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::{Range, TextEdit},
};

pub(crate) fn validate(text: &Text, range: Range) -> Result<()> {
    if range.start > range.end
        || text.offset(range.start).is_none()
        || text.offset(range.end).is_none()
    {
        return Err(Error::invalid_params("Invalid formatting range"));
    }
    Ok(())
}

/// Align lines by their content, allowing indentation changes at each anchor.
/// Unaligned runs are atomic: a wrap/join crossing the selection is skipped.
/// The middle LCS table is bounded; large unmatched runs remain a single edit.
pub(crate) fn edits(text: &Text, formatted: String, range: Range) -> Vec<TextEdit> {
    if range.start == range.end {
        return vec![];
    }
    let formatted = formatted
        .replace("\r\n", "\n")
        .replace('\n', text.newline());
    let before: Vec<_> = text.source.split_inclusive('\n').collect();
    let after: Vec<_> = formatted.split_inclusive('\n').collect();
    let old: Vec<_> = before.iter().map(|line| line.trim()).collect();
    let new: Vec<_> = after.iter().map(|line| line.trim()).collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut anchors: Vec<_> = (0..prefix).map(|i| (i, i)).collect();
    let rows = old.len() - prefix - suffix;
    let cols = new.len() - prefix - suffix;
    const MAX_CELLS: usize = 1_000_000;
    if rows
        .saturating_add(1)
        .saturating_mul(cols.saturating_add(1))
        <= MAX_CELLS
    {
        let width = cols + 1;
        let mut table = vec![0u32; (rows + 1) * width];
        for i in (0..rows).rev() {
            for j in (0..cols).rev() {
                table[i * width + j] = if old[prefix + i] == new[prefix + j] {
                    1 + table[(i + 1) * width + j + 1]
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < rows && j < cols {
            if old[prefix + i] == new[prefix + j] {
                anchors.push((prefix + i, prefix + j));
                i += 1;
                j += 1;
            } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    anchors.extend((0..suffix).map(|i| (old.len() - suffix + i, new.len() - suffix + i)));
    let offsets = |lines: &[&str]| {
        let mut result = vec![0];
        for line in lines {
            result.push(result.last().unwrap() + line.len());
        }
        result
    };
    let old_offsets = offsets(&before);
    let new_offsets = offsets(&after);
    let mut result: Vec<TextEdit> = Vec::new();
    let mut append = |start, end, new_start, new_end| {
        if let Some(edit) =
            formatting::replacement(text, start..end, &formatted[new_start..new_end])
        {
            if range.start <= edit.range.start
                && edit.range.end <= range.end
                && (edit.range.start < range.end || range.end == text.position(text.source.len()))
            {
                if let Some(previous) = result.last_mut() {
                    if previous.range.end == edit.range.start {
                        previous.range.end = edit.range.end;
                        previous.new_text.push_str(&edit.new_text);
                        return;
                    }
                }
                result.push(edit);
            }
        }
    };
    let (mut old_start, mut new_start) = (0, 0);
    for (i, j) in anchors {
        append(old_start, old_offsets[i], new_start, new_offsets[j]);
        append(
            old_offsets[i],
            old_offsets[i + 1],
            new_offsets[j],
            new_offsets[j + 1],
        );
        old_start = old_offsets[i + 1];
        new_start = new_offsets[j + 1];
    }
    append(old_start, text.source.len(), new_start, formatted.len());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Position, TextDocumentContentChangeEvent};

    fn apply(source: &str, formatted: &str, start: usize, end: usize) -> String {
        let text = Text::new(source.into());
        let range = text.range(start, end);
        let edits = edits(&text, formatted.into(), range);
        for edit in &edits {
            assert!(range.start <= edit.range.start && edit.range.end <= range.end);
        }
        for pair in edits.windows(2) {
            assert!(pair[0].range.end < pair[1].range.start);
        }
        text.changed(
            edits
                .into_iter()
                .rev()
                .map(|edit| TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
    }

    #[test]
    fn indentation_uses_full_document_context_but_preserves_outside_unicode_crlf() {
        let source = "\\begin{itemize}\r\n\\item 😀中文\r\n\\item untouched\r\n\\end{itemize}\r\n";
        let formatted = source.replace("\\item", "    \\item").replace("\r\n", "\n");
        let start = source.find("\\item").unwrap();
        let end = source.find("\\item untouched").unwrap();
        assert_eq!(
            apply(source, &formatted, start, end),
            source.replacen("\\item", "    \\item", 1)
        );
    }

    #[test]
    fn align_after_wrapping_outside_range_and_skip_crossing_hunks() {
        let source = "alpha beta gamma\nkeep\nselected\nend\n";
        let formatted = "alpha beta\ngamma\nkeep\n  selected\nend\n";
        let start = source.find("selected").unwrap();
        assert_eq!(
            apply(source, formatted, start, start + 8),
            source.replace("selected", "  selected")
        );
        let source = "long text\nsecond line\nend\n";
        let formatted = "long text second line\nend\n";
        assert_eq!(apply(source, formatted, 0, 9), source);
        assert_eq!(apply(source, formatted, 0, source.len()), formatted);
    }

    #[test]
    fn full_ranges_reconstruct_output_with_insertions_deletions_and_empty_files() {
        for (source, formatted) in [
            ("a\nb\nc\n", "  a\ninserted\n  b\n  c\n"),
            ("a\nb\nc\n", "  a\n  c\n"),
            ("a\nb\nc\n", "x\ny\nz\n"),
            ("a\n", ""),
            ("a", "a\n"),
            ("😀 a\r\n", "😀  a\n"),
        ] {
            assert_eq!(
                apply(source, formatted, 0, source.len()),
                formatted
                    .replace("\r\n", "\n")
                    .replace('\n', Text::new(source.into()).newline())
            );
        }
        assert_eq!(apply("", "inserted", 0, 0), "");
    }

    #[test]
    fn alignment_work_is_bounded_and_partial_lines_are_respected() {
        let source = "a\n".repeat(1100);
        let formatted = "b\n".repeat(1100);
        assert_eq!(apply(&source, &formatted, 0, 2), source);
        assert_eq!(apply(&source, &formatted, 0, source.len()), formatted);
        assert_eq!(apply("  a  \n", "a\n", 3, 5), "  a  \n");
        assert_eq!(apply("  a  \n", "a\n", 0, 6), "a\n");
    }

    #[test]
    fn invalid_ranges_reject_surrogates_reversed_and_nonexistent_positions() {
        let text = Text::new("😀\r\nx".into());
        for (start, end) in [
            ((0, 1), (0, 2)),
            ((0, 0), (2, 0)),
            ((1, 1), (0, 0)),
            ((0, 0), (0, 3)),
        ] {
            assert!(validate(
                &text,
                Range::new(Position::new(start.0, start.1), Position::new(end.0, end.1))
            )
            .is_err());
        }
        assert!(validate(&text, text.range(0, text.source.len())).is_ok());
    }
}
