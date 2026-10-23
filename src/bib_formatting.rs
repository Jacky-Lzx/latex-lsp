//! Format only proven whitespace outside bibliography values and comments.
use crate::{
    formatting,
    syntax::{self, Syntax},
    text::Text,
};
use std::{collections::BTreeSet, ops::Range as Span};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::{FormattingOptions, Range},
};

pub(crate) fn start(
    tasks: &crate::tasks::Tasks,
    options: FormattingOptions,
    source: String,
    range: Option<Range>,
) -> Result<crate::tasks::Task<String>> {
    formatting::validate_options(&options)?;
    tasks.start(move |cancel| run(source, &options, range, || cancel.cancelled()))
}

fn whitespace(
    source: &str,
    changes: &mut Vec<(Span<usize>, String)>,
    span: Span<usize>,
    value: &str,
) {
    if source[span.clone()]
        .bytes()
        .all(|byte| byte.is_ascii_whitespace())
        && source[span.clone()] != *value
    {
        changes.push((span, value.into()));
    }
}

fn run(
    source: String,
    options: &FormattingOptions,
    range: Option<Range>,
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    if cancelled() {
        return Err(Error::request_cancelled());
    }
    let text = Text::new(source);
    let selected = if let Some(range) = range {
        let range = crate::range_formatting::normalize(&text, range)?;
        Some(text.offset(range.start).unwrap()..text.offset(range.end).unwrap())
    } else {
        None
    };
    let syntax = Syntax::bibtex(&text);
    let source = &text.source;
    let bytes = source.as_bytes();
    let index = &syntax.bibliography_completion;
    let mut operands: Vec<_> = syntax
        .bibliography_strings
        .literal_spans
        .iter()
        .chain(&syntax.bibliography_strings.number_spans)
        .cloned()
        .chain(
            syntax
                .bibliography_strings
                .references
                .iter()
                .map(|entry| entry.span.clone()),
        )
        .collect();
    operands.sort_by_key(|span| span.start);
    let closed: BTreeSet<_> = syntax
        .bibliography_entries
        .iter()
        .map(|(span, _)| (span.start, span.end))
        .collect();
    let indent = if options.insert_spaces {
        " ".repeat(options.tab_size as usize)
    } else {
        "\t".into()
    };
    let prefix = format!("{}{indent}", text.newline());
    let mut changes = Vec::new();
    for entry in &index.entries {
        if cancelled() {
            return Err(Error::request_cancelled());
        }
        let Some(first) = entry.fields.first() else {
            continue;
        };
        if entry
            .fields
            .iter()
            .any(|field| !field.assigned || !field.value_valid)
        {
            continue;
        }
        let Some(owner) = index
            .types
            .partition_point(|kind| kind.span.start < first.span.start)
            .checked_sub(1)
            .and_then(|i| index.types.get(i))
        else {
            continue;
        };
        let Some(whole) = &owner.entry_span else {
            continue;
        };
        if !closed.contains(&(whole.start, whole.end)) {
            continue;
        }
        let Some(key) = &owner.key else {
            continue;
        };
        let body_end = whole.end - 1;
        let comma = syntax::skip_space(source, key.end);
        if bytes.get(comma) != Some(&b',') {
            continue;
        }
        let mut cursor = comma + 1;
        let mut local = Vec::new();
        let mut valid = true;
        let mut trailing_comma = false;
        for (i, field) in entry.fields.iter().enumerate() {
            if cancelled() {
                return Err(Error::request_cancelled());
            }
            let eq = syntax::skip_space(source, field.span.end);
            let value = syntax::skip_space(source, eq + 1);
            let last = operands
                .partition_point(|span| span.start < field.range.end)
                .checked_sub(1)
                .and_then(|i| operands.get(i));
            let Some(last) = last.filter(|span| value <= span.start && span.end <= field.range.end)
            else {
                valid = false;
                break;
            };
            let delimiter = syntax::skip_space(source, field.range.end);
            if cursor > field.span.start
                || bytes.get(eq) != Some(&b'=')
                || delimiter > body_end
                || (delimiter != body_end && bytes[delimiter] != b',')
                || (i + 1 < entry.fields.len() && delimiter == body_end)
            {
                valid = false;
                break;
            }
            whitespace(source, &mut local, cursor..field.span.start, &prefix);
            whitespace(source, &mut local, field.span.end..eq, " ");
            whitespace(source, &mut local, eq + 1..value, " ");
            whitespace(
                source,
                &mut local,
                last.end..delimiter,
                if delimiter == body_end {
                    text.newline()
                } else {
                    ""
                },
            );
            cursor = delimiter + usize::from(delimiter < body_end);
            trailing_comma = delimiter < body_end;
        }
        if valid && cursor <= body_end {
            if trailing_comma {
                whitespace(source, &mut local, cursor..body_end, text.newline());
            }
            changes.extend(local);
        }
    }
    changes.sort_by_key(|(span, _)| span.start);
    if let Some(selected) = selected {
        changes.retain(|(span, _)| {
            selected.start < selected.end
                && selected.start <= span.start
                && span.end <= selected.end
                && (span.start < selected.end || selected.end == source.len())
        });
    }
    if changes
        .windows(2)
        .any(|pair| pair[0].0.end > pair[1].0.start)
    {
        return Err(formatting::failure(
            "Overlapping bibliography formatting changes",
        ));
    }
    let mut length = source.len();
    for (span, replacement) in &changes {
        length = length
            .checked_sub(span.len())
            .and_then(|length| length.checked_add(replacement.len()))
            .ok_or_else(|| formatting::failure("Bibliography formatting output is too large"))?;
    }
    if length > 16 * 1024 * 1024 {
        return Err(formatting::failure("Formatter output exceeds 16 MiB"));
    }
    // Build linearly: repeated insertions into a large String would be quadratic.
    let mut output = String::with_capacity(length);
    let mut cursor = 0;
    for (span, replacement) in changes {
        if cancelled() {
            return Err(Error::request_cancelled());
        }
        output.push_str(&source[cursor..span.start]);
        output.push_str(&replacement);
        cursor = span.end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> FormattingOptions {
        FormattingOptions {
            tab_size: 2,
            insert_spaces: true,
            ..Default::default()
        }
    }
    fn format(source: &str) -> String {
        run(source.into(), &options(), None, || false).unwrap()
    }
    fn values(source: &str) -> Vec<String> {
        let text = Text::new(source.into());
        let syntax = Syntax::bibtex(&text);
        let mut spans = syntax.bibliography_strings.literal_spans;
        spans.extend(syntax.bibliography_strings.number_spans);
        spans.extend(
            syntax
                .bibliography_strings
                .references
                .into_iter()
                .map(|entry| entry.span),
        );
        spans.sort_by_key(|span| span.start);
        spans.into_iter().map(|span| source[span].into()).collect()
    }

    #[test]
    fn formats_complete_fields_preserving_keys_types_values_and_special_entries() {
        let source = "% heading\r\n@string {Short=\"Journal\"}\r\n@preamble{\"prefix\"}\r\n@BoOk(key,title={😀 {TeX},  raw},author = \"A {B}\",year=2026,month=jan # { extra},)\r\n@comment{untouched}\r\n";
        let expected = "% heading\r\n@string {Short=\"Journal\"}\r\n@preamble{\"prefix\"}\r\n@BoOk(key,\r\n  title = {😀 {TeX},  raw},\r\n  author = \"A {B}\",\r\n  year = 2026,\r\n  month = jan # { extra},\r\n)\r\n@comment{untouched}\r\n";
        let result = format(source);
        assert_eq!(result, expected);
        assert_eq!(values(source), values(&result));
        assert_eq!(format(&result), result);
    }

    #[test]
    fn preserves_comments_and_their_newlines_at_every_boundary() {
        let source = "@book{k,% lead\n title% name comment\n =% value comment\n {Text}% tail\n ,year=2026% final\n}";
        let expected = "@book{k,% lead\n title% name comment\n =% value comment\n {Text}% tail\n ,\n  year = 2026% final\n}";
        let result = format(source);
        assert_eq!(result, expected);
        assert_eq!(values(source), values(&result));
        assert_eq!(format(&result), result);
        let source = "@book{k,title={100% literal},% between\n year=2026, % closing\n}";
        let result = format(source);
        assert!(result.contains("{100% literal},% between\n year = 2026, % closing\n}"));
        assert_eq!(format(&result), result);
    }

    #[test]
    fn incomplete_or_invalid_entries_remain_unchanged_while_other_entries_format() {
        for source in [
            "@book{k,title={unterminated}",
            "@book{k,title=foo bar,year=2026}",
            "@book{k,title=,year=2026}",
            "@book{k,title}",
            "@book{key}",
            "@string{Foo={X}}",
            "@preamble{\"text\"}",
        ] {
            assert_eq!(format(source), source);
        }
        let source = "@book{k,title=foo bar}\n@book{good,title={Value}}";
        assert_eq!(
            format(source),
            "@book{k,title=foo bar}\n@book{good,\n  title = {Value}\n}"
        );
    }

    #[test]
    fn keeps_duplicate_fields_order_multiline_literals_and_mixed_line_endings() {
        let source = "@book{中,title={A\r\n  B\nC},TITLE=\"second\",year=2026}";
        let result = format(source);
        assert_eq!(
            result,
            "@book{中,\r\n  title = {A\r\n  B\nC},\r\n  TITLE = \"second\",\r\n  year = 2026\r\n}"
        );
        assert_eq!(values(source), values(&result));
        assert_eq!(format(&result), result);
        let result = run(
            "@book{k,title={T}}".into(),
            &FormattingOptions {
                insert_spaces: false,
                ..options()
            },
            None,
            || false,
        )
        .unwrap();
        assert_eq!(result, "@book{k,\n\ttitle = {T}\n}");
    }

    #[test]
    fn formats_only_selected_whitespace_even_for_adjacent_single_line_entries() {
        let first = "@book{one,title={T}}";
        let second = "@book{two,title={Outside}}";
        let source = format!("{first}{second}");
        let text = Text::new(source.clone());
        let result = run(
            source.clone(),
            &options(),
            Some(text.range(0, first.len())),
            || false,
        )
        .unwrap();
        assert_eq!(result, format!("{}{second}", format(first)));
        let point = text.range(5, 5);
        assert_eq!(
            run(source.clone(), &options(), Some(point), || false).unwrap(),
            source
        );
    }

    #[test]
    fn scales_to_many_entries_and_can_cancel_during_field_processing() {
        let source: String = (0..2000)
            .map(|i| format!("@book{{key{i},title={{T}},year=2026}}\n"))
            .collect();
        let result = format(&source);
        assert_eq!(result.matches("  title = {T}").count(), 2000);
        assert_eq!(format(&result), result);
        let calls = std::cell::Cell::new(0);
        let error = run(source, &options(), None, || {
            calls.set(calls.get() + 1);
            calls.get() > 5
        })
        .unwrap_err();
        assert_eq!(error.code, tower_lsp::jsonrpc::ErrorCode::RequestCancelled);
    }

    #[test]
    #[ignore = "requires bibtex on PATH"]
    fn real_bibtex_produces_identical_bbl_before_and_after_formatting() {
        let dir = tempfile::tempdir().unwrap();
        let source = "@string{pub=\"Publisher\"}\n@preamble{\"\\providecommand{\\extra}{Text}\"}\n@book{key,author=\"Writer, Alice\",title={A {TeX}\n  Book},publisher=pub # { House},year=2026,month=jan}\n";
        let formatted = format(source);
        assert_ne!(source, formatted);
        let mut results = Vec::new();
        for bibliography in [source, formatted.as_str()] {
            std::fs::write(dir.path().join("refs.bib"), bibliography).unwrap();
            std::fs::write(
                dir.path().join("main.aux"),
                "\\relax\n\\citation{key}\n\\bibdata{refs}\n\\bibstyle{plain}\n",
            )
            .unwrap();
            let output = std::process::Command::new("bibtex")
                .arg("main")
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            results.push(std::fs::read_to_string(dir.path().join("main.bbl")).unwrap());
        }
        assert_eq!(results[0], results[1]);
    }
}
