//! Recover file/line diagnostics from TeX's file-line errors and traditional logs.
use crate::text::Text;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ops::Range as ByteRange,
    path::{Path, PathBuf},
    rc::Rc,
};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Url};

const AUXILIARY_EXTENSIONS: &[&str] = &[
    "aux", "bbl", "toc", "lof", "lot", "out", "nav", "snm", "vrb",
];

fn log_extensions() -> impl Iterator<Item = &'static str> {
    crate::workspace::SOURCE_EXTENSIONS
        .iter()
        .copied()
        .chain(AUXILIARY_EXTENSIONS.iter().copied())
}

fn log_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| log_extensions().any(|candidate| candidate == extension))
}

/// TeX's byte-based print width can split a UTF-8 scalar across a line break.
/// Remove a break only when the surrounding bytes form a valid scalar.
pub(crate) fn decode_output(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Cow::Borrowed(text);
    }
    let mut repaired = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let width = match bytes[i] {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        let mut scalar = [0; 4];
        scalar[0] = bytes[i];
        let mut count = 1;
        let mut after = i + 1;
        let mut wrapped = false;
        while count < width && after < bytes.len() {
            if !wrapped && bytes[after] == b'\n' {
                wrapped = true;
                after += 1;
            } else if !wrapped && bytes[after..].starts_with(b"\r\n") {
                wrapped = true;
                after += 2;
            } else if (0x80..=0xbf).contains(&bytes[after]) {
                scalar[count] = bytes[after];
                count += 1;
                after += 1;
            } else {
                break;
            }
        }
        if wrapped && count == width && std::str::from_utf8(&scalar[..width]).is_ok() {
            repaired.extend_from_slice(&scalar[..width]);
            i = after;
        } else {
            repaired.push(bytes[i]);
            i += 1;
        }
    }
    Cow::Owned(String::from_utf8_lossy(&repaired).into_owned())
}

fn output_summary_pages(summary: &str) -> Option<u32> {
    let (filename, tail) = summary
        .strip_prefix("Output written on ")?
        .rsplit_once(" (")?;
    let filename = if let Some(filename) = filename.strip_prefix('"') {
        filename.strip_suffix('"')?
    } else {
        filename
    };
    // latexmk can produce a PDF from XeTeX's intermediate XDV or a DVI log.
    // A directory component containing `(9 pages).` is not an output summary.
    if filename.contains('"')
        || ![".pdf", ".dvi", ".xdv"]
            .iter()
            .any(|extension| filename.ends_with(extension))
    {
        return None;
    }
    let (pages, tail) = tail.split_once(" page")?;
    let tail = tail.strip_prefix('s').unwrap_or(tail);
    let valid = tail == ")."
        || tail
            .strip_prefix(", ")
            .and_then(|bytes| {
                bytes
                    .strip_suffix(" bytes).")
                    .or_else(|| bytes.strip_suffix(" byte)."))
            })
            .is_some_and(|bytes| {
                !bytes.is_empty()
                    && bytes.bytes().all(|byte| byte.is_ascii_digit())
                    && bytes.parse::<u64>().is_ok()
            });
    if !valid || pages.is_empty() || !pages.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    pages.parse::<u32>().ok().filter(|pages| *pages > 0)
}

pub(crate) enum OutputSummary {
    Pages(u32),
    NoPages,
}

fn no_pages_report(line: &str) -> bool {
    line == "No pages of output."
        || line
            .strip_prefix("warning")
            .is_some_and(|tail| tail.trim_start() == "(pdf backend): no pages of output.")
}

/// An explicit no-output report supersedes page counts from earlier passes.
pub(crate) fn output_summary(log: &str) -> Option<OutputSummary> {
    let lines = log.lines().collect::<Vec<_>>();
    let index = lines
        .iter()
        .rposition(|line| line.starts_with("Output written on ") || no_pages_report(line))?;
    if no_pages_report(lines[index]) {
        // An omitted later pass could have produced output. Only reject a PDF
        // when no hidden output can supersede the observed no-pages report.
        return (!lines[index + 1..].contains(&crate::output::TRUNCATED))
            .then_some(OutputSummary::NoPages);
    }
    let mut summary = String::new();
    for (offset, line) in lines.iter().skip(index).take(128).enumerate() {
        if *line == crate::output::TRUNCATED || summary.len() + line.len() > 8192 {
            return None;
        }
        if offset > 0
            && (line.is_empty()
                || *line == ")"
                || [
                    "Transcript written on ",
                    "PDF statistics:",
                    "SyncTeX written on ",
                    "Latexmk:",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix)))
        {
            return output_summary_pages(&summary).map(OutputSummary::Pages);
        }
        summary.push_str(line);
        // Read to a real footer or EOF before accepting a summary. A path
        // can itself contain `main.pdf (9 pages).` at a wrap boundary.
        if index + offset + 1 == lines.len() {
            return output_summary_pages(&summary).map(OutputSummary::Pages);
        }
    }
    None
}

/// Read TeX's final PDF summary, including wrapped paths and page/byte counts.
pub fn page_count(log: &str) -> Option<u32> {
    match output_summary(log)? {
        OutputSummary::Pages(pages) => Some(pages),
        OutputSummary::NoPages => None,
    }
}

fn location(root: &Path, name: &str) -> Option<PathBuf> {
    let path = root.parent()?.join(name.trim().trim_matches('"'));
    let path = path.canonicalize().ok()?;
    path.is_file().then_some(path)
}

fn numbered_error(line: &str) -> Option<(&str, u32, &str)> {
    // Test each colon rather than splitting once: Windows drive letters and
    // filenames containing colons must not be mistaken for the line separator.
    for (colon, _) in line.match_indices(':') {
        let tail = &line[colon + 1..];
        let (number, message) = tail.split_once(':')?;
        if let Ok(number) = number.parse::<u32>() {
            let path = line[..colon].trim();
            if log_source(Path::new(path.trim_matches('"'))) {
                return Some((path, number, message.trim_start()));
            }
        }
    }
    None
}

fn input_line(message: &str) -> Option<u32> {
    let tail = message
        .rsplit_once("input line ")
        .or_else(|| message.rsplit_once("at lines "))
        .or_else(|| message.rsplit_once("at line "))?
        .1;
    tail.trim_start()
        .split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn undefined_reference_start(message: &str) -> Option<(bool, &str)> {
    // Reference-like text in a filename or an example is ordinary warning
    // prose. Only classify the body of a standard LaTeX warning header.
    let body = message
        .strip_prefix("LaTeX Warning:")
        .or_else(|| {
            let named = message
                .strip_prefix("Package ")
                .or_else(|| message.strip_prefix("Class "))?;
            let (name, body) = named.split_once(" Warning:")?;
            (!name.is_empty()).then_some(body)
        })?
        .trim_start();
    if let Some(tail) = body
        .strip_prefix("Reference `")
        .or_else(|| body.strip_prefix("Reference '"))
        .or_else(|| body.strip_prefix("Hyper reference `"))
        .or_else(|| body.strip_prefix("Hyper reference '"))
    {
        Some((false, tail))
    } else if let Some(tail) = body
        .strip_prefix("Citation `")
        .or_else(|| body.strip_prefix("Citation '"))
    {
        Some((true, tail))
    } else {
        None
    }
}

pub(crate) fn undefined_reference_key(message: &str) -> Option<(bool, &str)> {
    fn undefined_tail(tail: &str) -> bool {
        let normalized = tail.split_whitespace().collect::<Vec<_>>().join(" ");
        let tail = normalized.as_str();
        if tail.is_empty() || tail == "." {
            return true;
        }
        // Some packages repeat their name on a continuation line.
        let tail = if tail.starts_with('(') {
            let Some((_, rest)) = tail.split_once(')') else {
                return false;
            };
            rest.trim_start()
        } else {
            tail
        };
        let Some(line) = tail.strip_prefix("on input line ") else {
            return false;
        };
        let number = line.strip_suffix('.').unwrap_or(line);
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    }
    let (citation, tail) = undefined_reference_start(message)?;
    // The status must follow the closing key quote. A key or page name
    // containing "undefined" is insufficient evidence of an unresolved use.
    let key = tail
        .rsplit_once("' on ")
        .filter(|(_, context)| {
            context
                .strip_prefix("page ")
                .and_then(|page| page.rsplit_once(" undefined"))
                .is_some_and(|(_, tail)| undefined_tail(tail))
        })
        .map(|(key, _)| key)
        .or_else(|| {
            tail.rsplit_once("' undefined")
                .filter(|(_, tail)| undefined_tail(tail))
                .map(|(key, _)| key)
        })?;
    Some((citation, key))
}

fn duplicate_label_start(message: &str) -> Option<&str> {
    message
        .strip_prefix("LaTeX Warning: Label `")
        .or_else(|| message.strip_prefix("LaTeX Warning: Label '"))
}

pub(crate) fn duplicate_label_key(message: &str) -> Option<&str> {
    let key = duplicate_label_start(message)?.strip_suffix("' multiply defined.")?;
    (!key.is_empty()).then_some(key)
}

enum BiblatexRerun {
    Latex,
    Bibliography(&'static str),
}

fn biblatex_rerun(diagnostic: &Diagnostic) -> Option<BiblatexRerun> {
    if diagnostic.severity != Some(DiagnosticSeverity::WARNING) {
        return None;
    }
    let body = diagnostic
        .message
        .strip_prefix("Package biblatex Warning:")?
        .split_whitespace()
        .filter(|word| *word != "(biblatex)")
        .collect::<Vec<_>>()
        .join(" ");
    if matches!(
        body.as_str(),
        "Please rerun LaTeX." | "Please rerun LaTeX. Page breaks have changed."
    ) || body
        .strip_prefix("No localisation for language '")
        .and_then(|tail| tail.rsplit_once("' loaded. "))
        .is_some_and(|(language, tail)| {
            !language.is_empty()
                && tail == "Please rerun LaTeX to ensure languages can be requested properly."
        })
    {
        return Some(BiblatexRerun::Latex);
    }
    for (tool, prefix) in [
        ("biber", "Please (re)run Biber on the file: "),
        ("bibtex", "Please (re)run BibTeX on the file(s): "),
    ] {
        if body
            .strip_prefix(prefix)
            .and_then(|tail| tail.strip_suffix(" and rerun LaTeX afterwards."))
            .is_some_and(|files| !files.trim().is_empty())
        {
            return Some(BiblatexRerun::Bibliography(tool));
        }
    }
    None
}

pub(crate) fn required_bibliography_tool(diagnostic: &Diagnostic) -> Option<&'static str> {
    match biblatex_rerun(diagnostic)? {
        BiblatexRerun::Bibliography(tool) => Some(tool),
        BiblatexRerun::Latex => None,
    }
}

fn rerunfilecheck_request(message: &str) -> bool {
    message
        .strip_prefix("Package rerunfilecheck Warning: File `")
        .and_then(|tail| tail.rsplit_once("' has changed."))
        .is_some_and(|(file, tail)| {
            let request = tail
                .split_whitespace()
                .filter(|word| *word != "(rerunfilecheck)")
                .collect::<Vec<_>>()
                .join(" ");
            !file.trim().is_empty()
                && matches!(
                    request.as_str(),
                    "" | "Rerun."
                        | "Rerun to get outlines right."
                        | "Rerun to get outlines right or use package `bookmark'."
                )
        })
}

/// Share the confirmed rerun warning grammar with analysis and editor actions.
pub(crate) fn rerun_required(diagnostic: &Diagnostic) -> bool {
    if diagnostic.severity != Some(DiagnosticSeverity::WARNING) {
        return false;
    }
    if biblatex_rerun(diagnostic).is_some() {
        return true;
    }
    let message = diagnostic
        .message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    matches!(message.as_str(),
        "LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right."
        | "Package natbib Warning: Citation(s) may have changed. Rerun to get citations correct."
        | "Package natbib Warning: Citation(s) may have changed."
        | "Package longtable Warning: Table widths have changed. Rerun LaTeX."
        | "Package hyperref Warning: Rerun to get /PageLabels entry."
    ) || rerunfilecheck_request(&message)
}

fn generated_line_note(line: u32) -> String {
    format!(" (reported line {line} is no longer present in the generated file)")
}

enum FontSubstitution<'a> {
    Shape(&'a str, &'a str),
    Size(&'a str, &'a str, &'a str),
}

fn font_identity(value: &str) -> bool {
    let fields: Vec<_> = value.split('/').collect();
    fields.len() == 4
        && fields.iter().all(|field| !field.is_empty())
        && !value.chars().any(char::is_control)
}

fn font_size(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    let mut point = false;
    let mut digit = false;
    for byte in value.bytes() {
        if byte.is_ascii_digit() {
            digit = true;
        } else if byte == b'.' && !point {
            point = true;
        } else {
            return false;
        }
    }
    digit
}

fn font_warning_body(message: &str) -> Option<&str> {
    let body = message.strip_prefix("LaTeX Font Warning: Font shape ")?;
    body.strip_prefix('`').or_else(|| body.strip_prefix('\''))
}

fn font_continuation(tail: &str) -> &str {
    let tail = tail.trim_start();
    tail.strip_prefix("(Font)").unwrap_or(tail).trim_start()
}

fn font_warning_end(tail: &str) -> bool {
    let tail = tail.split_whitespace().collect::<Vec<_>>().join(" ");
    tail.strip_prefix("on input line ")
        .and_then(|line| line.strip_suffix('.'))
        .is_some_and(|line| !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()))
}

fn font_substitution(message: &str) -> Option<FontSubstitution<'_>> {
    let body = font_warning_body(message)?;
    if let Some((requested, tail)) = body.rsplit_once("' undefined") {
        let tail = font_continuation(tail).strip_prefix("using ")?.trim_start();
        let tail = tail.strip_prefix('`').or_else(|| tail.strip_prefix('\''))?;
        let (replacement, tail) = tail.rsplit_once("' instead")?;
        return (font_identity(requested) && font_identity(replacement) && font_warning_end(tail))
            .then_some(FontSubstitution::Shape(requested, replacement));
    }
    let (font, tail) = body.rsplit_once("' in size <")?;
    let (requested, tail) = tail.split_once("> not available")?;
    let tail = font_continuation(tail).strip_prefix("size <")?;
    let (replacement, tail) = tail.split_once("> substituted")?;
    (font_identity(font)
        && font_size(requested)
        && font_size(replacement)
        && font_warning_end(tail))
    .then_some(FontSubstitution::Size(font, requested, replacement))
}

#[derive(Clone, Copy)]
enum BoxMetric<'a> {
    Excess(&'a str),
    Badness(u32),
}

struct BoxWarning<'a> {
    kind: &'static str,
    axis: &'static str,
    metric: BoxMetric<'a>,
}

struct BoxSummary<'a> {
    count: usize,
    maximum: BoxMetric<'a>,
}

// Compare the compiler's decimal strings exactly, including distinctions that
// would disappear in floating point. Equal values use a stable spelling.
fn compare_box_excess(left: &str, right: &str) -> std::cmp::Ordering {
    fn parts(value: &str) -> (&str, &str) {
        let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
        (
            integer.trim_start_matches('0'),
            fraction.trim_end_matches('0'),
        )
    }
    let (left_integer, left_fraction) = parts(left);
    let (right_integer, right_fraction) = parts(right);
    left_integer
        .len()
        .cmp(&right_integer.len())
        .then_with(|| left_integer.cmp(right_integer))
        .then_with(|| left_fraction.cmp(right_fraction))
        .then_with(|| right.cmp(left))
}

const OVERSIZED_FLOAT_PREFIX: &str = "LaTeX Warning: Float too large for page by ";

fn nonnegative_decimal(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
        && value
            .parse::<f64>()
            .is_ok_and(|value| value.is_finite() && value >= 0.0)
}

fn oversized_float_excess(message: &str) -> Option<&str> {
    let (amount, line) = message
        .strip_prefix(OVERSIZED_FLOAT_PREFIX)?
        .split_once("pt on input line ")?;
    let line = line.strip_suffix('.')?;
    (nonnegative_decimal(amount)
        && !line.is_empty()
        && line.bytes().all(|byte| byte.is_ascii_digit())
        && line.parse::<u32>().is_ok())
    .then_some(amount)
}

/// Summarize observed final-log diagnostics without multiplying repeated uses.
pub fn compilation_analysis(
    diagnostics: &BTreeMap<Url, Vec<Diagnostic>>,
    diagnostics_truncated: bool,
) -> serde_json::Value {
    const MAX_KEYS: usize = 128;
    const MAX_KEY_CHARS: usize = 128;
    const MAX_GLYPH_FONTS: usize = 8;
    const MAX_FONT_CHARS: usize = 256;
    fn record_key(keys: &mut BTreeSet<String>, key: &str, truncated: &mut bool) {
        if key.chars().count() > MAX_KEY_CHARS || (keys.len() == MAX_KEYS && !keys.contains(key)) {
            *truncated = true;
        } else {
            keys.insert(key.to_string());
        }
    }
    fn record_substitution<T: Ord>(records: &mut BTreeSet<T>, item: T, truncated: &mut bool) {
        if records.len() == MAX_KEYS && !records.contains(&item) {
            *truncated = true;
        } else {
            records.insert(item);
        }
    }
    let mut references = BTreeSet::new();
    let mut citations = BTreeSet::new();
    let mut missing_destinations = BTreeSet::new();
    let mut duplicate_destinations = BTreeSet::new();
    let mut duplicate_labels = BTreeSet::new();
    let mut glyphs: BTreeMap<char, (DiagnosticSeverity, BTreeSet<String>)> = BTreeMap::new();
    let mut unsupported_unicode = BTreeSet::new();
    let mut bibliography_tools = BTreeSet::new();
    let mut font_substitutions = BTreeSet::new();
    let mut font_size_substitutions = BTreeSet::new();
    let mut box_warnings: BTreeMap<_, BoxSummary<'_>> = BTreeMap::new();
    let mut oversized_float_count = 0usize;
    let mut max_float_excess: Option<&str> = None;
    let mut rerun = false;
    let mut truncated = diagnostics_truncated;
    for diagnostic in diagnostics.values().flatten().filter(|diagnostic| {
        matches!(
            diagnostic.severity,
            Some(DiagnosticSeverity::ERROR | DiagnosticSeverity::WARNING)
        )
    }) {
        // Generated auxiliary files can have been rewritten since this error.
        // The source-line explanation added for display is not compiler text.
        let reported_message = diagnostic
            .data
            .as_ref()
            .filter(|data| data["unresolvedLine"] == true)
            .and_then(|data| data["reportedLine"].as_u64())
            .and_then(|line| u32::try_from(line).ok())
            .and_then(|line| {
                diagnostic
                    .message
                    .strip_suffix(generated_line_note(line).as_str())
            })
            .unwrap_or(&diagnostic.message);
        if diagnostic.severity == Some(DiagnosticSeverity::ERROR) {
            if let Some(character) = unsupported_unicode_character(reported_message) {
                record_substitution(&mut unsupported_unicode, character, &mut truncated);
            }
        }
        if let Some((character, font)) = missing_character_details(reported_message) {
            if glyphs.len() == MAX_KEYS && !glyphs.contains_key(&character) {
                truncated = true;
            } else {
                let severity = diagnostic.severity.expect("error or warning");
                let (level, fonts) = glyphs
                    .entry(character)
                    .or_insert_with(|| (severity, BTreeSet::new()));
                *level = (*level).min(severity);
                if font.chars().count() > MAX_FONT_CHARS
                    || (fonts.len() == MAX_GLYPH_FONTS && !fonts.contains(font))
                {
                    truncated = true;
                } else {
                    fonts.insert(font.to_string());
                }
            }
        }
        if diagnostic.severity != Some(DiagnosticSeverity::WARNING) {
            continue;
        }
        if let Some(excess) = oversized_float_excess(reported_message) {
            if excess.len() > MAX_KEY_CHARS {
                truncated = true;
            } else {
                oversized_float_count += 1;
                if max_float_excess
                    .is_none_or(|maximum| compare_box_excess(excess, maximum).is_gt())
                {
                    max_float_excess = Some(excess);
                }
            }
        }
        if let Some(warning) = box_warning_details(reported_message) {
            if matches!(warning.metric, BoxMetric::Excess(value) if value.len() > MAX_KEY_CHARS) {
                truncated = true;
            } else {
                let summary =
                    box_warnings
                        .entry((warning.kind, warning.axis))
                        .or_insert(BoxSummary {
                            count: 0,
                            maximum: warning.metric,
                        });
                summary.count += 1;
                match (warning.metric, &mut summary.maximum) {
                    (BoxMetric::Excess(value), BoxMetric::Excess(maximum)) => {
                        if compare_box_excess(value, maximum).is_gt() {
                            *maximum = value;
                        }
                    }
                    (BoxMetric::Badness(value), BoxMetric::Badness(maximum)) => {
                        *maximum = (*maximum).max(value);
                    }
                    _ => unreachable!("each box warning kind has one metric"),
                }
            }
        }
        match font_substitution(reported_message) {
            Some(FontSubstitution::Shape(requested, replacement)) => {
                if [requested, replacement]
                    .iter()
                    .any(|name| name.chars().count() > MAX_FONT_CHARS)
                {
                    truncated = true;
                } else {
                    record_substitution(
                        &mut font_substitutions,
                        (requested.to_owned(), replacement.to_owned()),
                        &mut truncated,
                    );
                }
            }
            Some(FontSubstitution::Size(font, requested, replacement)) => {
                if font.chars().count() > MAX_FONT_CHARS
                    || [requested, replacement]
                        .iter()
                        .any(|size| size.len() > MAX_KEY_CHARS)
                {
                    truncated = true;
                } else {
                    record_substitution(
                        &mut font_size_substitutions,
                        (
                            font.to_owned(),
                            requested.to_owned(),
                            replacement.to_owned(),
                        ),
                        &mut truncated,
                    );
                }
            }
            None => {}
        }
        rerun = rerun || rerun_required(diagnostic);
        if let Some(tool) = required_bibliography_tool(diagnostic) {
            bibliography_tools.insert(tool);
        }
        if let Some((citation, key)) = undefined_reference_key(&diagnostic.message) {
            let keys = if citation {
                &mut citations
            } else {
                &mut references
            };
            record_key(keys, key, &mut truncated);
        }
        if let Some(key) = duplicate_label_key(&diagnostic.message) {
            record_key(&mut duplicate_labels, key, &mut truncated);
        }
        if let Some((kind, name)) = destination_warning(&diagnostic.message) {
            let keys = match kind {
                DestinationWarningKind::Missing => &mut missing_destinations,
                DestinationWarningKind::Duplicate => &mut duplicate_destinations,
            };
            record_key(keys, &name, &mut truncated);
        }
    }
    let missing_characters: Vec<_> = glyphs
        .into_iter()
        .map(|(character, (severity, fonts))| {
            serde_json::json!({
                "character": character.to_string(),
                "codepoint": format!("U+{:04X}", character as u32),
                "severity": severity,
                "fonts": fonts,
            })
        })
        .collect();
    let font_substitutions: Vec<_> = font_substitutions.into_iter().map(|(requested, replacement)| {
        serde_json::json!({"requested":requested, "replacement":replacement})
    }).collect();
    let unsupported_unicode: Vec<_> = unsupported_unicode.into_iter().map(|character| {
        serde_json::json!({"character":character.to_string(), "codepoint":format!("U+{:04X}", character as u32)})
    }).collect();
    let font_size_substitutions: Vec<_> = font_size_substitutions.into_iter().map(|(font, requested, replacement)| {
        serde_json::json!({"font":font, "requestedSize":requested, "replacementSize":replacement})
    }).collect();
    let box_warnings: Vec<_> = box_warnings
        .into_iter()
        .map(|((kind, axis), summary)| {
            let mut value = serde_json::json!({"kind":kind, "axis":axis, "count":summary.count});
            match summary.maximum {
                BoxMetric::Excess(maximum) => value["maxExcessPt"] = maximum.into(),
                BoxMetric::Badness(maximum) => value["maxBadness"] = maximum.into(),
            }
            value
        })
        .collect();
    serde_json::json!({
        "rerunRequired": rerun,
        "requiredBibliographyTools": bibliography_tools,
        "unresolvedReferences": references,
        "unresolvedCitations": citations,
        "unresolvedDestinations": missing_destinations,
        "duplicateDestinations": duplicate_destinations,
        "duplicateLabels": duplicate_labels,
        "missingCharacters": missing_characters,
        "unsupportedUnicodeCharacters": unsupported_unicode,
        "fontSubstitutions": font_substitutions,
        "fontSizeSubstitutions": font_size_substitutions,
        "boxWarnings": box_warnings,
        "oversizedFloats": {"count": oversized_float_count, "maxExcessPt": max_float_excess},
        "truncated": truncated,
    })
}

fn destination_warning_name(message: &str) -> Option<&str> {
    if let Some(tail) = message.strip_prefix("pdfTeX warning (dest): name{") {
        let (name, tail) = tail.split_once("} has been referenced")?;
        let tail: String = tail.chars().filter(|ch| !ch.is_whitespace()).collect();
        if tail != "butdoesnotexist,replacedbyafixedone" {
            return None;
        }
        Some(name)
    } else {
        message
            .strip_prefix("warning")?
            .trim_start()
            .strip_prefix("(pdf backend): unreferenced destination with name '")?
            .strip_suffix('\'')
    }
}

fn duplicate_destination_warning_name(message: &str) -> Option<&str> {
    if let Some(tail) =
        message.strip_prefix("pdfTeX warning (ext4): destination with the same identifier (name{")
    {
        let (name, tail) = tail.split_once("}) has been already used")?;
        let tail: String = tail.chars().filter(|ch| !ch.is_whitespace()).collect();
        (tail == ",duplicateignored").then_some(name)
    } else {
        message
            .strip_prefix("warning")?
            .trim_start()
            .strip_prefix("(pdf backend): ignoring duplicate destination with the name '")?
            .strip_suffix('\'')
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DestinationWarningKind {
    Missing,
    Duplicate,
}

impl DestinationWarningKind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Missing => "undefined-pdf-destination",
            Self::Duplicate => "duplicate-pdf-destination",
        }
    }
}

/// Recognized pdfTeX and LuaTeX warnings can contain PDF string escapes.
/// Decode only static names, preserving raw Unicode from unescaped backends.
fn literal_destination_name(name: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(name.len());
    let mut bytes = name.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        match bytes.next()? {
            paren @ (b'(' | b')') => decoded.push(paren),
            first @ b'0'..=b'7' => {
                let second = bytes.next()?;
                let third = bytes.next()?;
                if !matches!(second, b'0'..=b'7') || !matches!(third, b'0'..=b'7') {
                    return None;
                }
                let value = u16::from(first - b'0') * 64
                    + u16::from(second - b'0') * 8
                    + u16::from(third - b'0');
                decoded.push(u8::try_from(value).ok()?);
            }
            _ => return None,
        }
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (crate::syntax::is_static_reference_key(&decoded) && !decoded.chars().any(char::is_control))
        .then_some(decoded)
}

pub(crate) fn destination_warning(message: &str) -> Option<(DestinationWarningKind, String)> {
    let (kind, name) = if let Some(name) = destination_warning_name(message) {
        (DestinationWarningKind::Missing, name)
    } else {
        (
            DestinationWarningKind::Duplicate,
            duplicate_destination_warning_name(message)?,
        )
    };
    literal_destination_name(name).map(|name| (kind, name))
}

pub(crate) fn undefined_destination_name(message: &str) -> Option<String> {
    literal_destination_name(destination_warning_name(message)?)
}

fn box_code(message: &str) -> Option<&'static str> {
    let code = if message.starts_with("Overfull ") {
        "overfull-box"
    } else if message.starts_with("Underfull ") {
        "underfull-box"
    } else {
        return None;
    };
    Some(code)
}

fn box_warning_details(message: &str) -> Option<BoxWarning<'_>> {
    fn number(value: &str) -> bool {
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && value.parse::<u32>().is_ok()
    }
    let (overfull, tail) = if let Some(tail) = message.strip_prefix("Overfull ") {
        (true, tail)
    } else {
        (false, message.strip_prefix("Underfull ")?)
    };
    let (horizontal, tail) = if let Some(tail) = tail.strip_prefix("\\hbox (") {
        (true, tail)
    } else {
        (false, tail.strip_prefix("\\vbox (")?)
    };
    let (amount, context) = tail.split_once(") ")?;
    let metric = if overfull {
        amount
            .strip_suffix(if horizontal {
                "pt too wide"
            } else {
                "pt too high"
            })
            .filter(|value| nonnegative_decimal(value))
            .map(BoxMetric::Excess)?
    } else {
        BoxMetric::Badness(
            amount
                .strip_prefix("badness ")
                .filter(|value| number(value))?
                .parse()
                .ok()?,
        )
    };
    let context = context.strip_suffix(" []").unwrap_or(context);
    let valid_context = if let Some(lines) = context
        .strip_prefix("in paragraph at lines ")
        .or_else(|| context.strip_prefix("in alignment at lines "))
    {
        lines
            .split_once("--")
            .is_some_and(|(start, end)| number(start) && number(end))
    } else if let Some(line) = context.strip_prefix("detected at line ") {
        number(line)
    } else {
        context == "has occurred while \\output is active"
    };
    valid_context.then_some(BoxWarning {
        kind: if overfull { "overfull" } else { "underfull" },
        axis: if horizontal { "horizontal" } else { "vertical" },
        metric,
    })
}

fn complete_box_header(message: &str) -> bool {
    box_warning_details(message).is_some()
}

/// Recover only a complete engine box header. Box dumps and speculative source
/// lines remain outside the diagnostic unless they complete this known grammar.
fn wrapped_box_message(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
    initial: &str,
) -> String {
    let mut message = initial.to_owned();
    let mut complete = None;
    for index in (*i..).take(16) {
        if complete_box_header(&message) {
            complete = Some((message.clone(), index));
        }
        // TeX permits max_print_line values starting at 60. Even a complete
        // header at that width can end halfway through its final line number.
        if lines[index].len() < 60 || message.len() >= 4096 {
            break;
        }
        let Some(next) = lines.get(index + 1) else {
            break;
        };
        let starts_header = message.is_empty()
            && (next.trim_start().starts_with("Overfull ")
                || next.trim_start().starts_with("Underfull "));
        if next.trim().is_empty()
            || (message_boundary(next) && !next.starts_with(") ") && !starts_header)
            || next.trim_start().starts_with(['<', '['])
            || message.len() + next.len() > 4096
        {
            break;
        }
        let mut next_index = index + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        message.push_str(if message.is_empty() {
            next.trim_start()
        } else {
            next
        });
    }
    if let Some((message, end)) = complete {
        *i = end;
        message
    } else {
        initial.to_owned()
    }
}

fn unsupported_unicode_character(message: &str) -> Option<char> {
    let tail = message.strip_prefix("LaTeX Error: Unicode character ")?;
    let (printed, tail) = tail.split_once(" (U+")?;
    let (hex, ending) = tail.split_once(')')?;
    if !(4..=6).contains(&hex.len())
        || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !ending.starts_with(char::is_whitespace)
        || !ending
            .split_whitespace()
            .eq(["not", "set", "up", "for", "use", "with", "LaTeX."])
    {
        return None;
    }
    let character = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
    (printed == character.to_string()).then_some(character)
}

fn missing_character(message: &str) -> Option<char> {
    missing_character_details(message).map(|(character, _)| character)
}

fn missing_character_details(message: &str) -> Option<(char, &str)> {
    let tail = message.strip_prefix("Missing character: There is no ")?;
    let (printed, tail) = tail.split_once(" (U+")?;
    let (hex, font) = tail.split_once(") in font ")?;
    if !(4..=6).contains(&hex.len())
        || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        || font.trim().is_empty()
    {
        return None;
    }
    let character = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
    // TeX terminates the printed font descriptor with punctuation. Remove one
    // terminator, preserving punctuation that belongs to the font itself.
    let font = font.trim();
    let font = font
        .strip_suffix('.')
        .or_else(|| font.strip_suffix('!'))
        .unwrap_or(font);
    (printed == character.to_string() && !font.is_empty()).then_some((character, font))
}

fn missing_character_ranges(
    text: &Text,
    line: u32,
    context: &str,
    character: char,
) -> Option<Vec<Range>> {
    let prefix = context.trim_end();
    if character.is_whitespace() || prefix.is_empty() {
        return None;
    }
    let start = text.offset(Position::new(line, 0))?;
    let value = text.source[start..].lines().next()?;
    let end = if let Some(suffix) = shortened_context(prefix) {
        if suffix.is_empty() || !unique_context_on_line(text, start, suffix) {
            return None;
        }
        start + value.find(suffix)? + suffix.len()
    } else {
        if !value.starts_with(prefix) {
            return None;
        }
        start + prefix.len()
    };
    let parsed = crate::syntax::Syntax::latex(text);
    // XeTeX shapes a complete literal run before reporting its missing glyphs.
    // All messages for that run can therefore share the same consumed prefix.
    let consumed = &text.source[start..end];
    let boundary = consumed.char_indices().rev().find(|(_, ch)| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\\' | '{' | '}' | '%' | '$' | '&' | '#' | '^' | '_' | '~'
            )
    });
    let run_start = match boundary {
        Some((offset, ch)) if ch.is_whitespace() => {
            // Blanks immediately after a control word are discarded by TeX.
            // A preceding macro may have generated text in this same run.
            let before = start + consumed[..offset].trim_end().len();
            if parsed.commands.iter().any(|command| {
                command.span.end == before
                    && crate::syntax::control_word(&text.source, command.span.start).is_some()
            }) {
                return None;
            }
            start + offset + ch.len_utf8()
        }
        Some(_) => return None,
        None => start,
    };
    let bodies = crate::signature::definition_body_spans(&text.source, &parsed);
    let ranges: Vec<_> = text.source[run_start..end]
        .char_indices()
        .filter(|(_, ch)| *ch == character)
        .map(|(offset, _)| run_start + offset)
        .filter(|at| {
            !parsed.is_ignored(*at)
                && !bodies.iter().any(|body| body.contains(at))
                && !parsed
                    .commands
                    .iter()
                    .any(|command| command.span.contains(at))
        })
        .map(|at| text.range(at, at + character.len_utf8()))
        .collect();
    (!ranges.is_empty()).then_some(ranges)
}

fn box_range(text: &Text, message: &str) -> Option<Range> {
    box_code(message)?;
    let tail = message.rsplit_once("at lines ")?.1;
    let (start, end) = tail.split_once("--")?;
    let start: u32 = start.parse().ok()?;
    let end: u32 = end
        .split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    if start == 0 || end < start {
        return None;
    }
    let last = text.text_line_count().saturating_sub(1) as u32;
    let start = (start - 1).min(last);
    let end = (end - 1).min(last);
    let first = text.source.lines().nth(start as usize).unwrap_or("");
    let last = text.source.lines().nth(end as usize).unwrap_or("");
    Some(Range::new(
        Position::new(
            start,
            first
                .chars()
                .take_while(|ch| ch.is_whitespace())
                .map(char::len_utf16)
                .sum::<usize>() as u32,
        ),
        Position::new(end, last.encode_utf16().count() as u32),
    ))
}

fn missing_reported_file(
    path: &Path,
    lines: &[&str],
    index: &mut usize,
    command_name: &str,
    cached: Option<&InvocationCalls<'_>>,
    project_stale: bool,
) -> Option<String> {
    let line = lines[*index];
    let text = Text::new(std::fs::read_to_string(path).ok()?);
    let syntax = crate::syntax::Syntax::latex(&text);
    if command_name == "verbatiminput"
        && !crate::code_inputs::safe_syntax(&text.source, &syntax, command_name)
    {
        return None;
    }
    let indirect = if command_name == "include" {
        file_commands_for_text(&text, &syntax, cached, project_stale)
    } else {
        vec![]
    };
    let mut candidates = Vec::new();
    let mut matched = None;
    for command in syntax
        .commands
        .iter()
        .chain(&indirect)
        .filter(|command| command.name == command_name)
        .take(1024)
    {
        let Some(argument) = crate::syntax::file_argument(&text.source, command) else {
            continue;
        };
        let value = if command_name == "include" {
            argument.value.trim()
        } else {
            argument.value.as_str()
        };
        if value.is_empty() || value.contains(['\\', '{', '}', '%', '#', '$', '\r', '\n']) {
            continue;
        }
        let names = if command_name == "include" {
            crate::syntax::file_lookup_names(command_name, value).collect::<Vec<_>>()
        } else {
            vec![Cow::Borrowed(value)]
        };
        for name in names {
            let mut messages = vec![format!("No file {name}.")];
            if command_name == "include" {
                messages.push(format!("No file \"{name}\"."));
            }
            for message in messages {
                if message.len() <= 8192 && message.starts_with(line) {
                    if message == line {
                        matched = Some((name.to_string(), *index));
                    } else {
                        candidates.push((message, name.to_string()));
                    }
                }
            }
        }
    }
    // Join only prefixes of a known literal call. This avoids swallowing
    // adjacent diagnostics or interpreting ordinary optional-file messages.
    if candidates.is_empty() {
        return matched.map(|(name, _)| name);
    }
    let mut joined = line.to_owned();
    for (next_index, next) in lines.iter().enumerate().skip(*index + 1).take(64) {
        if joined.len() + next.len() > 8192 {
            break;
        }
        joined.push_str(next);
        candidates.retain(|(message, _)| message.starts_with(&joined));
        if let Some((_, name)) = candidates.iter().find(|(message, _)| message == &joined) {
            matched = Some((name.clone(), next_index));
            candidates.retain(|(message, _)| message != &joined);
        }
        if candidates.is_empty() {
            break;
        }
    }
    matched.map(|(name, last)| {
        *index = last;
        name
    })
}

pub(crate) fn missing_verbatim_name(message: &str) -> Option<&str> {
    message
        .strip_prefix("Verbatim input file `")?
        .strip_suffix("` not found")
}

fn emit_missing_verbatim(result: &mut BTreeMap<Url, Vec<Diagnostic>>, path: &Path, name: &str) {
    let (Ok(uri), Ok(source)) = (Url::from_file_path(path), std::fs::read_to_string(path)) else {
        return;
    };
    let text = Text::new(source);
    let parsed = crate::syntax::Syntax::latex(&text);
    if !crate::code_inputs::safe_syntax(&text.source, &parsed, "verbatiminput") {
        return;
    }
    let message = format!("Verbatim input file `{name}` not found");
    for command in parsed
        .commands
        .iter()
        .filter(|command| command.name == "verbatiminput")
        .take(1024)
    {
        let Some(argument) = crate::syntax::file_argument(&text.source, command)
            .filter(|argument| argument.value == name)
        else {
            continue;
        };
        let range = text.range(argument.span.start, argument.span.end);
        let items = result.entry(uri.clone()).or_default();
        if !items
            .iter()
            .any(|item| item.range == range && item.message == message)
        {
            items.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("latex-build".into()),
                code: Some(NumberOrString::String("missing-verbatim-input".into())),
                message: message.clone(),
                data: Some(serde_json::json!({"file":name})),
                ..Default::default()
            });
        }
    }
}

pub(crate) fn missing_include_key(message: &str) -> Option<&str> {
    message
        .strip_prefix("Included file `")?
        .strip_suffix("` not found")
}

fn file_commands_for_text(
    text: &Text,
    parsed: &crate::syntax::Syntax,
    cached: Option<&InvocationCalls<'_>>,
    project_stale: bool,
) -> Vec<crate::syntax::Command> {
    if project_stale {
        return vec![];
    }
    if let Some(cached) = cached {
        return if cached.source == text.source {
            cached.file_commands.clone()
        } else {
            vec![]
        };
    }
    file_error_commands(
        &text.source,
        parsed,
        &crate::signature::CommandSignatures::new([(text.source.as_str(), parsed)]),
    )
}

fn missing_include_ranges(
    text: &Text,
    name: &str,
    cached: Option<&InvocationCalls<'_>>,
    project_stale: bool,
) -> Vec<Range> {
    let parsed = crate::syntax::Syntax::latex(text);
    let indirect = file_commands_for_text(text, &parsed, cached, project_stale);
    parsed
        .commands
        .iter()
        .chain(&indirect)
        .filter(|command| command.name == "include")
        .filter_map(|command| {
            let argument = crate::syntax::file_argument(&text.source, command)?;
            let span = crate::syntax::trimmed_span(&text.source, argument.span.clone());
            let matched = crate::syntax::file_lookup_names("include", argument.value.trim())
                .any(|candidate| candidate == name);
            matched.then(|| text.range(span.start, span.end))
        })
        .collect()
}

pub(crate) fn undefined_reference_ranges(text: &Text, line: u32, message: &str) -> Vec<Range> {
    let Some((citation, key)) = undefined_reference_key(message) else {
        return vec![];
    };
    let syntax = crate::syntax::Syntax::latex(text);
    let hyper_link = message.contains("Hyper reference `") || message.contains("Hyper reference '");
    let mut ranges = Vec::new();
    for command in &syntax.commands {
        if !(if citation {
            crate::syntax::warns_on_undefined_citation(&command.name)
        } else if hyper_link {
            command.name == "hyperref"
                || matches!(
                    command.name.as_str(),
                    "Ref" | "autopageref" | "vref" | "Vref" | "subref"
                ) && text
                    .source
                    .as_bytes()
                    .get(crate::syntax::skip_space(&text.source, command.span.end))
                    != Some(&b'*')
        } else {
            command.name != "hyperref"
                && (crate::syntax::warns_on_undefined_reference(&command.name)
                    || crate::syntax::is_counter_reference(&command.name))
        }) {
            continue;
        }
        if let Some(style) = citation
            .then(|| crate::citations::style(&command.name))
            .flatten()
        {
            let arguments = crate::citations::arguments(&text.source, command.span.end, style);
            if text.position(command.span.start).line <= line
                && text.position(arguments.end.saturating_sub(1)).line >= line
            {
                for argument in arguments.keys() {
                    for span in
                        crate::syntax::reference_list_spans(&text.source, argument.span.clone())
                    {
                        if text.source[span.clone()] == *key {
                            ranges.push(text.range(span.start, span.end));
                        }
                    }
                }
            }
            continue;
        }
        let separate_label = crate::syntax::separate_label_argument(&text.source, command);
        let argument = if crate::syntax::has_separate_label_argument(&command.name) {
            separate_label.as_ref()
        } else {
            command.argument.as_ref()
        };
        let Some(argument) = argument else {
            continue;
        };
        let range_end = crate::syntax::range_reference_end(&text.source, command);
        // hyperref reads the display text before warning, so a multiline link
        // reports the closing text line rather than the bracketed key's line.
        // Range references read both endpoints before reporting missing labels.
        let report_end = if command.name == "hyperref" {
            crate::syntax::group(
                &text.source,
                crate::syntax::skip_space(&text.source, argument.outer_span.end),
                b'{',
                b'}',
            )
            .map_or(argument.outer_span.end.saturating_sub(1), |(text, _)| {
                text.span.end
            })
        } else {
            range_end
                .as_ref()
                .map_or(argument.outer_span.end.saturating_sub(1), |end| {
                    end.outer_span.end.saturating_sub(1)
                })
        };
        if text.position(command.span.start).line > line || text.position(report_end).line < line {
            continue;
        }
        if citation && crate::syntax::is_single_citation(&command.name)
            || !citation
                && (crate::syntax::is_single_reference(&command.name)
                    || crate::syntax::has_separate_label_argument(&command.name))
        {
            let source_key = if command.name == "subref" && !hyper_link {
                key.strip_prefix("sub@")
            } else {
                Some(key)
            };
            if let Some(key) = source_key.filter(|key| crate::syntax::is_static_reference_key(key))
            {
                for argument in std::iter::once(argument).chain(range_end.as_ref()) {
                    if argument.value == key {
                        ranges.push(text.range(argument.span.start, argument.span.end));
                    }
                }
            }
            continue;
        }
        if !crate::syntax::is_static_reference_key(key) {
            continue;
        }
        for segment in crate::syntax::key_list_segments(&text.source, argument.span.clone()) {
            let bounded = &text.source[..segment.end];
            let start = crate::syntax::skip_space(bounded, segment.start);
            if bounded[start..].starts_with(key) {
                let end = start + key.len();
                if crate::syntax::skip_space(bounded, end) == segment.end {
                    ranges.push(text.range(start, end));
                }
            }
        }
    }
    ranges
}

fn emit(
    result: &mut BTreeMap<Url, Vec<Diagnostic>>,
    path: &Path,
    line: u32,
    message: String,
    severity: DiagnosticSeverity,
) {
    emit_with_context(result, path, line, message, severity, None, true);
}

pub(crate) fn unresolved_location(diagnostic: &Diagnostic) -> bool {
    diagnostic.data.as_ref().is_some_and(|data| {
        ["unresolvedSource", "unresolvedLine"]
            .iter()
            .any(|key| data.get(key).and_then(serde_json::Value::as_bool) == Some(true))
    })
}

struct PackageOptions {
    loaded: String,
    requested: String,
}

struct ErrorContext<'a> {
    source: &'a str,
    package_options: Option<PackageOptions>,
    name_frame: Option<&'a str>,
    macro_context: Option<InvocationContext<'a>>,
    project_stale: bool,
}

impl<'a> ErrorContext<'a> {
    fn source(source: &'a str) -> Self {
        Self {
            source,
            package_options: None,
            name_frame: None,
            macro_context: None,
            project_stale: false,
        }
    }

    fn with_project(
        self,
        message: &str,
        project: Option<&mut ProjectAnalysis<'a>>,
        loaded: &BTreeSet<Url>,
        stack: &[Option<LogSource>],
        path: &Path,
    ) -> Self {
        if let Some(kind) = InvocationKind::for_message(message) {
            self.with_project_kind(kind, project, loaded, stack, path)
        } else {
            self
        }
    }

    fn with_project_kind(
        mut self,
        kind: InvocationKind,
        project: Option<&mut ProjectAnalysis<'a>>,
        loaded: &BTreeSet<Url>,
        stack: &[Option<LogSource>],
        path: &Path,
    ) -> Self {
        if let Some(analysis) = project {
            let project = analysis.project;
            // Open ancestors are paused at their input command. Their
            // remaining declarations have not executed in this child.
            let anchor = path.canonicalize().unwrap_or_else(|_| path.into());
            let Ok(uri) = Url::from_file_path(&anchor) else {
                self.project_stale = true;
                return self;
            };
            // Dynamically loaded sources may be confirmed by the log but
            // absent from the pre-build graph. A partial signature set
            // must not hide declarations in the file reporting the error.
            if !project.dynamic_sources.is_disjoint(loaded) || !project.documents.contains_key(&uri)
            {
                self.project_stale = true;
                return self;
            }
            let mut available = loaded.clone();
            let mut paused = BTreeSet::new();
            for source in stack
                .iter()
                .flatten()
                .filter(|source| source.path != anchor)
            {
                if let Ok(uri) = Url::from_file_path(&source.path) {
                    available.remove(&uri);
                    paused.insert(uri);
                }
            }
            match analysis.context(&available, &paused, &uri, kind) {
                Ok(context) => self.macro_context = context,
                Err(()) => self.project_stale = true,
            }
        }
        self
    }
}

/// Help is printed after the two-line source echo, with byte wraps even inside
/// names and values. Keep an independent, bounded cursor so looking for help
/// cannot consume a later diagnostic or change the main log parser's position.
#[derive(Clone, Copy)]
struct OptionHelp<'a> {
    lines: &'a [&'a str],
    index: usize,
    bytes: usize,
}

impl<'a> OptionHelp<'a> {
    fn read_line(&mut self) -> Option<&'a str> {
        let line = *self.lines.get(self.index)?;
        if self.index >= 64 || self.bytes + line.len() > 8192 || line == crate::output::TRUNCATED {
            return None;
        }
        self.index += 1;
        self.bytes += line.len();
        Some(line)
    }

    fn expect(&mut self, mut value: &str) -> Option<()> {
        while !value.is_empty() {
            let line = self.read_line()?;
            if line.is_empty() {
                return None;
            }
            value = value.strip_prefix(line)?;
        }
        Some(())
    }

    fn options(&mut self, following: &str) -> Option<String> {
        let mut value = self.read_line()?.strip_prefix("  [")?.to_owned();
        loop {
            if value.len() > 4096 {
                return None;
            }
            if let Some(content) = value.strip_suffix(']') {
                let mut after = *self;
                if after.expect(following).is_some() {
                    *self = after;
                    return Some(content.into());
                }
            }
            let line = self.read_line()?;
            if line.trim().is_empty()
                || message_boundary(line)
                || line.trim_start().starts_with('(')
                || ["The package ", "There has now ", "Adding the global "]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            {
                return None;
            }
            value.push_str(line);
        }
    }
}

fn package_option_details(lines: &[&str], start: usize, message: &str) -> Option<PackageOptions> {
    let name = package_option_clash_name(message)?;
    let mut lines = lines.get(start..)?;
    // TeX's second source-echo line is indented. Only this immediate line may
    // precede the help header; arbitrary intervening output is not evidence.
    if lines
        .first()
        .is_some_and(|line| line.is_empty() || line.starts_with(' '))
    {
        lines = &lines[1..];
    }
    let mut help = OptionHelp {
        lines,
        index: 0,
        bytes: 0,
    };
    help.expect(&format!(
        "The package {name} has already been loaded with options:"
    ))?;
    let loaded = help.options("There has now been an attempt to load it with options")?;
    let requested = help.options("Adding the global options:")?;
    Some(PackageOptions { loaded, requested })
}

/// A direct TeX error prints the consumed source prefix on its l.N line.
/// Expansion frames precede that line. Allow a single recently read token for
/// undefined commands only when it also ends the consumed source prefix. Math
/// mode adds this frame even for direct calls. Other expansion frames still
/// require a whole-line diagnostic rather than blaming a valid macro call.
fn error_context<'a>(
    lines: &[&'a str],
    start: usize,
    number: u32,
    message: &str,
) -> Option<ErrorContext<'a>> {
    let mut context = lines
        .iter()
        .enumerate()
        .skip(start)
        .take(16)
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, &line)| (index, line));
    let (mut index, mut line) = context.next()?;
    if message == crate::special_characters::EXTRA_ALIGNMENT_TAB {
        // LaTeX alignments report the internal end-template token; raw TeX
        // alignments can report only the template. Expanded user frames must
        // still prevent attribution to a literal source tab.
        let mut internal_frame = false;
        if line.trim() == r"<recently read> \endtemplate" {
            internal_frame = true;
            (index, line) = context.next()?;
        }
        if line.starts_with("<template> ") {
            if !line.trim_end().ends_with(r"\endtemplate") {
                return None;
            }
            internal_frame = true;
            (index, line) = context.next()?;
        }
        if !internal_frame {
            return None;
        }
    }
    // A direct script token outside math mode always reports this exact
    // recovery frame. Additional expansion frames must still block attribution.
    if message == "Missing $ inserted." {
        if line.trim() != "<inserted text>" || context.next()?.1.trim() != "$" {
            return None;
        }
        (index, line) = context.next()?;
    }
    let recent = (message == "Undefined control sequence.")
        .then(|| line.trim_start().strip_prefix("<recently read>"))
        .flatten()
        .map(str::trim);
    if let Some(token) = recent {
        if crate::syntax::control_word(token, 0)? != (0..token.len()) {
            return None;
        }
        (index, line) = context.next()?;
    }
    let name_frame = (message == "Undefined control sequence." && recent.is_none())
        .then(|| line.trim_start().strip_prefix("<argument> "))
        .flatten()
        .map(str::trim_end);
    if let Some(frame) = name_frame {
        let token = frame
            .strip_prefix("FV@SVG@")
            .or_else(|| frame.strip_prefix("FV@SV@"))
            .or_else(|| frame.strip_prefix("r@"))
            .or_else(|| frame.strip_prefix("sub@"))
            .unwrap_or(frame);
        if crate::syntax::control_word(token, 0)? != (0..token.len()) {
            return None;
        }
        (index, line) = context.next()?;
    }
    let graphics = missing_graphic_name(message).is_some() && missing_file_name(message).is_none();
    let documentation = line.trim() == "See the LaTeX manual or LaTeX Companion for explanation."
        || undefined_length_name(message).is_some()
            && line.trim() == "See the etoolbox package documentation for explanation."
        || (undefined_color_name(message).is_some()
            || undefined_color_model_name(message).is_some())
            && line.trim() == "See the xcolor package documentation for explanation."
        || graphics
            && [
                "See the pdftex.def package documentation for explanation.",
                "See the luatex.def package documentation for explanation.",
            ]
            .contains(&line.trim());
    if recent.is_none() && documentation {
        if context.next()?.1.trim() != "Type  H <return>  for immediate help."
            || context.next()?.1.trim() != "..."
        {
            return None;
        }
        (index, line) = context.next()?;
    }
    // XeTeX reports this one internal read frame for a direct graphicx call.
    // Other expansion frames still prevent precise source attribution.
    if graphics
        && message.starts_with("Unable to load picture or PDF file ")
        && line.trim() == "<to be read again>"
    {
        if context.next()?.1.trim() != "}" {
            return None;
        }
        (index, line) = context.next()?;
    }
    // FancyVerb always expands this internal error macro, even for a direct
    // call. Accept only the frames printed by pdfTeX and XeTeX; an additional
    // user expansion frame must still prevent attribution to a literal path.
    if missing_fancyverb_name(message).is_some() && line.starts_with("\\FV@Error ") {
        match line.trim_end() {
            "\\FV@Error ... {FancyVerb Error:" => {
                if context.next()?.1 != "\\space \\space #1" || context.next()?.1 != "}" {
                    return None;
                }
            }
            "\\FV@Error ...ncyVerb Error:^^J\\space \\space #1^^J}" => {}
            _ => return None,
        }
        (index, line) = context.next()?;
    }
    let tail = line.trim_start().strip_prefix("l.")?;
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    if tail[..digits].parse::<u32>().ok()? != number {
        return None;
    }
    let prefix = tail[digits..].strip_prefix(' ')?;
    recent
        .is_none_or(|token| prefix.trim_end().ends_with(token))
        .then(|| ErrorContext {
            source: prefix,
            package_options: package_option_details(lines, index + 1, message),
            name_frame,
            macro_context: None,
            project_stale: false,
        })
}

pub(crate) fn missing_file_name(message: &str) -> Option<&str> {
    let quoted = message
        .strip_prefix("LaTeX Error: File ")
        .and_then(|tail| tail.strip_suffix(" not found."))
        .or_else(|| {
            message
                .strip_prefix("I can't find file ")?
                .strip_suffix('.')
        })?;
    let name = quoted
        .strip_prefix('`')
        .or_else(|| quoted.strip_prefix('\''))?
        .strip_suffix('\'')?;
    let name = name
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(name);
    (!name.is_empty()).then_some(name)
}

pub(crate) fn missing_listing_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package Listings Error: File `")?
        .strip_suffix("' not found.")?;
    (!name.is_empty() && name.len() <= 8192).then_some(name)
}

pub(crate) fn missing_fancyverb_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("FancyVerb Error:")?
        .trim_start()
        .strip_prefix("No verbatim file ")?
        .strip_suffix('.')?;
    let name = name
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(name);
    crate::syntax::is_literal_file_path(name).then_some(name)
}

fn missing_read_file(message: &str) -> bool {
    missing_file_name(message).is_some() || missing_listing_name(message).is_some()
}

pub(crate) fn missing_graphic_name(message: &str) -> Option<&str> {
    if let Some(name) = missing_file_name(message) {
        return Some(name);
    }
    let quoted = ["pdftex.def", "luatex.def"].into_iter().find_map(|driver| {
        message
            .strip_prefix(&format!("Package {driver} Error: File "))?
            .strip_suffix(" not found: using draft setting.")
    });
    let name = if let Some(quoted) = quoted {
        quoted.strip_prefix('`')?.strip_suffix('\'')?
    } else {
        message
            .strip_prefix("Unable to load picture or PDF file '")?
            .strip_suffix("'.")?
    };
    let name = name
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(name);
    (!name.is_empty()).then_some(name)
}

fn following_error_location<'a>(lines: &[&'a str], start: usize) -> Option<(u32, Option<&'a str>)> {
    let mut direct = true;
    for line in lines.iter().skip(start).take(16) {
        let line = line.trim_start();
        if let Some(tail) = line.strip_prefix("l.") {
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            return Some((
                tail[..digits].parse().ok()?,
                direct.then(|| tail[digits..].strip_prefix(' ')).flatten(),
            ));
        }
        if message_boundary(line) || line.starts_with('(') {
            break;
        }
        if !line.trim().is_empty() && line.trim() != "<read *>" {
            direct = false;
        }
    }
    None
}

fn uninterrupted_file_prompt(lines: &[&str], start: usize, end: usize) -> bool {
    end.saturating_sub(start) <= 32
        && lines.get(start..end).is_some_and(|lines| {
            lines
                .iter()
                .all(|line| !message_boundary(line) && !line.trim_start().starts_with('('))
        })
}

fn missing_path_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
    command_name: &str,
    indirect: &[crate::syntax::Command],
) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    if shortened.is_some_and(|suffix| !unique_context_on_line(text, start, suffix)) {
        return None;
    }
    let syntax = crate::syntax::Syntax::latex(text);
    let fancyverb = crate::fancyvrb::is_input(command_name);
    if crate::syntax::code_input_command(command_name)
        && !crate::code_inputs::safe_syntax(&text.source, &syntax, command_name)
    {
        return None;
    }
    let mut matches = syntax
        .commands
        .iter()
        .chain(indirect)
        .filter_map(|command| {
            if !(if fancyverb {
                crate::fancyvrb::is_input(&command.name)
            } else {
                command.name == command_name
            }) {
                return None;
            }
            let argument = crate::syntax::file_argument(&text.source, command)?;
            if crate::syntax::code_input_command(command_name)
                && !crate::code_inputs::safe_call(&text.source, command)
            {
                return None;
            }
            let span = crate::syntax::trimmed_span(&text.source, argument.span.clone());
            let value = if command_name == "includegraphics" {
                argument.value.as_str()
            } else {
                &text.source[span.clone()]
            };
            let matches_report = if command_name == "includegraphics" {
                crate::graphics::unquote(name).is_some_and(|reported| value == reported)
            } else if command_name == "lstinputlisting" {
                crate::listings::missing_report(value) == name
            } else if fancyverb {
                value == name
            } else {
                crate::syntax::file_lookup_names(command_name, value)
                    .any(|candidate| candidate == name)
            };
            if !crate::syntax::is_literal_file_path(value)
                || !matches_report
                || text.position(argument.outer_span.end).line != line
                || text.source[argument.outer_span.clone()].starts_with('"')
                    && !text.source[argument.outer_span.clone()].ends_with('"')
            {
                return None;
            }
            let consumed = &text.source[start..argument.outer_span.end];
            let matched = shortened.map_or(consumed == prefix, |suffix| {
                (fancyverb && !suffix.is_empty()
                    || suffix.ends_with(&text.source[argument.outer_span.clone()]))
                    && consumed.ends_with(suffix)
            });
            matched.then(|| text.range(span.start, span.end))
        });
    let range = matches.next()?;
    matches.next().is_none().then_some(range)
}

fn missing_input_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
    cached: Option<&InvocationCalls<'_>>,
) -> Option<Range> {
    let local;
    let commands = if let Some(cached) = cached {
        if cached.source != text.source {
            return None;
        }
        &cached.file_commands
    } else {
        let parsed = crate::syntax::Syntax::latex(text);
        local = file_error_commands(
            &text.source,
            &parsed,
            &crate::signature::CommandSignatures::new([(text.source.as_str(), &parsed)]),
        );
        &local
    };
    missing_path_range(text, line, context, name, "input", commands)
}

fn package_option_clash_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("LaTeX Error: Option clash for package ")?
        .strip_suffix('.')?;
    (crate::syntax::is_literal_file_path(name) && !name.contains([' ', '\t', '\r', '\n']))
        .then_some(name)
}

fn loader_failure_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
    indirect: &[crate::syntax::Command],
) -> Option<(Range, &'static str)> {
    if !name.ends_with(".sty") && !name.ends_with(".cls") {
        return None;
    }
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = Vec::new();
    for command in syntax.commands.iter().chain(indirect) {
        let command_name = match command.name.as_str() {
            "usepackage" => "usepackage",
            "RequirePackage" => "RequirePackage",
            "RequirePackageWithOptions" => "RequirePackageWithOptions",
            "documentclass" => "documentclass",
            "LoadClass" => "LoadClass",
            "LoadClassWithOptions" => "LoadClassWithOptions",
            _ => continue,
        };
        let Some(argument) = crate::syntax::file_argument(&text.source, command) else {
            continue;
        };
        let next = crate::syntax::skip_space(&text.source, argument.outer_span.end);
        // LaTeX reads the optional minimum version, or one lookahead token,
        // before the loader reports a failure. That token may be on
        // the next source line, while the path being diagnosed is above it.
        let tail = if text.source.as_bytes().get(next) == Some(&b'[') {
            crate::syntax::optional_note_end(&text.source, next)
        } else if text.source.as_bytes().get(next) == Some(&b'\\') {
            crate::syntax::control_word(&text.source, next)
                .map(|word| word.end)
                .or_else(|| {
                    text.source
                        .get(next + 1..)?
                        .chars()
                        .next()
                        .map(|token| next + 1 + token.len_utf8())
                })
        } else {
            text.source
                .get(next..)?
                .chars()
                .next()
                .map(|token| next + token.len_utf8())
        };
        if text.source.as_bytes().get(next) == Some(&b'[') && tail.is_none() {
            continue;
        }
        let confirmed = [Some(argument.outer_span.end), tail]
            .into_iter()
            .flatten()
            .any(|end| {
                if text.position(end).line != line {
                    return false;
                }
                let consumed = &text.source[start..end];
                let token = if end == argument.outer_span.end {
                    &text.source[argument.outer_span.clone()]
                } else {
                    &text.source[next..end]
                };
                shortened.map_or(consumed == prefix, |suffix| {
                    suffix.ends_with(token)
                        && consumed.ends_with(suffix)
                        && (text.position(argument.outer_span.start).line != line
                            || suffix.contains(&text.source[argument.outer_span.clone()]))
                        && text.source[start..]
                            .split('\n')
                            .next()
                            .unwrap_or_default()
                            .match_indices(suffix)
                            .take(2)
                            .count()
                            == 1
                })
            });
        if !confirmed {
            continue;
        }
        // A dynamic sibling can generate the reported filename. Require the
        // complete static list before attributing an error to one item.
        let spans = if crate::syntax::file_list(command_name) {
            let Some(spans) = crate::syntax::complete_file_list_spans(&text.source, command) else {
                continue;
            };
            spans
        } else {
            let span = crate::syntax::trimmed_span(&text.source, argument.span.clone());
            if !crate::syntax::is_literal_file_path(&text.source[span.clone()]) {
                continue;
            }
            vec![span]
        };
        for span in spans {
            if crate::syntax::file_lookup_names(command_name, &text.source[span.clone()])
                .any(|candidate| candidate == name)
            {
                matches.push((text.range(span.start, span.end), command_name));
            }
        }
    }
    (matches.len() == 1).then(|| matches[0])
}

pub(crate) fn undefined_environment_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("LaTeX Error: Environment ")?
        .strip_suffix(" undefined.")?;
    crate::syntax::is_static_key(name).then_some(name)
}

pub(crate) fn undefined_counter_name(message: &str) -> Option<&str> {
    let tail = message.strip_prefix("LaTeX Error: No counter ")?;
    let name = tail
        .strip_prefix('\'')
        .or_else(|| tail.strip_prefix('`'))?
        .strip_suffix("' defined.")?;
    crate::syntax::is_static_key(name).then_some(name)
}

pub(crate) fn undefined_length_name(message: &str) -> Option<&str> {
    let control = message
        .strip_prefix("Package etoolbox Error: Length '")?
        .strip_suffix("' undefined.")?;
    let span = crate::syntax::control_word(control, 0)?;
    (span.end == control.len()).then(|| &control[1..])
}

pub(crate) fn undefined_color_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package xcolor Error: Undefined color `")?
        .strip_suffix("'.")?;
    crate::named_colors::literal_name(name).then_some(name)
}

pub(crate) fn undefined_color_model_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package xcolor Error: Undefined color model `")?
        .strip_suffix("'.")?;
    crate::named_colors::literal_name(name).then_some(name)
}

fn color_error_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
    model: bool,
    cached: Option<&InvocationCalls<'_>>,
) -> Option<Range> {
    invocation_error_range(
        text,
        line,
        context,
        name,
        if model {
            InvocationKind::ColorModel
        } else {
            InvocationKind::Color
        },
        cached,
    )
}

fn invocation_error_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
    kind: InvocationKind,
    cached: Option<&InvocationCalls<'_>>,
) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    if shortened.is_some_and(|suffix| !unique_context_on_line(text, start, suffix)) {
        return None;
    }
    let local;
    let invocations = if let Some(cached) = cached {
        if cached.source != text.source {
            return None;
        }
        &cached.invocations
    } else {
        let syntax = crate::syntax::Syntax::latex(text);
        local = kind.invocations(
            &text.source,
            &syntax,
            &crate::signature::CommandSignatures::new([(text.source.as_str(), &syntax)]),
        );
        &local
    };
    let mut matches = invocations.iter().filter_map(|(span, tail)| {
        // Color calls require a single-line echo. Counters may have a value
        // on a later line; environment names may follow a command newline.
        // All candidates have already passed the macro-operand check.
        if text.source[span.clone()] != *name
            || text.position(tail.end).line != line
            || matches!(kind, InvocationKind::Color | InvocationKind::ColorModel)
                && tail.start < start
        {
            return None;
        }
        let consumed = &text.source[start..tail.end];
        let matched = shortened.map_or(consumed == prefix, |suffix| {
            suffix.ends_with(&text.source[tail.clone()]) && consumed.ends_with(suffix)
        });
        matched.then(|| text.range(span.start, span.end))
    });
    let range = matches.next()?;
    matches.next().is_none().then_some(range)
}

struct EnvironmentError<'a> {
    command: &'static str,
    name: &'a str,
    code: &'static str,
    expected: Option<&'a str>,
}

fn environment_error(message: &str) -> Option<EnvironmentError<'_>> {
    if let Some(name) = undefined_environment_name(message) {
        return Some(EnvironmentError {
            command: "begin",
            name,
            code: "undefined-environment",
            expected: None,
        });
    }
    let (name, expected) = mismatched_environment_names(message)?;
    Some(EnvironmentError {
        command: "end",
        name,
        code: "mismatched-environment-end",
        expected: Some(expected),
    })
}

pub(crate) fn mismatched_environment_names(message: &str) -> Option<(&str, &str)> {
    let tail = message.strip_prefix("LaTeX Error: \\begin{")?;
    let (expected, tail) = tail.split_once('}')?;
    let tail = if let Some(tail) = tail.strip_prefix(" on input line ") {
        let length = tail.bytes().take_while(u8::is_ascii_digit).count();
        if length == 0 {
            return None;
        }
        &tail[length..]
    } else {
        tail
    };
    let name = tail.strip_prefix(" ended by \\end{")?.strip_suffix("}.")?;
    (crate::syntax::is_static_key(name) && crate::syntax::is_static_key(expected))
        .then_some((name, expected))
}

pub(crate) fn environment_error_invocations_with_signatures(
    source: &str,
    parsed: &crate::syntax::Syntax,
    signatures: &crate::signature::CommandSignatures<'_>,
    command_name: &str,
) -> Vec<(ByteRange<usize>, ByteRange<usize>)> {
    if signatures
        .names()
        .any(|name| matches!(name, "begin" | "end"))
    {
        return vec![];
    }
    let operands = signatures.argument_tokens(source, parsed);
    parsed
        .commands
        .iter()
        .filter_map(|command| {
            if command.name != command_name || operands.contains(&command.span.start) {
                return None;
            }
            let argument = command.argument.as_ref()?;
            Some((argument.span.clone(), argument.outer_span.clone()))
        })
        .collect()
}

/// Certain indirect file loads in the compiler-confirmed macro scope.
pub(crate) fn file_error_commands(
    source: &str,
    parsed: &crate::syntax::Syntax,
    signatures: &crate::signature::CommandSignatures<'_>,
) -> Vec<crate::syntax::Command> {
    let positions = signatures.source_command_positions(source, parsed);
    signatures
        .indirect_source_invocations(source, parsed, &positions)
        .into_iter()
        .chain(signatures.indirect_loader_invocations(source, parsed, &positions))
        .chain(signatures.indirect_graphics_invocations(source, parsed, &positions))
        .filter(|call| call.certain)
        .map(|call| call.command)
        .collect()
}

pub(crate) fn file_error_invocations_with_signatures(
    source: &str,
    parsed: &crate::syntax::Syntax,
    signatures: &crate::signature::CommandSignatures<'_>,
) -> Vec<(ByteRange<usize>, ByteRange<usize>)> {
    let loaders_redefined = signatures
        .names()
        .any(|name| name == "input" || crate::syntax::loader_extension(name).is_some());
    let operands = signatures.argument_tokens(source, parsed);
    let positions = signatures.source_command_positions(source, parsed);
    let indirect = file_error_commands(source, parsed, signatures);
    parsed
        .commands
        .iter()
        .chain(&indirect)
        .filter_map(|command| {
            if (command.name != "input"
                && command.name != "includegraphics"
                && crate::syntax::loader_extension(&command.name).is_none())
                || if command.name == "includegraphics" {
                    !signatures.command_unchanged("includegraphics")
                        || !positions.contains(&command.span.start)
                } else {
                    loaders_redefined
                }
                || operands.contains(&command.span.start)
            {
                return None;
            }
            let argument = crate::syntax::file_argument(source, command)?;
            let spans = if crate::syntax::file_list(&command.name) {
                crate::syntax::complete_file_list_spans(source, command)?
            } else {
                vec![crate::syntax::trimmed_span(source, argument.span.clone())]
            };
            Some(
                spans
                    .into_iter()
                    .map(|span| (span, argument.outer_span.clone()))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

fn confirmed_file_invocation(
    text: &Text,
    range: Range,
    cached: Option<&InvocationCalls<'_>>,
) -> bool {
    let local;
    let calls = if let Some(cached) = cached {
        if cached.source != text.source {
            return false;
        }
        &cached.invocations
    } else {
        let parsed = crate::syntax::Syntax::latex(text);
        local = file_error_invocations_with_signatures(
            &text.source,
            &parsed,
            &crate::signature::CommandSignatures::new([(text.source.as_str(), &parsed)]),
        );
        &local
    };
    calls
        .iter()
        .any(|(span, _)| text.range(span.start, span.end) == range)
}

fn shortened_context(prefix: &str) -> Option<&str> {
    // TeX may cut the leading scalar in half when shortening error context.
    // Discard only its undecodable leading bytes; matching still requires a
    // complete command/argument and exactly one source occurrence.
    prefix
        .strip_prefix("...")
        .map(|suffix| suffix.trim_start_matches('\u{fffd}'))
}

fn unique_context_on_line(text: &Text, start: usize, context: &str) -> bool {
    // Unknown callers and opaque text can duplicate a supported invocation's
    // suffix. Check the whole source line before filtering candidate calls.
    let Some(line) = text
        .source
        .get(start..)
        .and_then(|tail| tail.lines().next())
    else {
        return false;
    };
    line.find(context)
        .is_some_and(|first| line.rfind(context) == Some(first))
}

fn undefined_command_range(text: &Text, line: u32, context: &str) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = syntax.commands.iter().filter(|command| {
        if text.position(command.span.start).line != line {
            return false;
        }
        let consumed = &text.source[start..command.span.end];
        shortened.map_or(consumed == prefix, |suffix| {
            suffix.ends_with(&text.source[command.span.clone()]) && consumed.ends_with(suffix)
        })
    });
    let command = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(text.range(command.span.start, command.span.end))
}

fn reference_macro_error_invocation(
    source: &str,
    command: &crate::syntax::Command,
    frame: &str,
) -> Option<(ByteRange<usize>, ByteRange<usize>)> {
    let frame = match command.name.as_str() {
        "vpageref"
        | "getrefnumber"
        | "getpagerefnumber"
        | "refused"
        | "IfRefUndefinedExpandable"
        | "setcounterref"
        | "addtocounterref"
        | "setcounterpageref"
        | "addtocounterpageref" => frame.strip_prefix("r@")?,
        "subref" => frame.strip_prefix("sub@")?,
        _ => frame,
    };
    let (start, open) = match command.name.as_str() {
        "ref"
        | "pageref"
        | "eqref"
        | "Ref"
        | "autoref"
        | "autopageref"
        | "nameref"
        | "vref"
        | "Vref"
        | "vpageref"
        | "getrefnumber"
        | "getpagerefnumber"
        | "refused"
        | "subref"
        | "getrefbykeydefault"
        | "IfRefUndefinedExpandable"
        | "IfRefUndefinedBabel" => (
            crate::syntax::reference_argument_start(source, &command.name, command.span.end)?,
            b'{',
        ),
        "cite" | "nocite" | "citet" | "citep" | "citeauthor" | "citeyear" | "citenum"
        | "bibentry" | "citeyearpar" | "citealt" | "citealp" | "citefullauthor" | "Citet"
        | "Citep" | "Citealt" | "Citealp" | "Citeauthor" | "citetalias" | "citepalias" => (
            crate::syntax::citation_argument_start(source, &command.name, command.span.end)?,
            b'{',
        ),
        "hyperref" => {
            crate::syntax::separate_label_argument_start(source, command)?;
            (crate::syntax::skip_space(source, command.span.end), b'[')
        }
        "setcounterref" | "addtocounterref" | "setcounterpageref" | "addtocounterpageref" => {
            let (counter, end) = crate::syntax::group(
                source,
                crate::syntax::skip_space(source, command.span.end),
                b'{',
                b'}',
            )?;
            if !crate::syntax::is_static_key(&counter.value) {
                return None;
            }
            (crate::syntax::skip_space(source, end), b'{')
        }
        _ => return None,
    };
    let close = if open == b'[' { b']' } else { b'}' };
    let (mut argument, _) = crate::syntax::group(source, start, open, close)?;
    let mut tail = argument.outer_span.clone();
    if command.name == "getrefbykeydefault" {
        // refcount consumes the selector before expanding the key, but has
        // not read the default yet. Dynamic selectors cannot identify a peer.
        let (selector, end) = crate::syntax::group(
            source,
            crate::syntax::skip_space(source, tail.end),
            b'{',
            b'}',
        )?;
        if !selector.value.is_empty() && !crate::syntax::is_static_key(&selector.value) {
            return None;
        }
        tail.end = end;
    }
    if command.name == "hyperref" && argument.value.starts_with('{') {
        let (inner, end) = crate::syntax::group(source, argument.span.start, b'{', b'}')?;
        if end != argument.span.end {
            return None;
        }
        argument = inner;
    }
    if argument.value != frame
        || crate::syntax::control_word(&argument.value, 0)? != (0..argument.value.len())
    {
        return None;
    }
    Some((argument.span, tail))
}

fn undefined_name_command_range(
    text: &Text,
    line: u32,
    context: &str,
    frame: &str,
) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    if shortened.is_some_and(|suffix| !unique_context_on_line(text, start, suffix)) {
        return None;
    }
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = syntax.commands.iter().filter_map(|command| {
        let (span, tail) = crate::saved_text::name_error_invocation(&text.source, command, frame)
            .or_else(|| crate::named_commands::name_error_invocation(&text.source, command, frame))
            .or_else(|| reference_macro_error_invocation(&text.source, command, frame))?;
        if text.position(tail.end).line != line
            || !(syntax
                .name_command_references
                .iter()
                .any(|entry| entry.span == span)
                || syntax.commands.iter().any(|command| command.span == span))
        {
            return None;
        }
        let consumed = &text.source[start..tail.end];
        let matched = shortened.map_or(consumed == prefix, |suffix| {
            suffix.ends_with(&text.source[tail]) && consumed.ends_with(suffix)
        });
        matched.then(|| text.range(span.start, span.end))
    });
    let range = matches.next()?;
    matches.next().is_none().then_some(range)
}

fn emit_with_context(
    result: &mut BTreeMap<Url, Vec<Diagnostic>>,
    path: &Path,
    line: u32,
    mut message: String,
    severity: DiagnosticSeverity,
    error: Option<ErrorContext<'_>>,
    source_confirmed: bool,
) {
    let context = error
        .as_ref()
        .filter(|context| context.name_frame.is_none())
        .map(|context| context.source);
    let package_options = error
        .as_ref()
        .and_then(|context| context.package_options.as_ref());
    let Ok(uri) = Url::from_file_path(path) else {
        return;
    };
    // Font/glyph diagnostics often repeat at shipout. Avoid reading and mapping
    // the same source file once for every repetition of an identical warning.
    if line != 0
        && context.is_none()
        && result.get(&uri).is_some_and(|diagnostics| {
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message == message
                    && diagnostic.severity == Some(severity)
                    && diagnostic.range.start.line == line.saturating_sub(1)
                    && unresolved_location(diagnostic) != source_confirmed
            })
        })
    {
        return;
    }
    let source = std::fs::read_to_string(path).unwrap_or_default();
    let text = Text::new(source);
    let last_line = text.text_line_count().saturating_sub(1) as u32;
    let reported_line = line;
    let invalid_line = line.checked_sub(1).is_none_or(|line| line > last_line);
    let rewritten = line.saturating_sub(1) > last_line
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| AUXILIARY_EXTENSIONS.contains(&extension));
    if rewritten {
        message.push_str(&generated_line_note(reported_line));
    }
    let line = line.saturating_sub(1).min(last_line);
    let value = text.source.lines().nth(line as usize).unwrap_or("");
    let start = value
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .map(char::len_utf16)
        .sum::<usize>() as u32;
    let end = value.encode_utf16().count() as u32;
    let line_range = Range::new(
        Position::new(line, start.min(end)),
        Position::new(line, end),
    );
    if !source_confirmed || invalid_line {
        // References and destinations can be located without an expansion
        // context. They must not borrow matching names from a fallback file
        // or a clamped line whose reported location could not be verified.
        let mut data = serde_json::json!({});
        if !source_confirmed {
            data["unresolvedSource"] = true.into();
        }
        if invalid_line {
            data["unresolvedLine"] = true.into();
            data["reportedLine"] = reported_line.into();
        }
        result.entry(uri).or_default().push(Diagnostic {
            range: line_range,
            severity: Some(severity),
            source: Some("latex-build".into()),
            message,
            data: Some(data),
            ..Default::default()
        });
        return;
    }
    let mut ranges = undefined_reference_ranges(&text, line, &message);
    let destination = destination_warning(&message);
    let destination_ranges: Vec<_> = destination
        .as_ref()
        .into_iter()
        .flat_map(|(kind, name)| {
            let syntax = crate::syntax::Syntax::latex(&text);
            let entries = match kind {
                DestinationWarningKind::Missing => syntax.anchor_references,
                DestinationWarningKind::Duplicate => syntax.anchors,
            };
            entries
                .into_iter()
                .filter(move |entry| entry.name == *name)
                .map(|entry| text.range(entry.span.start, entry.span.end))
        })
        .collect();
    let precise_destination = !destination_ranges.is_empty();
    ranges.extend(destination_ranges);
    let command_range = (message == "Undefined control sequence."
        && reported_line.checked_sub(1) == Some(line))
    .then(|| {
        let error = error.as_ref()?;
        if let Some(frame) = error.name_frame {
            undefined_name_command_range(&text, line, error.source, frame)
        } else {
            undefined_command_range(&text, line, error.source)
        }
    })
    .flatten();
    ranges.extend(command_range);
    let project_stale = error.as_ref().is_some_and(|context| context.project_stale);
    let macro_context = error
        .as_ref()
        .and_then(|context| context.macro_context.as_ref());
    let special_character = context
        .and_then(|context| crate::special_characters::error_character(&message, context))
        .filter(|_| severity == DiagnosticSeverity::ERROR);
    let special_range = special_character.and_then(|character| {
        if project_stale {
            return None;
        }
        crate::special_characters::source_range(
            &text,
            &crate::syntax::Syntax::latex(&text),
            line,
            context?,
            character,
            macro_context.map(|context| &context.macros.signatures),
        )
    });
    ranges.extend(special_range);
    let unsupported_unicode =
        unsupported_unicode_character(&message).filter(|_| severity == DiagnosticSeverity::ERROR);
    ranges.extend(unsupported_unicode.and_then(|character| {
        crate::special_characters::source_range(
            &text,
            &crate::syntax::Syntax::latex(&text),
            line,
            context?,
            character,
            None,
        )
    }));
    let character = missing_character(&message);
    let character_ranges =
        character.and_then(|character| missing_character_ranges(&text, line, context?, character));
    ranges.extend(character_ranges.into_iter().flatten());
    let cached_calls = macro_context.map(|context| context.calls.as_ref());
    let environment = environment_error(&message);
    let environment_range = environment
        .as_ref()
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|error| {
            invocation_error_range(
                &text,
                line,
                context?,
                error.name,
                if error.command == "begin" {
                    InvocationKind::EnvironmentBegin
                } else {
                    InvocationKind::EnvironmentEnd
                },
                cached_calls,
            )
        });
    ranges.extend(environment_range);
    let counter_name = undefined_counter_name(&message);
    let counter_range = counter_name
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| {
            invocation_error_range(
                &text,
                line,
                context?,
                name,
                InvocationKind::Counter,
                cached_calls,
            )
        });
    ranges.extend(counter_range);
    let length_name = undefined_length_name(&message);
    let length_range = length_name
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| {
            invocation_error_range(
                &text,
                line,
                context?,
                &format!("\\{name}"),
                InvocationKind::Length,
                cached_calls,
            )
        });
    ranges.extend(length_range);
    let color_name = undefined_color_name(&message);
    let color_range = color_name
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| color_error_range(&text, line, context?, name, false, cached_calls));
    ranges.extend(color_range);
    let color_model = undefined_color_model_name(&message);
    let model_range = color_model
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| color_error_range(&text, line, context?, name, true, cached_calls));
    ranges.extend(model_range);
    let missing_file = missing_file_name(&message);
    let input_range = missing_file
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_input_range(&text, line, context?, name, cached_calls))
        .filter(|range| confirmed_file_invocation(&text, *range, cached_calls));
    ranges.extend(input_range);
    let missing_listing = missing_listing_name(&message);
    let listing_range = missing_listing
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_path_range(&text, line, context?, name, "lstinputlisting", &[]));
    ranges.extend(listing_range);
    let fancyverb_range = missing_fancyverb_name(&message)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_path_range(&text, line, context?, name, "VerbatimInput", &[]));
    ranges.extend(fancyverb_range);
    let loader_range = missing_file
        .filter(|_| !project_stale)
        .and_then(|name| {
            let parsed = crate::syntax::Syntax::latex(&text);
            let indirect = file_commands_for_text(&text, &parsed, cached_calls, project_stale);
            loader_failure_range(&text, line, context?, name, &indirect)
        })
        .filter(|(range, _)| confirmed_file_invocation(&text, *range, cached_calls));
    ranges.extend(loader_range.map(|(range, _)| range));
    let package_clash = package_option_clash_name(&message);
    let clash_range = package_clash
        .filter(|_| !project_stale)
        .and_then(|name| {
            let parsed = crate::syntax::Syntax::latex(&text);
            let indirect = file_commands_for_text(&text, &parsed, cached_calls, project_stale);
            loader_failure_range(&text, line, context?, &format!("{name}.sty"), &indirect)
        })
        // WithOptions does not supply a new explicit options list. Do not
        // attribute a clash to a forwarding call based on its name alone.
        .filter(|(_, command)| matches!(*command, "usepackage" | "RequirePackage"))
        .filter(|(range, _)| confirmed_file_invocation(&text, *range, cached_calls));
    ranges.extend(clash_range.map(|(range, _)| range));
    let missing_graphic = missing_graphic_name(&message);
    let graphic_range = missing_graphic
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| {
            let parsed = crate::syntax::Syntax::latex(&text);
            let indirect = file_commands_for_text(&text, &parsed, cached_calls, project_stale);
            missing_path_range(&text, line, context?, name, "includegraphics", &indirect)
        })
        .filter(|range| confirmed_file_invocation(&text, *range, cached_calls));
    ranges.extend(graphic_range);
    let command_name = command_range.and_then(|range| {
        let start = text.offset(range.start)?;
        let end = text.offset(range.end)?;
        text.source[start..end].strip_prefix('\\')
    });
    if let Some(name) = missing_include_key(&message) {
        ranges.extend(missing_include_ranges(
            &text,
            name,
            cached_calls,
            project_stale,
        ));
        // Optional package inputs also print "No file". Report chapter
        // omissions only when the source identifies the corresponding include.
        if ranges.is_empty() {
            return;
        }
    }
    if let Some(range) = box_range(&text, &message) {
        ranges.push(range);
    }
    if ranges.is_empty() {
        ranges.push(line_range);
    }
    let displayed_message = match (clash_range, package_options) {
        (Some(_), Some(options)) => format!(
            "{message}\nAlready loaded options: [{}]\nRequested options: [{}]",
            options.loaded, options.requested
        ),
        _ => message.clone(),
    };
    let float_excess =
        oversized_float_excess(&message).filter(|_| severity == DiagnosticSeverity::WARNING);
    result
        .entry(uri)
        .or_default()
        .extend(ranges.into_iter().map(|range| {
            Diagnostic {
                range,
                severity: Some(severity),
                source: Some("latex-build".into()),
                code: box_code(&message)
                    .or_else(|| float_excess.map(|_| "oversized-float"))
                    .or_else(|| special_range.map(|_| crate::special_characters::diagnostic_code(&message)))
                    .or_else(|| character.map(|_| "missing-character"))
                    .or_else(|| unsupported_unicode.map(|_| "unsupported-unicode-character"))
                    .or_else(|| destination.as_ref().filter(|_| precise_destination).map(|(kind, _)| kind.code()))
                    .or_else(|| missing_include_key(&message).map(|_| "missing-include"))
                    .or_else(|| input_range.map(|_| "missing-input"))
                    .or_else(|| listing_range.map(|_| "missing-listing"))
                    .or_else(|| fancyverb_range.map(|_| "missing-fancyverb-input"))
                    .or_else(|| loader_range.map(|(_, command)| if crate::syntax::loader_extension(command) == Some("sty") { "missing-package" } else { "missing-class" }))
                    .or_else(|| clash_range.map(|_| "package-option-clash"))
                    .or_else(|| graphic_range.map(|_| "missing-image"))
                    .or_else(|| command_range.map(|_| "undefined-control-sequence"))
                    .or_else(|| environment_range.and(environment.as_ref()).map(|error| error.code))
                    .or_else(|| counter_range.map(|_| "undefined-counter"))
                    .or_else(|| length_range.map(|_| "undefined-length"))
                    .or_else(|| color_range.map(|_| "undefined-color"))
                    .or_else(|| model_range.map(|_| "undefined-color-model"))
                    .map(|code| NumberOrString::String(code.into())),
                message: displayed_message.clone(),
                data: precise_destination.then(|| serde_json::json!({"destination":destination.as_ref().map(|(_, name)| name)}))
                    .or_else(|| float_excess.map(|excess| serde_json::json!({"excessPt":excess})))
                    .or_else(|| special_range.and(special_character).map(|character| serde_json::json!({"character":character.to_string(),"sourceContext":context})))
                    .or_else(|| character.map(|character| serde_json::json!({"character":character.to_string(),"codepoint":format!("U+{:04X}", character as u32)})))
                    .or_else(|| unsupported_unicode.map(|character| serde_json::json!({"character":character.to_string(),"codepoint":format!("U+{:04X}", character as u32)})))
                    .or_else(|| command_name.map(|name| serde_json::json!({"command":name})))
                    .or_else(|| {
                        environment_range
                            .and(environment.as_ref())
                            .map(|error| match error.expected {
                                Some(expected) => serde_json::json!({"environment":error.name,"expectedEnvironment":expected}),
                                None => serde_json::json!({"environment":error.name}),
                            })
                    })
                    .or_else(|| counter_range.and(counter_name).map(|name| serde_json::json!({"counter":name})))
                    .or_else(|| length_range.and(length_name).map(|name| serde_json::json!({"length":name})))
                    .or_else(|| color_range.and(color_name).map(|name| serde_json::json!({"color":name})))
                    .or_else(|| model_range.and(color_model).map(|name| serde_json::json!({"model":name})))
                    .or_else(|| input_range.and(missing_file).map(|name| serde_json::json!({"file":name})))
                    .or_else(|| listing_range.map(|range| {
                        let start = text.offset(range.start).unwrap();
                        let end = text.offset(range.end).unwrap();
                        serde_json::json!({"file":&text.source[start..end]})
                    }))
                    .or_else(|| fancyverb_range.map(|range| {
                        let start = text.offset(range.start).unwrap();
                        let end = text.offset(range.end).unwrap();
                        serde_json::json!({"file":&text.source[start..end]})
                    }))
                    .or_else(|| loader_range.map(|(_, command)| serde_json::json!({"file":missing_file,"loaderCommand":command})))
                    .or_else(|| clash_range.map(|(_, command)| {
                        let mut data = serde_json::json!({"package":package_clash,"loaderCommand":command});
                        if let Some(options) = package_options {
                            data["loadedOptions"] = options.loaded.clone().into();
                            data["requestedOptions"] = options.requested.clone().into();
                        }
                        data
                    }))
                    .or_else(|| graphic_range.and(missing_graphic).map(|name| serde_json::json!({"file":name})))
                    .or_else(|| missing_include_key(&message).map(|name| serde_json::json!({"file":name})))
                    .map(|mut data| {
                        if special_range.is_some() || missing_include_key(&message).is_some() || color_range.is_some() || model_range.is_some() || counter_range.is_some() || length_range.is_some() || environment_range.is_some() || input_range.is_some() || loader_range.is_some() || clash_range.is_some() {
                            if let Some(context) = macro_context {
                                data["macroSources"] = serde_json::json!(context.macros.sources);
                                if !context.macros.paused_sources.is_empty() {
                                    data["macroPausedSources"] = serde_json::json!(context.macros.paused_sources);
                                }
                            }
                        }
                        data
                    }),
                ..Default::default()
            }
        }));
}

fn pdftex_error(message: &str) -> bool {
    message
        .strip_prefix("pdfTeX error")
        .is_some_and(|tail| tail.starts_with(':') || tail.starts_with(" ("))
}

fn error_message(line: &str) -> Option<&str> {
    line.strip_prefix("! ").or_else(|| {
        let message = line.strip_prefix('!')?;
        pdftex_error(message).then_some(message)
    })
}

fn fatal_summary(message: &str) -> bool {
    message.trim() == "==> Fatal error occurred, no output PDF file produced!"
}

fn incomplete_engine_file(message: &str) -> bool {
    pdftex_error(message)
        && message
            .rsplit_once("(file ")
            .is_some_and(|(_, file)| !file.contains("):"))
}

fn driver_message(line: &str) -> Option<(&'static str, DiagnosticSeverity, &str)> {
    let line = line.trim_start();
    let (driver, tail) = ["xdvipdfmx", "dvipdfmx"].into_iter().find_map(|driver| {
        line.strip_prefix(driver)
            .and_then(|tail| tail.strip_prefix(':'))
            .map(|tail| (driver, tail))
    })?;
    let (severity, body) = if let Some(body) = tail.strip_prefix("fatal:") {
        (DiagnosticSeverity::ERROR, body)
    } else {
        (DiagnosticSeverity::WARNING, tail.strip_prefix("warning:")?)
    };
    let body = body.trim();
    (!body.is_empty()).then_some((driver, severity, body))
}

fn warning_start(line: &str) -> bool {
    (line.starts_with("LaTeX ") || line.starts_with("Package ") || line.starts_with("Class "))
        && line.contains(" Warning:")
        || [
            "pdfTeX warning",
            "LuaTeX warning",
            "LuaHBTeX warning",
            "XeTeX warning",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        || line
            .strip_prefix("warning")
            .is_some_and(|tail| tail.trim_start().starts_with("(pdf backend):"))
}

fn information_prefix(line: &str) -> Option<&str> {
    for (header, continuation) in [
        ("LaTeX Info:", ""),
        ("LaTeX Font Info:", "Font"),
        ("LaTeX Encoding Info:", "Encoding"),
    ] {
        if line.starts_with(header) {
            return Some(continuation);
        }
    }
    let named = line
        .strip_prefix("Package ")
        .or_else(|| line.strip_prefix("Class "))?;
    let (name, _) = named.split_once(" Info:")?;
    (!name.trim().is_empty()).then_some(name)
}

/// Information text can contain file-like parentheses, but does not enter a
/// source. Commit speculative byte continuations only after a message footer
/// or the same emitter's continuation prefix confirms them.
fn skip_information(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
    prefix: &str,
) {
    let mut cursor = *i;
    let mut committed = *i;
    let mut bytes = lines[cursor].len();
    while let Some(next) = lines.get(cursor + 1) {
        let owned = if prefix.is_empty() {
            next.starts_with("            ")
        } else {
            next.trim_start()
                .strip_prefix('(')
                .and_then(|tail| tail.strip_prefix(prefix))
                .and_then(|tail| tail.strip_prefix(')'))
                .is_some_and(|tail| tail.starts_with(char::is_whitespace))
        };
        if owned {
            // The emitter's prefix proves that even diagnostic-shaped text
            // belongs to the information message, not the compiler protocol.
            cursor += 1;
            committed = cursor;
            bytes = next.len();
            continue;
        }
        if lines[cursor].len() < 60
            || next.trim().is_empty()
            || (message_boundary(next) && !next.trim_start().starts_with(')'))
            || next.trim_start().starts_with('<')
            || next.trim_start().starts_with("Here is how much")
            || cursor - committed >= 64
            || bytes + next.len() + 1 > 4096
        {
            break;
        }
        let mut next_index = cursor + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        let mut next_index = cursor + 1;
        let announcement = continued_file_line(root, lines, &mut next_index);
        if announcement
            .trim_start()
            .strip_prefix('(')
            .and_then(|tail| {
                let length = stack_file(root, tail)
                    .map(|(_, length)| length)
                    .into_iter()
                    .chain(unresolved_stack_file(tail))
                    .max()?;
                Some(tail[length..].trim_start_matches(')').trim())
            })
            .is_some_and(|tail| tail.is_empty() || tail.starts_with('('))
        {
            break;
        }
        cursor += 1;
        bytes += next.len() + 1;
        if next.trim_end().ends_with('.') {
            committed = cursor;
            bytes = next.len();
        }
    }
    *i = committed;
}

fn message_boundary(line: &str) -> bool {
    let line = line.trim_start();
    line == crate::output::TRUNCATED
        || warning_start(line)
        || information_prefix(line).is_some()
        || error_message(line).is_some()
        || driver_message(line).is_some()
        || line.starts_with("==> Fatal error occurred")
        || line.starts_with("Transcript written on ")
        || line.starts_with("Output written on ")
        || line == "No pages of output."
        || line == "No output PDF file written."
        || line.starts_with("Missing character:")
        || line.starts_with("Overfull ")
        || line.starts_with("Underfull ")
        || line.starts_with("No file ")
        || numbered_error(line).is_some()
        || line
            .strip_prefix("l.")
            .is_some_and(|tail| tail.starts_with(|ch: char| ch.is_ascii_digit()))
        || line.starts_with(')')
}

/// Confirm complete reference warnings before committing speculative byte wraps.
fn byte_wrapped_reference_message(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
    initial: &str,
) -> Option<String> {
    undefined_reference_start(initial)?;
    byte_wrapped_literal_message::<79>(root, lines, separator_lines, i, initial, |message| {
        message.ends_with('.') && undefined_reference_key(message).is_some()
    })
}

/// Probe byte wraps in literal warning names without consuming file-stack lines.
/// Commit the cursor only when a complete known warning is confirmed.
/// COMPLETE_WIDTH controls lookahead for apparent footers within a name.
fn byte_wrapped_literal_message<const COMPLETE_WIDTH: usize>(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
    initial: &str,
    is_complete: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut message = initial.to_owned();
    let mut complete = None;
    for end in (*i..).take(64) {
        if is_complete(&message) {
            // A complete warning suffix at the print width may belong to the name.
            complete = Some((message.clone(), end));
            // LuaTeX can wrap early to keep a whole UTF-8 character together.
            // An apparent complete warning near the width still needs lookahead.
            let unicode_continuation = lines
                .get(end + 1)
                .and_then(|line| line.chars().next())
                .is_some_and(|character| !character.is_ascii());
            let unicode_wrap =
                unicode_continuation && lines[end].len() >= COMPLETE_WIDTH - char::MAX.len_utf8();
            if lines[end].len() < COMPLETE_WIDTH && !unicode_wrap {
                break;
            }
        }
        if lines[end].len() < 60 || message.len() >= 4096 {
            break;
        }
        let Some(next) = lines.get(end + 1) else {
            break;
        };
        if message.len() + next.len() > 4096
            || next.trim().is_empty()
            || (message_boundary(next) && !next.trim_start().starts_with(')'))
            || next.trim_start().starts_with('<')
            || next.trim_start().starts_with("Here is how much")
        {
            break;
        }
        let mut next_index = end + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        message.push_str(next);
    }
    let (message, end) = complete?;
    *i = end;
    Some(message)
}

fn byte_wrapped_font_message(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
    initial: &str,
) -> Option<String> {
    font_warning_body(initial)?;
    let mut message = initial.to_owned();
    let mut complete = None;
    for end in (*i..).take(64) {
        if font_substitution(&message).is_some() {
            complete = Some((message.clone(), end));
            // At a print-width boundary an apparent footer can still be
            // literal font-name text. LuaTeX can stop before a UTF-8 scalar.
            if lines[end].len() < 57 {
                break;
            }
        }
        let Some(next) = lines.get(end + 1) else {
            break;
        };
        let semantic_line = next.trim_start().starts_with("(Font)")
            && (message.ends_with("' undefined") || message.ends_with("> not available"));
        // At narrow print widths the requested size can start a physical
        // line. Its angle bracket belongs to the warning, not a font resource.
        let size_continuation = next.starts_with('<')
            && font_warning_body(&message)
                .and_then(|body| body.rsplit_once("' in size "))
                .is_some_and(|(_, tail)| tail.is_empty());
        if !semantic_line && lines[end].len() < 57
            || message.len() + next.len() + 1 > 4096
            || next.trim().is_empty()
            || message_boundary(next)
            || next.trim_start().starts_with('<') && !size_continuation
            || next.trim_start().starts_with("Here is how much")
        {
            break;
        }
        let mut next_index = end + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        if !semantic_line && next.trim_start().starts_with('(') {
            let announcement = continued_file_line(root, lines, &mut next_index);
            if announcement
                .trim_start()
                .strip_prefix('(')
                .is_some_and(|tail| {
                    stack_file(root, tail).is_some() || unresolved_file_opening(tail)
                })
            {
                break;
            }
        }
        if semantic_line {
            message.push(' ');
            message.push_str(next.trim_start());
        } else {
            message.push_str(next);
        }
    }
    let (message, end) = complete?;
    *i = end;
    Some(message)
}

fn wrapped_message(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
) -> String {
    let initial = lines[*i].trim_start();
    if initial.starts_with(OVERSIZED_FLOAT_PREFIX) {
        return byte_wrapped_literal_message::<79>(
            root,
            lines,
            separator_lines,
            i,
            initial,
            |message| oversized_float_excess(message).is_some(),
        )
        .unwrap_or_else(|| initial.to_owned());
    }
    if font_warning_body(initial).is_some() {
        // Do not invent a name by adding spaces to an unconfirmed wrap, or
        // consume an adjacent file/error while looking for a missing footer.
        return byte_wrapped_font_message(root, lines, separator_lines, i, initial)
            .unwrap_or_else(|| initial.to_owned());
    }
    // Engines byte-wrap missing-glyph messages inside font descriptors too.
    // Use the same literal joining and source boundaries as their error form.
    if initial.starts_with("Missing character:") {
        return wrapped_error_message(root, lines, separator_lines, i, initial);
    }
    let lua_destination = initial.strip_prefix("warning").is_some_and(|tail| {
        tail.trim_start()
            .starts_with("(pdf backend): unreferenced destination with name '")
            || tail
                .trim_start()
                .starts_with("(pdf backend): ignoring duplicate destination with the name '")
    });
    if let Some(message) = lua_destination
        .then(|| {
            byte_wrapped_literal_message::<79>(
                root,
                lines,
                separator_lines,
                i,
                initial,
                |message| destination_warning(message).is_some(),
            )
        })
        .flatten()
    {
        return message;
    }
    if let Some(message) = duplicate_label_start(initial).and_then(|_| {
        byte_wrapped_literal_message::<79>(root, lines, separator_lines, i, initial, |message| {
            duplicate_label_key(message).is_some()
        })
    }) {
        return message;
    }
    if initial.starts_with("pdfTeX warning (dest): name{")
        || initial.starts_with("pdfTeX warning (ext4): destination with the same identifier (name{")
        || lua_destination
    {
        // The engines break these messages at a byte count, even within a
        // destination name or a word. Join the original bytes; adding spaces
        // or trimming a continuation can change the destination's identity.
        let mut message = initial.to_owned();
        for _ in 0..64 {
            let complete = destination_warning_name(&message).is_some()
                || duplicate_destination_warning_name(&message).is_some();
            // LuaTeX prints names without escaping apostrophes. An apostrophe
            // exactly at the print width can belong to a continued name.
            if (complete && (!lua_destination || lines[*i].len() < 79)) || message.len() >= 4096 {
                break;
            }
            let Some(next) = lines.get(*i + 1) else { break };
            let closes_destination = initial
                .starts_with("pdfTeX warning (ext4): destination with the same identifier (name{")
                && message.ends_with('}')
                && next.starts_with(") has been already used");
            let continues_escape = initial.starts_with("pdfTeX warning (")
                && message.ends_with('\\')
                && next.starts_with(['(', ')']);
            if next.trim().is_empty()
                || (message_boundary(next) && !closes_destination && !continues_escape)
                || (next.trim_start().starts_with(['<', '(']) && !continues_escape)
                || [
                    "Output written on ",
                    "Transcript written on ",
                    "Here is how much",
                ]
                .iter()
                .any(|prefix| next.trim_start().starts_with(prefix))
            {
                break;
            }
            let mut next_index = *i + 1;
            if numbered_error(&continued_error_line(
                root,
                lines,
                separator_lines,
                &mut next_index,
            ))
            .is_some()
            {
                break;
            }
            message.push_str(next);
            *i += 1;
        }
        return message;
    }
    if let Some(message) = byte_wrapped_reference_message(root, lines, separator_lines, i, initial)
    {
        return message;
    }
    let reference = undefined_reference_start(initial).is_some();
    let rerunfilecheck = initial.starts_with("Package rerunfilecheck Warning:");
    // TeX byte-wraps long warning lines, including inside a literal key.
    // Keep blanks at those boundaries instead of inventing a separator.
    let mut message = if rerunfilecheck {
        // A filename can continue with parentheses that look like a source
        // announcement. Only consume them when the full warning is confirmed.
        byte_wrapped_literal_message::<60>(
            root,
            lines,
            separator_lines,
            i,
            initial,
            rerunfilecheck_request,
        )
        .unwrap_or_else(|| initial.to_owned())
    } else if reference {
        initial.to_owned()
    } else {
        lines[*i].trim().to_owned()
    };
    while (!message.ends_with(['.', '!'])
        || (reference && lines[*i].len() >= 60 && undefined_reference_key(&message).is_none())
        || (rerunfilecheck && lines[*i].len() >= 60)
        // biblatex puts explanatory sentences after a complete first sentence.
        // Keep its own continuation lines with the original warning.
        || initial.starts_with("Package biblatex Warning:")
            && lines.get(*i + 1).is_some_and(|line| {
                line.trim_start()
                    .strip_prefix("(biblatex)")
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
            })
        || rerunfilecheck
            && lines.get(*i + 1).is_some_and(|line| {
                line.trim_start()
                    .strip_prefix("(rerunfilecheck)")
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
            }))
        && *i + 1 < lines.len()
        && message.len() < 4096
    {
        let next = lines[*i + 1];
        if next.trim().is_empty() || message_boundary(next) {
            break;
        }
        let mut next_index = *i + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        if next.trim_start().starts_with('(') {
            let mut next_index = *i + 1;
            let announcement = continued_file_line(root, lines, &mut next_index);
            if announcement
                .trim_start()
                .strip_prefix('(')
                .is_some_and(|tail| {
                    stack_file(root, tail).is_some() || unresolved_file_opening(tail)
                })
            {
                break;
            }
        }
        let byte_wrapped = (reference || rerunfilecheck)
            && lines[*i].len() >= 60
            && !next.trim_start().starts_with('(');
        *i += 1;
        if byte_wrapped {
            message.push_str(next);
        } else {
            message.push(' ');
            message.push_str(next.trim());
        }
    }
    message
}

fn continued_error_line<'a>(
    root: &Path,
    lines: &[&'a str],
    separator_lines: &[usize],
    i: &mut usize,
) -> Cow<'a, str> {
    let line = lines[*i];
    if line.len() < 60 || numbered_error(line).is_some() || message_boundary(line) {
        return Cow::Borrowed(line);
    }
    // Most long lines are ordinary compiler output. Look ahead only when a
    // possible file-line suffix exists nearby, avoiding repeated scans of
    // large typeout logs that contain no diagnostics.
    let next = separator_lines.partition_point(|index| *index <= *i);
    if separator_lines
        .get(next)
        .is_none_or(|index| index - *i > 64)
    {
        return Cow::Borrowed(line);
    }
    let mut joined = line.to_owned();
    for (index, next) in lines.iter().enumerate().skip(*i + 1).take(64) {
        // A suffix of a wrapped filename can itself look like a file-line
        // error. Only treat it as an independent diagnostic if it resolves.
        let numbered = numbered_error(next);
        if next.trim().is_empty()
            || numbered.is_some_and(|(name, _, _)| location(root, name).is_some())
            || numbered.is_none() && message_boundary(next)
            || joined.len() + next.len() > 8192
        {
            break;
        }
        joined.push_str(next);
        if numbered_error(&joined).is_some_and(|(name, _, _)| location(root, name).is_some()) {
            *i = index;
            return Cow::Owned(joined);
        }
    }
    Cow::Borrowed(line)
}

fn wrapped_error_message(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
    initial: &str,
) -> String {
    let box_prefix = !initial.is_empty()
        && ["Overfull ", "Underfull "]
            .iter()
            .any(|prefix| initial.starts_with(prefix) || prefix.starts_with(initial));
    let box_after_location = initial.is_empty()
        && lines.get(*i + 1).is_some_and(|line| {
            line.trim_start().starts_with("Overfull ")
                || line.trim_start().starts_with("Underfull ")
        });
    if box_prefix || box_after_location {
        return wrapped_box_message(root, lines, separator_lines, i, initial);
    }
    if initial == "FancyVerb Error:"
        && lines
            .get(*i + 1)
            .is_some_and(|line| line.starts_with("  No verbatim file "))
    {
        // The package deliberately puts its final period on a separate line.
        // A period at a print-width boundary can instead belong to a filename.
        // Do not consume an incomplete block or an adjacent diagnostic.
        let mut message = initial.to_owned();
        for (index, next) in lines.iter().enumerate().skip(*i + 1).take(64) {
            if *next == "." && message.contains("  No verbatim file ") {
                message.push('.');
                *i = index;
                return message;
            }
            if next.trim().is_empty()
                || message_boundary(next)
                || next.starts_with('\\')
                || message.len() + next.len() > 8192
            {
                break;
            }
            message.push_str(next);
        }
        return initial.to_owned();
    }
    let mut message = initial.to_owned();
    for _ in 0..64 {
        // A wrap can land on a period inside a driver or filename. It is only
        // the end of a recognized file error once its closing text is present.
        let incomplete_file = [
            "LaTeX Error: File ",
            "I can't find file ",
            "Package pdftex.def Error: File ",
            "Package luatex.def Error: File ",
            "Package Listings Error: File ",
            "Unable to load picture or PDF file ",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix) || prefix.starts_with(&message))
            && missing_graphic_name(&message).is_none()
            && missing_listing_name(&message).is_none()
            || incomplete_engine_file(&message);
        let incomplete_float = message.starts_with(OVERSIZED_FLOAT_PREFIX)
            && oversized_float_excess(&message).is_none();
        if message.trim_end().ends_with(['.', '!']) && !incomplete_file && !incomplete_float
            || message.len() >= 4096
        {
            break;
        }
        let Some(next) = lines.get(*i + 1) else { break };
        // pdfTeX can reach its print width exactly at the Unicode header,
        // emitting a blank line before LaTeX's indented continuation. Bridge
        // that one gap only when the complete, consistent error is confirmed.
        if next.trim().is_empty() && message.starts_with("LaTeX Error: Unicode character ") {
            if let Some(continuation) = lines
                .get(*i + 2)
                .filter(|line| message.len() + line.len() <= 4096)
            {
                let complete = format!("{message}{continuation}");
                if unsupported_unicode_character(&complete).is_some() {
                    message = complete;
                    *i += 2;
                    continue;
                }
            }
        }
        let engine_file_close =
            incomplete_engine_file(&message) && next.trim_start().starts_with("):");
        if next.trim().is_empty()
            || (message_boundary(next) && !engine_file_close)
            || [
                "<inserted text>",
                "<recently read>",
                "<to be read again>",
                "<argument>",
                "<*>",
                "See the LaTeX manual",
                "Type  H <return>",
            ]
            .iter()
            .any(|prefix| next.trim_start().starts_with(prefix))
        {
            break;
        }
        let mut next_index = *i + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        if next.trim_start().starts_with('(') {
            let announcement = continued_file_line(root, lines, &mut next_index);
            if announcement
                .trim_start()
                .strip_prefix('(')
                .is_some_and(|tail| {
                    stack_file(root, tail).is_some() || unresolved_file_opening(tail)
                })
            {
                break;
            }
        }
        // TeX wraps at a byte count, including inside words and filenames.
        // Preserve the spaces it actually printed when removing the break.
        message.push_str(next);
        *i += 1;
    }
    message.trim().to_owned()
}

fn stack_file(root: &Path, tail: &str) -> Option<(PathBuf, usize)> {
    if let Some(quoted) = tail.strip_prefix('"') {
        let end = quoted.find('"')?;
        let name = &quoted[..end];
        return log_source(Path::new(name))
            .then(|| location(root, name))
            .flatten()
            .map(|path| (path, end + 2));
    }
    // TeX also emits unquoted names containing spaces or parentheses. Require
    // an existing source file rather than splitting the name at whitespace.
    let mut result = None;
    for (dot, _) in tail
        .match_indices('.')
        .take_while(|(index, _)| *index < 4096)
    {
        for extension in log_extensions() {
            let end = dot + 1 + extension.len();
            if tail.get(dot + 1..end) != Some(extension)
                || tail[end..]
                    .starts_with(|ch: char| !ch.is_whitespace() && !matches!(ch, '(' | ')' | '['))
            {
                continue;
            }
            if let Some(path) = location(root, &tail[..end]) {
                result = Some((path, end));
            }
        }
    }
    result
}

#[derive(Clone, PartialEq, Eq)]
struct LogSource {
    path: PathBuf,
    confirmed: bool,
}

/// A recognizable source name still blocks its parent when that file has
/// disappeared before analysis. Generic parentheses do not change sources.
fn unresolved_stack_file(tail: &str) -> Option<usize> {
    if let Some(quoted) = tail.strip_prefix('"') {
        let end = quoted.bytes().take(4096).position(|byte| byte == b'"')?;
        return log_source(Path::new(&quoted[..end])).then_some(end + 2);
    }
    let mut result = None;
    for (dot, _) in tail
        .as_bytes()
        .iter()
        .take(4096)
        .enumerate()
        .filter(|(_, byte)| **byte == b'.')
    {
        for extension in log_extensions() {
            let end = dot + 1 + extension.len();
            if tail.get(dot + 1..end) != Some(extension)
                || tail[end..]
                    .starts_with(|ch: char| !ch.is_whitespace() && !matches!(ch, '(' | ')' | '['))
            {
                continue;
            }
            // Do not swallow an ordinary parenthesis followed by a separate
            // file entry on the same line. Balanced parentheses in a name,
            // and arbitrary parentheses inside quoted names, remain valid.
            let mut depth = 0usize;
            let balanced = tail[..end].bytes().all(|byte| match byte {
                b'(' => {
                    depth += 1;
                    true
                }
                b')' => {
                    if depth == 0 {
                        false
                    } else {
                        depth -= 1;
                        true
                    }
                }
                _ => true,
            });
            if balanced && depth == 0 {
                result = Some(end);
            }
        }
    }
    result
}

// A closed parenthesis mentioning a missing .tex file can be warning prose.
// It opens no persistent context; retain it in the surrounding message.
fn unresolved_file_opening(tail: &str) -> bool {
    let Some(end) = unresolved_stack_file(tail) else {
        return false;
    };
    let tail = &tail[end..];
    let mut depth = 1usize;
    for (index, byte) in tail.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return tail[index + 1..].contains('(');
                }
            }
            _ => {}
        }
    }
    true
}

fn update_file_stack(
    line: &str,
    root: &Path,
    stack: &mut Vec<Option<LogSource>>,
    sources: &mut BTreeSet<Url>,
) {
    let mut i = 0;
    while i < line.len() {
        match line.as_bytes()[i] {
            b'(' => {
                let tail = &line[i + 1..];
                let unresolved = unresolved_stack_file(tail);
                let resolved = stack_file(root, tail)
                    .filter(|(_, length)| unresolved.is_none_or(|longer| longer <= *length));
                if let Some((file, length)) = resolved {
                    if sources.len() < 4096 && crate::workspace::is_source(&file) {
                        if let Ok(uri) = Url::from_file_path(&file) {
                            sources.insert(uri);
                        }
                    }
                    stack.push(Some(LogSource {
                        path: file,
                        confirmed: true,
                    }));
                    i += length;
                } else if let Some(length) = unresolved {
                    stack.push(Some(LogSource {
                        path: root.into(),
                        confirmed: false,
                    }));
                    i += length;
                } else {
                    stack.push(None);
                }
            }
            b')' => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
}

fn continued_file_line<'a>(root: &Path, lines: &[&'a str], i: &mut usize) -> Cow<'a, str> {
    let line = lines[*i];
    // TeX wraps file announcements at max_print_line (normally 79). Join only
    // when the continuation completes a file, with bounded lookahead. Explicit
    // unresolved paths retain their source barrier after byte wrapping too.
    if line.len() >= 60 {
        for (opening, _) in line.match_indices('(') {
            let tail = &line[opening + 1..];
            let complete =
                stack_file(root, tail).is_some() || unresolved_stack_file(tail).is_some();
            // A closing quote proves the filename ended. An unquoted name at
            // the print boundary can still continue, even when its prefix is
            // also an existing file. Prefer an actual longer continuation.
            if complete && tail.starts_with('"') {
                continue;
            }
            let explicit_path =
                tail.starts_with(['"', '/']) || tail.starts_with("./") || tail.starts_with("../");
            let mut joined = line.to_owned();
            let mut plain_continuation = true;
            for (next_index, next) in lines.iter().enumerate().skip(*i + 1).take(64) {
                if next.is_empty()
                    || *next == crate::output::TRUNCATED
                    || warning_start(next)
                    || information_prefix(next).is_some()
                    || error_message(next).is_some()
                    || driver_message(next).is_some()
                    || numbered_error(next).is_some()
                    || joined.len() + next.len() > 8192
                {
                    break;
                }
                joined.push_str(next);
                plain_continuation &= !next.chars().any(|ch| ch.is_whitespace() || ch == ':');
                let tail = &joined[opening + 1..];
                if stack_file(root, tail)
                    .is_some_and(|(_, length)| length > line.len() - opening - 1)
                    || explicit_path
                        && (!complete || plain_continuation)
                        && unresolved_stack_file(tail)
                            .is_some_and(|length| length > line.len() - opening - 1)
                {
                    *i = next_index;
                    return Cow::Owned(joined);
                }
            }
        }
    }
    Cow::Borrowed(line)
}

pub fn parse(root: &Path, log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    parse_with_sources(root, log).0
}

/// PDF driver errors can appear only on stderr, after TeX closes its log. Keep
/// intervening blank barriers so unrelated driver detail blocks stay separate.
pub(crate) fn parse_driver_output(root: &Path, output: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut log = String::new();
    for line in output.lines() {
        if driver_message(line).is_some() || line == crate::output::TRUNCATED {
            log.push_str(line);
        }
        log.push('\n');
    }
    parse(root, &log)
}

/// The recorder also lists existence probes. File-entry events in the final
/// log provide narrower evidence for diagnostics with no source line.
pub(crate) fn parse_with_sources(
    root: &Path,
    log: &str,
) -> (BTreeMap<Url, Vec<Diagnostic>>, BTreeSet<Url>) {
    parse_in_project(root, log, None)
}

struct MacroContext<'a> {
    signatures: crate::signature::CommandSignatures<'a>,
    sources: Vec<Url>,
    paused_sources: Vec<Url>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InvocationKind {
    Color,
    ColorModel,
    Counter,
    Length,
    EnvironmentBegin,
    EnvironmentEnd,
    File,
    Literal,
}

impl InvocationKind {
    fn for_message(message: &str) -> Option<Self> {
        if crate::special_characters::error_character(message, "").is_some()
            || message == "Missing $ inserted."
        {
            Some(Self::Literal)
        } else if undefined_color_name(message).is_some() {
            Some(Self::Color)
        } else if undefined_color_model_name(message).is_some() {
            Some(Self::ColorModel)
        } else if undefined_counter_name(message).is_some() {
            Some(Self::Counter)
        } else if undefined_length_name(message).is_some() {
            Some(Self::Length)
        } else {
            environment_error(message)
                .map(|error| {
                    if error.command == "begin" {
                        Self::EnvironmentBegin
                    } else {
                        Self::EnvironmentEnd
                    }
                })
                .or_else(|| {
                    (missing_file_name(message).is_some()
                        || package_option_clash_name(message).is_some()
                        || missing_graphic_name(message).is_some())
                    .then_some(Self::File)
                })
        }
    }

    fn invocations(
        self,
        source: &str,
        syntax: &crate::syntax::Syntax,
        signatures: &crate::signature::CommandSignatures<'_>,
    ) -> Vec<(ByteRange<usize>, ByteRange<usize>)> {
        match self {
            Self::Literal => vec![],
            Self::Color => {
                crate::named_colors::error_invocations_with_signatures(source, syntax, signatures)
            }
            Self::ColorModel => crate::named_colors::model_error_invocations_with_signatures(
                source, syntax, signatures,
            ),
            Self::Counter => {
                crate::counters::error_invocations_with_signatures(source, syntax, signatures)
            }
            Self::Length => {
                crate::lengths::error_invocations_with_signatures(source, syntax, signatures)
            }
            Self::File => file_error_invocations_with_signatures(source, syntax, signatures),
            Self::EnvironmentBegin | Self::EnvironmentEnd => {
                environment_error_invocations_with_signatures(
                    source,
                    syntax,
                    signatures,
                    if self == Self::EnvironmentBegin {
                        "begin"
                    } else {
                        "end"
                    },
                )
            }
        }
    }
}

struct InvocationCalls<'a> {
    source: &'a str,
    invocations: Vec<(ByteRange<usize>, ByteRange<usize>)>,
    file_commands: Vec<crate::syntax::Command>,
}

struct InvocationContext<'a> {
    macros: Rc<MacroContext<'a>>,
    calls: Rc<InvocationCalls<'a>>,
}

/// Saved explicit source dependencies, captured before TeX starts. No directory
/// scan or distribution-package lookup participates in this snapshot.
pub(crate) struct ProjectSources {
    documents: BTreeMap<Url, crate::workspace::Document>,
    dynamic_sources: BTreeSet<Url>,
}

impl ProjectSources {
    pub(crate) fn capture(root: &Path) -> Self {
        let mut workspace = crate::workspace::Workspace::default();
        let mut sources = BTreeSet::new();
        let mut source_calls = crate::source_inputs::Index::default();
        if let Ok(uri) = Url::from_file_path(root.canonicalize().unwrap_or_else(|_| root.into())) {
            workspace.reload(&uri);
            workspace.load_dependencies(&uri, 0);
            sources = workspace.source_documents_from(&uri, &uri);
            source_calls = crate::source_inputs::Index::new(&workspace, &uri);
        }
        let documents: BTreeMap<_, _> = workspace
            .documents
            .into_iter()
            .filter(|(uri, doc)| {
                sources.contains(uri) && doc.kind == crate::workspace::DocumentKind::Latex
            })
            .collect();
        // An unresolved source load can redefine previously known commands,
        // even after it has returned to a file included in the snapshot.
        let dynamic_sources = documents
            .iter()
            .filter(|(uri, doc)| {
                let alias_bodies =
                    crate::signature::file_load_alias_body_positions(&doc.text.source, &doc.syntax);
                source_calls.incomplete
                    || doc
                        .syntax
                        .commands
                        .iter()
                        .chain(
                            source_calls
                                .calls
                                .get(*uri)
                                .into_iter()
                                .flatten()
                                .chain(source_calls.loaders.get(*uri).into_iter().flatten())
                                .map(|call| &call.command),
                        )
                        .any(|command| {
                            if alias_bodies.contains(&command.span.start) {
                                return false;
                            }
                            if !matches!(
                                command.name.as_str(),
                                "input" | "InputIfFileExists" | "include" | "subfile"
                            ) && crate::syntax::loader_extension(&command.name).is_none()
                            {
                                return false;
                            }
                            if crate::syntax::file_list(&command.name) {
                                return crate::syntax::complete_file_list_spans(
                                    &doc.text.source,
                                    command,
                                )
                                .is_none();
                            }
                            crate::syntax::file_argument(&doc.text.source, command).is_none_or(
                                |argument| {
                                    let mut name = argument.value.trim().to_owned();
                                    for character in ['_', '%', '#', '&', '$', ' '] {
                                        name = name.replace(&format!("\\{character}"), "_");
                                    }
                                    !crate::syntax::is_literal_file_path(&name)
                                },
                            )
                        })
            })
            .map(|(uri, _)| uri.clone())
            .collect();
        Self {
            documents,
            dynamic_sources,
        }
    }
}

/// Keep only the latest macro scope and one document per error kind. The cache
/// lasts for this log parse; source contents are still checked on every error.
struct ProjectAnalysis<'a> {
    project: &'a ProjectSources,
    macros: Option<Rc<MacroContext<'a>>>,
    calls: [Option<(Url, Rc<InvocationCalls<'a>>)>; 8],
}

impl<'a> ProjectAnalysis<'a> {
    fn new(project: &'a ProjectSources) -> Self {
        Self {
            project,
            macros: None,
            calls: std::array::from_fn(|_| None),
        }
    }

    fn context(
        &mut self,
        loaded: &BTreeSet<Url>,
        paused: &BTreeSet<Url>,
        uri: &Url,
        kind: InvocationKind,
    ) -> Result<Option<InvocationContext<'a>>, ()> {
        let documents: Vec<_> = self
            .project
            .documents
            .iter()
            .filter(|(uri, _)| loaded.contains(*uri))
            .collect();
        let ancestors: Vec<_> = self
            .project
            .documents
            .iter()
            .filter(|(uri, _)| paused.contains(*uri))
            .collect();
        if documents.iter().chain(&ancestors).any(|(uri, doc)| {
            uri.to_file_path()
                .ok()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .as_deref()
                != Some(doc.text.source.as_str())
        }) {
            return Err(());
        }
        if documents.is_empty() {
            return Ok(None);
        }
        let sources: Vec<_> = documents.iter().map(|(uri, _)| (*uri).clone()).collect();
        let paused_sources: Vec<_> = ancestors.iter().map(|(uri, _)| (*uri).clone()).collect();
        if self.macros.as_ref().is_none_or(|cached| {
            cached.sources != sources || cached.paused_sources != paused_sources
        }) {
            self.macros = Some(Rc::new(MacroContext {
                sources,
                paused_sources,
                signatures: crate::signature::CommandSignatures::with_ambiguous_environments(
                    documents
                        .into_iter()
                        .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
                    // Parent declarations may precede or follow the paused input.
                    // They can shadow existing meanings, but cannot certify arity.
                    ancestors.iter().copied().flat_map(|(_, doc)| {
                        doc.syntax
                            .definitions
                            .iter()
                            .map(|entry| entry.name.as_str())
                    }),
                    ancestors.iter().copied().flat_map(|(_, doc)| {
                        doc.syntax
                            .environments
                            .iter()
                            .map(|entry| entry.name.as_str())
                    }),
                )
                .with_literal_sources(
                    ancestors
                        .iter()
                        .copied()
                        .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
                ),
            }));
            self.calls = std::array::from_fn(|_| None);
        }
        let macros = self.macros.as_ref().expect("nonempty macro scope");
        let document = self.project.documents.get(uri).ok_or(())?;
        let cached_calls = &mut self.calls[kind as usize];
        if cached_calls
            .as_ref()
            .is_some_and(|(cached, _)| cached != uri)
        {
            *cached_calls = None;
        }
        let (_, calls) = cached_calls.get_or_insert_with(|| {
            let source = document.text.source.as_str();
            let invocations = kind.invocations(source, &document.syntax, &macros.signatures);
            (
                uri.clone(),
                Rc::new(InvocationCalls {
                    source,
                    invocations,
                    file_commands: if kind == InvocationKind::File {
                        file_error_commands(source, &document.syntax, &macros.signatures)
                    } else {
                        vec![]
                    },
                }),
            )
        });
        Ok(Some(InvocationContext {
            macros: Rc::clone(macros),
            calls: Rc::clone(calls),
        }))
    }
}

pub(crate) fn parse_with_project_sources(
    root: &Path,
    log: &str,
    project: &ProjectSources,
) -> (BTreeMap<Url, Vec<Diagnostic>>, BTreeSet<Url>) {
    parse_in_project(root, log, Some(project))
}

fn parse_in_project<'a>(
    root: &Path,
    log: &'a str,
    project: Option<&'a ProjectSources>,
) -> (BTreeMap<Url, Vec<Diagnostic>>, BTreeSet<Url>) {
    let mut result = BTreeMap::new();
    // Every message for one shaped glyph run expands to all matching source
    // positions. Expand identical contexts once, avoiding quadratic diagnostics
    // and repeated source parsing when the run repeats the same character.
    let mut glyph_contexts = BTreeSet::new();
    let mut emit = |result: &mut BTreeMap<Url, Vec<Diagnostic>>,
                    path: &Path,
                    line: u32,
                    message: String,
                    severity: DiagnosticSeverity,
                    error: Option<ErrorContext<'a>>,
                    confirmed: bool| {
        if let Some(context) = error
            .as_ref()
            .filter(|_| missing_character(&message).is_some())
        {
            if !glyph_contexts.insert((
                path.to_path_buf(),
                line,
                message.clone(),
                severity,
                context.source,
                confirmed,
            )) {
                return;
            }
        }
        emit_with_context(result, path, line, message, severity, error, confirmed);
    };
    let mut sources = BTreeSet::new();
    let mut analysis = project.map(ProjectAnalysis::new);
    let root_path = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Ok(uri) = Url::from_file_path(root_path) {
        sources.insert(uri);
    }
    let mut stack: Vec<Option<LogSource>> = Vec::new();
    let mut fallback_confirmed = true;
    let mut pending: Option<(LogSource, String, usize)> = None;
    let lines = log.lines().collect::<Vec<_>>();
    let separator_lines: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let bytes = line.as_bytes();
            line.match_indices(':')
                .any(|(offset, _)| {
                    offset == 0
                        || bytes[offset - 1].is_ascii_digit()
                        || bytes.get(offset + 1).is_some_and(u8::is_ascii_digit)
                })
                .then_some(index)
        })
        .collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line == crate::output::TRUNCATED {
            if let Some((path, message, _)) = pending.take() {
                emit(
                    &mut result,
                    &path.path,
                    1,
                    message,
                    DiagnosticSeverity::ERROR,
                    None,
                    path.confirmed,
                );
            }
            stack.clear();
            fallback_confirmed = false;
            i += 1;
            continue;
        }
        let current = || {
            stack
                .iter()
                .rev()
                .find_map(Option::as_ref)
                .cloned()
                .unwrap_or_else(|| LogSource {
                    path: root.into(),
                    confirmed: fallback_confirmed,
                })
        };
        if let Some((driver, severity, body)) = driver_message(line) {
            if let Some((path, cause, _)) = pending.take() {
                emit(
                    &mut result,
                    &path.path,
                    1,
                    cause,
                    DiagnosticSeverity::ERROR,
                    None,
                    path.confirmed,
                );
            }
            let level = if severity == DiagnosticSeverity::ERROR {
                "fatal"
            } else {
                "warning"
            };
            let mut message = format!("{driver}:{level}: {body}");
            if severity == DiagnosticSeverity::WARNING {
                for _ in 0..64 {
                    let Some((next_driver, next_severity, next_body)) =
                        lines.get(i + 1).and_then(|line| driver_message(line))
                    else {
                        break;
                    };
                    if next_driver != driver
                        || next_severity != severity
                        || !next_body.starts_with(">>")
                        || message.len() + next_body.len() + 1 > 4096
                    {
                        break;
                    }
                    message.push('\n');
                    message.push_str(next_body);
                    i += 1;
                }
            }
            // The PDF driver has no TeX source-line context. Its messages must
            // not borrow an open input file or a later l.N from the TeX log.
            emit(&mut result, root, 1, message, severity, None, false);
            i += 1;
            continue;
        }
        if let Some(prefix) = information_prefix(line) {
            skip_information(root, &lines, &separator_lines, &mut i, prefix);
            i += 1;
            continue;
        }
        let mut structural = false;
        let diagnostic_start = i;
        let error_line = continued_error_line(root, &lines, &separator_lines, &mut i);
        if let Some((name, number, message)) = numbered_error(&error_line) {
            let reported_path = location(root, name);
            let confirmed_path = reported_path.is_some();
            let path = reported_path.unwrap_or_else(|| root.into());
            let message = wrapped_error_message(root, &lines, &separator_lines, &mut i, message);
            if let Some((cause_path, cause, context_start)) = pending.take() {
                // File lookup prompts end with a separate Emergency stop. The
                // numbered error supplies the missing location of its cause.
                if cause_path.path != path
                    || cause_path.confirmed != confirmed_path
                    || cause != message
                {
                    let associated = confirmed_path
                        && cause_path.confirmed
                        && cause_path.path == path
                        && message == "Emergency stop."
                        && missing_read_file(&cause)
                        && uninterrupted_file_prompt(&lines, context_start, diagnostic_start);
                    let cause_line = if associated { number } else { 1 };
                    let context = associated
                        .then(|| following_error_location(&lines, i + 1))
                        .flatten()
                        .filter(|(line, _)| *line == number)
                        .and_then(|(_, context)| context);
                    let context = context.map(|source| {
                        ErrorContext::source(source).with_project(
                            &cause,
                            analysis.as_mut(),
                            &sources,
                            &stack,
                            &cause_path.path,
                        )
                    });
                    emit(
                        &mut result,
                        &cause_path.path,
                        cause_line,
                        cause,
                        DiagnosticSeverity::ERROR,
                        context,
                        cause_path.confirmed,
                    );
                }
            }
            // XeTeX uses the numbered error form for tracinglostchars=3.
            // Plain missing-character lines remain warnings below; font names
            // containing warning-like prose must not downgrade this error.
            let severity = if message.starts_with("Missing character:") {
                DiagnosticSeverity::ERROR
            } else if message.contains("Warning:")
                || warning_start(&message)
                || box_code(&message).is_some()
                || missing_include_key(&message).is_some()
            {
                DiagnosticSeverity::WARNING
            } else {
                DiagnosticSeverity::ERROR
            };
            // An unresolved reported source falls back to the root for display.
            // Its context must not identify coincidentally matching root text.
            let context = confirmed_path
                .then(|| error_context(&lines, i + 1, number, &message))
                .flatten()
                .map(|context| {
                    context.with_project(&message, analysis.as_mut(), &sources, &stack, &path)
                });
            emit(
                &mut result,
                &path,
                number,
                message,
                severity,
                context,
                confirmed_path,
            );
        } else if let Some(message) = error_message(line) {
            let message = wrapped_error_message(root, &lines, &separator_lines, &mut i, message);
            if let Some((path, cause, context_start)) = pending.take() {
                let (cause_line, context) = if path.confirmed
                    && path == current()
                    && message == "Emergency stop."
                    && missing_read_file(&cause)
                    && uninterrupted_file_prompt(&lines, context_start, diagnostic_start)
                {
                    following_error_location(&lines, i + 1).unwrap_or((1, None))
                } else {
                    (1, None)
                };
                let context = context.map(|source| {
                    ErrorContext::source(source).with_project(
                        &cause,
                        analysis.as_mut(),
                        &sources,
                        &stack,
                        &path.path,
                    )
                });
                emit(
                    &mut result,
                    &path.path,
                    cause_line,
                    cause,
                    DiagnosticSeverity::ERROR,
                    context,
                    path.confirmed,
                );
            }
            pending = Some((current(), message, i + 1));
        } else if let Some(tail) = line.trim_start().strip_prefix("l.") {
            let number = tail
                .split(|ch: char| !ch.is_ascii_digit())
                .next()
                .and_then(|value| value.parse().ok());
            if let (Some(number), Some((path, message, context_start))) = (number, pending.take()) {
                let context = path
                    .confirmed
                    .then(|| error_context(&lines, context_start, number, &message))
                    .flatten()
                    .map(|context| {
                        context.with_project(
                            &message,
                            analysis.as_mut(),
                            &sources,
                            &stack,
                            &path.path,
                        )
                    });
                emit(
                    &mut result,
                    &path.path,
                    number,
                    message,
                    DiagnosticSeverity::ERROR,
                    context,
                    path.confirmed,
                );
            }
        } else if warning_start(line) || line.starts_with("Missing character:") {
            let path = current();
            // TeX wraps warnings at its configured print width.
            let message = wrapped_message(root, &lines, &separator_lines, &mut i);
            let number = input_line(&message).unwrap_or(1);
            emit(
                &mut result,
                &path.path,
                number,
                message,
                DiagnosticSeverity::WARNING,
                None,
                path.confirmed,
            );
        } else if line.starts_with("No file ") {
            let path = current();
            if path.confirmed {
                let context = ErrorContext::source("").with_project_kind(
                    InvocationKind::File,
                    analysis.as_mut(),
                    &sources,
                    &stack,
                    &path.path,
                );
                let calls = context
                    .macro_context
                    .as_ref()
                    .map(|context| context.calls.as_ref());
                // Both commands can print the same text. Match independently
                // and prefer the complete longest message across families.
                let mut include_end = i;
                let include = missing_reported_file(
                    &path.path,
                    &lines,
                    &mut include_end,
                    "include",
                    calls,
                    context.project_stale,
                );
                let mut verbatim_end = i;
                let verbatim = missing_reported_file(
                    &path.path,
                    &lines,
                    &mut verbatim_end,
                    "verbatiminput",
                    None,
                    false,
                );
                i = include_end.max(verbatim_end);
                if let Some(name) = include.filter(|_| include_end == i) {
                    emit(
                        &mut result,
                        &path.path,
                        1,
                        format!("Included file `{name}` not found"),
                        DiagnosticSeverity::WARNING,
                        Some(context),
                        true,
                    );
                }
                if let Some(name) = verbatim.filter(|_| verbatim_end == i) {
                    emit_missing_verbatim(&mut result, &path.path, &name);
                }
            }
        } else if line.starts_with("Overfull ") || line.starts_with("Underfull ") {
            let path = current();
            let message = wrapped_box_message(root, &lines, &separator_lines, &mut i, line);
            emit(
                &mut result,
                &path.path,
                input_line(&message).unwrap_or(1),
                message,
                DiagnosticSeverity::WARNING,
                None,
                path.confirmed,
            );
        } else {
            structural = true;
        }
        // User messages and echoed TeX source can contain unmatched parentheses.
        // They describe diagnostics, rather than file entry/exit events.
        if structural {
            let line = continued_file_line(root, &lines, &mut i);
            update_file_stack(&line, root, &mut stack, &mut sources);
        }
        i += 1;
    }
    if let Some((path, message, _)) = pending {
        emit(
            &mut result,
            &path.path,
            1,
            message,
            DiagnosticSeverity::ERROR,
            None,
            path.confirmed,
        );
    }
    // LuaTeX also marks its generic fatal trailer as an error. Keep it only
    // when the log provides no specific cause, otherwise it can sort before
    // the actual failure and become the build's misleading summary.
    let specific_error = result.values().flatten().any(|diagnostic| {
        diagnostic.severity == Some(DiagnosticSeverity::ERROR)
            && !diagnostic.message.trim().is_empty()
            && !fatal_summary(&diagnostic.message)
    });
    if specific_error {
        for diagnostics in result.values_mut() {
            diagnostics.retain(|diagnostic| !fatal_summary(&diagnostic.message));
        }
        result.retain(|_, diagnostics| !diagnostics.is_empty());
    }
    for diagnostics in result.values_mut() {
        diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.range.start,
                diagnostic.message.clone(),
                diagnostic.severity != Some(DiagnosticSeverity::ERROR),
            )
        });
        diagnostics.dedup_by(|a, b| {
            a.range == b.range
                && a.message == b.message
                && a.severity == b.severity
                && a.code == b.code
                && a.data == b.data
        });
    }
    (result, sources)
}

/// BibTeX and Biber write a separate log, often after TeX has already produced
/// a PDF. Preserve those errors instead of hiding them behind latexmk's exit code.
pub fn parse_bibliography(root: &Path, log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    parse_bibliography_log(root, &root.with_extension("blg"), log)
}

pub fn parse_bibliography_log(
    root: &Path,
    log_path: &Path,
    log: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let paths = crate::recorder::BibliographyPaths::new(root, log_path, log);
    let mut result = parse_bibliography_in(root, &paths, log);
    if !crate::output::truncated(log) {
        if let Ok(uri) = Url::from_file_path(root) {
            let sources = BTreeSet::from([uri]);
            locate_missing_bibliography_styles(&sources, &mut result);
            locate_missing_bibliography_databases(&sources, &mut result);
        }
    }
    result
}

#[derive(Clone, Copy)]
enum MissingBibliographyKind {
    Style,
    Database,
}

impl MissingBibliographyKind {
    fn code(self) -> &'static str {
        match self {
            Self::Style => "missing-bibliography-style",
            Self::Database => "missing-bibliography-database",
        }
    }

    fn commands(self) -> &'static [&'static str] {
        match self {
            Self::Style => &["bibliographystyle"],
            Self::Database => &[
                "bibliography",
                "nobibliography",
                "addbibresource",
                "addglobalbib",
            ],
        }
    }

    fn matches(self, diagnostic: &Diagnostic) -> bool {
        diagnostic.code == Some(NumberOrString::String(self.code().into()))
            && (diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                || matches!(self, Self::Database)
                    && diagnostic.severity == Some(DiagnosticSeverity::WARNING)
                    && diagnostic
                        .message
                        .starts_with("Latexmk: Bibliography database not found: "))
    }

    fn name(self, message: &str) -> Option<&str> {
        if matches!(self, Self::Database) {
            if let Some(name) = message
                .strip_prefix("Biber: Cannot find '")
                .and_then(|tail| tail.strip_suffix("'!"))
            {
                return crate::bib_resources::literal_path(name).then_some(name);
            }
        }
        let (prefix, suffix) = match self {
            Self::Style => ("BibTeX: I couldn't open style file ", ".bst"),
            Self::Database => ("BibTeX: I couldn't open database file ", ".bib"),
        };
        let name = message.strip_prefix(prefix).or_else(|| {
            matches!(self, Self::Database)
                .then(|| message.strip_prefix("Latexmk: Bibliography database not found: "))
                .flatten()
        })?;
        let stem = name.strip_suffix(suffix)?;
        (!stem.is_empty()
            && crate::syntax::is_literal_file_path(name)
            && !name.chars().any(char::is_whitespace))
        .then_some(name)
    }
}

pub(crate) fn missing_bibliography_style_name(message: &str) -> Option<&str> {
    MissingBibliographyKind::Style.name(message)
}

pub(crate) fn missing_bibliography_database_name(message: &str) -> Option<&str> {
    MissingBibliographyKind::Database.name(message)
}

pub(crate) fn locate_missing_bibliography_styles(
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    locate_missing_bibliography_files(MissingBibliographyKind::Style, sources, diagnostics, false);
}

pub(crate) fn locate_missing_bibliography_databases(
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    locate_missing_bibliography_files(
        MissingBibliographyKind::Database,
        sources,
        diagnostics,
        false,
    );
}

/// Resolve resource calls only after the build manager has checked its input
/// snapshots. These sources are both project inputs and final-log entries;
/// distribution packages and unread alias providers cannot certify a call.
pub(crate) fn locate_missing_bibliography_resources(
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    for kind in [
        MissingBibliographyKind::Style,
        MissingBibliographyKind::Database,
    ] {
        locate_missing_bibliography_files(kind, sources, diagnostics, true);
    }
}

/// Auxiliary line numbers identify generated data, not the originating TeX
/// declaration. Only a unique static declaration in confirmed sources can
/// provide a source range; unread namesakes must not participate.
fn locate_missing_bibliography_files(
    kind: MissingBibliographyKind,
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
    source_aware: bool,
) {
    use std::io::Read;

    let requested: BTreeSet<_> = diagnostics
        .values()
        .flatten()
        .filter(|item| kind.matches(item))
        .filter_map(|item| kind.name(&item.message).map(str::to_owned))
        .collect();
    if requested.is_empty() || requested.len() > 64 {
        return;
    }
    if source_aware {
        // Earlier direct-only parsing may have selected a declaration in an
        // unused macro body. Certify the location afresh in the full scope.
        for item in diagnostics
            .values_mut()
            .flatten()
            .filter(|item| kind.matches(item))
        {
            if let Some(name) = kind.name(&item.message) {
                if item
                    .data
                    .as_ref()
                    .and_then(|data| data.get("file"))
                    .and_then(|value| value.as_str())
                    == Some(name)
                {
                    item.data = Some(serde_json::json!({"file":name,"unresolvedSource":true}));
                }
            }
        }
    }
    let mut occurrences: BTreeMap<String, Vec<(Url, Range, String, bool)>> = BTreeMap::new();
    let mut documents = Vec::new();
    let mut total = 0usize;
    let mut count = 0usize;
    for uri in sources {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("bib" | "bst")
        ) {
            continue;
        }
        count += 1;
        if count > 1024
            || std::fs::metadata(&path).map_or(true, |metadata| metadata.len() > 2 * 1024 * 1024)
        {
            return;
        }
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        let mut source = String::new();
        if file
            .take(2 * 1024 * 1024 + 1)
            .read_to_string(&mut source)
            .is_err()
        {
            return;
        }
        total += source.len();
        if source.len() > 2 * 1024 * 1024 || total > 16 * 1024 * 1024 {
            return;
        }
        let text = Text::new(source);
        let syntax = crate::syntax::Syntax::latex(&text);
        if syntax
            .definitions
            .iter()
            .any(|entry| kind.commands().contains(&entry.name.as_str()))
        {
            return;
        }
        documents.push((uri.clone(), text, syntax));
    }
    let resource_calls = documents.iter().any(|(_, _, syntax)| {
        syntax
            .commands
            .iter()
            .any(|command| crate::bib_resources::loads(&command.name))
    });
    let signatures = (source_aware || resource_calls).then(|| {
        crate::signature::CommandSignatures::source_context(
            documents
                .iter()
                .map(|(_, text, parsed)| (text.source.as_str(), parsed)),
        )
    });
    let macro_sources: BTreeSet<_> = documents.iter().map(|(uri, _, _)| uri.clone()).collect();
    for (uri, text, syntax) in &documents {
        let positions = signatures
            .as_ref()
            .map(|signatures| signatures.source_command_positions(&text.source, syntax));
        let indirect =
            signatures
                .as_ref()
                .filter(|_| source_aware)
                .map_or_else(Vec::new, |signatures| {
                    signatures.indirect_bibliography_invocations(
                        &text.source,
                        syntax,
                        positions.as_ref().expect("source-aware positions"),
                    )
                });
        if indirect
            .iter()
            .any(|call| !call.certain && kind.commands().contains(&call.command.name.as_str()))
        {
            return;
        }
        for command in syntax
            .commands
            .iter()
            .filter(|command| {
                !(source_aware || crate::bib_resources::loads(&command.name))
                    || positions
                        .as_ref()
                        .is_none_or(|positions| positions.contains(&command.span.start))
            })
            .chain(
                indirect
                    .iter()
                    .filter(|call| call.certain)
                    .map(|call| &call.command),
            )
            .filter(|command| kind.commands().contains(&command.name.as_str()))
        {
            // An expanded or spliced name might be the actual failed file.
            // Do not choose a static namesake by guessing execution order.
            let spans = match kind {
                MissingBibliographyKind::Style => {
                    let Some(argument) = crate::syntax::file_argument(&text.source, command) else {
                        return;
                    };
                    vec![argument.span.clone()]
                }
                MissingBibliographyKind::Database => {
                    if crate::bib_resources::loads(&command.name) {
                        let Some(argument) = crate::syntax::file_argument(&text.source, command)
                        else {
                            return;
                        };
                        vec![argument.span.clone()]
                    } else {
                        let Some(spans) =
                            crate::syntax::complete_file_list_spans(&text.source, command)
                        else {
                            return;
                        };
                        spans
                    }
                }
            };
            for span in spans {
                let value = &text.source[span.clone()];
                let name = match kind {
                    // BibTeX prints an extra suffix for explicit style suffixes,
                    // whereas database names are reported with just one .bib.
                    MissingBibliographyKind::Style => format!("{value}.bst"),
                    MissingBibliographyKind::Database => {
                        if crate::bib_resources::loads(&command.name) {
                            value.to_owned()
                        } else {
                            let Some(name) =
                                crate::syntax::file_lookup_names(&command.name, value).next()
                            else {
                                return;
                            };
                            name.into_owned()
                        }
                    }
                };
                if requested.contains(&name) {
                    let entries = occurrences.entry(name).or_default();
                    if entries.len() < 2 {
                        entries.push((
                            uri.clone(),
                            text.range(span.start, span.end),
                            command.name.clone(),
                            !syntax.commands.iter().any(|direct| {
                                direct.name == command.name && direct.span == command.span
                            }),
                        ));
                    }
                }
            }
        }
    }
    let mut relocated = Vec::new();
    for entries in diagnostics.values_mut() {
        entries.retain(|item| {
            if !kind.matches(item) {
                return true;
            }
            let Some(name) = kind.name(&item.message) else {
                return true;
            };
            if item
                .data
                .as_ref()
                .and_then(|data| data.get("file"))
                .and_then(|name| name.as_str())
                != Some(name)
            {
                return true;
            }
            let Some(candidates) = occurrences.get(name).filter(|entries| entries.len() == 1)
            else {
                return true;
            };
            let (uri, range, command, indirect) = &candidates[0];
            let mut item = item.clone();
            item.range = *range;
            item.data = Some(match kind {
                MissingBibliographyKind::Style => serde_json::json!({"file":name}),
                MissingBibliographyKind::Database => {
                    serde_json::json!({"file":name,"bibliographyCommand":command})
                }
            });
            if *indirect {
                item.data.as_mut().expect("bibliography file data")["macroSources"] =
                    serde_json::json!(macro_sources);
            }
            relocated.push((uri.clone(), item));
            false
        });
    }
    diagnostics.retain(|_, entries| !entries.is_empty());
    for (uri, item) in relocated {
        diagnostics.entry(uri).or_default().push(item);
    }
}

/// latexmk can skip BibTeX when a database is absent, even with a successful
/// exit and a prebuilt .bbl. Read only its final log-inspection block, so TeX
/// messages and earlier passes cannot revive an already repaired failure.
pub(crate) fn parse_latexmk_missing_databases(
    root: &Path,
    log_path: &Path,
    output: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut result = BTreeMap::new();
    if crate::output::truncated(output) {
        return result;
    }
    let Ok(expected) = log_path.canonicalize() else {
        return result;
    };
    let parent = root.parent().unwrap_or_else(|| Path::new("."));
    let lines: Vec<_> = output.lines().collect();
    let Some(start) = lines.iter().rposition(|line| {
        line.strip_prefix("Latexmk: Getting log file '")
            .and_then(|name| name.strip_suffix('\''))
            .is_some_and(|name| {
                parent
                    .join(name)
                    .canonicalize()
                    .is_ok_and(|path| path == expected)
            })
    }) else {
        return result;
    };
    let tail = &lines[start + 1..];
    let end = tail
        .iter()
        .position(|line| {
            line.starts_with("Latexmk: applying rule ")
                || line.starts_with("Running '")
                || line.starts_with("Latexmk: Getting log file ")
        })
        .unwrap_or(tail.len());
    let driver = &tail[..end];
    let Some(header) = driver
        .iter()
        .rposition(|line| *line == "Latexmk: Found bibliography file(s):")
    else {
        return result;
    };
    let mut index = header + 1;
    let mut found = BTreeSet::new();
    while let Some(name) = driver.get(index).and_then(|line| line.strip_prefix("  ")) {
        if found.len() == 4096 || name.len() > 8192 {
            return result;
        }
        found.insert(name);
        index += 1;
    }
    if found.is_empty() || driver.get(index) != Some(&"Bib file(s) not found in search path:") {
        return result;
    }
    index += 1;
    let mut missing = BTreeSet::new();
    while let Some(name) = driver.get(index).and_then(|line| line.strip_prefix("  ")) {
        let message = format!("Latexmk: Bibliography database not found: {name}");
        if missing.len() == 64
            || name.len() > 8192
            || !found.contains(name)
            || missing_bibliography_database_name(&message).is_none()
        {
            return result;
        }
        missing.insert(name);
        index += 1;
    }
    // A process can stop midway through an otherwise literal filename. A
    // following driver event confirms that the entire list was printed.
    if !tail
        .iter()
        .skip(index)
        .take(16)
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.starts_with("Latexmk:") || line.starts_with("Running '"))
    {
        return result;
    }
    for name in missing {
        let message = format!("Latexmk: Bibliography database not found: {name}");
        emit(
            &mut result,
            root,
            1,
            message.clone(),
            DiagnosticSeverity::WARNING,
        );
        if let Ok(uri) = Url::from_file_path(root) {
            if let Some(entries) = result.get_mut(&uri) {
                for item in entries.iter_mut().filter(|item| item.message == message) {
                    item.code = Some(NumberOrString::String(
                        "missing-bibliography-database".into(),
                    ));
                    item.data = Some(serde_json::json!({"file":name,"unresolvedSource":true}));
                }
            }
        }
    }
    result
}

pub(crate) fn parse_bibliography_in(
    root: &Path,
    paths: &crate::recorder::BibliographyPaths<'_>,
    log: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut result = BTreeMap::new();
    let databases: Vec<_> = log
        .lines()
        .filter_map(crate::recorder::bibliography_database_source)
        .filter_map(|name| paths.resolve(name))
        .collect();
    let mut previous = "";
    for line in log.lines() {
        if line == crate::output::TRUNCATED {
            previous = "Error context omitted from a large bibliography log";
            continue;
        }
        if let Some((message, tail)) = line.split_once("---line ") {
            if let Some((number, name)) = tail.split_once(" of file ") {
                if let Ok(number) = number.trim().parse::<u32>() {
                    // The logged input name identifies its role. A source
                    // alias can resolve to any suffix; an auxiliary alias
                    // must not become a source because its target is .bib.
                    let location = matches!(
                        Path::new(name.trim().trim_matches('"'))
                            .extension()
                            .and_then(|ext| ext.to_str()),
                        Some("tex" | "bib" | "bst" | "sty" | "cls")
                    )
                    .then(|| paths.resolve(name))
                    .flatten();
                    let (path, number) = location
                        .map(|path| (path, number))
                        .unwrap_or_else(|| (root.into(), 1));
                    let message = if message.trim().is_empty() {
                        previous
                    } else {
                        message
                    };
                    let message = format!("BibTeX: {}", message.trim());
                    let missing_file = (Path::new(name.trim().trim_matches('"'))
                        .extension()
                        .and_then(|extension| extension.to_str())
                        == Some("aux"))
                    .then(|| {
                        [
                            MissingBibliographyKind::Style,
                            MissingBibliographyKind::Database,
                        ]
                        .into_iter()
                        .find_map(|kind| kind.name(&message).map(|name| (kind, name)))
                    })
                    .flatten();
                    emit(
                        &mut result,
                        &path,
                        number,
                        message.clone(),
                        DiagnosticSeverity::ERROR,
                    );
                    if let (Some((kind, file)), Ok(uri)) = (missing_file, Url::from_file_path(root))
                    {
                        if let Some(entries) = result.get_mut(&uri) {
                            for entry in entries.iter_mut().filter(|entry| {
                                entry.message == message
                                    && entry.severity == Some(DiagnosticSeverity::ERROR)
                            }) {
                                entry.code = Some(NumberOrString::String(kind.code().into()));
                                entry.data =
                                    Some(serde_json::json!({"file":file,"unresolvedSource":true}));
                            }
                        }
                    }
                    continue;
                }
            }
        }
        if !line.trim().is_empty() {
            previous = line.trim();
        }
        if let Some((message, _)) = line.split_once("---while reading file ") {
            emit(
                &mut result,
                root,
                1,
                format!("BibTeX: {}", message.trim()),
                DiagnosticSeverity::ERROR,
            );
            continue;
        }
        let (message, severity, tool) = if let Some(message) = line.strip_prefix("Warning--") {
            (message, DiagnosticSeverity::WARNING, "BibTeX")
        } else if let Some((_, message)) = line.split_once("ERROR - ") {
            (message, DiagnosticSeverity::ERROR, "Biber")
        } else if let Some((_, message)) = line.split_once("WARN - ") {
            (message, DiagnosticSeverity::WARNING, "Biber")
        } else {
            continue;
        };
        if tool == "Biber" && severity == DiagnosticSeverity::ERROR {
            let message = format!("Biber: {}", message.trim());
            if let Some(file) = missing_bibliography_database_name(&message) {
                emit(&mut result, root, 1, message.clone(), severity);
                if let Ok(uri) = Url::from_file_path(root) {
                    if let Some(item) = result.get_mut(&uri).and_then(|items| items.last_mut()) {
                        item.code = Some(NumberOrString::String(
                            "missing-bibliography-database".into(),
                        ));
                        item.data = Some(serde_json::json!({"file":file,"unresolvedSource":true}));
                    }
                }
                continue;
            }
        }
        let location = bibliography_path(paths, message, tool == "Biber");
        let filename = match &location {
            BibliographyLocation::Named { span, .. } => Some(span),
            BibliographyLocation::Inferred => None,
        };
        let named_entry = bibliography_entry(message, filename);
        let number = if tool == "Biber" {
            bibliography_line(message, filename)
        } else {
            // BibTeX reports source lines in its separate ---line format.
            // Warnings ending in `in key` preserve the entire literal key.
            None
        };
        let (explicit_path, databases, source_confirmed) = match location {
            BibliographyLocation::Inferred => (None, databases.as_slice(), true),
            // A named file restricts entry lookup even when it is missing.
            // Another database cannot supply its source range.
            BibliographyLocation::Named { path, .. } => {
                let confirmed = path.is_some();
                (path, &[][..], confirmed)
            }
        };
        let trailing_entry = message.rsplit_once(" in ").map(|(_, key)| key.trim());
        let key = if tool == "Biber" {
            match named_entry {
                BibliographyEntry::Absent => trailing_entry,
                BibliographyEntry::Key(span) => Some(&message[span]),
                BibliographyEntry::Unconfirmed(_) => None,
            }
        } else {
            // BibTeX appends the literal key, including any quotes in it.
            trailing_entry.or(match named_entry {
                BibliographyEntry::Key(span) => Some(&message[span]),
                _ => None,
            })
        };
        let mut entry = None;
        if let Some(key) = key {
            let candidates: Vec<_> = explicit_path
                .iter()
                .chain(databases.iter())
                .filter_map(|path| {
                    let text = Text::new(std::fs::read_to_string(path).ok()?);
                    let syntax = crate::syntax::Syntax::bibtex(&text);
                    let declaration = syntax.citations.iter().find(|entry| entry.name == key)?;
                    Some((
                        path.clone(),
                        text.range(declaration.span.start, declaration.span.end),
                    ))
                })
                .collect();
            if let Some(first) = candidates
                .first()
                .filter(|first| candidates.iter().all(|candidate| candidate == *first))
            {
                entry = Some(first.clone());
            }
        }
        let path = explicit_path
            .as_deref()
            .or_else(|| entry.as_ref().map(|(path, _)| path.as_path()))
            .unwrap_or(root);
        let diagnostic_message = format!("{tool}: {}", message.trim());
        emit_with_context(
            &mut result,
            path,
            if source_confirmed {
                number.unwrap_or(1)
            } else {
                1
            },
            diagnostic_message,
            severity,
            None,
            source_confirmed,
        );
        if number.is_none() {
            if let Some((_, range)) = &entry {
                if let Ok(uri) = Url::from_file_path(path) {
                    result.get_mut(&uri).unwrap().last_mut().unwrap().range = *range;
                }
            }
        }
    }
    for diagnostics in result.values_mut() {
        diagnostics.sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.message.clone()));
        diagnostics.dedup_by(|a, b| a.range == b.range && a.message == b.message);
    }
    result
}

/// Closing quotes must be separated from the rest of a literal name.
/// Apostrophes joined to more key text, including other quotes, are literal.
fn bibliography_quote_end(message: &str, start: usize) -> Option<usize> {
    let quote = message.as_bytes()[start] as char;
    message[start + 1..]
        .match_indices(quote)
        .map(|(index, _)| start + 1 + index)
        .find(|&index| {
            let tail = &message[index + 1..];
            tail.is_empty()
                || tail.starts_with(char::is_whitespace)
                || tail.starts_with([',', ')', '}'])
                || {
                    let after = tail.trim_start_matches(['.', ':', ';', ']', '!', '?']);
                    after.len() != tail.len()
                        && (after.is_empty() || after.starts_with(char::is_whitespace))
                }
        })
}

/// Metadata is only meaningful outside the complete source field and quotes.
fn bibliography_fields<'a>(
    message: &'a str,
    filename: Option<&'a ByteRange<usize>>,
) -> impl Iterator<Item = (usize, char)> + 'a {
    bibliography_fields_with(message, filename, move |start| {
        bibliography_quote_end(message, start)
    })
}

fn bibliography_fields_with<'a>(
    message: &'a str,
    filename: Option<&'a ByteRange<usize>>,
    quote_end: impl Fn(usize) -> Option<usize> + 'a,
) -> impl Iterator<Item = (usize, char)> + 'a {
    let mut quoted_until = 0;
    message.char_indices().filter(move |&(index, ch)| {
        if index < quoted_until || filename.is_some_and(|span| span.contains(&index)) {
            return false;
        }
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if word_start && matches!(ch, '\'' | '"') {
            quoted_until = quote_end(index).map_or(message.len(), |end| end + 1);
            return false;
        }
        true
    })
}

enum BibliographyEntry {
    Absent,
    Key(ByteRange<usize>),
    Unconfirmed(ByteRange<usize>),
}

fn bibliography_entry(message: &str, filename: Option<&ByteRange<usize>>) -> BibliographyEntry {
    // BibTeX quotes the complete literal key in this standard warning. Match
    // the whole message so ordinary examples cannot redirect diagnostics.
    const TYPE_PREFIX: &str = "entry type for \"";
    const TYPE_SUFFIX: &str = "\" isn't style-file defined";
    if let Some(key) = message
        .strip_prefix(TYPE_PREFIX)
        .and_then(|tail| tail.strip_suffix(TYPE_SUFFIX))
        .filter(|key| !key.is_empty())
    {
        return BibliographyEntry::Key(TYPE_PREFIX.len()..TYPE_PREFIX.len() + key.len());
    }
    const DUPLICATE_PREFIX: &str = "Duplicate entry key: ";
    if message
        .strip_prefix(DUPLICATE_PREFIX)
        .is_some_and(|tail| tail.starts_with(['\'', '"']))
    {
        let start = DUPLICATE_PREFIX.len();
        return match bibliography_quote_end(message, start) {
            Some(end) => BibliographyEntry::Key(start + 1..end),
            None => BibliographyEntry::Unconfirmed(start + 1..message.len()),
        };
    }
    for (index, _) in bibliography_fields(message, filename) {
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if !word_start
            || !(message[index..].starts_with("entry ") || message[index..].starts_with("Entry "))
        {
            continue;
        }
        let start = index + 6;
        if !message[start..].starts_with(['\'', '"']) {
            continue;
        }
        return match bibliography_quote_end(message, start) {
            Some(end) => BibliographyEntry::Key(start + 1..end),
            None => BibliographyEntry::Unconfirmed(start + 1..message.len()),
        };
    }
    BibliographyEntry::Absent
}

fn bibliography_line(message: &str, filename: Option<&ByteRange<usize>>) -> Option<u32> {
    for (index, _) in bibliography_fields(message, filename) {
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if !word_start || !message[index..].starts_with("line ") {
            continue;
        }
        let tail = &message[index + 5..];
        let end = tail
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(tail.len());
        let after = &tail[end..];
        if after.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_')
            || after
                .strip_prefix('.')
                .is_some_and(|tail| tail.starts_with(|ch: char| ch.is_ascii_digit()))
        {
            continue;
        }
        if let Ok(number) = tail[..end].parse() {
            return Some(number);
        }
    }
    None
}

enum BibliographyLocation {
    Inferred,
    Named {
        path: Option<PathBuf>,
        /// Complete source field in the diagnostic, before canonicalization.
        span: ByteRange<usize>,
    },
}

fn bibliography_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    biber: bool,
) -> BibliographyLocation {
    // BibTeX's `in ...` warning suffix is the literal entry key. It can
    // contain quotes, spaces or .bib and can even name an existing file.
    if !biber && message.contains(" in ") {
        return BibliographyLocation::Inferred;
    }
    // In BibTeX warnings, `in 'entry'` names a literal bibliography key.
    let markers: &[&str] = if biber {
        &["file ", "File ", "in ", "In "]
    } else {
        &["file ", "File "]
    };
    let mut unconfirmed = None;
    let explicit = bibliography_source_fields(paths, message).find_map(|(start, _)| {
        if message[..start].ends_with(|ch: char| ch.is_alphanumeric() || matches!(ch, '_' | '-')) {
            return None;
        }
        markers.iter().find_map(|marker| {
            let tail = message[start..].strip_prefix(*marker)?.trim_start();
            let generic = matches!(*marker, "in " | "In ");
            let file = tail
                .strip_prefix("file ")
                .or_else(|| tail.strip_prefix("File "));
            let tail = if generic {
                file.unwrap_or(tail).trim_start()
            } else {
                tail
            };
            let start = message.len() - tail.len();
            let location = if tail.starts_with(['\'', '"']) {
                let QuotedBibliographyPath {
                    path,
                    span,
                    recorded,
                } = bibliography_quoted_path(paths, message, start);
                // A generic quoted word is prose, unless it names a
                // bibliography or belongs to a complete logged source.
                if generic
                    && file.is_none()
                    && path.is_none()
                    && !recorded
                    && bibliography_quote_end(message, start)
                        .is_some_and(|end| !message[start + 1..end].contains(".bib"))
                {
                    return None;
                }
                BibliographyLocation::Named { path, span }
            } else {
                bibliography_explicit_unquoted_path(
                    paths,
                    message,
                    start,
                    biber && (!generic || file.is_some()),
                )?
            };
            // Prose such as `in parser` or `in 'parser'` must not preempt
            // the real file field. Retain an unresolved generic field only
            // if no stronger source follows it.
            if generic
                && file.is_none()
                && matches!(location, BibliographyLocation::Named { path: None, .. })
            {
                unconfirmed.get_or_insert(location);
                None
            } else {
                Some(location)
            }
        })
    });
    if let Some(location) = explicit.or(unconfirmed) {
        return location;
    }
    // A reported entry key is literal metadata even when it names an
    // existing .bib file. Only text outside that field can name a source.
    let entry_span = match bibliography_entry(message, None) {
        BibliographyEntry::Key(span) | BibliographyEntry::Unconfirmed(span) => {
            Some(span.start - 1..(span.end + 1).min(message.len()))
        }
        BibliographyEntry::Absent => None,
    };
    let mut quoted_until = 0;
    for (index, ch) in message.char_indices() {
        if index < quoted_until
            || entry_span
                .as_ref()
                .is_some_and(|span| span.contains(&index))
        {
            continue;
        }
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if word_start && matches!(ch, '\'' | '"') {
            let QuotedBibliographyPath {
                path,
                span,
                recorded,
            } = bibliography_quoted_path(paths, message, index);
            if path.is_some() || recorded {
                return BibliographyLocation::Named { path, span };
            }
            let body = &message[index + 1..span.end];
            let first_quote = bibliography_quote_end(message, index);
            // A plain quoted word cannot consume another quoted database
            // merely because whitespace separates their closing quotes.
            let filename = first_quote.is_none_or(|end| message[index + 1..end].contains(".bib"))
                && body
                    .match_indices(ch)
                    .any(|(end, _)| body[..end].ends_with(".bib"));
            // Preserve uncertain filenames as a whole, but keep independent
            // quoted message strings from hiding a bare path between them.
            quoted_until = if filename {
                span.end
            } else {
                first_quote.map_or(message.len(), |end| end + 1)
            };
            continue;
        }
        if !ch.is_whitespace() && bibliography_token_start(message, index) {
            if let Some(location) = bibliography_recorded_unquoted_path(paths, message, index) {
                return location;
            }
        }
        if !message[index..].starts_with(".bib") {
            continue;
        }
        let prefix = &message[..index];
        let token_start = prefix
            .char_indices()
            .rev()
            .find(|&(_, ch)| ch.is_whitespace() || matches!(ch, ':' | '(' | '[' | '{'))
            .map_or(0, |(index, ch)| index + ch.len_utf8());
        if let Some(location @ BibliographyLocation::Named { path: Some(_), .. }) =
            bibliography_unquoted_path(paths, message, token_start)
        {
            return location;
        }
    }
    BibliographyLocation::Inferred
}

fn bibliography_token_start(message: &str, start: usize) -> bool {
    start == 0
        || message[..start]
            .ends_with(|ch: char| ch.is_whitespace() || matches!(ch, ':' | '(' | '[' | '{'))
}

/// Known bare filenames can themselves contain quotes or apparent fields.
/// Skip their complete spans before looking for explicit source markers.
fn bibliography_source_fields<'a>(
    paths: &'a crate::recorder::BibliographyPaths<'_>,
    message: &'a str,
) -> impl Iterator<Item = (usize, char)> + 'a {
    let mut skip_until = 0;
    message.char_indices().filter(move |&(start, ch)| {
        if start < skip_until {
            return false;
        }
        let word_start = !message[..start].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if word_start && matches!(ch, '\'' | '"') {
            skip_until = bibliography_recorded_quote_end(paths, message, start)
                .map(|(end, _)| end)
                .or_else(|| bibliography_quote_end(message, start))
                .map_or(message.len(), |end| end + 1);
            return false;
        }
        if !ch.is_whitespace() && bibliography_token_start(message, start) {
            if let Some(BibliographyLocation::Named { span, .. }) =
                bibliography_recorded_unquoted_path(paths, message, start)
            {
                skip_until = span.end;
                return false;
            }
        }
        true
    })
}

fn bibliography_unquoted_terminated(tail: &str) -> bool {
    tail.is_empty()
        || tail.starts_with(|ch: char| {
            ch.is_whitespace() || matches!(ch, ',' | ':' | ';' | ')' | ']' | '}')
        })
        || tail
            .strip_prefix('.')
            .is_some_and(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
}

/// Strip balanced message wrappers, preserving unmatched literal openers.
fn bibliography_unquoted_name(body: &str, end: usize) -> (&str, usize) {
    let leading = body[..end]
        .find(|ch| !matches!(ch, '(' | '[' | '{'))
        .unwrap_or(end);
    let mut after = &body[end..];
    let wrapped = body[..leading].chars().rev().all(|open| {
        let close = match open {
            '(' => ')',
            '[' => ']',
            _ => '}',
        };
        if let Some(tail) = after.strip_prefix(close) {
            after = tail;
            true
        } else {
            false
        }
    });
    if wrapped {
        (&body[leading..end], body.len() - after.len())
    } else {
        (&body[..end], end)
    }
}

fn bibliography_recorded_unquoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<BibliographyLocation> {
    let body = &message[start..];
    // Database names are bounded separately from the rest of a tool message.
    let mut scan_end = body.len().min(8192);
    while !body.is_char_boundary(scan_end) {
        scan_end -= 1;
    }
    let scanned = &body[..scan_end];
    let token_end = body.find(char::is_whitespace).unwrap_or(body.len());
    // A recorded short name must not truncate a complete longer bare token.
    let minimum_end = body[..token_end]
        .rmatch_indices(".bib")
        .map(|(index, _)| index + 4)
        .find(|&end| bibliography_unquoted_terminated(&body[end..]))
        .unwrap_or_else(|| {
            body[..token_end]
                .trim_end_matches([',', ':', ';', ')', ']', '}', '.'])
                .len()
        });
    std::iter::once(scan_end)
        .chain(scanned.char_indices().rev().map(|(end, _)| end))
        .filter(|&end| end > 0)
        .find_map(|end| {
            if end < minimum_end || !bibliography_unquoted_terminated(&body[end..]) {
                return None;
            }
            let (name, span_end) = bibliography_unquoted_name(body, end);
            if !paths.could_name_database(&body[..end]) && !paths.could_name_database(name) {
                return None;
            }
            // A literal opening bracket in a logged name takes precedence over
            // treating it as a wrapper supplied by the diagnostic message.
            let (name, span_end, source) = paths
                .recorded_bibliography_source(&body[..end])
                .map(|source| (&body[..end], end, source))
                .or_else(|| {
                    paths
                        .recorded_bibliography_source(name)
                        .map(|source| (name, span_end, source))
                })?;
            let path = if source.selected {
                source.path
            } else {
                source.path.or_else(|| paths.resolve(name))
            };
            Some(BibliographyLocation::Named {
                path,
                span: start..start + span_end,
            })
        })
}

fn bibliography_unquoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<BibliographyLocation> {
    if let Some(location) = bibliography_recorded_unquoted_path(paths, message, start) {
        return Some(location);
    }
    let body = &message[start..];
    let token_end = body.find(char::is_whitespace).unwrap_or(body.len());
    // A whole filename takes precedence over internal .bib fragments or
    // punctuation. A missing explicit source still restricts entry lookup.
    let mut extensions = body[..token_end].rmatch_indices(".bib").peekable();
    extensions.peek()?;
    let end = extensions.find_map(|(index, _)| {
        let end = index + 4;
        bibliography_unquoted_terminated(&body[end..]).then_some(end)
    });
    let (path, end) = match end {
        Some(end) => {
            let (name, span_end) = bibliography_unquoted_name(body, end);
            (paths.resolve(name), span_end)
        }
        None => (None, token_end),
    };
    Some(BibliographyLocation::Named {
        path,
        span: start..start + end,
    })
}

/// An explicit file field restricts lookup even without an input record.
/// Its complete name can contain spaces; quoted error text is not a boundary.
fn bibliography_explicit_unquoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
    explicit_file: bool,
) -> Option<BibliographyLocation> {
    let recorded = bibliography_recorded_unquoted_path(paths, message, start);
    let body = &message[start..];
    let last_extension = bibliography_fields(body, None)
        .filter_map(|(index, _)| body[index..].starts_with(".bib").then_some(index + 4))
        .last();
    if let Some(BibliographyLocation::Named { path, span }) = recorded {
        // Keep the selected input's identity, including disappearance. A
        // complete longer explicit name can still override a logged prefix.
        if last_extension.is_none_or(|end| start + end <= span.end) {
            return Some(BibliographyLocation::Named { path, span });
        }
    }
    let Some(end) = last_extension else {
        return bibliography_unquoted_path(paths, message, start).or_else(|| {
            // An explicit unknown file must not borrow another database's
            // entry. A generic `in parser` remains ordinary message prose.
            explicit_file.then(|| BibliographyLocation::Named {
                path: None,
                span: start..start + body.find(char::is_whitespace).unwrap_or(body.len()),
            })
        });
    };
    let (path, end) = if bibliography_unquoted_terminated(&body[end..]) {
        let (name, span_end) = bibliography_unquoted_name(body, end);
        (paths.resolve(name), span_end)
    } else {
        // A longer incomplete/unsupported name cannot borrow its existing
        // .bib prefix, nor a quoted filename elsewhere in the message.
        let end = body[end..]
            .find(char::is_whitespace)
            .map_or(body.len(), |offset| end + offset);
        (None, end)
    };
    Some(BibliographyLocation::Named {
        path,
        span: start..start + end,
    })
}

fn bibliography_quote_terminated(tail: &str) -> bool {
    let after = tail.trim_start_matches([',', ':', ';', ')', ']', '}', '.']);
    tail.is_empty()
        || (after.len() != tail.len()
            && (after.is_empty() || after.starts_with(char::is_whitespace)))
}

/// Logged names disambiguate literal quotes even when punctuation and spaces
/// follow them. Match the longest complete source across equivalent paths;
/// shorter existing files cannot replace a missing recorded source.
fn bibliography_recorded_quote_end(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<(usize, crate::recorder::RecordedBibliographySource)> {
    let quote = message.as_bytes()[start] as char;
    let body = &message[start + 1..];
    body.rmatch_indices(quote).find_map(|(end, _)| {
        let name = &body[..end];
        let tail = &body[end + 1..];
        if !tail.starts_with(char::is_whitespace) && !bibliography_quote_terminated(tail) {
            return None;
        }
        paths
            .recorded_bibliography_source(name)
            .map(|source| (start + 1 + end, source))
    })
}

struct QuotedBibliographyPath {
    path: Option<PathBuf>,
    span: ByteRange<usize>,
    recorded: bool,
}

fn bibliography_quoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> QuotedBibliographyPath {
    let parsed = bibliography_name_quote_end(message, start);
    let recorded = bibliography_recorded_quote_end(paths, message, start).filter(|(end, _)| {
        // A recorded prefix cannot truncate an otherwise complete longer
        // filename, including a source missing from this tool's input list.
        !parsed.is_some_and(|parsed| {
            parsed > *end
                && (message[start + 1..parsed].ends_with(".bib")
                    || !message[start + 1..*end].ends_with(".bib"))
        })
    });
    let is_recorded = recorded.is_some();
    let (end, path) = if let Some((end, source)) = recorded {
        // A complete explicit file can be outside the input list. A source
        // actually selected by this context must never switch after deletion.
        let path = if source.selected {
            source.path
        } else {
            source
                .path
                .or_else(|| paths.resolve(&message[start + 1..end]))
        };
        (Some(end), path)
    } else {
        (
            parsed,
            parsed.and_then(|end| {
                let name = &message[start + 1..end];
                name.ends_with(".bib")
                    .then(|| paths.resolve(name))
                    .flatten()
            }),
        )
    };
    QuotedBibliographyPath {
        path,
        span: start..end.map_or(message.len(), |end| end + 1),
        recorded: is_recorded,
    }
}

fn bibliography_name_quote_end(message: &str, start: usize) -> Option<usize> {
    let quote = message.as_bytes()[start] as char;
    let body = &message[start + 1..];
    let mut candidate = None;
    let mut ambiguous = false;
    for (end, _) in body.match_indices(quote) {
        let tail = &body[end + 1..];
        // Message punctuation must itself end at a separator. For example,
        // the quote in refs.bib':copy.bib is still part of the filename.
        // A whitespace-only boundary is tentative unless it introduces a
        // complete line field: `line 700 copy.bib` is still name text.
        let line = tail.trim_start().strip_prefix("line ").is_some_and(|tail| {
            let end = tail
                .find(|ch: char| !ch.is_ascii_digit())
                .unwrap_or(tail.len());
            end > 0 && bibliography_quote_terminated(tail[end..].trim_start())
        });
        let strong = bibliography_quote_terminated(tail) || line;
        if strong {
            return Some(start + 1 + end);
        }
        if tail.starts_with(char::is_whitespace) {
            ambiguous |= candidate.is_some();
            candidate = Some(start + 1 + end);
        }
    }
    if ambiguous {
        None
    } else {
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_sources_track_closed_nested_and_wrapped_file_entries_not_echoed_messages() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let long = format!("章节-{}.tex", "segment".repeat(20));
        for name in [
            "main.tex",
            "one.tex",
            "two.tex",
            "ignored.tex",
            "package.sty",
            "main.aux",
            long.as_str(),
        ] {
            std::fs::write(root.join(name), "Text\n").unwrap();
        }
        let entry = format!("(\"./{long}\")");
        let split = entry
            .char_indices()
            .map(|(at, _)| at)
            .find(|at| *at >= 79)
            .unwrap();
        let log = format!("(./main.tex\n(./one.tex)(./two.tex)\n(./package.sty) (./main.aux)\n{}\n{}\nLaTeX Warning: Do not read (./ignored.tex).\nl.3 (./ignored.tex)\n)\n", &entry[..split], &entry[split..]);
        let (_, sources) = parse_with_sources(&main, &log);
        let expected: BTreeSet<_> = [
            "main.tex",
            "one.tex",
            "two.tex",
            "package.sty",
            long.as_str(),
        ]
        .into_iter()
        .map(|name| Url::from_file_path(root.join(name)).unwrap())
        .collect();
        assert_eq!(sources, expected);
    }

    #[test]
    fn truncated_logs_keep_only_the_root_and_source_entries_actually_observed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        for name in ["main.tex", "observed.tex", "not-seen.tex"] {
            std::fs::write(root.join(name), "Text\n").unwrap();
        }
        let log = format!(
            "(./not-se\n{}\n(./observed.tex)\n",
            crate::output::TRUNCATED
        );
        let (_, sources) = parse_with_sources(&main, &log);
        assert_eq!(
            sources,
            [
                Url::from_file_path(&main).unwrap(),
                Url::from_file_path(root.join("observed.tex")).unwrap()
            ]
            .into()
        );
    }

    #[test]
    fn duplicate_destination_warnings_select_declarations_and_preserve_shipout_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\hypertarget{section.1}{Target}\\label{section.1}\\hyperlink{section.1}{Link}\r\n\\hyperdef[reference-label]{section}% keep\r\n{1}{Target}\r\n";
        std::fs::write(&main, source).unwrap();
        let doc = crate::workspace::Document::new(
            &Url::from_file_path(&main).unwrap(),
            source.into(),
            None,
        );
        for message in [
            "pdfTeX warning (ext4): destination with the same identifier (name{section.1}) has been already used, duplicate ignored",
            "warning  (pdf backend): ignoring duplicate destination with the name 'section.1'",
        ] {
            let log = format!("{message}\n<argument> ...shipout:D \\box_use:N \\l_shipout_box\nl.4 \\end{{document}}\nLaTeX Warning: Keep.\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            let duplicates: Vec<_> = items.iter().filter(|item| item.code == Some(NumberOrString::String("duplicate-pdf-destination".into()))).collect();
            assert_eq!(duplicates.len(), 2, "{items:?}");
            assert_eq!(items.len(), 3, "{items:?}");
            for (item, entry) in duplicates.iter().zip(&doc.syntax.anchors) {
                assert_eq!(item.range, doc.text.range(entry.span.start, entry.span.end));
                assert_eq!(item.message, message);
                assert_eq!(item.data, Some(serde_json::json!({"destination":"section.1"})));
            }
        }
    }

    #[test]
    fn duplicate_destination_messages_recover_byte_wraps_without_normalizing_name_spaces() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}", "long".repeat(25));
        std::fs::write(&main, format!("\\hypertarget{{{name}}}{{Target}}\n")).unwrap();
        for (prefix, suffix) in [
            (
                "pdfTeX warning (ext4): destination with the same identifier (name{",
                "}) has been already used, duplicate ignored",
            ),
            (
                "warning  (pdf backend): ignoring duplicate destination with the name '",
                "'",
            ),
        ] {
            let message = format!("{prefix}{name}{suffix}");
            for split in prefix.len()..message.len() {
                let log = format!(
                    "{}\n{}\n<argument> shipout\nl.1 text\n",
                    &message[..split],
                    &message[split..]
                );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 1, "split {split}: {items:?}");
                assert_eq!(items[0].message, message, "split {split}");
                assert_eq!(
                    items[0].data,
                    Some(serde_json::json!({"destination":name})),
                    "split {split}"
                );
            }
            let spaced = format!("{prefix} {name}{suffix}");
            assert!(destination_warning(&spaced).is_none());
        }
        for message in [
            "pdfTeX warning (ext4): destination with the same identifier (name{target}) has been already used",
            "warning  (pdf backend): ignoring duplicate destination with the name '\\dynamic'",
        ] {
            assert!(destination_warning(message).is_none());
        }
    }

    #[test]
    fn destination_warning_byte_wraps_preserve_names_and_stop_at_resource_output() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}.!{}", "long".repeat(25), "tail".repeat(15));
        let source = format!("\\hyperlink{{{name}}}{{Text}}\n");
        std::fs::write(&main, &source).unwrap();
        for (prefix, suffix) in [
            (
                "pdfTeX warning (dest): name{",
                "} has been referenced but does not exist, replaced by a fixed one",
            ),
            (
                "warning  (pdf backend): unreferenced destination with name '",
                "'",
            ),
        ] {
            let message = format!("{prefix}{name}{suffix}");
            // Include breaks immediately after punctuation and inside the
            // explanatory words, not just at convenient token boundaries.
            for split in prefix.len()..message.len() {
                let log = format!(
                    "{}\n{}\n</fonts/resource.otf>\nLaTeX Warning: Next warning.\n",
                    &message[..split],
                    &message[split..]
                );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2, "split {split}: {items:?}");
                let item = items
                    .iter()
                    .find(|item| item.message != "LaTeX Warning: Next warning.")
                    .unwrap();
                assert_eq!(item.message, message, "split {split}");
                assert_eq!(
                    item.data,
                    Some(serde_json::json!({"destination":name})),
                    "split {split}"
                );
                assert_eq!(
                    item.range,
                    Text::new(source.clone()).range(11, 11 + name.len())
                );
            }
        }
    }

    #[test]
    fn destination_wraps_preserve_real_whitespace_and_incomplete_message_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "\\hyperlink{section.target}{Text}\n").unwrap();
        for message in [
            "pdfTeX warning (dest): name{section. target} has been referenced but does not exist, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name 'section. target'",
        ] {
            let split = message.find(" target").unwrap();
            let log = format!("{}\n{}\n</fonts/resource.otf>\n", &message[..split], &message[split..]);
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "{items:?}");
            assert_eq!(items[0].message, message);
            assert!(items[0].code.is_none());
            assert!(items[0].data.is_none());
        }
        for boundary in [
            "</fonts/resource.otf>",
            "LaTeX Warning: Next warning.",
            "! Next error.",
            crate::output::TRUNCATED,
            "(./main.tex",
        ] {
            let log = format!("pdfTeX warning (dest): name{{section.\n{boundary}\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            let item = items
                .iter()
                .find(|item| item.message.starts_with("pdfTeX warning"))
                .unwrap();
            assert_eq!(item.message, "pdfTeX warning (dest): name{section.");
            assert!(item.code.is_none());
        }
    }

    #[test]
    fn unicode_destinations_survive_log_breaks_inside_utf8_characters() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}终点", "章节".repeat(20));
        std::fs::write(&main, format!("\\hyperlink{{{name}}}{{Text}}\n")).unwrap();
        let message = format!("pdfTeX warning (dest): name{{{name}}} has been referenced but does not exist, replaced by a fixed one");
        for split in "pdfTeX warning (dest): name{".len()..message.len() {
            let log = [
                &message.as_bytes()[..split],
                b"\n",
                &message.as_bytes()[split..],
                b"\n</fonts/resource.pfb>\n",
            ]
            .concat();
            let diagnostics = parse(&main, &decode_output(&log));
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "split {split}: {items:?}");
            assert_eq!(
                items[0].data,
                Some(serde_json::json!({"destination":name})),
                "split {split}: {items:?}"
            );
        }
    }

    #[test]
    fn lua_destination_apostrophes_at_print_width_do_not_target_a_different_shorter_name() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let prefix = "warning  (pdf backend): unreferenced destination with name '";
        for width in [79, 80] {
            let shorter = "x".repeat(width - prefix.len() - 1);
            let name = format!("{shorter}'suffix");
            std::fs::write(
                &main,
                format!("\\hyperlink{{{name}}}{{Long}}\\hyperlink{{{shorter}}}{{Short}}\n"),
            )
            .unwrap();
            let message = format!("{prefix}{name}'");
            let log = format!(
                "{}\n{}\n</fonts/resource.otf>\n",
                &message[..width],
                &message[width..]
            );
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "{items:?}");
            assert_eq!(items[0].message, message);
            assert_eq!(items[0].data, Some(serde_json::json!({"destination":name})));
        }
    }

    #[test]
    fn destination_warnings_locate_direct_and_compound_links_without_inventing_lines() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\hyperlink{section.lost}{Text}\\label{section.lost}\r\n\\hyperref{}{section}% keep\r\n{lost}{Link}\r\n\\hyperref{remote}{section}{lost}{Remote}\n\\verb|\\hyperlink{section.lost}{Ignored}|\n";
        std::fs::write(&main, source).unwrap();
        let text = Text::new(source.into());
        let first = source.find("section.lost").unwrap();
        let second = source.find("{section}").unwrap() + 1;
        let last = source.find("{lost}").unwrap() + 5;
        for warning in [
            "pdfTeX warning (dest): name{section.lost} has been referenced but does not exis\nt, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name 'section.lost'",
        ] {
            let log = format!("(./main.tex\n{warning}\n<font/resource.otf>\n! Other error.\nl.1 bad\n)\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 3, "{items:?}");
            let references: Vec<_> = items.iter().filter(|item| item.code == Some(NumberOrString::String("undefined-pdf-destination".into()))).collect();
            assert_eq!(references.len(), 2, "{items:?}");
            assert_eq!(references[0].range, text.range(first, first + 12));
            assert_eq!(references[1].range, text.range(second, last));
            assert!(references.iter().all(|item| item.data == Some(serde_json::json!({"destination":"section.lost"})) && !item.message.contains("font/")));
        }
    }

    #[test]
    fn backend_destination_string_escapes_map_exact_keys_across_all_byte_wraps() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        for name in ["dest(old)", "dest(old", "dest)old"] {
            let source = format!("😀 \\hypertarget{{{name}}}{{Target}}\\label{{{name}}}\\hyperlink{{{name}}}{{Link}}\n");
            std::fs::write(&main, &source).unwrap();
            let text = Text::new(source.clone());
            let encoded = name.replace('(', "\\(").replace(')', "\\)");
            for (prefix, suffix, code, offset) in [
                (
                    "pdfTeX warning (dest): name{",
                    "} has been referenced but does not exist, replaced by a fixed one",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "pdfTeX warning (ext4): destination with the same identifier (name{",
                    "}) has been already used, duplicate ignored",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): unreferenced destination with name '",
                    "'",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): ignoring duplicate destination with the name '",
                    "'",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
            ] {
                let message = format!("{prefix}{encoded}{suffix}");
                assert_eq!(
                    destination_warning(&message).map(|(_, key)| key),
                    Some(name.to_owned())
                );
                for split in prefix.len()..message.len() {
                    let log = format!(
                        "{}\n{}\n</fonts/resource.otf>\nLaTeX Warning: Keep.\n",
                        &message[..split],
                        &message[split..]
                    );
                    let diagnostics = parse(&main, &log);
                    let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                    assert_eq!(items.len(), 2, "{name}: split {split}: {items:?}");
                    let mapped: Vec<_> = items
                        .iter()
                        .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                        .collect();
                    assert_eq!(mapped.len(), 1, "{name}: split {split}: {items:?}");
                    assert_eq!(mapped[0].range, text.range(offset, offset + name.len()));
                    assert_eq!(
                        mapped[0].data,
                        Some(serde_json::json!({"destination":name}))
                    );
                    assert_eq!(mapped[0].message, message);
                }
            }
        }
        for encoded in [r"dest\", r"dest\dynamic", r"dest\\(old)", r"dest\( old\)"] {
            for message in [
                format!("pdfTeX warning (dest): name{{{encoded}}} has been referenced but does not exist, replaced by a fixed one"),
                format!("pdfTeX warning (ext4): destination with the same identifier (name{{{encoded}}}) has been already used, duplicate ignored"),
                format!("warning  (pdf backend): unreferenced destination with name '{encoded}'"),
                format!("warning  (pdf backend): ignoring duplicate destination with the name '{encoded}'"),
            ] {
                assert!(destination_warning(&message).is_none(), "{message}");
            }
        }
    }

    #[test]
    fn backend_octal_destination_bytes_preserve_unicode_and_reject_invalid_encodings() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        for (name, encoded) in [
            (
                "章节.目标",
                r"\347\253\240\350\212\202.\347\233\256\346\240\207",
            ),
            ("emoji😀", r"emoji\360\237\230\200"),
            (
                "mix(目标😀)",
                r"mix\(\347\233\256\346\240\207\360\237\230\200\)",
            ),
            ("dest(old)", r"dest\050old\051"),
        ] {
            let source = format!("😀 \\hypertarget{{{name}}}{{Target}}\\label{{{name}}}\\hyperlink{{{name}}}{{Link}}\r\n");
            std::fs::write(&main, &source).unwrap();
            let text = Text::new(source.clone());
            for (prefix, suffix, code, offset) in [
                (
                    "pdfTeX warning (dest): name{",
                    "} has been referenced but does not exist, replaced by a fixed one",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "pdfTeX warning (ext4): destination with the same identifier (name{",
                    "}) has been already used, duplicate ignored",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): unreferenced destination with name '",
                    "'",
                    "undefined-pdf-destination",
                    source.rfind(name).unwrap(),
                ),
                (
                    "warning  (pdf backend): ignoring duplicate destination with the name '",
                    "'",
                    "duplicate-pdf-destination",
                    source.find(name).unwrap(),
                ),
            ] {
                let message = format!("{prefix}{encoded}{suffix}");
                assert_eq!(
                    destination_warning(&message).map(|(_, key)| key),
                    Some(name.to_owned())
                );
                for split in prefix.len()..message.len() {
                    let log = format!(
                        "{}\n{}\n</fonts/resource.otf>\nLaTeX Warning: Keep.\n",
                        &message[..split],
                        &message[split..]
                    );
                    let diagnostics = parse(&main, &log);
                    let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                    assert_eq!(items.len(), 2, "{name}: split {split}: {items:?}");
                    let mapped: Vec<_> = items
                        .iter()
                        .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                        .collect();
                    assert_eq!(mapped.len(), 1, "{name}: split {split}: {items:?}");
                    assert_eq!(mapped[0].range, text.range(offset, offset + name.len()));
                    assert_eq!(
                        mapped[0].data,
                        Some(serde_json::json!({"destination":name}))
                    );
                    assert_eq!(mapped[0].message, message);
                }
            }
        }
        for encoded in [
            r"dest\4",
            r"dest\40",
            r"dest\400",
            r"dest\777",
            r"dest\08a",
            r"dest\999",
            r"dest\300\257",
            r"dest\355\240\200",
            r"dest\377",
            r"dest\000",
            r"dest\012",
            r"dest\177",
            r"dest\302\237",
            r"dest\134",
            r"dest\173",
            r"dest\175",
            r"dest\045",
            r"dest\043",
            r"dest\040",
        ] {
            for message in [
                format!("pdfTeX warning (dest): name{{{encoded}}} has been referenced but does not exist, replaced by a fixed one"),
                format!("pdfTeX warning (ext4): destination with the same identifier (name{{{encoded}}}) has been already used, duplicate ignored"),
                format!("warning  (pdf backend): unreferenced destination with name '{encoded}'"),
                format!("warning  (pdf backend): ignoring duplicate destination with the name '{encoded}'"),
            ] {
                assert!(destination_warning(&message).is_none(), "{message}");
            }
        }
    }

    #[test]
    fn raw_destination_parentheses_at_print_width_keep_complete_names_and_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        for (prefix, code, declaration) in [
            (
                "warning  (pdf backend): unreferenced destination with name '",
                "undefined-pdf-destination",
                false,
            ),
            (
                "warning  (pdf backend): ignoring duplicate destination with the name '",
                "duplicate-pdf-destination",
                true,
            ),
        ] {
            for suffix in ["(paired)", "(unpaired", ")unpaired", "(目标😀)"] {
                let name = format!("{}{suffix}", "x".repeat(80 - prefix.len()));
                let source = format!("😀 \\hypertarget{{{name}}}{{Target}}\\label{{{name}}}\\hyperlink{{{name}}}{{Link}}\r\n");
                std::fs::write(&main, &source).unwrap();
                let text = Text::new(source.clone());
                let at = if declaration {
                    source.find(&name)
                } else {
                    source.rfind(&name)
                }
                .unwrap();
                let message = format!("{prefix}{name}'");
                let mut log = String::new();
                let mut start = 0;
                while start < message.len() {
                    let mut end = (start + 80).min(message.len());
                    while !message.is_char_boundary(end) {
                        end -= 1;
                    }
                    log.push_str(&message[start..end]);
                    log.push('\n');
                    start = end;
                }
                log.push_str("</fonts/resource.otf>\nLaTeX Warning: Keep.\n");
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2, "{message}: {items:?}");
                let mapped: Vec<_> = items
                    .iter()
                    .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                    .collect();
                assert_eq!(mapped.len(), 1, "{message}: {items:?}");
                assert_eq!(mapped[0].range, text.range(at, at + name.len()));
                assert_eq!(
                    mapped[0].data,
                    Some(serde_json::json!({"destination":name}))
                );
                assert_eq!(mapped[0].message, message);
                assert!(items
                    .iter()
                    .any(|item| item.message == "LaTeX Warning: Keep."));
            }
        }
    }

    #[test]
    fn incomplete_raw_destination_wraps_preserve_file_stack_and_following_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let child = directory.path().join("child.tex");
        std::fs::write(&main, "Main source\n").unwrap();
        std::fs::write(&child, "Child source\n").unwrap();
        for prefix in [
            "warning  (pdf backend): unreferenced destination with name '",
            "warning  (pdf backend): ignoring duplicate destination with the name '",
        ] {
            let partial = format!("{prefix}{}", "x".repeat(80 - prefix.len()));
            for log in [
                format!("{partial}\n(./child.tex\nLaTeX Warning: Child sentinel.\n)\nLaTeX Warning: Main sentinel.\n"),
                format!("(./child.tex\n{partial}\n)\nLaTeX Warning: Main sentinel.\n"),
                format!("{partial}\n</fonts/resource.otf>\nchild.tex:1: LaTeX Error: Child sentinel.\nLaTeX Warning: Main sentinel.\n"),
            ] {
                let diagnostics = parse(&main, &log);
                let all: Vec<_> = diagnostics.values().flatten().collect();
                assert!(all.iter().all(|item| item.code.is_none() && item.data.is_none()), "{log}: {diagnostics:?}");
                assert_eq!(all.iter().filter(|item| item.message == partial).count(), 1, "{log}: {diagnostics:?}");
                let main_items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert!(main_items.iter().any(|item| item.message == "LaTeX Warning: Main sentinel."), "{log}: {diagnostics:?}");
                let child_items = &diagnostics[&Url::from_file_path(&child).unwrap()];
                assert!(child_items.iter().any(|item| item.message.contains("Child sentinel.") || item.message == partial), "{log}: {diagnostics:?}");
            }
        }
    }

    #[test]
    fn lua_unicode_early_wraps_do_not_diagnose_a_healthy_short_prefix() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let missing = "warning  (pdf backend): unreferenced destination with name '";
        let duplicate = "warning  (pdf backend): ignoring duplicate destination with the name '";
        // Widths observed in successful native LuaTeX compilations, where the
        // next whole Unicode character no longer fits on the physical line.
        let cases: [(&str, char, &[usize], bool); 6] = [
            (missing, 'é', &[77, 78], false),
            (missing, '目', &[76, 77, 78], false),
            (missing, '😀', &[75, 76, 77, 78], false),
            (duplicate, 'é', &[76, 77, 78], true),
            (duplicate, '目', &[75, 76, 77, 78], true),
            (duplicate, '😀', &[75, 76, 77, 78], true),
        ];
        for (prefix, character, widths, declaration) in cases {
            for width in widths {
                let short = "x".repeat(width - prefix.len() - 1);
                let name = format!("{short}'{character}tail");
                let source = if declaration {
                    format!("😀 \\hypertarget{{{short}}}{{Healthy}}\\hypertarget{{{name}}}{{One}}\\hypertarget{{{name}}}{{Two}}\\label{{{name}}}")
                } else {
                    format!("😀 \\hypertarget{{{short}}}{{Healthy}}\\hyperlink{{{short}}}{{Healthy}}\\hyperlink{{{name}}}{{Missing}}\\label{{{name}}}")
                };
                std::fs::write(&main, &source).unwrap();
                let text = Text::new(source.clone());
                let message = format!("{prefix}{name}'");
                let log = format!(
                "{prefix}{short}'\n{character}tail'\n</fonts/resource.otf>\nLaTeX Warning: Keep.\n"
            );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                let code = if declaration {
                    "duplicate-pdf-destination"
                } else {
                    "undefined-pdf-destination"
                };
                let mapped: Vec<_> = items
                    .iter()
                    .filter(|item| item.code == Some(NumberOrString::String(code.into())))
                    .collect();
                let count = if declaration { 2 } else { 1 };
                assert_eq!(items.len(), count + 1, "{message}: {items:?}");
                assert_eq!(mapped.len(), count, "{message}: {items:?}");
                for (item, (at, _)) in mapped.iter().zip(source.match_indices(&name).take(count)) {
                    assert_eq!(item.range, text.range(at, at + name.len()));
                    assert_eq!(item.data, Some(serde_json::json!({"destination":name})));
                    assert_eq!(item.message, message);
                }
                assert!(items
                    .iter()
                    .any(|item| item.message == "LaTeX Warning: Keep."));
            }
        }
    }

    #[test]
    fn unicode_lookahead_after_a_complete_destination_keeps_independent_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let child = directory.path().join("文件.tex");
        std::fs::write(&child, "Child source\n").unwrap();
        let prefix = "warning  (pdf backend): unreferenced destination with name '";
        for width in [74, 75, 78] {
            let name = "x".repeat(width - prefix.len() - 1);
            let source = format!("😀 \\hyperlink{{{name}}}{{Link}}\n");
            std::fs::write(&main, &source).unwrap();
            let text = Text::new(source.clone());
            let at = source.find(&name).unwrap();
            let message = format!("{prefix}{name}'");
            for (next, child_error) in [
                ("文件.tex:1: LaTeX Error: Unicode sentinel.", true),
                ("正文 without a closing apostrophe", false),
            ] {
                let log = format!("{message}\n{next}\nLaTeX Warning: Keep.\n");
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2, "{log}: {diagnostics:?}");
                let mapped: Vec<_> = items
                    .iter()
                    .filter(|item| {
                        item.code
                            == Some(NumberOrString::String("undefined-pdf-destination".into()))
                    })
                    .collect();
                assert_eq!(mapped.len(), 1, "{log}: {diagnostics:?}");
                assert_eq!(mapped[0].range, text.range(at, at + name.len()));
                assert_eq!(mapped[0].message, message);
                assert_eq!(
                    mapped[0].data,
                    Some(serde_json::json!({"destination":name}))
                );
                assert!(items
                    .iter()
                    .any(|item| item.message == "LaTeX Warning: Keep."));
                let children = diagnostics
                    .get(&Url::from_file_path(&child).unwrap())
                    .map_or(&[][..], Vec::as_slice);
                assert_eq!(
                    children.len(),
                    usize::from(child_error),
                    "{log}: {diagnostics:?}"
                );
                if child_error {
                    assert_eq!(children[0].message, "LaTeX Error: Unicode sentinel.");
                }
            }
        }
    }

    #[test]
    fn destination_warning_names_preserve_semantic_spaces_and_reject_uncertain_messages() {
        for name in [
            "lost",
            "section.foo",
            "章节.目标",
            "dest(old)",
            "dest(old",
            "dest)old",
        ] {
            for message in [format!("pdfTeX warning (dest): name{{{name}}} has been referenced but does not exist , replaced by a fixed one"), format!("warning  (pdf backend): unreferenced destination with name '{name}'")] {
                assert_eq!(undefined_destination_name(&message).as_deref(), Some(name));
            }
        }
        for message in [
            "pdfTeX warning (dest): name{a b} has been referenced but does not exist, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name ' lost'",
            "warning  (pdf backend): unreferenced destination with name 'lost ' ",
            "pdfTeX warning (dest): name{lost} has been referenced",
            "warning  (pdf backend): unreferenced destination with name '\\dynamic'",
            "pdfTeX warning (dest): name{lost} has been referenced but was found",
        ] {
            assert_eq!(undefined_destination_name(message), None, "{message}");
        }
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First line\n\\hyperlink{\\dynamic}{Text}\n").unwrap();
        let diagnostics = parse(&main, "warning  (pdf backend): unreferenced destination with name 'unknown'\n<font/resource.otf>\n");
        let items = &diagnostics[&Url::from_file_path(main).unwrap()];
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none());
        assert!(items[0].data.is_none());
        assert!(!items[0].message.contains("font/"));
    }

    #[test]
    fn final_pdf_page_counts_support_engine_variants_and_wrapped_summaries() {
        for (log, expected) in [
            ("Output written on main.pdf (1 page, 123 bytes).\n", Some(1)),
            ("Output written on main.pdf (12 pages).\n", Some(12)),
            ("Output written on /long/path (draft)/\nmain.pdf (2 pa\nges, 3456 bytes).\n", Some(2)),
            ("Output written on main.pdf (3 pages, 123 bytes).\nOutput written on main.pdf (4 pages, 456 bytes).\n", Some(4)),
            ("Writing main.pdf\n", None),
            ("Output written on main.pdf (0 pages, 0 bytes).\n", None),
            ("Output written on main.pdf (999999999999 pages, 9 bytes).\n", None),
            ("Output written on main.pdf (2 pages, invalid bytes).\n", None),
        ] {
            assert_eq!(page_count(log), expected, "{log}");
        }
    }

    #[test]
    fn tex_byte_wrapping_recovers_split_utf8_without_joining_valid_or_malformed_lines() {
        for scalar in ["é", "中", "😀"] {
            for split in 1..scalar.len() {
                for newline in [b"\n".as_slice(), b"\r\n".as_slice()] {
                    let bytes = [
                        b"No file dir/",
                        &scalar.as_bytes()[..split],
                        newline,
                        &scalar.as_bytes()[split..],
                        b".tex.\n",
                    ]
                    .concat();
                    assert_eq!(
                        decode_output(&bytes),
                        format!("No file dir/{scalar}.tex.\n")
                    );
                }
            }
        }
        assert_eq!(decode_output("中文\n😀\n".as_bytes()), "中文\n😀\n");
        for bytes in [
            b"\xff\n\x80".as_slice(),
            b"\xe0\x80\n\x80".as_slice(),
            b"\xe4\nA".as_slice(),
            b"\xe4\n\n\xb8\xad".as_slice(),
        ] {
            assert_eq!(decode_output(bytes), String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_line_errors_follow_source_aliases_with_unrelated_target_extensions() {
        for extension in ["payload", "png", "tex", "bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let database = root.join(format!("database.{extension}"));
            let style = root.join(format!("style.{extension}"));
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(
                &database,
                "% database\n@book{entry,title={Book}}\n@book{entry,title={Duplicate}}\n",
            )
            .unwrap();
            std::fs::write(&style, "ENTRY {title} {} {}\nFUNCTION {broken} {unknown}\n").unwrap();
            std::os::unix::fs::symlink(&database, root.join("with spaces.bib")).unwrap();
            std::os::unix::fs::symlink(&style, root.join("local.bst")).unwrap();
            let diagnostics = parse_bibliography(&main, "Database file #1: with spaces.bib\nThe style file: local.bst\nRepeated entry---line 3 of file \"with spaces.bib\"\nunknown is an unknown function---line 2 of file local.bst\nWarning--empty author in entry\n");
            let database_uri = Url::from_file_path(&database).unwrap();
            let style_uri = Url::from_file_path(&style).unwrap();
            assert_eq!(diagnostics.len(), 2, "{extension}: {diagnostics:?}");
            let errors = &diagnostics[&database_uri];
            assert_eq!(errors.len(), 2);
            assert!(errors
                .iter()
                .any(|item| item.message == "BibTeX: Repeated entry"
                    && item.range.start.line == 2
                    && item.severity == Some(DiagnosticSeverity::ERROR)));
            assert!(errors
                .iter()
                .any(|item| item.message == "BibTeX: empty author in entry"
                    && item.range.start == Position::new(1, 6)
                    && item.severity == Some(DiagnosticSeverity::WARNING)));
            assert_eq!(diagnostics[&style_uri][0].range.start.line, 1);
            assert_eq!(
                diagnostics[&style_uri][0].message,
                "BibTeX: unknown is an unknown function"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_line_errors_do_not_treat_auxiliary_alias_targets_as_sources() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let physical = root.join("physical.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&physical, "% database\n@book{entry,\n title={Book}}\n").unwrap();
        for name in ["main.aux", "raw.csv"] {
            std::os::unix::fs::symlink(&physical, root.join(name)).unwrap();
        }
        std::fs::create_dir(root.join("directory.bst")).unwrap();
        for name in ["main.aux", "raw.csv", "missing.bib", "directory.bst"] {
            let log = format!("Error in auxiliary input---line 3 of file {name}\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].range.start.line, 0);
            assert_eq!(items[0].message, "BibTeX: Error in auxiliary input");
        }
    }

    #[test]
    fn bibliography_diagnostics_use_the_tool_working_directory_for_database_and_style_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("build")).unwrap();
        for name in ["refs.bib", "build/refs.bib"] {
            std::fs::write(
                root.join(name),
                "% database\n@book{example,title={Example}}\n",
            )
            .unwrap();
        }
        for name in ["local.bst", "build/local.bst"] {
            std::fs::write(
                root.join(name),
                "ENTRY {title} {} {}\nFUNCTION {bad} {unknown}\n",
            )
            .unwrap();
        }
        for (auxiliary, prefix) in [("main.aux", "build/"), ("build/main.aux", "")] {
            let log = format!("The top-level auxiliary file: {auxiliary}\nDatabase file #1: refs.bib\nThe style file: local.bst\nWarning--empty author in example\nRepeated entry---line 2 of file refs.bib\nunknown is an unknown function---line 2 of file local.bst\n");
            let diagnostics =
                parse_bibliography_log(&root.join("main.tex"), &root.join("build/main.blg"), &log);
            let bib = Url::from_file_path(root.join(format!("{prefix}refs.bib"))).unwrap();
            let style = Url::from_file_path(root.join(format!("{prefix}local.bst"))).unwrap();
            assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
            assert_eq!(diagnostics[&bib].len(), 2);
            assert!(diagnostics[&bib]
                .iter()
                .all(|diagnostic| diagnostic.range.start.line == 1));
            assert_eq!(diagnostics[&style][0].range.start.line, 1);
        }
    }

    #[test]
    fn biber_diagnostic_paths_do_not_resolve_prefixes_of_longer_filenames() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "\\cite{entry}\nText.\n").unwrap();
        std::fs::write(
            root.join("refs.bib"),
            "% decoy\n@book{decoy,title={Wrong}}\n",
        )
        .unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        for name in [
            "refs.bib.backup",
            "refs.bib2",
            "refs.bib-archive",
            "refs.bib_archive",
            "refs.bib中文",
            "refs.bib/nonexistent.bib",
        ] {
            if !name.contains('/') {
                std::fs::write(
                    root.join(name),
                    "% actual input\n@book{entry,title={Book}}\n",
                )
                .unwrap();
            }
            for quoted in [false, true] {
                let name = if quoted {
                    format!("'{name}'")
                } else {
                    name.into()
                };
                let message = format!("syntax error in file {name}, line 2");
                let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
                let items = diagnostics
                    .get(&main_uri)
                    .unwrap_or_else(|| panic!("{name}: {diagnostics:?}"));
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].message, format!("Biber: {message}"));
            }
        }
    }

    #[test]
    fn biber_diagnostic_paths_keep_complete_names_before_message_punctuation() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% database\n@book{entry,title={Book}}\n").unwrap();
        for message in [
            "syntax error in file 'refs.bib', line 2",
            "syntax error in file \"refs.bib\", line 2",
            "syntax error in refs.bib, line 2",
            "refs.bib: syntax error at line 2",
            "syntax error in refs.bib; line 2",
            "syntax error (refs.bib) at line 2",
            "syntax error [refs.bib] at line 2",
            "syntax error in refs.bib. See line 2",
            "syntax error on line 2 of refs.bib.",
            "syntax error on line 2 of refs.bib",
        ] {
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].message, format!("Biber: {message}"));
            assert_eq!(items[0].range.start.line, 1);
        }
    }

    #[test]
    fn biber_quoted_paths_keep_spaces_apostrophes_and_internal_bib_suffixes() {
        for name in [
            "refs.bib copy.bib",
            "refs.bib's copy.bib",
            "refs.bib' copy.bib",
            "中文 refs.bib copy's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let target = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            for decoy in [
                "refs.bib",
                "copy.bib",
                "s copy.bib",
                "copy's refs.bib",
                "中文 refs.bib",
            ] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&target, "% database\n\n@book{entry,title={Correct}}\n").unwrap();
            for quote in ['\'', '"'] {
                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - Error in {quote}{name}{quote} line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\nWARN - Missing another field for entry 'entry' in file {quote}{name}{quote}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{name} {quote}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&target).unwrap()];
                assert_eq!(items.len(), 3);
                assert!(items.iter().all(|item| item.range.start.line == 2));
                for warning in items
                    .iter()
                    .filter(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                {
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_explicit_paths_preserve_unreported_files_and_do_not_replace_changed_inputs() {
        for name in ["plain.bib", "refs.bib': copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let selected = root.join("selected");
            std::fs::create_dir(&selected).unwrap();
            let main = root.join("main.tex");
            let blg = root.join("main.blg");
            let explicit = root.join(name);
            let input = selected.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&explicit, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            std::fs::write(&input, "@book{entry,title={Read input}}\n").unwrap();
            for source in [explicit.to_str().unwrap().to_owned(), format!("../{name}")] {
                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file '{source}', line 3\nWARN - Missing field for entry 'entry' (file '{source}')\n");
                let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                    .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
                let diagnostics = parse_bibliography_in(&main, &paths, &log);
                let items = diagnostics
                    .get(&Url::from_file_path(&explicit).unwrap())
                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(items.len(), 2);
                assert!(
                    items
                        .iter()
                        .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                    "{log}: {items:?}"
                );
                assert_eq!(
                    crate::recorder::bibliography_inputs_in(&paths, &log),
                    [Url::from_file_path(&input).unwrap()].into()
                );
            }
            let log = format!("INFO - Found BibTeX data source '{name}'\nWARN - Missing field for entry 'entry' (file '{name}')\n");
            let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
            assert_eq!(
                paths.recorded_bibliography_source(name).unwrap().path,
                Some(input.clone())
            );
            std::fs::remove_file(&input).unwrap();
            let diagnostics = parse_bibliography_in(&main, &paths, &log);
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_sources_share_absolute_relative_and_dot_spellings() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("inputs")).unwrap();
        std::fs::create_dir(root.join("build")).unwrap();
        let main = root.join("main.tex");
        let blg = root.join("build/main.blg");
        let name = "inputs/中文 refs.bib': in file 'wrong.bib' line 700 copy.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
        for decoy in ["inputs/中文 refs.bib", "wrong.bib", "copy.bib"] {
            std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
        }
        let spellings = [
            name.to_owned(),
            format!("./{name}"),
            bib.to_str().unwrap().to_owned(),
            root.join(format!("./{name}")).to_str().unwrap().to_owned(),
        ];
        for missing in [false, true] {
            if missing {
                std::fs::remove_file(&bib).unwrap();
            }
            for input in &spellings {
                for source in &spellings {
                    for quote in ['\'', '"'] {
                        for message in [
                            format!("syntax error in file {quote}{source}{quote}, line 3"),
                            format!("syntax error {quote}{source}{quote}, line 3"),
                            format!("Missing field {quote}{source}{quote} for entry 'entry'"),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source '{input}'\n{} - {message}\n",
                                if message.starts_with("Missing") {
                                    "WARN"
                                } else {
                                    "ERROR"
                                }
                            );
                            let diagnostics = parse_bibliography_log(&main, &blg, &log);
                            let target = if missing { &main } else { &bib };
                            let items = diagnostics
                                .get(&Url::from_file_path(target).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                            assert_eq!(items.len(), 1);
                            if missing {
                                assert_eq!(items[0].range.start.line, 0);
                                assert_eq!(
                                    items[0].data.as_ref().unwrap()["unresolvedSource"],
                                    true
                                );
                            } else {
                                assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                if message.starts_with("Missing") {
                                    assert_eq!(
                                        items[0].range,
                                        Range::new(Position::new(2, 6), Position::new(2, 11))
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_sources_respect_search_paths_and_selected_working_directories() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let output = root.join("build");
        std::fs::create_dir(&output).unwrap();
        let main = root.join("main.tex");
        let blg = output.join("main.blg");
        let name = "refs.bib': line 700 copy.bib";
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        for base in [root, &output] {
            std::fs::write(base.join(name), "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(base.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        }
        for auxiliary in ["main.aux", "build/main.aux"] {
            for selected in [root, &output] {
                for absolute_input in [false, true] {
                    let input = if absolute_input {
                        selected.join(name).to_str().unwrap().to_owned()
                    } else {
                        name.to_owned()
                    };
                    let source = if absolute_input {
                        name.to_owned()
                    } else {
                        selected.join(name).to_str().unwrap().to_owned()
                    };
                    let log = format!("The top-level auxiliary file: {auxiliary}\nINFO - Found BibTeX data source '{input}'\nWARN - Missing field for entry 'entry' (file '{source}')\n");
                    let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                        .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
                    let diagnostics = parse_bibliography_in(&main, &paths, &log);
                    let target = Url::from_file_path(selected.join(name)).unwrap();
                    let items = diagnostics
                        .get(&target)
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1);
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
        // An absolute input record disambiguates an unreported full-name
        // lookalike in cwd, even if cwd precedes the log directory.
        let actual = output.join(name);
        let log = format!("INFO - Found BibTeX data source '{}'\nWARN - Missing field for entry 'entry' (file '{name}')\n", actual.display());
        for missing in [false, true] {
            if missing {
                std::fs::remove_file(&actual).unwrap();
            }
            for search in [String::new(), format!("BIBINPUTS='{}'\n", root.display())] {
                let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                    .with_search_output(&search);
                let diagnostics = parse_bibliography_in(&main, &paths, &log);
                let target = if missing { &main } else { &actual };
                let items = &diagnostics[&Url::from_file_path(target).unwrap()];
                assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
                assert_eq!(items.len(), 1);
                if missing {
                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                } else {
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn repeated_bibliography_records_do_not_hide_a_later_complete_source_name() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let name = "refs.bib': copy.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
        let mut log = "INFO - Found BibTeX data source 'refs.bib'\n".repeat(4096);
        log.push_str(&format!("INFO - Found BibTeX data source '{name}'\nWARN - Missing field for entry 'entry' in file '{name}'\n"));
        let diagnostics = parse_bibliography(&main, &log);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let items = diagnostics
            .get(&Url::from_file_path(&bib).unwrap())
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].range,
            Range::new(Position::new(1, 6), Position::new(1, 11))
        );
    }

    #[test]
    #[cfg(unix)]
    fn biber_logged_quoted_names_disambiguate_punctuation_spaces_and_metadata() {
        for suffix in [
            ": copy.bib",
            "; copy.bib",
            ") copy.bib",
            "] copy.bib",
            "} copy.bib",
            ", copy.bib",
            ": line 700 copy.bib",
            ", in file 'wrong.bib' copy.bib",
            ", entry 'wrong' copy.bib",
        ] {
            for quote in ['\'', '"'] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let name = format!("中文 refs.bib{quote}{suffix}");
                let bib = root.join(&name);
                std::fs::write(&main, "\\cite{entry}\n").unwrap();
                for decoy in ["中文 refs.bib", "copy.bib", "wrong.bib"] {
                    std::fs::write(
                        root.join(decoy),
                        "@book{entry,title={Wrong}}\n@book{wrong}\n",
                    )
                    .unwrap();
                }
                std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
                for missing in [false, true] {
                    if missing {
                        std::fs::remove_file(&bib).unwrap();
                    }
                    for outer in ['\'', '"'] {
                        for message in [
                            format!("syntax error in file {outer}{name}{outer}, line 3"),
                            format!("syntax error on line 3 in file {outer}{name}{outer}"),
                            format!("syntax error {outer}{name}{outer}: line 3"),
                            format!("Missing field for entry 'entry' (file {outer}{name}{outer})"),
                            format!("Missing field {outer}{name}{outer} for entry 'entry'"),
                        ] {
                            // Both names are reported; their order and existence
                            // cannot make a literal long name select the short one.
                            for names in [
                                format!("INFO - Found BibTeX data source '中文 refs.bib'\nINFO - Found BibTeX data source '{name}'\n"),
                                format!("INFO - Found BibTeX data source '{name}'\nINFO - Found BibTeX data source '中文 refs.bib'\n"),
                            ] {
                                let log = format!("{names}{} - {message}\n", if message.starts_with("Missing") { "WARN" } else { "ERROR" });
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics.get(&Url::from_file_path(target).unwrap()).unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                                assert_eq!(items.len(), 1);
                                if missing {
                                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true, "{log}: {items:?}");
                                    assert_eq!(items[0].range.start.line, 0);
                                } else {
                                    assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                    assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                    if message.starts_with("Missing") {
                                        assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)), "{log}: {items:?}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_quoted_paths_keep_punctuation_and_line_text_after_literal_quotes() {
        for suffix in [
            ":copy.bib",
            ";copy.bib",
            ")copy.bib",
            "]copy.bib",
            "}copy.bib",
            ",copy.bib",
            "),;copy.bib",
            " line 700 copy.bib",
            " line 700.copy.bib",
        ] {
            for quote in ['\'', '"'] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let name = format!("中文 refs.bib{quote}{suffix}");
                let target = root.join(&name);
                std::fs::write(&main, "\\cite{entry}\n").unwrap();
                for decoy in ["中文 refs.bib", "copy.bib"] {
                    std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
                }
                std::fs::write(&target, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
                for message in [
                    format!("syntax error in file {quote}{name}{quote}, line 3"),
                    format!("syntax error on line 3 in file {quote}{name}{quote}"),
                    format!("syntax error {quote}{name}{quote}: line 3"),
                ] {
                    let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    let items = diagnostics
                        .get(&Url::from_file_path(&target).unwrap())
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                    assert_eq!(items.len(), 2);
                    assert!(
                        items
                            .iter()
                            .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                        "{log}: {items:?}"
                    );
                    let warning = items
                        .iter()
                        .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                        .unwrap();
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
                std::fs::remove_file(&target).unwrap();
                let log = format!("INFO - Found BibTeX data source '中文 refs.bib'\nERROR - syntax error in file {quote}{name}{quote}, line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2);
                assert!(
                    items.iter().all(|item| item
                        .data
                        .as_ref()
                        .is_some_and(|data| data["unresolvedSource"] == true)),
                    "{log}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_missing_quoted_paths_do_not_borrow_prefix_suffix_or_other_database_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let source = "\\cite{entry}\n";
        std::fs::write(&main, source).unwrap();
        for name in ["refs.bib", "copy.bib", "unrelated.bib"] {
            std::fs::write(root.join(name), "% decoy\n\n\n@book{entry,title={Wrong}}\n").unwrap();
        }
        std::fs::create_dir(root.join("directory.bib")).unwrap();
        for quote in ['\'', '"'] {
            for name in ["refs.bib missing copy.bib", "directory.bib"] {
                for closed in [false, true] {
                    let closing = if closed {
                        quote.to_string()
                    } else {
                        String::new()
                    };
                    let log = format!("INFO - Found BibTeX data source 'unrelated.bib'\nERROR - Error in {quote}{name}{closing} line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{closing})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{quote}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                    assert_eq!(items.len(), 2);
                    let text = Text::new(source.into());
                    assert!(items
                        .iter()
                        .all(|item| item.range.start.line == 0
                            && text.offset(item.range.end).is_some()));
                    assert!(items.iter().all(|item| item
                        .data
                        .as_ref()
                        .is_some_and(|data| data["unresolvedSource"] == true)));
                }
            }
        }
    }

    #[test]
    fn biber_explicit_file_warnings_never_reuse_another_databases_entry_range() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let target = root.join("actual.bib");
        let contents = "@book{other,title={Actual}}\n";
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&target, contents).unwrap();
        std::fs::write(
            root.join("unrelated.bib"),
            "% other\n\n\n@book{entry,title={Wrong}}\n",
        )
        .unwrap();
        let log = "INFO - Found BibTeX data source 'unrelated.bib'\nWARN - Missing field for entry 'entry' (file 'actual.bib')\n";
        let diagnostics = parse_bibliography(&main, log);
        assert_eq!(diagnostics.len(), 1);
        let items = &diagnostics[&Url::from_file_path(&target).unwrap()];
        assert_eq!(items[0].range.start, Position::new(0, 0));
        assert!(Text::new(contents.into())
            .offset(items[0].range.end)
            .is_some());
    }

    #[test]
    fn bibtex_entry_keys_are_not_interpreted_as_namesake_source_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{'entry'}\n").unwrap();
        std::fs::write(root.join("entry.bib"), "@book{wrong,title={Decoy}}\n").unwrap();
        for key in ["'entry'", "'entry.bib'", "entry.bib"] {
            std::fs::write(&bib, format!("@book{{{key},title={{Book}}}}\n")).unwrap();
            let log =
                format!("Database file #1: refs.bib\nWarning--empty author and editor in {key}\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{key}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items.len(), 1);
            assert_eq!(
                items[0].range,
                Range::new(
                    Position::new(0, 6),
                    Position::new(0, 6 + key.encode_utf16().count() as u32)
                )
            );
        }
    }

    #[test]
    fn biber_line_fields_ignore_complete_quoted_source_names() {
        for name in [
            "line 700 refs.bib",
            "outline 700 refs.bib",
            "line 700 author's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n% end\n").unwrap();
            for quote in ['\'', '"'] {
                for message in [
                    format!("syntax error in file {quote}{name}{quote}, line 2"),
                    format!("syntax error on line 2 in file {quote}{name}{quote}"),
                ] {
                    let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
                    assert_eq!(items.len(), 2);
                    assert!(
                        items
                            .iter()
                            .all(|item| item.range.start.line == 1 && !unresolved_location(item)),
                        "{message}: {items:?}"
                    );
                    let warning = items
                        .iter()
                        .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                        .unwrap();
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(1, 6), Position::new(1, 11))
                    );
                }
            }
        }
    }

    #[test]
    fn biber_line_fields_require_unquoted_complete_words_and_numbers() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n% end\n").unwrap();
        for prefix in [
            "outline 700; error",
            "pipeline 700; error",
            "_line 700; error",
            "unexpected token 'line 700'",
            "unexpected token \"line 700\"",
            "unexpected token 'author's line 700'",
            "invalid line 700bad; error",
            "invalid line 700.5; error",
        ] {
            let message = format!("{prefix} in file 'refs.bib', line 2");
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].range.start.line, 1, "{message}: {items:?}");
            assert!(!unresolved_location(&items[0]), "{message}: {items:?}");
        }
        for number in [0, u32::MAX] {
            let message = format!("syntax error in file 'refs.bib', line {number}");
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].data.as_ref().unwrap()["reportedLine"], number);
            assert!(unresolved_location(&items[0]));
        }
    }

    #[test]
    fn bibtex_warning_text_does_not_override_the_entry_location_with_a_line_number() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n").unwrap();
        let diagnostics = parse_bibliography(
            &main,
            "Database file #1: refs.bib\nWarning--line 700 value in entry\n",
        );
        let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(
            items[0].range,
            Range::new(Position::new(1, 6), Position::new(1, 11))
        );
        assert!(!unresolved_location(&items[0]));
    }

    #[test]
    fn biber_entry_fields_preserve_internal_leading_and_trailing_quotes() {
        for key in [
            "O'Connor2026",
            "'start",
            "end'",
            "two''quotes",
            "name'-part",
            "name'_part",
            "name'.part",
            "name':part",
            "name';part",
            "name']part",
            "name'!part",
            "name'?part",
            "王'文2026",
            "😀'key",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join("refs.bib");
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            // Prefix and suffix declarations must not replace the full key.
            std::fs::write(&bib, format!("@book{{O,title={{Prefix}}}}\n@book{{Connor2026,title={{Suffix}}}}\n@book{{{key},title={{Actual}}}}\n")).unwrap();
            for label in ["entry", "Entry"] {
                for quote in ['\'', '"'] {
                    let message = format!("Missing required field for {label} {quote}{key}{quote}");
                    let log =
                        format!("INFO - Found BibTeX data source 'refs.bib'\nWARN - {message}\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
                    assert_eq!(
                        items[0].range,
                        Range::new(
                            Position::new(2, 6),
                            Position::new(2, 6 + key.encode_utf16().count() as u32)
                        ),
                        "{message}: {items:?}"
                    );
                    assert_eq!(items[0].message, format!("Biber: {message}"));
                }
            }
        }
    }

    #[test]
    fn biber_entry_keys_do_not_select_namesake_database_files() {
        for key in ["entry.bib", "'entry.bib'", "O'Connor.bib", "dir/entry.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join("refs.bib");
            let decoy = root.join(key);
            std::fs::create_dir_all(decoy.parent().unwrap()).unwrap();
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, format!("% header\n@book{{{key},title={{Actual}}}}\n")).unwrap();
            std::fs::write(
                &decoy,
                format!("% decoy\n\n\n@book{{{key},title={{Wrong}}}}\n"),
            )
            .unwrap();
            for label in ["entry", "Entry"] {
                for quote in ['\'', '"'] {
                    for source in ["", " in refs.bib", " in file 'refs.bib'"] {
                        let log = format!("INFO - Found BibTeX data source 'refs.bib'\nWARN - Missing field for {label} {quote}{key}{quote}{source}\n");
                        let diagnostics = parse_bibliography(&main, &log);
                        assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                        let items = diagnostics
                            .get(&Url::from_file_path(&bib).unwrap())
                            .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                        assert_eq!(
                            items[0].range,
                            Range::new(
                                Position::new(1, 6),
                                Position::new(1, 6 + key.encode_utf16().count() as u32)
                            ),
                            "{log}: {items:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_logged_unquoted_sources_keep_complete_names_outside_message_fields() {
        for name in [
            "refs.bib line 700 copy.bib",
            "中文 refs.bib in file 'wrong.bib' entry 'wrong' copy.bib",
            "refs.bib': copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            for decoy in ["refs.bib", "wrong.bib", "copy.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for input in [name.to_owned(), bib.to_str().unwrap().to_owned()] {
                    for source in [
                        name.to_owned(),
                        format!("./{name}"),
                        bib.to_str().unwrap().to_owned(),
                    ] {
                        for (open, close) in [("", ""), ("(", ")"), ("[({", "})]")] {
                            for message in [
                                format!("Unexpected token \"in 'wrong.bib', line 900\" in file {open}{source}{close}, line 3"),
                                format!("syntax error {open}{source}{close}, line 3"),
                                format!("Missing field {open}{source}{close} for entry 'entry'"),
                                format!("Missing field for entry 'entry' (file {open}{source}{close})"),
                            ] {
                                let log = format!("INFO - Found BibTeX data source '{input}'\nERROR - {message}\n");
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics
                                    .get(&Url::from_file_path(target).unwrap())
                                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                                assert_eq!(items.len(), 1);
                                assert_eq!(items[0].message, format!("Biber: {message}"));
                                if missing {
                                    assert_eq!(items[0].range.start, Position::new(0, 0));
                                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                                } else {
                                    assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                    assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                    if message.starts_with("Missing") {
                                        assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_unquoted_prefixes_do_not_shorten_complete_unreported_tokens() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        for name in ["refs.bib,copy.bib", "refs.bib:copy.bib"] {
            let bib = root.join(name);
            std::fs::write(&bib, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for message in [
                    format!("syntax error in file {name}, line 3"),
                    format!("Missing field for entry 'entry' (file {name})"),
                ] {
                    let log =
                        format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    let target = if missing { &main } else { &bib };
                    let items = diagnostics
                        .get(&Url::from_file_path(target).unwrap())
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1);
                    assert_eq!(items[0].range.start.line, if missing { 0 } else { 2 });
                    assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_unquoted_sources_preserve_explicit_files_and_missing_selected_inputs() {
        for name in ["plain.bib", "refs.bib line 700 copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let selected = root.join("selected");
            std::fs::create_dir(&selected).unwrap();
            let main = root.join("main.tex");
            let explicit = root.join(name);
            let input = selected.join(name);
            let blg = root.join("main.blg");
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&explicit, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            std::fs::write(&input, "@book{entry,title={Selected}}\n").unwrap();
            let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file {}, line 3\nWARN - Missing field for entry 'entry' (file {})\n", explicit.display(), explicit.display());
            let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
            let diagnostics = parse_bibliography_in(&main, &paths, &log);
            assert_eq!(diagnostics.len(), 1);
            let items = &diagnostics[&Url::from_file_path(&explicit).unwrap()];
            assert_eq!(items.len(), 2);
            assert!(
                items
                    .iter()
                    .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                "{log}: {items:?}"
            );
            assert_eq!(
                paths.recorded_bibliography_source(name).unwrap().path,
                Some(input.clone())
            );
            std::fs::remove_file(&input).unwrap();
            let diagnostics = parse_bibliography_in(
                &main,
                &paths,
                &format!("WARN - Missing field for entry 'entry' (file {name})\n"),
            );
            assert_eq!(diagnostics.len(), 1);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].range.start, Position::new(0, 0));
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_unreported_explicit_sources_keep_spaces_and_do_not_borrow_message_files() {
        for name in [
            "missing refs.bib",
            "refs.bib line 700 copy.bib",
            "refs.bib, copy.bib",
            "中文 refs.bib': copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            for decoy in ["refs.bib", "wrong.bib", "copy.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for marker in ["file", "File", "in", "In", "in file", "in \tFile"] {
                    for (open, close) in [("", ""), ("[({", "})]")] {
                        for message in [
                            format!(
                                "Unexpected token 'wrong.bib' {marker} {open}{name}{close}, line 3"
                            ),
                            format!(
                                "Missing field for entry 'entry' ({marker} {open}{name}{close})"
                            ),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n"
                            );
                            let diagnostics = parse_bibliography(&main, &log);
                            let target = if missing { &main } else { &bib };
                            let items = diagnostics
                                .get(&Url::from_file_path(target).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(diagnostics.len(), 1);
                            assert_eq!(items.len(), 1);
                            assert_eq!(items[0].message, format!("Biber: {message}"));
                            assert_eq!(
                                items[0].range.start.line,
                                if missing { 0 } else { 2 },
                                "{log}: {items:?}"
                            );
                            assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                            if !missing && message.starts_with("Missing") {
                                assert_eq!(
                                    items[0].range,
                                    Range::new(Position::new(2, 6), Position::new(2, 11))
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_explicit_multiword_non_bibliography_names_do_not_borrow_short_sources() {
        for name in ["missing refs.bib.backup", "missing refs.bib2"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            std::fs::write(&main, "Text\n").unwrap();
            for file in [name, "refs.bib", "wrong.bib"] {
                std::fs::write(root.join(file), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for message in [
                format!("Unexpected token 'wrong.bib' in file {name}, line 3"),
                format!("Missing field for entry 'entry' (file {name})"),
            ] {
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_quoted_generic_prose_does_not_hide_the_actual_source_field() {
        for name in ["refs.bib", "中文 refs.bib': copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for prose in ['\'', '"'] {
                    for quote in ['\'', '"'] {
                        for marker in ["file", "in file", "in"] {
                            for message in [
                                format!("Error in {prose}parser{prose}; {marker} {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose}; for entry 'entry' ({marker} {quote}{name}{quote})"),
                                format!("Error in {prose}parser{prose}; {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose}; {quote}{name}{quote} for entry 'entry'"),
                                format!("Error in {prose}parser{prose}; {name}, line 3"),
                                format!("Missing field in {prose}parser{prose}; {name} for entry 'entry'"),
                                format!("Error in {prose}parser{prose} {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose} {quote}{name}{quote} for entry 'entry'"),
                            ] {
                                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\n");
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics.get(&Url::from_file_path(target).unwrap())
                                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1);
                                assert_eq!(items[0].message, format!("Biber: {message}"));
                                assert_eq!(items[0].range.start.line, if missing { 0 } else { 2 }, "{log}: {items:?}");
                                assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                                if !missing && message.starts_with("Missing") {
                                    assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_unquoted_sources_take_precedence_over_quoted_error_filenames() {
        for name in [
            "refs.bib",
            "author's.bib",
            "中文.bib",
            "dir.bib/refs.bib",
            "refs.bib,copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::create_dir_all(bib.parent().unwrap()).unwrap();
            std::fs::write(&main, "Text\n").unwrap();
            for decoy in ["wrong.bib", "s.bib", "copy.bib", "refs.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
            for content in [
                "\"in 'wrong.bib', line 900\"",
                "'file \"wrong.bib\"'",
                "\"wrong.bib\"",
                "'wrong.bib'",
            ] {
                for marker in [
                    "in file",
                    "file",
                    "File",
                    "in",
                    "In",
                    "in file ",
                    "in file \u{3000}",
                ] {
                    for message in [
                        format!(
                            "Error in parser: unexpected token {content} {marker} {name}, line 2"
                        ),
                        format!("{marker} {name}, unexpected token {content}, line 2"),
                        format!("Unexpected token {content} {marker} ({name}), line 2"),
                        format!("{marker} [{name}], unexpected token {content}, line 2"),
                        format!("Unexpected token {content} {marker} {{{name}}}, line 2"),
                        format!("Unexpected token {content} {marker} [({name})], line 2"),
                    ] {
                        let log = format!(
                            "INFO - Found BibTeX data source '{name}'\nERROR - {message}\n"
                        );
                        let diagnostics = parse_bibliography(&main, &log);
                        assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                        let items = diagnostics
                            .get(&Url::from_file_path(&bib).unwrap())
                            .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                        assert_eq!(
                            items[0].range.start,
                            Position::new(1, 0),
                            "{log}: {items:?}"
                        );
                        assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                        assert_eq!(items[0].message, format!("Biber: {message}"));
                    }
                }
            }
        }
    }

    #[test]
    fn biber_unquoted_missing_sources_do_not_borrow_quoted_files_or_entry_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        for name in ["wrong.bib", "refs.bib"] {
            std::fs::write(root.join(name), "% header\n\n@book{entry,title={Wrong}}\n").unwrap();
        }
        std::fs::create_dir(root.join("directory.bib")).unwrap();
        for name in [
            "missing.bib",
            "directory.bib",
            "refs.bib.backup",
            "refs.bib2",
            "refs.bib's-backup",
            "refs.bib/nonexistent.bib",
            "(refs.bib",
            "[refs.bib",
            "{refs.bib",
        ] {
            for message in [
                format!("Unexpected token \"in 'wrong.bib'\" in file {name}, line 2"),
                format!("Missing field for entry 'entry' in file {name}"),
            ] {
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = diagnostics
                    .get(&Url::from_file_path(&main).unwrap())
                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(
                    items[0].data.as_ref().unwrap()["unresolvedSource"],
                    true,
                    "{log}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_unquoted_bare_sources_ignore_nested_filenames_in_quoted_errors() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        for name in ["wrong.bib", "s.bib", "refs.bib", "author's.bib"] {
            std::fs::write(root.join(name), "% header\n@book{entry,title={Actual}}\n").unwrap();
        }
        for name in ["refs.bib", "author's.bib"] {
            for tail in [
                format!("{name}: syntax error at line 2"),
                format!("syntax error ({name}) at line 2"),
                format!("syntax error [{name}] at line 2"),
                format!("syntax error on line 2 of {name}."),
                format!("syntax error \u{3000}{name}, line 2"),
                format!("syntax error \u{a0}{name}, line 2"),
            ] {
                for message in [
                    format!("Unexpected token \"in 'wrong.bib', line 900\"; {tail}"),
                    format!("Unexpected token \"bang\" {tail} \"detail\""),
                    format!("Unexpected token \"in 'wrong.bib', line 900\" {tail} \"detail\""),
                ] {
                    let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = diagnostics
                        .get(&Url::from_file_path(root.join(name)).unwrap())
                        .unwrap_or_else(|| panic!("{message}: {diagnostics:?}"));
                    assert_eq!(
                        items[0].range.start,
                        Position::new(1, 0),
                        "{message}: {items:?}"
                    );
                    assert!(!unresolved_location(&items[0]));
                }
            }
        }
    }

    #[test]
    fn biber_standalone_quoted_sources_preserve_complete_filename_boundaries() {
        for name in [
            "refs.bib copy.bib",
            "refs.bib's copy.bib",
            "refs.bib' copy.bib",
            "中文 refs.bib copy's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            for decoy in [
                "refs.bib",
                "copy.bib",
                "s copy.bib",
                "copy's refs.bib",
                "中文 refs.bib",
            ] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            for quote in ['\'', '"'] {
                let message = format!("syntax error {quote}{name}{quote}, line 3");
                let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                let items = diagnostics
                    .get(&Url::from_file_path(&bib).unwrap())
                    .unwrap_or_else(|| panic!("{message}: {diagnostics:?}"));
                assert_eq!(
                    items[0].range.start,
                    Position::new(2, 0),
                    "{message}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_source_fields_ignore_markers_in_quoted_error_content() {
        for name in ["refs.bib", "author's 中文 refs.bib", "entry 'wrong'.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
            for (outer, inner) in [('"', '\''), ('\'', '"')] {
                for marker in ["in", "In", "file", "File"] {
                    let content = format!(
                        "Unexpected token {outer}{marker} {inner}wrong.bib{inner}, line 900{outer}"
                    );
                    for quote in ['\'', '"'] {
                        let source = format!("in file {quote}{name}{quote}");
                        for message in [
                            format!("{content} {source}, line 2"),
                            format!("{source}, {content}, line 2"),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source '{name}'\nERROR - {message}\n"
                            );
                            let diagnostics = parse_bibliography(&main, &log);
                            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                            let items = diagnostics
                                .get(&Url::from_file_path(&bib).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(
                                items[0].range.start,
                                Position::new(1, 0),
                                "{log}: {items:?}"
                            );
                            assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                            assert_eq!(items[0].message, format!("Biber: {message}"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_missing_source_fields_do_not_use_files_named_in_quoted_error_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Other}}\n").unwrap();
        std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        for (outer, inner) in [('"', '\''), ('\'', '"')] {
            for marker in ["in", "In", "file", "File"] {
                let message = format!("Unexpected token {outer}{marker} {inner}wrong.bib{inner}, line 900{outer} in file 'missing.bib', line 2");
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                assert_eq!(items[0].message, format!("Biber: {message}"));
            }
        }
    }

    #[test]
    fn biber_incomplete_entry_keys_do_not_select_namesake_database_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("entry.bib"), "@book{entry.bib,title={Wrong}}\n").unwrap();
        for quote in ['\'', '"'] {
            let log = format!("WARN - Missing field for entry {quote}entry.bib\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
            let items = diagnostics
                .get(&Url::from_file_path(&main).unwrap())
                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
            assert_eq!(items[0].range.start, Position::new(0, 0));
        }
    }

    #[test]
    fn biber_entry_fields_skip_names_inside_files_and_find_metadata_after_them() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let name = "entry 'wrong'.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{O'Connor2026}\n").unwrap();
        std::fs::write(
            &bib,
            "% header\n@book{wrong,title={Wrong}}\n@book{O'Connor2026,title={Actual}}\n",
        )
        .unwrap();
        let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file \"{name}\"\nWARN - Missing field in file \"{name}\" for entry 'O'Connor2026'\n");
        let diagnostics = parse_bibliography(&main, &log);
        let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(items.len(), 2);
        let error = items
            .iter()
            .find(|item| item.severity == Some(DiagnosticSeverity::ERROR))
            .unwrap();
        assert_eq!(error.range.start, Position::new(0, 0));
        let warning = items
            .iter()
            .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
            .unwrap();
        assert_eq!(
            warning.range,
            Range::new(Position::new(2, 6), Position::new(2, 18))
        );
    }

    #[test]
    fn biber_unterminated_entry_fields_do_not_borrow_trailing_keys() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "\\cite{other}\n").unwrap();
        std::fs::write(
            root.join("refs.bib"),
            "% header\n@book{other,title={Wrong}}\n",
        )
        .unwrap();
        let log = "INFO - Found BibTeX data source 'refs.bib'\nWARN - Missing field for entry 'missing in other\n";
        let diagnostics = parse_bibliography(&main, log);
        assert_eq!(diagnostics.len(), 1);
        let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(items[0].range.start, Position::new(0, 0));
    }

    #[test]
    fn biber_entry_fields_ignore_quoted_content_and_partial_words() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(
            &bib,
            "% header\n@book{wrong,title={Wrong}}\n@book{O'Connor2026,title={Actual}}\n",
        )
        .unwrap();
        for prefix in [
            "unexpected token \"entry 'wrong'\"",
            "unexpected token 'entry \"wrong\"'",
            "subentry 'wrong'",
            "_entry 'wrong'",
            "王entry 'wrong'",
        ] {
            for suffix in ["", " for Entry 'O'Connor2026': missing author"] {
                let log = format!(
                    "INFO - Found BibTeX data source 'refs.bib'\nWARN - {prefix}{suffix}\n"
                );
                let diagnostics = parse_bibliography(&main, &log);
                let expected = if suffix.is_empty() { &main } else { &bib };
                let items = &diagnostics[&Url::from_file_path(expected).unwrap()];
                assert_eq!(items.len(), 1, "{log}: {items:?}");
                if suffix.is_empty() {
                    assert_eq!(items[0].range.start, Position::new(0, 0));
                } else {
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 18))
                    );
                }
            }
        }
    }

    #[test]
    fn biber_entry_quote_boundaries_do_not_expose_quoted_line_fields() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
        for quoted in [
            "'name'-line 700'",
            "'name'.line 700'",
            "'name':line 700'",
            "'name''line 700'",
            "'name';line 700'",
            "'name']line 700'",
        ] {
            let log =
                format!("WARN - Unexpected token {quoted} for entry 'entry' in file 'refs.bib'\n");
            let diagnostics = parse_bibliography(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(
                items[0].range,
                Range::new(Position::new(1, 6), Position::new(1, 11)),
                "{log}: {items:?}"
            );
            assert!(!unresolved_location(&items[0]));
        }
    }

    #[test]
    fn biber_databases_with_apostrophes_keep_entry_warning_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let bib = root.join("author's refs.bib");
        std::fs::write(&bib, "% database\n@book{example,title={Example}}\n").unwrap();
        let diagnostics = parse_bibliography(&root.join("main.tex"), "INFO - Found BibTeX data source 'author's refs.bib'\nWARN - Missing field for entry 'example'\nERROR - Error in 'author's refs.bib' line 2\n");
        let diagnostics = &diagnostics[&Url::from_file_path(bib).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 1));
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING))
            .unwrap();
        assert_eq!(diagnostic.range.start.line, 1);
        assert_eq!(diagnostic.range.start.character, 6);
    }

    #[test]
    fn missing_include_messages_locate_paths_without_reporting_normal_missing_auxiliary_files() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let child = directory.path().join("child.tex");
        std::fs::write(&main, "\\input{child}\n").unwrap();
        let source = "😀 \\include{missing chapter}\n% \\include{missing chapter}\n\\verb|\\include{missing chapter}|\n\\include{other}\n\\include{ missing chapter }\n";
        std::fs::write(&child, source).unwrap();
        let log = "(./main.tex\n(./child.tex\nNo file main.aux.\nNo file main.bbl.\nNo file optional.tex.\nNo file \"missing chapter.tex\".\nPackage local Warning: Unfinished message (\nNo file other.tex.\nNo file missing chapter.tex.\n)\nNo file optional.tex.\n)";
        let result = parse(&main, log);
        assert!(!result.contains_key(&Url::from_file_path(&main).unwrap()));
        let diagnostics = &result[&Url::from_file_path(&child).unwrap()];
        let missing: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("missing-include".into()))
            })
            .collect();
        assert_eq!(missing.len(), 3, "{diagnostics:?}");
        assert_eq!(missing[0].range.start.character, 12);
        let text = Text::new(source.into());
        assert_eq!(
            missing
                .iter()
                .map(
                    |diagnostic| &source[text.offset(diagnostic.range.start).unwrap()
                        ..text.offset(diagnostic.range.end).unwrap()]
                )
                .collect::<Vec<_>>(),
            vec!["missing chapter", "other", "missing chapter"]
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "Package local Warning: Unfinished message ("));
        assert!(missing
            .iter()
            .all(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING)));
    }

    #[test]
    fn wrapped_missing_includes_preserve_file_context_and_adjacent_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let child = root.join("child.tex");
        std::fs::write(&main, "\\input{child}\n\\include{parent}\n").unwrap();
        let chapter = format!("chapters/{} (中文 file)", "long-path-".repeat(12));
        let source = format!("😀 \\include{{{chapter}}}\n");
        std::fs::write(&child, &source).unwrap();
        for width in [16, 79, 121] {
            for quoted in [false, true] {
                let missing = if quoted {
                    format!("No file \"{chapter}.tex\".")
                } else {
                    format!("No file {chapter}.tex.")
                };
                let wrapped = missing
                    .chars()
                    .collect::<Vec<_>>()
                    .chunks(width)
                    .map(|chars| chars.iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                let log = format!("(./main.tex\n(./child.tex\nNo file optional(broken).tex.\n{wrapped}\nNo file chapters/long-path-\nPackage local Warning: Adjacent warning on input line 1.\n)\nNo file parent.tex.\n)");
                let result = parse(&main, &log);
                let child_diagnostics = &result[&Url::from_file_path(&child).unwrap()];
                assert_eq!(child_diagnostics.len(), 2, "{result:?}");
                let missing = child_diagnostics
                    .iter()
                    .find(|diagnostic| missing_include_key(&diagnostic.message).is_some())
                    .unwrap();
                assert_eq!(
                    missing.message,
                    format!("Included file `{chapter}.tex` not found")
                );
                assert_eq!(
                    missing.range,
                    Range::new(
                        Position::new(0, 12),
                        Position::new(0, 12 + chapter.encode_utf16().count() as u32)
                    )
                );
                let parent = &result[&Url::from_file_path(&main).unwrap()];
                assert_eq!(parent.len(), 1);
                assert_eq!(parent[0].message, "Included file `parent.tex` not found");
                assert_eq!(parent[0].range.start.line, 1);
            }
        }
    }

    #[test]
    fn box_diagnostics_span_reported_source_lines_and_keep_valid_utf16_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First\r\n  😀 second\r\n 中文 third\r\nLast\r\n").unwrap();
        let log = "(./main.tex\nOverfull \\hbox (12pt too wide) in paragraph at lines 2--3\nUnderfull \\hbox (badness 10000) in paragraph at lines 3--200\nOverfull \\vbox (2pt too high) detected at line 4\nUnderfull \\hbox (badness 10000) in paragraph at lines 3--1\n)";
        let result = parse(&main, log);
        let diagnostics = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 4);
        let find = |prefix: &str| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message.starts_with(prefix))
                .unwrap()
        };
        let overfull = find("Overfull \\hbox");
        assert_eq!(
            overfull.range,
            Range::new(Position::new(1, 2), Position::new(2, 9))
        );
        assert_eq!(
            overfull.code,
            Some(NumberOrString::String("overfull-box".into()))
        );
        let underfull = find("Underfull \\hbox (badness 10000) in paragraph at lines 3--200");
        assert_eq!(
            underfull.range,
            Range::new(Position::new(2, 1), Position::new(4, 0))
        );
        assert_eq!(
            underfull.code,
            Some(NumberOrString::String("underfull-box".into()))
        );
        assert_eq!(
            find("Overfull \\vbox").range,
            Range::new(Position::new(3, 0), Position::new(3, 4))
        );
        assert_eq!(
            find("Underfull \\hbox (badness 10000) in paragraph at lines 3--1").range,
            Range::new(Position::new(2, 1), Position::new(2, 9))
        );
    }

    #[test]
    fn single_quoted_citation_warnings_highlight_keys_and_preserve_apostrophes() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\autocite{first, O'Connor}\n\\ref{missing}\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nPackage biblatex Warning: Citation 'O'Connor' on page 1 undefined on input line 1.\nPackage biblatex Warning: Citation 'first' undefined\n(biblatex) on input line 1.\nLaTeX Warning: Reference 'missing' on page 1 undefined on input line 2.\n)";
        let result = parse(&main, log);
        let diagnostics = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 3);
        let text = Text::new(source.into());
        let values: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| {
                &source[text.offset(diagnostic.range.start).unwrap()
                    ..text.offset(diagnostic.range.end).unwrap()]
            })
            .collect();
        assert_eq!(values, vec!["first", "O'Connor", "missing"]);
        assert_eq!(diagnostics[0].range.start.character, 13);
    }

    #[test]
    fn bibliography_errors_and_entry_warnings_point_to_the_database() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let bib = directory.path().join("with spaces.bib");
        std::fs::write(&main, "\\cite{example}\n").unwrap();
        std::fs::write(
            &bib,
            "\n@book{example,\n title={Example}\n}\n@book{example, title={Duplicate}}\n",
        )
        .unwrap();
        let log = "Database file #1: with spaces.bib\nRepeated entry---line 5 of file with spaces.bib\nI was expecting a `,' or a `}'---line 3 of file with spaces.bib\nWarning--empty author in example\n";
        let diagnostics = parse_bibliography(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].range.start, Position::new(1, 6));
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(diagnostics[1].range.start.line, 2);
        assert_eq!(diagnostics[2].range.start.line, 4);
        let log = "[0] INFO - Found BibTeX data source 'with spaces.bib'\n[1] ERROR - syntax error in file 'with spaces.bib', line 3\n[2] WARN - Missing required field for entry 'example'\n";
        let diagnostics = parse_bibliography(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].range.start.line, 1);
        assert_eq!(diagnostics[1].range.start.line, 2);
        assert!(diagnostics[1].message.starts_with("Biber:"));
        let diagnostics = parse_bibliography(&main,"I couldn't open database file missing.bib\n---line 4 of file main.aux\nI found no database files---while reading file main.aux\n");
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("couldn't open database file missing.bib")));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 0));
    }

    #[test]
    fn file_line_errors_and_multiline_warnings_use_the_right_file() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let chapter = directory.path().join("章节.tex");
        std::fs::write(&main, "\\input{章节}\n\\ref{missing}\n").unwrap();
        std::fs::write(&chapter, "hello\n😀 \\bad\n").unwrap();
        let log = "(./main.tex\n(./章节.tex\n./章节.tex:2: Undefined control sequence.\nl.2 \\bad\n)\nLaTeX Warning: Reference `missing' on page 1 undefined on input\n line 2.\n\nOverfull \\hbox (12pt too wide) in paragraph at lines 2--2\n)";
        let diagnostics = parse(&main, log);
        let chapter = &diagnostics[&Url::from_file_path(chapter).unwrap()];
        assert_eq!(chapter.len(), 1);
        assert_eq!(chapter[0].range.start.line, 1);
        assert_eq!(chapter[0].range.end.character, 7);
        let main = &diagnostics[&Url::from_file_path(main).unwrap()];
        assert_eq!(main.len(), 2);
        assert!(main
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 1));
    }

    #[test]
    fn font_class_engine_and_glyph_warnings_preserve_adjacent_errors_and_deduplicate_shipout_messages(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First\nSecond\nThird\n").unwrap();
        let log = "(./main.tex\nClass memoir Warning: Option ignored\nLaTeX Font Warning: Font shape unavailable\n(Font) using a substitute on input line 3.\nMissing character: There is no 😀 (U+1F600) in font\n Latin Modern Roman!\nMissing character: There is no 😀 (U+1F600) in font\n Latin Modern Roman!\npdfTeX warning (dest): destination missing!\n! First error.\n! Second error.\nl.2 bad\n)\n";
        let diagnostics = parse(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 6, "{diagnostics:?}");
        let font = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.starts_with("LaTeX Font Warning:"))
            .unwrap();
        assert_eq!(font.range.start.line, 2);
        assert!(font.message.contains("(Font) using a substitute"));
        let glyph = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.starts_with("Missing character:"))
            .collect::<Vec<_>>();
        assert_eq!(glyph.len(), 1);
        assert!(glyph[0].message.ends_with("Latin Modern Roman!"));
        assert_eq!(glyph[0].severity, Some(DiagnosticSeverity::WARNING));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "Class memoir Warning: Option ignored"));
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
                .count(),
            2
        );
    }

    #[test]
    fn traditional_errors_and_quoted_paths_are_supported() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let chapter = directory.path().join("with spaces.tex");
        std::fs::write(&main, "main\n").unwrap();
        std::fs::write(&chapter, "hello\n\\bad\n").unwrap();
        let diagnostics = parse(
            &main,
            "(./main.tex\n(\"./with spaces.tex\"\n! Undefined control sequence.\nl.2 \\bad\n)\n)",
        );
        assert_eq!(
            diagnostics[&Url::from_file_path(chapter).unwrap()][0]
                .range
                .start
                .line,
            1
        );
    }

    #[test]
    fn source_paths_and_diagnostic_parentheses_preserve_nested_file_context() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let definition = directory.path().join("配置 (旧).def");
        let options = directory.path().join("layout options.clo");
        for path in [&main, &definition, &options] {
            std::fs::write(path, "One\n😀 Two\nThree\nFour\nFive\n").unwrap();
        }
        let log = "(./main.tex\n(./配置 (旧).def\nPackage local Warning: Closing ) marker on input line 2.\nPackage local Warning: Still local on input line 3.\n! Error with a ( marker.\nl.4 \\bad)\nPackage local Warning: After error on input line 5.\n)\n(./layout options.clo\nLaTeX Warning: Options warning on input line 2.\n)\nLaTeX Warning: Back in main on input line 3.\n)";
        let diagnostics = parse(&main, log);
        let local = &diagnostics[&Url::from_file_path(&definition).unwrap()];
        assert_eq!(local.len(), 4, "{diagnostics:?}");
        assert_eq!(local[0].range.end.character, 6);
        assert!(local
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 3
                && diagnostic.severity == Some(DiagnosticSeverity::ERROR)));
        assert_eq!(
            diagnostics[&Url::from_file_path(&options).unwrap()][0]
                .range
                .start
                .line,
            1
        );
        assert_eq!(
            diagnostics[&Url::from_file_path(&main).unwrap()][0]
                .range
                .start
                .line,
            2
        );
    }

    #[test]
    fn undefined_references_highlight_keys_in_lists_multiline_calls_and_utf16_text() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\ref{missing} \\cref{found, missing} \\cite{lost}\n% \\ref{missing}\n\\verb|\\ref{missing}|\n\\ref\n{missing}\n\\ref{O'Connor}\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\nPackage natbib Warning: Citation `lost' on page 1 undefined on input line 1.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 2.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 3.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 5.\nLaTeX Warning: Reference `O'Connor' undefined on input line 6.\n)";
        let diagnostics = parse(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 7, "{diagnostics:?}");
        let text = Text::new(source.into());
        let value = |range: Range| {
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()]
        };
        let first: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.range.start.line == 0)
            .collect();
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].range.start.character, 8);
        assert_eq!(
            first
                .iter()
                .map(|diagnostic| value(diagnostic.range))
                .collect::<Vec<_>>(),
            vec!["missing", "missing", "lost"]
        );
        assert_eq!(value(diagnostics[3].range), "% \\ref{missing}");
        assert_eq!(value(diagnostics[4].range), "\\verb|\\ref{missing}|");
        assert_eq!(value(diagnostics[5].range), "missing");
        assert_eq!(value(diagnostics[6].range), "O'Connor");
    }

    #[test]
    fn single_reference_ranges_do_not_normalize_whitespace_but_cref_lists_keep_key_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\ref{missing} \\ref{ missing } \\cref{found, missing }\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\nLaTeX Warning: Reference ` missing ' on page 1 undefined on input line 1.\n)\n";
        let diagnostics = parse(&main, log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 3, "{errors:?}");
        let text = Text::new(source.into());
        let precise: Vec<_> = errors
            .iter()
            .filter(|error| !error.message.contains("` missing '"))
            .collect();
        assert_eq!(precise.len(), 2);
        for (error, start) in precise.iter().zip([
            source.find("missing").unwrap(),
            source.rfind("missing").unwrap(),
        ]) {
            assert_eq!(error.range, text.range(start, start + 7));
        }
        let fallback = errors
            .iter()
            .find(|error| error.message.contains("` missing '"))
            .unwrap();
        assert_eq!(fallback.range, text.range(0, source.len() - 1));
    }

    #[test]
    fn silent_refcount_queries_do_not_inherit_warnings_from_another_call_on_the_line() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = concat!(
            "\\refused{missing} \\getrefnumber{missing} \\getpagerefnumber{missing} ",
            "\\getrefbykeydefault{missing}{page}{0} ",
            "\\IfRefUndefinedExpandable{missing}{A}{B} \\IfRefUndefinedBabel{missing}{A}{B}\n",
        );
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\n)\n";
        let diagnostics = parse(&main, log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 1, "{errors:?}");
        let start = source.find("missing").unwrap();
        assert_eq!(
            errors[0].range,
            Text::new(source.into()).range(start, start + 7)
        );
    }

    #[test]
    fn wrapped_file_announcements_rejoin_existing_names_without_swallowing_warnings() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("settings {}(old).def", "long-name-".repeat(12));
        let definition = directory.path().join(&name);
        std::fs::write(&main, "Main\n").unwrap();
        std::fs::write(&definition, "First\nSecond\n").unwrap();
        for quoted in [false, true] {
            let opening = if quoted {
                format!("(\"./{name}\"")
            } else {
                format!("(./{name}")
            };
            let wrapped = format!("{}\n{}", &opening[..79], &opening[79..]);
            let log = format!("(./main.tex\n{wrapped}\nLaTeX Warning: Local warning on input line 2.\n)\nLaTeX Warning: Root warning on input line 1.\n)\n");
            let diagnostics = parse(&main, &log);
            assert_eq!(
                diagnostics[&Url::from_file_path(&definition).unwrap()][0]
                    .range
                    .start
                    .line,
                1
            );
            assert_eq!(diagnostics[&Url::from_file_path(&main).unwrap()].len(), 1);
        }
        let log = format!(
            "(./main.tex\n({}\nLaTeX Warning: Do not swallow this on input line 1.\n",
            "not-a-file-".repeat(9)
        );
        let diagnostics = parse(&main, &log);
        assert_eq!(diagnostics[&Url::from_file_path(&main).unwrap()].len(), 1);
    }
}

#[cfg(test)]
mod bibliography_style_locations {
    use super::*;

    fn missing(root: &Path) -> BTreeMap<Url, Vec<Diagnostic>> {
        parse_bibliography(
            root,
            "I couldn't open style file local,styel.bst\n---line 2 of file main.aux\n",
        )
    }

    #[test]
    fn style_locations_use_only_confirmed_sources_and_preserve_ambiguous_failures() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let unread = root.join("unread.tex");
        std::fs::write(&main, "\\input{settings}\n").unwrap();
        let source = "% emoji😀\r\n\\bibliographystyle{% before\r\n local,styel% after\r\n}\r\n";
        std::fs::write(&settings, source).unwrap();
        std::fs::write(&unread, source).unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let settings_uri = Url::from_file_path(&settings).unwrap();
        let unread_uri = Url::from_file_path(&unread).unwrap();
        let sources = BTreeSet::from([main_uri.clone(), settings_uri.clone()]);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&main_uri));
        let item = &diagnostics[&settings_uri][0];
        let start = source.find(" local,styel").unwrap() + 1;
        assert_eq!(
            item.range,
            Text::new(source.into()).range(start, start + "local,styel".len())
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"file":"local,styel.bst"}))
        );

        let mut sources = sources;
        sources.insert(unread_uri);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&settings_uri));
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(&unread, r"\bibliographystyle{\chosen}").unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
    }

    #[test]
    fn unavailable_oversized_or_truncated_style_sources_do_not_create_false_unique_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let other = root.join("other.tex");
        std::fs::write(&main, "Main.\n").unwrap();
        std::fs::write(&settings, r"\bibliographystyle{local,styel}").unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let sources = [main.clone(), settings, other.clone()]
            .into_iter()
            .map(|path| Url::from_file_path(path).unwrap())
            .collect();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(&main, r"\bibliographystyle{local,styel}").unwrap();
        let log = format!(
            "I couldn't open style file local,styel.bst\n---line 2 of file main.aux\n{}\n",
            crate::output::TRUNCATED
        );
        let diagnostics = parse_bibliography(&main, &log);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
    }
}

#[cfg(test)]
mod bibliography_database_locations {
    use super::*;

    fn missing(root: &Path) -> BTreeMap<Url, Vec<Diagnostic>> {
        parse_bibliography(
            root,
            "I couldn't open database file reffs.bib\n---line 3 of file main.aux\n",
        )
    }

    #[test]
    fn databases_use_confirmed_sources_including_local_packages_and_reject_ambiguity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, r"\usepackage{settings}").unwrap();
        let source = "\\bibliography{first,% before\r\n re ffs.bib% after\r\n,second}";
        let settings = root.join("settings.sty");
        let unread = root.join("unread.tex");
        std::fs::write(&settings, source).unwrap();
        std::fs::write(&unread, source).unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let settings_uri = Url::from_file_path(&settings).unwrap();
        let unread_uri = Url::from_file_path(&unread).unwrap();
        let mut sources = BTreeSet::from([main_uri.clone(), settings_uri.clone()]);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&main_uri));
        let item = &diagnostics[&settings_uri][0];
        let start = source.find("re ffs").unwrap();
        assert_eq!(
            item.range,
            Text::new(source.into()).range(start, start + "re ffs.bib".len())
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"file":"reffs.bib","bibliographyCommand":"bibliography"}))
        );
        sources.insert(unread_uri);
        for source in [
            source,
            r"\nobibliography{\chosen}",
            r"\renewcommand{\bibliography}[1]{}",
        ] {
            std::fs::write(&unread, source).unwrap();
            let mut diagnostics = missing(&main);
            locate_missing_bibliography_databases(&sources, &mut diagnostics);
            assert_eq!(
                diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
                true
            );
        }
    }

    #[test]
    fn incomplete_database_sources_keep_unresolved_errors() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let other = root.join("other.tex");
        std::fs::write(&main, "Main.\n").unwrap();
        std::fs::write(&settings, r"\bibliography{reffs}").unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let sources = [&main, &settings, &other]
            .into_iter()
            .map(|path| Url::from_file_path(path).unwrap())
            .collect();
        let assert_unresolved = |diagnostics: &BTreeMap<Url, Vec<Diagnostic>>| {
            assert_eq!(
                diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
                true
            )
        };
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert_unresolved(&diagnostics);
        std::fs::write(&other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert_unresolved(&diagnostics);
        std::fs::write(&main, r"\bibliography{reffs}").unwrap();
        let log = format!(
            "I couldn't open database file reffs.bib\n---line 3 of file main.aux\n{}\n",
            crate::output::TRUNCATED
        );
        assert_unresolved(&parse_bibliography(&main, &log));
    }

    #[test]
    fn style_and_database_uncertainty_is_independent() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let log="I couldn't open database file reffs.bib\n---line 3 of file main.aux\nI couldn't open style file styel.bst\n---line 2 of file main.aux\n";
        for (source, precise, unresolved) in [
            (
                r"\bibliographystyle{styel}\bibliography{\chosen}",
                "missing-bibliography-style",
                "missing-bibliography-database",
            ),
            (
                r"\bibliographystyle{\chosen}\bibliography{reffs}",
                "missing-bibliography-database",
                "missing-bibliography-style",
            ),
        ] {
            std::fs::write(&main, source).unwrap();
            let diagnostics = parse_bibliography(&main, log);
            let find = |code: &str| {
                diagnostics
                    .values()
                    .flatten()
                    .find(|item| item.code == Some(NumberOrString::String(code.into())))
                    .unwrap()
            };
            assert!(!unresolved_location(find(precise)));
            assert!(unresolved_location(find(unresolved)));
        }
    }
}

#[cfg(test)]
mod latexmk_database_warnings {
    use super::*;

    fn section(found: &[&str], missing: &[&str]) -> String {
        let mut output = "Latexmk: Found bibliography file(s):\n".to_string();
        for name in found {
            output.push_str(&format!("  {name}\n"));
        }
        if !missing.is_empty() {
            output.push_str("Bib file(s) not found in search path:\n");
            for name in missing {
                output.push_str(&format!("  {name}\n"));
            }
        }
        output.push_str("Latexmk: All targets are up-to-date\n");
        output
    }
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let log = directory.path().join("out (local)");
        std::fs::create_dir(&log).unwrap();
        let log = log.join("main's.log");
        std::fs::write(
            &root,
            "% 😀 before\r\n\\bibliography{first, re ffs.bib,second}\r\n",
        )
        .unwrap();
        std::fs::write(&log, "Current TeX log.\n").unwrap();
        (directory, root, log)
    }
    fn inspect(log: &str, tail: &str) -> String {
        format!("Transcript written on {log}.\nLatexmk: Getting log file '{log}'\nLatexmk: Examining '{log}'\n{tail}")
    }

    #[test]
    fn final_driver_lists_create_bounded_warnings_and_support_exact_source_lookup() {
        let (_directory, root, log) = setup();
        let tail = section(
            &["first.bib", "reffs.bib", "second.bib"],
            &["reffs.bib", "reffs.bib"],
        );
        let with_blank_line = tail.replace("\nLatexmk: All targets", "\n\nLatexmk: All targets");
        assert!(!parse_latexmk_missing_databases(
            &root,
            &log,
            &inspect("out (local)/main's.log", &with_blank_line)
        )
        .is_empty());
        for printed in [
            "out (local)/main's.log".to_string(),
            log.display().to_string(),
        ] {
            let mut diagnostics =
                parse_latexmk_missing_databases(&root, &log, &inspect(&printed, &tail));
            let uri = Url::from_file_path(&root).unwrap();
            assert_eq!(diagnostics[&uri].len(), 1);
            let item = &diagnostics[&uri][0];
            assert_eq!(
                item.message,
                "Latexmk: Bibliography database not found: reffs.bib"
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert!(unresolved_location(item));
            locate_missing_bibliography_databases(&BTreeSet::from([uri.clone()]), &mut diagnostics);
            let item = &diagnostics[&uri][0];
            let source = std::fs::read_to_string(&root).unwrap();
            let start = source.find("re ffs.bib").unwrap();
            assert_eq!(
                item.range,
                Text::new(source).range(start, start + "re ffs.bib".len())
            );
            assert_eq!(
                item.data,
                Some(serde_json::json!({"file":"reffs.bib","bibliographyCommand":"bibliography"}))
            );
        }
    }

    #[test]
    fn earlier_passes_user_messages_other_jobs_and_child_tool_output_do_not_supply_warnings() {
        let (_directory, root, log) = setup();
        let missing = section(&["reffs.bib"], &["reffs.bib"]);
        let recovered = section(&["./refs.bib"], &[]);
        let marker = inspect("out (local)/main's.log", "");
        for output in [
            missing.clone(),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib"),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\n"),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\nUnexpected tool output\n"),
            format!("{missing}{marker}Latexmk: All targets are up-to-date\n"),
            format!("{marker}{missing}{marker}{recovered}"),
            format!("{marker}{missing}{recovered}"),
            inspect("other.log",&missing),
            format!("{marker}Latexmk: Getting log file 'other.log'\n{missing}"),
            format!("{marker}Latexmk: applying rule 'bibtex main'...\n{missing}"),
            format!("{marker}Running 'unrelated compiler'\n{missing}"),
            format!("{marker}Latexmk: Bibliography file(s) from .bcf file:\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\n"),
            format!("{marker}Bib file(s) not found in search path:\n  reffs.bib\n"),
        ] {
            assert!(parse_latexmk_missing_databases(&root,&log,&output).is_empty(),"{output}");
        }
    }

    #[test]
    fn incomplete_invalid_or_unconfirmed_driver_names_do_not_create_partial_warnings() {
        let (_directory, root, log) = setup();
        for (found, missing) in [
            (vec!["reffs.bib"], vec!["other.bib"]),
            (
                vec!["reffs.bib", "wrong.tex"],
                vec!["reffs.bib", "wrong.tex"],
            ),
            (
                vec!["reffs.bib", "spaced name.bib"],
                vec!["reffs.bib", "spaced name.bib"],
            ),
            (
                vec!["reffs.bib", r"\chosen.bib"],
                vec!["reffs.bib", r"\chosen.bib"],
            ),
            (vec![".bib"], vec![".bib"]),
            (vec![], vec!["reffs.bib"]),
        ] {
            let output = inspect("out (local)/main's.log", &section(&found, &missing));
            assert!(
                parse_latexmk_missing_databases(&root, &log, &output).is_empty(),
                "{output}"
            );
        }
        let output = inspect(
            "out (local)/main's.log",
            &section(&["reffs.bib"], &["reffs.bib"]),
        );
        let truncated = format!("{output}{}\n", crate::output::TRUNCATED);
        assert!(parse_latexmk_missing_databases(&root, &log, &truncated).is_empty());
        std::fs::remove_file(&log).unwrap();
        assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
    }

    #[test]
    fn driver_database_lists_reject_excessive_counts_and_names() {
        let (_directory, root, log) = setup();
        for (count, limit) in [(65, 64), (4097, 0)] {
            let names: Vec<_> = (0..count).map(|n| format!("refs{n}.bib")).collect();
            let found: Vec<_> = names.iter().map(String::as_str).collect();
            let missing = if limit == 0 { &found[..1] } else { &found[..] };
            let output = inspect("out (local)/main's.log", &section(&found, missing));
            assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
        }
        let name = format!("{}.bib", "r".repeat(8193));
        let output = inspect("out (local)/main's.log", &section(&[&name], &[&name]));
        assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
    }
}

#[cfg(test)]
mod project_color_tests {
    use super::*;

    const HELP: &str = "See the xcolor package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\n";
    const COUNTER_HELP: &str = "See the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...\n\n";

    fn fixture(definitions: &str, call: &str) -> (tempfile::TempDir, PathBuf, String) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("main.tex");
        let source = format!("\\usepackage{{palette}}\n{call}\n");
        std::fs::write(&root, &source).unwrap();
        std::fs::write(temp.path().join("palette.sty"), definitions).unwrap();
        (temp, root, source)
    }

    fn log(call: &str, model: bool, before: &str, after: &str) -> String {
        let message = if model {
            "Undefined color model `rbg'."
        } else {
            "Undefined color `accnet'."
        };
        format!("(./main.tex\n{before}\n./main.tex:2: Package xcolor Error: {message}\n{HELP}l.2 {call}\n{after}\n)\n")
    }

    fn error(root: &Path, log: &str, project: &ProjectSources) -> Diagnostic {
        parse_with_project_sources(root, log, project).0[&Url::from_file_path(root).unwrap()]
            .iter()
            .find(|d| d.message.starts_with("Package xcolor Error:"))
            .unwrap()
            .clone()
    }

    #[test]
    fn loaded_saved_dependencies_supply_arities_and_alias_target_grammars() {
        for (definitions, prefix) in [
            (r"\newcommand{\bridge}{}", r"\bridge"),
            (r"\newcommand{\done}{}\def\bridge{\done}", r"\bridge"),
            (
                r"\NewDocumentCommand{\done}{}{}\NewDocumentCommand{\bridge}{}{\done}",
                r"\bridge",
            ),
            (
                r"\newcommand{\done}[1]{}\def\bridge{\done}",
                r"\bridge{Unused}",
            ),
            (
                r"\newcommand{\done}[1][default]{}\def\bridge{\done}",
                r"\bridge[note]",
            ),
            (
                r"\NewDocumentCommand{\done}{s t+ O{default} m}{}\def\bridge{\done}",
                r"\bridge*+[note]{Unused}",
            ),
            (
                r"\NewDocumentCommand{\done}{d() m}{}\def\bridge{\done}",
                r"\bridge(note){Unused}",
            ),
        ] {
            for model in [false, true] {
                let call = format!(
                    "{prefix}{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                let (_temp, root, source) = fixture(definitions, &call);
                let project = ProjectSources::capture(&root);
                let log = log(&call, model, "(./palette.sty)", "");
                let diagnostic = error(&root, &log, &project);
                let name = if model { "rbg" } else { "accnet" };
                let start = source.find(name).unwrap();
                assert_eq!(
                    diagnostic.range,
                    Text::new(source).range(start, start + name.len()),
                    "{definitions}/{call}"
                );
                assert_eq!(
                    diagnostic.code,
                    Some(NumberOrString::String(
                        if model {
                            "undefined-color-model"
                        } else {
                            "undefined-color"
                        }
                        .into()
                    ))
                );
            }
        }
    }

    #[test]
    fn repeated_color_and_model_errors_follow_changes_in_the_loaded_macro_scope() {
        let color = r"\bridge\textcolor{accnet}{MarkerUnique}";
        let model = r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}";
        let (temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let calls = [color, model, color, model, r"\input{later}", color, model];
        let source = format!("\\usepackage{{palette}}\n{}\n", calls.join("\n"));
        std::fs::write(&root, &source).unwrap();
        std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[3]{}").unwrap();
        let project = ProjectSources::capture(&root);
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for (index, call) in calls.iter().enumerate() {
            if index == 4 {
                log.push_str("(./later.tex)\n");
                continue;
            }
            let line = index + 2;
            let name = if *call == color {
                "color `accnet'"
            } else {
                "color model `rbg'"
            };
            log.push_str(&format!("./main.tex:{line}: Package xcolor Error: Undefined {name}.\n{HELP}l.{line} {call}\n"));
        }
        log.push_str(")\n");
        let result = parse_with_project_sources(&root, &log, &project).0;
        let diagnostics = &result[&Url::from_file_path(&root).unwrap()];
        assert_eq!(diagnostics.len(), 6);
        let text = Text::new(source);
        for diagnostic in diagnostics {
            if diagnostic.range.start.line < 5 {
                let name = if diagnostic.range.start.line % 2 == 1 {
                    "accnet"
                } else {
                    "rbg"
                };
                let line_start = text
                    .offset(Position::new(diagnostic.range.start.line, 0))
                    .unwrap();
                let start = line_start + text.source[line_start..].find(name).unwrap();
                assert_eq!(diagnostic.range, text.range(start, start + name.len()));
                assert!(diagnostic.code.is_some());
                let sources = diagnostic.data.as_ref().unwrap()["macroSources"]
                    .as_array()
                    .unwrap();
                assert_eq!(sources.len(), 2);
            } else {
                assert!(
                    diagnostic.code.is_none() && diagnostic.data.is_none(),
                    "{diagnostic:?}"
                );
            }
        }
    }

    #[test]
    fn project_counter_errors_require_a_complete_error_time_macro_scope() {
        let call = r"\bridge\setcounter{equatoin}{7}";
        for case in [
            "direct",
            "callback",
            "not-loaded",
            "later",
            "displayed",
            "dynamic",
        ] {
            let declarations = if case == "callback" {
                r"\newcommand{\done}[3]{}\def\bridge{\done}"
            } else {
                r"\newcommand{\done}{}\def\bridge{\done}"
            };
            let (temp, root, _) = fixture(declarations, call);
            let prefix = match case {
                "displayed" => r"\usepackage{palette}\lstinputlisting{later.tex}",
                "dynamic" => r"\usepackage{palette}\input{\filename}",
                _ => r"\usepackage{palette}",
            };
            let source = format!("{prefix}\n{call}\n\\input{{later}}\n");
            std::fs::write(&root, &source).unwrap();
            std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[3]{}").unwrap();
            // A display-only file must not also have an explicit source role.
            if case == "displayed" {
                std::fs::write(&root, format!("{prefix}\n{call}\n")).unwrap();
            }
            let project = ProjectSources::capture(&root);
            let before = if case == "not-loaded" {
                ""
            } else {
                "(./palette.sty)"
            };
            let before = format!(
                "{before}{}",
                if case == "displayed" {
                    "\n(./later.tex)"
                } else {
                    ""
                }
            );
            let log = format!("(./main.tex\n{before}\n./main.tex:2: LaTeX Error: No counter 'equatoin' defined.\n{COUNTER_HELP}l.2 {call}\n(./later.tex)\n)\n");
            let result = parse_with_project_sources(&root, &log, &project).0;
            let error = &result[&Url::from_file_path(&root).unwrap()][0];
            let precise = matches!(case, "direct" | "later" | "displayed");
            assert_eq!(error.code.is_some(), precise, "{case}: {error:?}");
            if precise {
                assert_eq!(error.data.as_ref().unwrap()["counter"], "equatoin");
                let sources: Vec<Url> =
                    serde_json::from_value(error.data.as_ref().unwrap()["macroSources"].clone())
                        .unwrap();
                assert_eq!(sources.len(), 2);
                assert!(
                    !sources.contains(&Url::from_file_path(temp.path().join("later.tex")).unwrap())
                );
            } else {
                assert!(error.data.is_none(), "{case}: {error:?}");
            }
        }
    }

    #[test]
    fn repeated_color_model_and_counter_errors_keep_separate_call_analyses() {
        let calls = [
            (
                r"\bridge\textcolor{accnet}{MarkerUnique}",
                "Package xcolor Error: Undefined color `accnet'.",
                "accnet",
            ),
            (
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}",
                "Package xcolor Error: Undefined color model `rbg'.",
                "rbg",
            ),
            (
                r"\bridge\setcounter{equatoin}{7}",
                "LaTeX Error: No counter 'equatoin' defined.",
                "equatoin",
            ),
        ];
        let (_temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let mut source = "\\usepackage{palette}\n".to_owned();
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for index in 0..9 {
            let (call, message, _) = calls[index % 3];
            let line = index + 2;
            let help = if index % 3 == 2 { COUNTER_HELP } else { HELP };
            source.push_str(&format!("{call}\n"));
            log.push_str(&format!(
                "./main.tex:{line}: {message}\n{help}l.{line} {call}\n"
            ));
        }
        log.push_str(")\n");
        std::fs::write(&root, &source).unwrap();
        let project = ProjectSources::capture(&root);
        let result = parse_with_project_sources(&root, &log, &project).0;
        let errors = &result[&Url::from_file_path(root).unwrap()];
        assert_eq!(errors.len(), 9);
        let text = Text::new(source);
        for (index, error) in errors.iter().enumerate() {
            let (_, _, name) = calls[index % 3];
            let line_start = text.offset(Position::new(index as u32 + 1, 0)).unwrap();
            let start = line_start + text.source[line_start..].find(name).unwrap();
            assert_eq!(error.range, text.range(start, start + name.len()));
            assert!(error.code.is_some(), "{error:?}");
        }
    }

    #[test]
    fn project_environment_errors_require_error_time_macro_sources() {
        for command in ["begin", "end"] {
            let call = format!(r"\bridge\{command}{{algin}}");
            let message = if command == "begin" {
                "LaTeX Error: Environment algin undefined."
            } else {
                r"LaTeX Error: \begin{equation} ended by \end{algin}."
            };
            for case in [
                "direct",
                "callback",
                "not-loaded",
                "later",
                "displayed",
                "dynamic",
            ] {
                let declarations = if case == "callback" {
                    r"\newcommand{\done}[2]{}\def\bridge{\done}"
                } else {
                    r"\newcommand{\done}{}\def\bridge{\done}"
                };
                let (temp, root, _) = fixture(declarations, &call);
                let prefix = match case {
                    "displayed" => r"\usepackage{palette}\lstinputlisting{later.tex}",
                    "dynamic" => r"\usepackage{palette}\input{\filename}",
                    _ => r"\usepackage{palette}",
                };
                let source = if case == "displayed" {
                    format!("{prefix}\n{call}\n")
                } else {
                    format!("{prefix}\n{call}\n\\input{{later}}\n")
                };
                std::fs::write(&root, source).unwrap();
                std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[2]{}")
                    .unwrap();
                let project = ProjectSources::capture(&root);
                let before = if case == "not-loaded" {
                    ""
                } else {
                    "(./palette.sty)"
                };
                let display = if case == "displayed" {
                    "(./later.tex)"
                } else {
                    ""
                };
                let log = format!("(./main.tex\n{before}\n{display}\n./main.tex:2: {message}\n{COUNTER_HELP}l.2 {call}\n(./later.tex)\n)\n");
                let result = parse_with_project_sources(&root, &log, &project).0;
                let error = &result[&Url::from_file_path(&root).unwrap()][0];
                let precise = matches!(case, "direct" | "later" | "displayed");
                assert_eq!(error.code.is_some(), precise, "{command}/{case}: {error:?}");
                if precise {
                    let sources: Vec<Url> = serde_json::from_value(
                        error.data.as_ref().unwrap()["macroSources"].clone(),
                    )
                    .unwrap();
                    assert_eq!(sources.len(), 2);
                    assert!(!sources
                        .contains(&Url::from_file_path(temp.path().join("later.tex")).unwrap()));
                } else {
                    assert!(error.data.is_none(), "{command}/{case}: {error:?}");
                }
            }
        }
    }

    #[test]
    fn interleaved_invocation_errors_keep_five_independent_call_caches() {
        let calls = [
            (
                r"\bridge\textcolor{accnet}{MarkerUnique}",
                "Package xcolor Error: Undefined color `accnet'.",
                "accnet",
            ),
            (
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}",
                "Package xcolor Error: Undefined color model `rbg'.",
                "rbg",
            ),
            (
                r"\bridge\setcounter{equatoin}{7}",
                "LaTeX Error: No counter 'equatoin' defined.",
                "equatoin",
            ),
            (
                r"\bridge\begin{panle}",
                "LaTeX Error: Environment panle undefined.",
                "panle",
            ),
            (
                r"\bridge\end{algin}",
                r"LaTeX Error: \begin{document} ended by \end{algin}.",
                "algin",
            ),
        ];
        let (_temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let mut source = "\\usepackage{palette}\n".to_owned();
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for index in 0..15 {
            let (call, message, _) = calls[index % 5];
            let line = index + 2;
            let help = if index % 5 < 2 { HELP } else { COUNTER_HELP };
            source.push_str(&format!("{call}\n"));
            log.push_str(&format!(
                "./main.tex:{line}: {message}\n{help}l.{line} {call}\n"
            ));
        }
        log.push_str(")\n");
        std::fs::write(&root, &source).unwrap();
        let project = ProjectSources::capture(&root);
        let result = parse_with_project_sources(&root, &log, &project).0;
        let errors = &result[&Url::from_file_path(root).unwrap()];
        assert_eq!(errors.len(), 15);
        let text = Text::new(source);
        for (index, error) in errors.iter().enumerate() {
            let (_, _, name) = calls[index % 5];
            let line_start = text.offset(Position::new(index as u32 + 1, 0)).unwrap();
            let start = line_start + text.source[line_start..].find(name).unwrap();
            assert_eq!(error.range, text.range(start, start + name.len()));
            assert!(error.code.is_some(), "{error:?}");
        }
    }

    #[test]
    fn project_file_errors_require_macros_loaded_before_the_failure() {
        for command in ["input", "usepackage"] {
            let call = format!(r"\bridge\{command}{{palete}}\relax");
            let echo = if command == "input" {
                call.strip_suffix(r"\relax").unwrap()
            } else {
                &call
            };
            let extension = if command == "input" { "tex" } else { "sty" };
            for case in [
                "direct",
                "callback",
                "not-loaded",
                "later",
                "displayed",
                "dynamic",
                "dynamic-definition",
            ] {
                let declarations = if case == "callback" {
                    r"\newcommand{\done}[2]{}\def\bridge{\done}"
                } else {
                    r"\newcommand{\done}{}\def\bridge{\done}"
                };
                let (temp, root, _) = fixture(declarations, &call);
                let prefix = match case {
                    "displayed" => r"\usepackage{palette}\lstinputlisting{later.tex}",
                    "dynamic" => r"\usepackage{palette}\input{\filename}",
                    "dynamic-definition" => {
                        r"\usepackage{palette}\newcommand{\unused}[1]{\input{#1}}"
                    }
                    _ => r"\usepackage{palette}",
                };
                let source = if case == "displayed" {
                    format!("{prefix}\n{call}\n")
                } else {
                    format!("{prefix}\n{call}\n\\input{{later}}\n")
                };
                std::fs::write(&root, source).unwrap();
                std::fs::write(temp.path().join("later.tex"), r"\renewcommand{\done}[2]{}")
                    .unwrap();
                let project = ProjectSources::capture(&root);
                let before = if case == "not-loaded" {
                    ""
                } else {
                    "(./palette.sty)"
                };
                let display = if case == "displayed" {
                    "(./later.tex)"
                } else {
                    ""
                };
                let log = format!("(./main.tex\n{before}\n{display}\n! LaTeX Error: File `palete.{extension}' not found.\n./main.tex:2: Emergency stop.\n<read *>\nl.2 {echo}\n(./later.tex)\n)\n");
                let result = parse_with_project_sources(&root, &log, &project).0;
                let error = result[&Url::from_file_path(&root).unwrap()]
                    .iter()
                    .find(|e| e.message.contains("not found"))
                    .unwrap();
                let precise = matches!(case, "direct" | "later" | "displayed");
                assert_eq!(error.code.is_some(), precise, "{command}/{case}: {error:?}");
                if precise {
                    let sources: Vec<Url> = serde_json::from_value(
                        error.data.as_ref().unwrap()["macroSources"].clone(),
                    )
                    .unwrap();
                    assert_eq!(sources.len(), 2);
                    assert!(!sources
                        .contains(&Url::from_file_path(temp.path().join("later.tex")).unwrap()));
                } else {
                    assert!(error.data.is_none());
                }
            }
        }
    }

    #[test]
    fn file_input_and_loader_errors_share_only_the_file_call_cache() {
        let calls = [
            (
                r"\bridge\input{palete}",
                "LaTeX Error: File `palete.tex' not found.",
                "palete",
            ),
            (
                r"\bridge\textcolor{accnet}{MarkerUnique}",
                "Package xcolor Error: Undefined color `accnet'.",
                "accnet",
            ),
            (
                r"\bridge\usepackage{palete}\relax",
                "LaTeX Error: File `palete.sty' not found.",
                "palete",
            ),
            (
                r"\bridge\setcounter{equatoin}{7}",
                "LaTeX Error: No counter 'equatoin' defined.",
                "equatoin",
            ),
            (
                r"\bridge\begin{panle}",
                "LaTeX Error: Environment panle undefined.",
                "panle",
            ),
            (
                r"\bridge\end{algin}",
                r"LaTeX Error: \begin{document} ended by \end{algin}.",
                "algin",
            ),
            (
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}",
                "Package xcolor Error: Undefined color model `rbg'.",
                "rbg",
            ),
            (
                r"\bridge\deflength{\measuer}{1pt}",
                "Package etoolbox Error: Length '\\measuer' undefined.",
                r"\measuer",
            ),
        ];
        let (_temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        let mut source = "\\usepackage{palette}\n".to_owned();
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for index in 0..calls.len() * 2 {
            let (call, message, _) = calls[index % calls.len()];
            let line = index + 2;
            source.push_str(&format!("{call}\n"));
            if matches!(index % calls.len(), 0 | 2) {
                log.push_str(&format!(
                    "! {message}\n./main.tex:{line}: Emergency stop.\n<read *>\nl.{line} {call}\n"
                ));
            } else {
                let help = if matches!(index % calls.len(), 1 | 6) {
                    HELP
                } else {
                    COUNTER_HELP
                };
                log.push_str(&format!(
                    "./main.tex:{line}: {message}\n{help}l.{line} {call}\n"
                ));
            }
        }
        log.push_str(")\n");
        std::fs::write(&root, &source).unwrap();
        let project = ProjectSources::capture(&root);
        let result = parse_with_project_sources(&root, &log, &project).0;
        let errors: Vec<_> = result[&Url::from_file_path(root).unwrap()]
            .iter()
            .filter(|e| e.message != "Emergency stop.")
            .collect();
        assert_eq!(errors.len(), calls.len() * 2);
        let text = Text::new(source);
        for (index, error) in errors.iter().enumerate() {
            let (_, _, name) = calls[index % calls.len()];
            let line_start = text.offset(Position::new(index as u32 + 1, 0)).unwrap();
            let start = line_start + text.source[line_start..].find(name).unwrap();
            assert_eq!(
                error.range,
                text.range(start, start + name.len()),
                "{index}: {error:?}"
            );
            assert!(error.code.is_some(), "{error:?}");
        }
    }

    #[test]
    fn returning_to_a_loaded_file_keeps_each_documents_color_ranges() {
        let (temp, root, _) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", "");
        std::fs::write(
            &root,
            r"\usepackage{palette}\input{first}\input{second}\input{first}",
        )
        .unwrap();
        let color = r"\bridge\textcolor{accnet}{MarkerUnique}";
        let model = r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}";
        let mut documents = BTreeMap::new();
        for (name, prefix) in [("first", ""), ("second", r"\typeout{😀} ")] {
            let source = format!("{prefix}{color}\n{prefix}{model}\n");
            let path = temp.path().join(format!("{name}.tex"));
            std::fs::write(&path, &source).unwrap();
            documents.insert(name, Text::new(source));
        }
        let project = ProjectSources::capture(&root);
        let mut log = "(./main.tex\n(./palette.sty)\n".to_owned();
        for name in ["first", "second", "first"] {
            log.push_str(&format!("(./{name}.tex\n"));
            for (index, (call, message)) in
                [(color, "color `accnet'"), (model, "color model `rbg'")]
                    .into_iter()
                    .enumerate()
            {
                let prefix = if name == "first" {
                    ""
                } else {
                    r"\typeout{😀} "
                };
                let line = index + 1;
                log.push_str(&format!("./{name}.tex:{line}: Package xcolor Error: Undefined {message}.\n{HELP}l.{line} {prefix}{call}\n"));
            }
            log.push_str(")\n");
        }
        log.push_str(")\n");
        let result = parse_with_project_sources(&root, &log, &project).0;
        for (name, expected_count) in [("first", 4), ("second", 2)] {
            let uri = Url::from_file_path(temp.path().join(format!("{name}.tex"))).unwrap();
            let diagnostics = &result[&uri];
            assert_eq!(diagnostics.len(), expected_count);
            let text = &documents[name];
            for diagnostic in diagnostics {
                let name = if diagnostic.range.start.line == 0 {
                    "accnet"
                } else {
                    "rbg"
                };
                let start = text.source.find(name).unwrap();
                assert_eq!(diagnostic.range, text.range(start, start + name.len()));
                assert!(diagnostic.code.is_some());
            }
        }
    }

    #[test]
    fn reused_analysis_rechecks_sources_and_rejects_a_different_emission_text() {
        let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
        for paused_parent in [false, true] {
            let (temp, root, original) = fixture(r"\newcommand{\done}{}\def\bridge{\done}", call);
            let child = temp.path().join("content.tex");
            std::fs::write(&child, call).unwrap();
            if paused_parent {
                std::fs::write(&root, r"\usepackage{palette}\input{content}").unwrap();
            }
            let project = ProjectSources::capture(&root);
            let mut analysis = ProjectAnalysis::new(&project);
            let root_uri = Url::from_file_path(&root).unwrap();
            let uri = if paused_parent {
                Url::from_file_path(&child).unwrap()
            } else {
                root_uri.clone()
            };
            let palette_uri = Url::from_file_path(temp.path().join("palette.sty")).unwrap();
            let loaded = [uri.clone(), palette_uri].into();
            let paused = if paused_parent {
                [root_uri].into()
            } else {
                BTreeSet::new()
            };
            let context = analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .unwrap()
                .unwrap();
            let source = if paused_parent { call } else { &original };
            let line = u32::from(!paused_parent);
            assert!(color_error_range(
                &Text::new(source.into()),
                line,
                call,
                "accnet",
                false,
                Some(&context.calls)
            )
            .is_some());
            assert!(color_error_range(
                &Text::new(format!("{source}\n% changed")),
                line,
                call,
                "accnet",
                false,
                Some(&context.calls)
            )
            .is_none());
            let saved_root = std::fs::read_to_string(&root).unwrap();
            std::fs::write(
                &root,
                format!("{saved_root}\n\\renewcommand{{\\done}}[3]{{}}"),
            )
            .unwrap();
            assert!(analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .is_err());
            std::fs::write(&root, saved_root).unwrap();
            assert!(analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .unwrap()
                .is_some());
            std::fs::remove_file(temp.path().join("palette.sty")).unwrap();
            assert!(analysis
                .context(&loaded, &paused, &uri, InvocationKind::Color)
                .is_err());
        }
    }

    #[test]
    fn callbacks_and_redefinitions_in_other_files_cannot_become_direct_color_sites() {
        for definitions in [
            r"\newcommand{\wrapped}[3]{}\def\bridge{\wrapped}",
            r"\newcommand{\bridge}{}\renewcommand{\bridge}[3]{}",
            r"\def\bridge{\missing}",
            r"\def\bridge{\bridge}",
            r"\newcommand{\bridge}{}\renewcommand{\textcolor}[2]{}",
        ] {
            for model in [false, true] {
                let call = format!(
                    r"\bridge{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                let (_temp, root, _) = fixture(definitions, &call);
                let project = ProjectSources::capture(&root);
                let diagnostic = error(&root, &log(&call, model, "(./palette.sty)", ""), &project);
                assert!(
                    diagnostic.code.is_none() && diagnostic.data.is_none(),
                    "{definitions}/{call}: {diagnostic:?}"
                );
            }
        }
    }

    #[test]
    fn source_graph_requires_prior_log_entry_and_ignores_unrelated_files() {
        for (before, after) in [("", ""), ("", "(./palette.sty)"), ("(./unrelated.sty)", "")] {
            let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
            let (temp, root, _) = fixture(r"\def\bridge{}", call);
            std::fs::write(temp.path().join("unrelated.sty"), r"\def\bridge{}").unwrap();
            let project = ProjectSources::capture(&root);
            let diagnostic = error(&root, &log(call, false, before, after), &project);
            assert!(
                diagnostic.code.is_none(),
                "{before}/{after}: {diagnostic:?}"
            );
        }
    }

    #[test]
    fn later_inputs_and_displayed_code_do_not_override_the_error_time_signature_scope() {
        for displayed in [false, true] {
            let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
            let (temp, root, source) = fixture(r"\def\bridge{}", call);
            let extra = temp.path().join("extra.tex");
            std::fs::write(&extra, r"\renewcommand{\bridge}[3]{}").unwrap();
            let source = if displayed {
                format!("\\usepackage{{palette}}\\lstinputlisting{{extra.tex}}\n{call}\n")
            } else {
                format!("{source}\\input{{extra}}\n")
            };
            std::fs::write(&root, source).unwrap();
            let project = ProjectSources::capture(&root);
            let before = if displayed {
                "(./palette.sty)\n(./extra.tex)"
            } else {
                "(./palette.sty)"
            };
            let after = if displayed { "" } else { "(./extra.tex)" };
            let log = log(call, false, before, after);
            let (diagnostics, loaded) = parse_with_project_sources(&root, &log, &project);
            let uri = Url::from_file_path(&root).unwrap();
            let diagnostic = &diagnostics[&uri][0];
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("undefined-color".into())),
                "{displayed}: {diagnostic:?}"
            );
            let sources: BTreeSet<Url> =
                serde_json::from_value(diagnostic.data.as_ref().unwrap()["macroSources"].clone())
                    .unwrap();
            let extra = Url::from_file_path(extra).unwrap();
            assert!(loaded.contains(&extra));
            assert!(!sources.contains(&extra));
            assert!(sources.contains(&uri));
            assert_eq!(sources.len(), 2);
        }
    }

    #[test]
    fn a_paused_parent_cannot_lend_future_definitions_to_a_child_error() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("main.tex");
        let class = temp.path().join("local.cls");
        let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
        std::fs::write(&root, r"\documentclass{local}\newcommand{\bridge}{}").unwrap();
        std::fs::write(&class, call).unwrap();
        let project = ProjectSources::capture(&root);
        let log = format!("(./main.tex\n(./local.cls\n./local.cls:1: Package xcolor Error: Undefined color `accnet'.\n{HELP}l.1 {call}\n)\n)\n");
        let result = parse_with_project_sources(&root, &log, &project).0;
        let diagnostic = &result[&Url::from_file_path(class).unwrap()][0];
        assert!(
            diagnostic.code.is_none() && diagnostic.data.is_none(),
            "{diagnostic:?}"
        );
    }

    #[test]
    fn paused_parent_declarations_invalidate_existing_signatures_and_alias_targets() {
        for model in [false, true] {
            for (parent, prefix, precise) in [
                (r"\newenvironment{bridge}[3]{}{}", r"\bridge", false),
                (r"\newenvironment{done}[3]{}{}", r"\bridge", false),
                (r"\newenvironment{unrelated}[3]{}{}", r"\bridge", true),
                (r"\renewcommand{\bridge}[3]{}", r"\bridge", false),
                (r"\renewcommand{\done}[3]{}", r"\bridge", false),
                (r"\def\relax#1#2#3{}", r"\relax", false),
                (r"\renewcommand{\textcolor}[2]{}", "", false),
                (r"\newcommand{\unrelated}[3]{}", r"\bridge", true),
            ] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().join("main.tex");
                let child = temp.path().join("content.tex");
                let palette = temp.path().join("palette.sty");
                let call = format!(
                    "{prefix}{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                std::fs::write(
                    &root,
                    format!("\\usepackage{{palette}}{parent}\\input{{content}}"),
                )
                .unwrap();
                std::fs::write(&palette, r"\newcommand{\done}{}\def\bridge{\done}").unwrap();
                std::fs::write(&child, &call).unwrap();
                let project = ProjectSources::capture(&root);
                let message = if model {
                    "Undefined color model `rbg'."
                } else {
                    "Undefined color `accnet'."
                };
                let log = format!("(./main.tex\n(./palette.sty)\n(./content.tex\n./content.tex:1: Package xcolor Error: {message}\n{HELP}l.1 {call}\n)\n)\n");
                let result = parse_with_project_sources(&root, &log, &project).0;
                let diagnostic = &result[&Url::from_file_path(&child).unwrap()][0];
                assert_eq!(
                    diagnostic.code.is_some(),
                    precise,
                    "{parent}: {diagnostic:?}"
                );
                if precise {
                    let data = diagnostic.data.as_ref().unwrap();
                    assert_eq!(
                        data["macroPausedSources"],
                        serde_json::json!([Url::from_file_path(&root).unwrap()])
                    );
                    // A changed paused source must also invalidate the snapshot.
                    std::fs::write(&root, "changed parent").unwrap();
                    let result = parse_with_project_sources(&root, &log, &project).0;
                    assert!(result[&Url::from_file_path(&child).unwrap()][0]
                        .code
                        .is_none());
                }
            }
        }
    }

    #[test]
    fn dynamically_loaded_children_cannot_borrow_an_incomplete_signature_set() {
        for model in [false, true] {
            let call = if model {
                r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
            } else {
                r"\bridge\textcolor{accnet}{MarkerUnique}"
            };
            let (temp, root, _) = fixture(r"\newcommand{\bridge}{}", "");
            let child = temp.path().join("dynamic.tex");
            std::fs::write(
                &root,
                r"\usepackage{palette}\def\filename{dynamic}\input{\filename}",
            )
            .unwrap();
            std::fs::write(&child, format!("\\renewcommand{{\\bridge}}[3]{{}}\n{call}")).unwrap();
            let project = ProjectSources::capture(&root);
            assert!(!project
                .documents
                .contains_key(&Url::from_file_path(&child).unwrap()));
            let message = if model {
                "Undefined color model `rbg'."
            } else {
                "Undefined color `accnet'."
            };
            let log = format!("(./main.tex\n(./palette.sty)\n(./dynamic.tex\n./dynamic.tex:2: Package xcolor Error: {message}\n{HELP}l.2 {call}\n)\n)\n");
            let result = parse_with_project_sources(&root, &log, &project).0;
            let diagnostic = &result[&Url::from_file_path(child).unwrap()][0];
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{diagnostic:?}"
            );
        }
    }

    #[test]
    fn dynamic_loads_can_change_signatures_after_returning_to_the_saved_parent() {
        for loader in [
            r"\input{\filename}",
            r"\InputIfFileExists{\filename}{}{}",
            r"\include{\filename}",
            r"\subfile{\filename}",
            r"\usepackage{palette,\filename}",
            r"\LoadClass{\filename}",
        ] {
            let call = r"\bridge\textcolor{accnet}{MarkerUnique}";
            let (temp, root, _) = fixture(r"\newcommand{\bridge}{}", call);
            std::fs::write(&root, format!("\\usepackage{{palette}}{loader}\n{call}\n")).unwrap();
            std::fs::write(
                temp.path().join("dynamic.tex"),
                r"\renewcommand{\bridge}[3]{}",
            )
            .unwrap();
            let project = ProjectSources::capture(&root);
            let diagnostic = error(
                &root,
                &log(call, false, "(./palette.sty)\n(./dynamic.tex)", ""),
                &project,
            );
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{loader}: {diagnostic:?}"
            );
        }
    }

    #[test]
    fn changed_or_deleted_dependency_invalidates_precise_color_attribution() {
        for model in [false, true] {
            for deleted in [false, true] {
                let call = format!(
                    r"\bridge{}",
                    if model {
                        r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}"
                    } else {
                        r"\textcolor{accnet}{MarkerUnique}"
                    }
                );
                let (temp, root, _) = fixture(r"\def\bridge{}", &call);
                let project = ProjectSources::capture(&root);
                if deleted {
                    std::fs::remove_file(temp.path().join("palette.sty")).unwrap();
                } else {
                    std::fs::write(
                        temp.path().join("palette.sty"),
                        r"\newcommand{\bridge}[3]{}",
                    )
                    .unwrap();
                }
                let diagnostic = error(&root, &log(&call, model, "(./palette.sty)", ""), &project);
                assert!(
                    diagnostic.code.is_none(),
                    "{model}/{deleted}: {diagnostic:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod indirect_file_error_tests {
    use super::*;

    #[test]
    fn indirect_input_errors_use_only_loaded_unchanged_alias_definitions() {
        for call in [
            r"\readpart{chapte}",
            r"\csuse{readpart}{chapte}",
            r"\csname readpart\endcsname{chapte}",
        ] {
            for case in ["loaded", "not-loaded", "changed", "shadowed"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path().join("main.tex");
                let package = directory.path().join("local.sty");
                std::fs::write(&root, format!("\\usepackage{{local}}\n{call}\n")).unwrap();
                std::fs::write(&package, r"\def\readpart{\input}").unwrap();
                if case == "shadowed" {
                    std::fs::write(
                        &root,
                        format!("\\usepackage{{local}}\\def\\input#1{{}}\n{call}\n"),
                    )
                    .unwrap();
                }
                let project = ProjectSources::capture(&root);
                if case == "changed" {
                    std::fs::write(&package, r"\def\readpart#1{}").unwrap();
                }
                let loaded = if case == "not-loaded" {
                    ""
                } else {
                    "(./local.sty)"
                };
                let log = format!("(./main.tex\n{loaded}\n! LaTeX Error: File `chapte.tex' not found.\n./main.tex:2: Emergency stop.\n<read *>\nl.2 {call}\n)");
                let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                let diagnostic = diagnostics[&Url::from_file_path(&root).unwrap()]
                    .iter()
                    .find(|d| d.message.contains("not found"))
                    .unwrap();
                assert_eq!(
                    diagnostic.code
                        == Some(tower_lsp::lsp_types::NumberOrString::String(
                            "missing-input".into()
                        )),
                    case == "loaded",
                    "{case}/{call}: {diagnostic:?}"
                );
                if case == "loaded" {
                    assert_eq!(
                        diagnostic.data.as_ref().unwrap()["macroSources"]
                            .as_array()
                            .unwrap()
                            .len(),
                        2
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod alias_load_snapshot_tests {
    use super::*;

    #[test]
    fn alias_declarations_do_not_mask_dynamic_or_expanded_input_definitions() {
        for (definition, dynamic) in [
            (r"\def\reader{\input}", false),
            (r"\def\reader{\include}", false),
            (r"\def\reader{\subfile}", true),
            (r"\def\reader{\InputIfFileExists}", true),
            (r"\def\reader{\usepackage}", false),
            (r"\def\reader{\RequirePackage}", false),
            (r"\def\reader{\RequirePackageWithOptions}", false),
            (r"\def\reader{\documentclass}", false),
            (r"\def\reader{\LoadClass}", false),
            (r"\def\reader{\LoadClassWithOptions}", false),
            (r"\def\reader{\documentclass}\reader{\filename}", true),
            (r"\edef\reader{\LoadClass}", true),
            (r"\newcommand{\reader}[1]{\LoadClass{#1}}", true),
            (r"\edef\reader{\usepackage}", true),
            (r"\xdef\reader{\RequirePackage}", true),
            (r"\newcommand{\reader}[1]{\usepackage{#1}}", true),
            (r"\csuse{usepackage}{\filename}", true),
            (r"\csname RequirePackage\endcsname{known,\filename}", true),
            (r"\def\reader{\usepackage}\reader{\filename}", true),
            (
                r"\def\reader{\RequirePackage}\reader{known,\filename}",
                true,
            ),
            (r"\csuse{input}{\filename}", true),
            (r"\csname input\endcsname{\filename}", true),
            (r"\def\reader{\input}\reader{\filename}", true),
            (r"\def\reader{\include}\reader{\filename}", true),
            (r"\gdef\reader{\input}", false),
            (r"\newcommand{\reader}{\input}", false),
            (r"\newcommand{\reader}[0]{\input}", false),
            (r"\NewDocumentCommand{\reader}{}{\input}", false),
            (r"\csdef{reader}{\input}", false),
            (r"\def\reader#1{\input}", true),
            (r"\newcommand{\reader}[1]{\input}", true),
            (r"\NewDocumentCommand{\reader}{m}{\input}", true),
            (r"\edef\reader{\input}", true),
            (r"\xdef\reader{\input}", true),
            (r"\csedef{reader}{\input}", true),
            (r"\csxdef{reader}{\input}", true),
            (r"\def\reader{\relax\input}", true),
            (r"\def\reader{\input{\filename}}", true),
            (r"\newcommand{\reader}[1]{\input{#1}}", true),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("main.tex");
            std::fs::write(&root, definition).unwrap();
            let snapshot = ProjectSources::capture(&root);
            assert_eq!(
                snapshot
                    .dynamic_sources
                    .contains(&Url::from_file_path(&root).unwrap()),
                dynamic,
                "{definition}; alias tokens: {:?}; commands: {:?}",
                crate::signature::file_load_alias_body_positions(
                    definition,
                    &snapshot.documents[&Url::from_file_path(&root).unwrap()].syntax
                ),
                snapshot.documents[&Url::from_file_path(&root).unwrap()]
                    .syntax
                    .commands
            );
        }
    }
}

#[cfg(test)]
mod indirect_include_warning_tests {
    use super::*;

    #[test]
    fn indirect_chapter_warnings_use_loaded_unchanged_aliases_and_record_their_scope() {
        for call in [
            r"\readpart{chapte}",
            r"\csuse{readpart}{chapte}",
            r"\csname readpart\endcsname{chapte}",
        ] {
            for case in ["loaded", "not-loaded", "changed", "shadowed"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path().join("main.tex");
                let package = directory.path().join("local.sty");
                let source = format!("\\usepackage{{local}}\n{call}\n");
                std::fs::write(
                    &root,
                    if case == "shadowed" {
                        format!("\\usepackage{{local}}\\def\\include#1{{}}\n{call}\n")
                    } else {
                        source
                    },
                )
                .unwrap();
                std::fs::write(&package, r"\newcommand{\readpart}{\include}").unwrap();
                let project = ProjectSources::capture(&root);
                if case == "changed" {
                    std::fs::write(&package, r"\def\readpart#1{}").unwrap();
                }
                let loaded = if case == "not-loaded" {
                    ""
                } else {
                    "(./local.sty)"
                };
                let log = format!("(./main.tex\n{loaded}\nNo file main.aux.\nNo file chapte.aux.\nNo file chapte.tex.\n)");
                let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                let warnings: Vec<_> = diagnostics
                    .values()
                    .flatten()
                    .filter(|d| d.code == Some(NumberOrString::String("missing-include".into())))
                    .collect();
                assert_eq!(
                    warnings.len(),
                    usize::from(case == "loaded"),
                    "{case}/{call}: {warnings:?}"
                );
                if case == "loaded" {
                    let data = warnings[0].data.as_ref().unwrap();
                    assert_eq!(data["file"], "chapte.tex");
                    assert_eq!(data["macroSources"].as_array().unwrap().len(), 2);
                    assert_eq!(warnings[0].severity, Some(DiagnosticSeverity::WARNING));
                }
            }
        }
    }

    #[test]
    fn paused_parent_aliases_do_not_certify_indirect_chapter_warnings_in_children() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        std::fs::write(&root, r"\def\readpart{\include}\input{child}").unwrap();
        std::fs::write(directory.path().join("child.tex"), r"\readpart{chapte}").unwrap();
        let project = ProjectSources::capture(&root);
        let log = "(./main.tex\n(./child.tex\nNo file chapte.tex.\n)\n)";
        let diagnostics = parse_with_project_sources(&root, log, &project).0;
        assert!(
            !diagnostics
                .values()
                .flatten()
                .any(|d| d.code == Some(NumberOrString::String("missing-include".into()))),
            "{diagnostics:?}"
        );
    }
}

#[cfg(test)]
mod indirect_package_scope_tests {
    use super::*;

    #[test]
    fn missing_packages_use_only_loaded_unchanged_indirect_dependencies() {
        for loader in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
            for call in [
                r"\loadpkg",
                r"\csuse{loadpkg}",
                r"\csname loadpkg\endcsname",
            ] {
                for case in ["loaded", "not-loaded", "changed", "shadowed", "dynamic"] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path().join("main.tex");
                    let package = directory.path().join("local.sty");
                    let source = format!(
                        "\\csname usepackage\\endcsname{{local}}\n{call}{{fixtuer}}\\relax\n"
                    );
                    std::fs::write(&root, &source).unwrap();
                    let declaration = format!("\\def\\loadpkg{{\\{loader}}}");
                    std::fs::write(
                        &package,
                        match case {
                            "shadowed" => format!("{declaration}\\def\\{loader}#1{{}}"),
                            "dynamic" => format!("{declaration}\\input{{\\filename}}"),
                            _ => declaration,
                        },
                    )
                    .unwrap();
                    let project = ProjectSources::capture(&root);
                    let root_uri = Url::from_file_path(&root).unwrap();
                    let package_uri = Url::from_file_path(&package).unwrap();
                    // Redefining usepackage in the very package discovered
                    // through it makes the dependency graph oscillate. That
                    // file must not enter the saved signature scope.
                    assert_eq!(
                        project.documents.contains_key(&package_uri),
                        case != "shadowed" || loader != "usepackage",
                        "{case}/{loader}/{call}"
                    );
                    if case == "changed" {
                        std::fs::write(&package, r"\def\loadpkg#1{}").unwrap();
                    }
                    let loaded = if case == "not-loaded" {
                        ""
                    } else {
                        "(./local.sty)"
                    };
                    let log = format!("(./main.tex\n{loaded}\n! LaTeX Error: File `fixtuer.sty' not found.\n! Emergency stop.\n<read *>\nl.2 {call}{{fixtuer}}\\relax\n");
                    let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                    let diagnostic = diagnostics[&root_uri]
                        .iter()
                        .find(|d| d.message.contains("not found"))
                        .unwrap();
                    assert_eq!(
                        diagnostic.code,
                        (case == "loaded")
                            .then(|| NumberOrString::String("missing-package".into())),
                        "{case}/{loader}/{call}: {diagnostic:?}"
                    );
                    if case == "loaded" {
                        let data = diagnostic.data.as_ref().unwrap();
                        assert_eq!(data["loaderCommand"], loader);
                        assert!(data["macroSources"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::json!(package_uri)));
                    }
                }
            }
        }
    }

    #[test]
    fn paused_parent_declarations_cannot_certify_a_child_package_alias() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let child = directory.path().join("content.tex");
        std::fs::write(&root, r"\def\loadpkg{\usepackage}\input{content}").unwrap();
        std::fs::write(&child, "\\loadpkg{fixtuer}\\relax\n").unwrap();
        let project = ProjectSources::capture(&root);
        let log = "(./main.tex (./content.tex\n! LaTeX Error: File `fixtuer.sty' not found.\n! Emergency stop.\n<read *>\nl.1 \\loadpkg{fixtuer}\\relax\n";
        let diagnostics = parse_with_project_sources(&root, log, &project).0;
        let diagnostic = diagnostics[&Url::from_file_path(&child).unwrap()]
            .iter()
            .find(|d| d.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{diagnostic:?}");
    }
}
#[cfg(test)]
mod indirect_class_scope_tests {
    use super::*;

    #[test]
    fn class_errors_use_only_loaded_unchanged_alias_definitions() {
        for loader in ["documentclass", "LoadClass", "LoadClassWithOptions"] {
            for call in [
                r"\loadclass",
                r"\csuse{loadclass}",
                r"\csname loadclass\endcsname",
            ] {
                for case in ["loaded", "not-loaded", "changed", "shadowed", "dynamic"] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path().join("main.tex");
                    let package = directory.path().join("definitions.sty");
                    let source =
                        format!("\\RequirePackage{{definitions}}\n{call}{{fixtuer}}\\relax\n");
                    std::fs::write(&root, &source).unwrap();
                    let declaration = format!("\\def\\loadclass{{\\{loader}}}");
                    std::fs::write(
                        &package,
                        match case {
                            "shadowed" => format!("{declaration}\\def\\{loader}#1{{}}"),
                            "dynamic" => format!("{declaration}\\input{{\\filename}}"),
                            _ => declaration,
                        },
                    )
                    .unwrap();
                    let project = ProjectSources::capture(&root);
                    let package_uri = Url::from_file_path(&package).unwrap();
                    assert!(project.documents.contains_key(&package_uri));
                    if case == "changed" {
                        std::fs::write(&package, r"\def\loadclass#1{}").unwrap();
                    }
                    let loaded = if case == "not-loaded" {
                        ""
                    } else {
                        "(./definitions.sty)"
                    };
                    let log=format!("(./main.tex\n{loaded}\n! LaTeX Error: File `fixtuer.cls' not found.\n! Emergency stop.\n<read *>\nl.2 {call}{{fixtuer}}\\relax\n");
                    let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                    let diagnostic = diagnostics[&Url::from_file_path(&root).unwrap()]
                        .iter()
                        .find(|d| d.message.contains("not found"))
                        .unwrap();
                    assert_eq!(
                        diagnostic.code,
                        (case == "loaded").then(|| NumberOrString::String("missing-class".into())),
                        "{case}/{loader}/{call}: {diagnostic:?}"
                    );
                    if case == "loaded" {
                        let data = diagnostic.data.as_ref().unwrap();
                        assert_eq!(data["loaderCommand"], loader);
                        assert!(data["macroSources"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::json!(package_uri)));
                    }
                }
            }
        }
    }

    #[test]
    fn paused_parent_declarations_do_not_certify_child_class_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let child = directory.path().join("content.tex");
        std::fs::write(&root, r"\def\loadclass{\documentclass}\input{content}").unwrap();
        std::fs::write(&child, "\\loadclass{fixtuer}\\relax\n").unwrap();
        let project = ProjectSources::capture(&root);
        let log="(./main.tex (./content.tex\n! LaTeX Error: File `fixtuer.cls' not found.\n! Emergency stop.\n<read *>\nl.1 \\loadclass{fixtuer}\\relax\n";
        let diagnostics = parse_with_project_sources(&root, log, &project).0;
        let diagnostic = diagnostics[&Url::from_file_path(&child).unwrap()]
            .iter()
            .find(|d| d.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{diagnostic:?}");
    }
}

#[cfg(test)]
mod indirect_option_clash_scope_tests {
    use super::*;

    fn error(echo: &str, line: usize) -> String {
        format!("! LaTeX Error: Option clash for package local.\n\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \nl.{line} {echo}\n                                                  \nThe package local has already been loaded with options:\n  [draft]\nThere has now been an attempt to load it with options\n  [final]\nAdding the global options:\n")
    }

    #[test]
    fn option_clashes_use_only_loaded_and_unchanged_alias_definitions() {
        for loader in ["usepackage", "RequirePackage"] {
            for call in [
                r"\loadpkg",
                r"\csuse{loadpkg}",
                r"\csname loadpkg\endcsname",
            ] {
                for case in [
                    "loaded",
                    "not-loaded",
                    "changed",
                    "root-changed",
                    "shadowed",
                    "dynamic",
                ] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path().join("main.tex");
                    let package = directory.path().join("definitions.sty");
                    let echo = format!("{call}[final]{{local}}\\relax");
                    std::fs::write(&root, format!("\\RequirePackage{{definitions}}\n{echo}\n"))
                        .unwrap();
                    let declaration = format!("\\def\\loadpkg{{\\{loader}}}");
                    std::fs::write(
                        &package,
                        match case {
                            "shadowed" => format!("{declaration}\\def\\{loader}#1{{}}"),
                            "dynamic" => format!("{declaration}\\input{{\\filename}}"),
                            _ => declaration,
                        },
                    )
                    .unwrap();
                    let project = ProjectSources::capture(&root);
                    if case == "changed" {
                        std::fs::write(&package, r"\def\loadpkg#1{}").unwrap();
                    }
                    if case == "root-changed" {
                        std::fs::write(
                            &root,
                            format!("\\RequirePackage{{definitions}}\n{echo}\n\\relax\n"),
                        )
                        .unwrap();
                    }
                    let loaded = if case == "not-loaded" {
                        ""
                    } else {
                        "(./definitions.sty)"
                    };
                    let log = format!("(./main.tex\n{loaded}\n{}", error(&echo, 2));
                    let diagnostics = parse_with_project_sources(&root, &log, &project).0;
                    let diagnostic = &diagnostics[&Url::from_file_path(&root).unwrap()][0];
                    assert_eq!(
                        diagnostic.code,
                        (case == "loaded")
                            .then(|| NumberOrString::String("package-option-clash".into())),
                        "{case}/{loader}/{call}: {diagnostic:?}"
                    );
                    if case == "loaded" {
                        let data = diagnostic.data.as_ref().unwrap();
                        assert_eq!(data["loaderCommand"], loader);
                        assert_eq!(data["loadedOptions"], "draft");
                        assert_eq!(data["requestedOptions"], "final");
                        assert!(data["macroSources"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::json!(Url::from_file_path(&package).unwrap())));
                    } else {
                        assert!(diagnostic
                            .data
                            .as_ref()
                            .is_none_or(|data| data.get("loadedOptions").is_none()));
                    }
                }
            }
        }
    }

    #[test]
    fn paused_parents_only_allow_independent_child_loader_aliases() {
        for conflict in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("main.tex");
            let child = directory.path().join("content.tex");
            std::fs::write(
                &root,
                if conflict {
                    r"\def\loadpkg{\usepackage}\input{content}"
                } else {
                    r"\input{content}"
                },
            )
            .unwrap();
            let echo = r"\loadpkg[final]{local}\relax";
            std::fs::write(&child, format!("\\def\\loadpkg{{\\usepackage}}\n{echo}\n")).unwrap();
            let project = ProjectSources::capture(&root);
            let log = format!("(./main.tex (./content.tex\n{}", error(echo, 2));
            let diagnostics = parse_with_project_sources(&root, &log, &project).0;
            let diagnostic = &diagnostics[&Url::from_file_path(&child).unwrap()][0];
            assert_eq!(
                diagnostic.code,
                (!conflict).then(|| NumberOrString::String("package-option-clash".into())),
                "{conflict}: {diagnostic:?}"
            );
            if !conflict {
                assert_eq!(
                    diagnostic.data.as_ref().unwrap()["macroPausedSources"],
                    serde_json::json!([Url::from_file_path(&root).unwrap()])
                );
            }
        }
    }
}
#[cfg(test)]
mod indirect_bibliography_error_scopes {
    use super::*;

    fn call(form: &str, command: &str) -> String {
        match form {
            "named" => format!("\\csuse{{{command}}}"),
            "literal" => format!("\\csname {command}\\endcsname"),
            "alias" => r"\loadbib".into(),
            "named-alias" => r"\csuse{loadbib}".into(),
            "literal-alias" => r"\csname loadbib\endcsname".into(),
            _ => panic!(),
        }
    }

    fn failure(main: &Path, command: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
        let (kind, name) = if command == "bibliographystyle" {
            ("style", "local,styel.bst")
        } else {
            ("database", "reffs.bib")
        };
        parse_bibliography(
            main,
            &format!("I couldn't open {kind} file {name}\n---line 3 of file main.aux\n"),
        )
    }

    #[test]
    fn resource_errors_use_confirmed_alias_providers_and_record_the_full_scope() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let provider = directory.path().join("definitions.sty");
                let name = if command == "bibliographystyle" {
                    "local,styel"
                } else {
                    "re ffs.bib"
                };
                let source = format!(
                    "% 😀\r\n\\RequirePackage{{definitions}}\n{}{{% before\r\n{name}% tail\r\n}}",
                    call(form, command)
                );
                std::fs::write(&main, &source).unwrap();
                std::fs::write(&provider, format!("\\def\\loadbib{{\\{command}}}")).unwrap();
                let main_uri = Url::from_file_path(&main).unwrap();
                let provider_uri = Url::from_file_path(&provider).unwrap();
                let sources = BTreeSet::from([main_uri.clone(), provider_uri]);
                let mut diagnostics = failure(&main, command);
                locate_missing_bibliography_resources(&sources, &mut diagnostics);
                let diagnostic = &diagnostics[&main_uri][0];
                let start = source.find(name).unwrap();
                assert_eq!(
                    diagnostic.range,
                    Text::new(source.clone()).range(start, start + name.len()),
                    "{command}/{form}"
                );
                assert!(
                    !unresolved_location(diagnostic),
                    "{command}/{form}: {diagnostic:?}"
                );
                assert_eq!(
                    serde_json::from_value::<BTreeSet<Url>>(
                        diagnostic.data.as_ref().unwrap()["macroSources"].clone()
                    )
                    .unwrap(),
                    sources
                );
            }
        }
    }

    #[test]
    fn unread_alias_providers_do_not_certify_bibliography_failures() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let provider = directory.path().join("definitions.sty");
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            std::fs::write(&main, format!("\\loadbib{{{name}}}")).unwrap();
            std::fs::write(provider, format!("\\def\\loadbib{{\\{command}}}")).unwrap();
            let main_uri = Url::from_file_path(&main).unwrap();
            let mut diagnostics = failure(&main, command);
            locate_missing_bibliography_resources(
                &BTreeSet::from([main_uri.clone()]),
                &mut diagnostics,
            );
            assert!(unresolved_location(&diagnostics[&main_uri][0]));
        }
    }

    #[test]
    fn inactive_uncertain_and_ambiguous_resource_calls_keep_unresolved_locations() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let main_uri = Url::from_file_path(&main).unwrap();
            let sources = BTreeSet::from([main_uri.clone()]);
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            for body in [
                format!("\\newcommand{{\\hold}}[1]{{}}\\hold{{\\loadbib{{{name}}}}}"),
                format!("\\newcommand{{\\later}}{{\\loadbib{{{name}}}}}"),
                format!("\\endinput\n\\loadbib{{{name}}}"),
                r"\loadbib{\chosen}".into(),
                format!("\\renewcommand{{\\{command}}}[1]{{}}\\loadbib{{{name}}}"),
                format!("\\loadbib{{{name}}}\\{command}{{{name}}}"),
                format!("\\loadbib{{{name}}}\\loadbib{{\\chosen}}"),
                // The old direct-only lookup must not preserve this unused
                // declaration as a unique source occurrence.
                format!("\\newcommand{{\\hold}}[1]{{}}\\hold{{\\{command}{{{name}}}}}"),
            ] {
                let source = format!("\\def\\loadbib{{\\{command}}}\n{body}");
                std::fs::write(&main, source).unwrap();
                let mut diagnostics = failure(&main, command);
                locate_missing_bibliography_resources(&sources, &mut diagnostics);
                assert!(
                    unresolved_location(&diagnostics[&main_uri][0]),
                    "{command}/{body}: {diagnostics:?}"
                );
            }
        }
    }

    #[test]
    fn unused_alias_bodies_do_not_hide_direct_bibliography_declarations() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let main_uri = Url::from_file_path(&main).unwrap();
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            let source = format!("\\def\\unused{{\\{command}}}\n\\{command}{{{name}}}");
            std::fs::write(&main, &source).unwrap();
            let mut diagnostics = failure(&main, command);
            locate_missing_bibliography_resources(
                &BTreeSet::from([main_uri.clone()]),
                &mut diagnostics,
            );
            let item = &diagnostics[&main_uri][0];
            assert!(!unresolved_location(item), "{command}: {item:?}");
            let start = source.rfind(name).unwrap();
            assert_eq!(
                item.range,
                Text::new(source.clone()).range(start, start + name.len())
            );
            assert!(item.data.as_ref().unwrap().get("macroSources").is_none());
        }
    }

    #[test]
    fn incomplete_source_scopes_revoke_old_direct_bibliography_locations() {
        for command in ["bibliography", "nobibliography", "bibliographystyle"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let other = directory.path().join("other.tex");
            let main_uri = Url::from_file_path(&main).unwrap();
            let name = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            std::fs::write(&main, format!("\\{command}{{{name}}}")).unwrap();
            let sources = BTreeSet::from([main_uri.clone(), Url::from_file_path(&other).unwrap()]);
            for oversized in [false, true] {
                if oversized {
                    std::fs::write(&other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
                }
                let mut diagnostics = failure(&main, command);
                assert!(!unresolved_location(&diagnostics[&main_uri][0]));
                locate_missing_bibliography_resources(&sources, &mut diagnostics);
                assert!(unresolved_location(&diagnostics[&main_uri][0]));
            }
        }
    }
}
