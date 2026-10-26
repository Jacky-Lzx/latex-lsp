//! Recover file/line diagnostics from TeX's file-line errors and traditional logs.
mod bibliography;
#[cfg(test)]
use bibliography::locate_missing_bibliography_databases;
pub(crate) use bibliography::{
    locate_missing_bibliography_resources, locate_missing_bibliography_styles,
    missing_bibliography_database_name, missing_bibliography_style_name, parse_bibliography_in,
    parse_latexmk_missing_databases,
};
pub use bibliography::{parse_bibliography, parse_bibliography_log};

use crate::text::Text;
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Url};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ops::Range as ByteRange,
    path::{Path, PathBuf},
    rc::Rc,
};

const AUXILIARY_EXTENSIONS: &[&str] = &[
    "aux", "bbl", "toc", "lof", "lot", "out", "nav", "snm", "vrb",
];

fn log_extensions() -> impl Iterator<Item = &'static str> {
    crate::project::workspace::SOURCE_EXTENSIONS
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
        return (!lines[index + 1..].contains(&crate::process::output::TRUNCATED))
            .then_some(OutputSummary::NoPages);
    }
    let mut summary = String::new();
    for (offset, line) in lines.iter().skip(index).take(128).enumerate() {
        if *line == crate::process::output::TRUNCATED || summary.len() + line.len() > 8192 {
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
    Bibliography(&'static str, String),
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
        if let Some(files) = body
            .strip_prefix(prefix)
            .and_then(|tail| tail.strip_suffix(" and rerun LaTeX afterwards."))
            .filter(|files| !files.trim().is_empty())
        {
            return Some(BiblatexRerun::Bibliography(tool, files.into()));
        }
    }
    None
}

pub(crate) fn required_bibliography_tool(diagnostic: &Diagnostic) -> Option<&'static str> {
    match biblatex_rerun(diagnostic)? {
        BiblatexRerun::Bibliography(tool, _) => Some(tool),
        BiblatexRerun::Latex => None,
    }
}

pub(crate) fn required_bibliography_job(diagnostic: &Diagnostic) -> Option<String> {
    match biblatex_rerun(diagnostic)? {
        BiblatexRerun::Bibliography(_, files) => Some(files),
        BiblatexRerun::Latex => None,
    }
}

enum RerunFilecheckRequest<'a> {
    Latex,
    Index(IndexRequest<'a>),
}

pub(crate) struct IndexRequest<'a> {
    pub file: &'a str,
    pub glossary: bool,
}

fn rerunfilecheck_request(message: &str) -> Option<RerunFilecheckRequest<'_>> {
    let (header, tail) = message.split_once('`')?;
    if header.split_whitespace().collect::<Vec<_>>().join(" ")
        != "Package rerunfilecheck Warning: File"
    {
        return None;
    }
    let (file, tail) = tail.rsplit_once("' has changed.")?;
    if file.trim().is_empty() {
        return None;
    }
    let request = tail
        .split_whitespace()
        .filter(|word| *word != "(rerunfilecheck)")
        .collect::<Vec<_>>()
        .join(" ");
    match request.as_str() {
        ""
        | "Rerun."
        | "Rerun to get outlines right."
        | "Rerun to get bibliographical references right."
        | "Rerun to get outlines right or use package `bookmark'." => {
            Some(RerunFilecheckRequest::Latex)
        }
        "Rerun LaTeX/makeindex to get index right." => {
            Some(RerunFilecheckRequest::Index(IndexRequest {
                file,
                glossary: false,
            }))
        }
        "Rerun LaTeX/makeindex to get glossary right." => {
            Some(RerunFilecheckRequest::Index(IndexRequest {
                file,
                glossary: true,
            }))
        }
        _ => None,
    }
}

pub(crate) fn required_index_tool(diagnostic: &Diagnostic) -> Option<&'static str> {
    required_index_input(diagnostic).map(|_| "makeindex")
}

pub(crate) fn required_index_input(diagnostic: &Diagnostic) -> Option<IndexRequest<'_>> {
    if diagnostic.severity != Some(DiagnosticSeverity::WARNING) {
        return None;
    }
    match rerunfilecheck_request(&diagnostic.message)? {
        RerunFilecheckRequest::Index(request) => Some(request),
        RerunFilecheckRequest::Latex => None,
    }
}

const STANDARD_RERUN_REQUESTS: &[&str] = &[
    "LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.",
    "LaTeX Warning: Temporary extra page added at the end. Rerun to get it removed.",
    "Package natbib Warning: Citation(s) may have changed. Rerun to get citations correct.",
    "Package natbib Warning: Citation(s) may have changed.",
    "Package longtable Warning: Table widths have changed. Rerun LaTeX.",
    "Package hyperref Warning: Rerun to get /PageLabels entry.",
];

fn standard_rerun_prefix(message: &str) -> Option<&'static str> {
    [
        ("LaTeX Warning: Label(s)", "               "),
        ("LaTeX Warning: Temporary extra page", "               "),
        ("LaTeX Warning: Hook 'shipout/lastpage'", "               "),
        ("Package natbib Warning: Citation(s)", "(natbib)"),
        ("Package longtable Warning: Table widths", "(longtable)"),
        ("Package hyperref Warning: Rerun", "(hyperref)"),
    ]
    .into_iter()
    .find_map(|(header, prefix)| message.starts_with(header).then_some(prefix))
}

fn normalized_rerun_message(message: &str) -> String {
    let prefix = standard_rerun_prefix(message);
    message
        .split_whitespace()
        .filter(|word| Some(*word) != prefix)
        .collect::<Vec<_>>()
        .join(" ")
}

// The kernel reports the previous and actual absolute page counts, then
// emits the request on an indented MessageBreak continuation.
fn last_page_hook_rerun_tail(message: &str) -> Option<&str> {
    let message =
        message.strip_prefix("LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (")?;
    let (pages, tail) = message.split_once(").")?;
    let (previous, actual) = pages.split_once(" not ")?;
    let page = |value: &str| {
        value
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| value.parse::<u32>().ok())
            .flatten()
            .filter(|value| *value > 0)
    };
    (page(previous)? != page(actual)?).then_some(tail.trim_start())
}

// A MessageBreak after a complete first sentence is still part of the
// request. Keep unexpected trailing prose too, so it cannot borrow a valid
// short natbib header while its actual continuation asks for something else.
fn rerun_sentence_continuation(message: &str, next: &str) -> bool {
    if !next.trim_start().starts_with("Rerun ") {
        return false;
    }
    let message = normalized_rerun_message(message);
    let prefix = format!("{message} ");
    STANDARD_RERUN_REQUESTS
        .iter()
        .any(|request| request.starts_with(&prefix))
        || last_page_hook_rerun_tail(&message) == Some("")
}

/// Share the confirmed rerun warning grammar with analysis and editor actions.
pub(crate) fn rerun_required(diagnostic: &Diagnostic) -> bool {
    if diagnostic.severity != Some(DiagnosticSeverity::WARNING) {
        return false;
    }
    if biblatex_rerun(diagnostic).is_some() {
        return true;
    }
    let message = normalized_rerun_message(&diagnostic.message);
    STANDARD_RERUN_REQUESTS.contains(&message.as_str())
        || last_page_hook_rerun_tail(&message) == Some("Rerun to correct this.")
        || rerunfilecheck_request(&message).is_some()
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
    let mut index_tools = BTreeSet::new();
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
        if let Some(tool) = required_index_tool(diagnostic) {
            index_tools.insert(tool);
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
        "requiredIndexTools": index_tools,
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
    (crate::parser::syntax::is_static_reference_key(&decoded)
        && !decoded.chars().any(char::is_control))
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
    let parsed = crate::parser::syntax::Syntax::latex(text);
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
                    && crate::parser::syntax::control_word(&text.source, command.span.start)
                        .is_some()
            }) {
                return None;
            }
            start + offset + ch.len_utf8()
        }
        Some(_) => return None,
        None => start,
    };
    let bodies = crate::symbols::commands::definition_body_spans(&text.source, &parsed);
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
    let syntax = crate::parser::syntax::Syntax::latex(&text);
    if command_name == "verbatiminput"
        && !crate::project::code_inputs::safe_syntax(&text.source, &syntax, command_name)
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
        let Some(argument) = crate::parser::syntax::file_argument(&text.source, command) else {
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
            crate::parser::syntax::file_lookup_names(command_name, value).collect::<Vec<_>>()
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
    let parsed = crate::parser::syntax::Syntax::latex(&text);
    if !crate::project::code_inputs::safe_syntax(&text.source, &parsed, "verbatiminput") {
        return;
    }
    let message = format!("Verbatim input file `{name}` not found");
    for command in parsed
        .commands
        .iter()
        .filter(|command| command.name == "verbatiminput")
        .take(1024)
    {
        let Some(argument) = crate::parser::syntax::file_argument(&text.source, command)
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
    parsed: &crate::parser::syntax::Syntax,
    cached: Option<&InvocationCalls<'_>>,
    project_stale: bool,
) -> Vec<crate::parser::syntax::Command> {
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
        &crate::symbols::commands::CommandSignatures::new([(text.source.as_str(), parsed)]),
    )
}

fn missing_include_ranges(
    text: &Text,
    name: &str,
    cached: Option<&InvocationCalls<'_>>,
    project_stale: bool,
) -> Vec<Range> {
    let parsed = crate::parser::syntax::Syntax::latex(text);
    let indirect = file_commands_for_text(text, &parsed, cached, project_stale);
    parsed
        .commands
        .iter()
        .chain(&indirect)
        .filter(|command| command.name == "include")
        .filter_map(|command| {
            let argument = crate::parser::syntax::file_argument(&text.source, command)?;
            let span = crate::parser::syntax::trimmed_span(&text.source, argument.span.clone());
            let matched =
                crate::parser::syntax::file_lookup_names("include", argument.value.trim())
                    .any(|candidate| candidate == name);
            matched.then(|| text.range(span.start, span.end))
        })
        .collect()
}

pub(crate) fn undefined_reference_ranges(text: &Text, line: u32, message: &str) -> Vec<Range> {
    let Some((citation, key)) = undefined_reference_key(message) else {
        return vec![];
    };
    let valid = if citation {
        crate::parser::syntax::is_static_citation_key
    } else {
        crate::parser::syntax::is_static_reference_key
    };
    let syntax = crate::parser::syntax::Syntax::latex(text);
    let hyper_link = message.contains("Hyper reference `") || message.contains("Hyper reference '");
    let mut ranges = Vec::new();
    for command in &syntax.commands {
        if !(if citation {
            crate::parser::syntax::warns_on_undefined_citation(&command.name)
        } else if hyper_link {
            command.name == "hyperref"
                || matches!(
                    command.name.as_str(),
                    "Ref" | "autopageref" | "vref" | "Vref" | "subref"
                ) && text
                    .source
                    .as_bytes()
                    .get(crate::parser::syntax::skip_space(
                        &text.source,
                        command.span.end,
                    ))
                    != Some(&b'*')
        } else {
            command.name != "hyperref"
                && (crate::parser::syntax::warns_on_undefined_reference(&command.name)
                    || crate::parser::syntax::is_counter_reference(&command.name))
        }) {
            continue;
        }
        if let Some(style) = citation
            .then(|| crate::bibliography::citations::style(&command.name))
            .flatten()
        {
            let arguments =
                crate::bibliography::citations::arguments(&text.source, command.span.end, style);
            if text.position(command.span.start).line <= line
                && text.position(arguments.end.saturating_sub(1)).line >= line
            {
                for argument in arguments.keys() {
                    for span in
                        crate::bibliography::citations::key_spans(&text.source, argument, style)
                    {
                        if text.source[span.clone()] == *key {
                            ranges.push(text.range(span.start, span.end));
                        }
                    }
                }
            }
            continue;
        }
        let separate_label = crate::parser::syntax::separate_label_argument(&text.source, command);
        let argument = if crate::parser::syntax::has_separate_label_argument(&command.name) {
            separate_label.as_ref()
        } else {
            command.argument.as_ref()
        };
        let Some(argument) = argument else {
            continue;
        };
        let range_end = crate::parser::syntax::range_reference_end(&text.source, command);
        // hyperref reads the display text before warning, so a multiline link
        // reports the closing text line rather than the bracketed key's line.
        // Range references read both endpoints before reporting missing labels.
        let report_end = if command.name == "hyperref" {
            crate::parser::syntax::group(
                &text.source,
                crate::parser::syntax::skip_space(&text.source, argument.outer_span.end),
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
        if citation && crate::parser::syntax::is_single_citation(&command.name)
            || !citation
                && (crate::parser::syntax::is_single_reference(&command.name)
                    || crate::parser::syntax::has_separate_label_argument(&command.name))
        {
            let source_key = if command.name == "subref" && !hyper_link {
                key.strip_prefix("sub@")
            } else {
                Some(key)
            };
            if let Some(key) = source_key.filter(|key| valid(key)) {
                for argument in std::iter::once(argument).chain(range_end.as_ref()) {
                    if argument.value == key {
                        ranges.push(text.range(argument.span.start, argument.span.end));
                    }
                }
            }
            continue;
        }
        if !valid(key) {
            continue;
        }
        for segment in crate::parser::syntax::key_list_segments(&text.source, argument.span.clone())
        {
            let bounded = &text.source[..segment.end];
            let start = crate::parser::syntax::skip_space(bounded, segment.start);
            if bounded[start..].starts_with(key) {
                let end = start + key.len();
                if crate::parser::syntax::skip_space(bounded, end) == segment.end {
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
    right_delimiter_frame: Option<&'a str>,
    delimiter_operand_token: Option<&'a str>,
    package_options: Option<PackageOptions>,
    name_frame: Option<&'a str>,
    macro_context: Option<InvocationContext<'a>>,
    project_stale: bool,
}

impl<'a> ErrorContext<'a> {
    fn source(source: &'a str) -> Self {
        Self {
            source,
            right_delimiter_frame: None,
            delimiter_operand_token: None,
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
        if self.index >= 64
            || self.bytes + line.len() > 8192
            || line == crate::process::output::TRUNCATED
        {
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
    let right_delimiter_frame = (message == crate::parser::right_delimiters::ERROR)
        .then(|| crate::parser::right_delimiters::expansion_frame(line))
        .flatten();
    if right_delimiter_frame.is_some() {
        (index, line) = context.next()?;
    }
    let delimiter_operand_token = if message == crate::parser::delimiter_operands::ERROR {
        if line.trim() != "<to be read again>" {
            return None;
        }
        let token = crate::parser::delimiter_operands::recovery_token(context.next()?.1)?;
        (index, line) = context.next()?;
        Some(token)
    } else {
        None
    };
    if message == crate::symbols::special_characters::EXTRA_ALIGNMENT_TAB {
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
    // Accept only the engine's exact recovery token and immediate source
    // echo. Additional expansion frames must still block attribution.
    let inserted = match message {
        "Missing $ inserted." => Some("$"),
        crate::parser::right_delimiters::MISSING_ERROR => Some(r"\right ."),
        _ => None,
    };
    if let Some(inserted) = inserted {
        if line.trim() != "<inserted text>" || context.next()?.1.trim() != inserted {
            return None;
        }
        (index, line) = context.next()?;
    }
    let recent = (message == "Undefined control sequence.")
        .then(|| line.trim_start().strip_prefix("<recently read>"))
        .flatten()
        .map(str::trim);
    if let Some(token) = recent {
        if crate::parser::syntax::control_word(token, 0)? != (0..token.len()) {
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
        if crate::parser::syntax::control_word(token, 0)? != (0..token.len()) {
            return None;
        }
        (index, line) = context.next()?;
    }
    let graphics = missing_graphic_name(message).is_some() && missing_file_name(message).is_none();
    let documentation = line.trim() == "See the LaTeX manual or LaTeX Companion for explanation."
        || (undefined_length_name(message).is_some()
            || undefined_toggle_name(message).is_some()
            || undefined_boolean_name(message).is_some())
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
            right_delimiter_frame,
            delimiter_operand_token,
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
    crate::parser::syntax::is_literal_file_path(name).then_some(name)
}

pub(crate) fn missing_minted_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package minted Error: Cannot find input file \"")?
        .strip_suffix("\"; inserting placeholder.")?;
    let name = name
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(name);
    crate::parser::syntax::is_literal_file_path(name).then_some(name)
}

fn missing_read_file(message: &str) -> bool {
    missing_file_name(message).is_some()
        || missing_listing_name(message).is_some()
        || missing_minted_name(message).is_some()
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

/// The second source-echo line and TeX's exact delimiter help are diagnostic
/// data. Their parentheses must not enter or leave a source file. Commit only
/// the immediate echo continuation and complete, known help paragraphs.
fn delimiter_error_tail(lines: &[&str], start: usize, number: u32) -> Option<usize> {
    let header = format!("l.{number} ");
    let mut echo = None;
    for (index, line) in lines.iter().enumerate().skip(start).take(16) {
        if line.trim_start().starts_with(&header) {
            echo = Some(index);
            break;
        }
        if message_boundary(line) || line.trim_start().starts_with('(') {
            return None;
        }
    }
    let echo = echo?;
    let mut end = echo;
    if let Some(next) = lines.get(echo + 1) {
        let indent = next.bytes().take_while(|byte| *byte == b' ').count();
        if next.is_empty()
            || indent > 0 && (indent == lines[echo].len() || indent == lines[echo].chars().count())
        {
            end += 1;
        }
    }
    let mut help = OptionHelp {
        lines: lines.get(end + 1..)?,
        index: 0,
        bytes: 0,
    };
    let complete = (|| {
        help.expect(r"I was expecting to see something like `(' or `\{' or")?;
        help.expect(r"`\}' here. If you typed, e.g., `{' instead of `\{', you")?;
        help.expect("should probably delete the `{' by typing `1' now, so that")?;
        let mut period = help;
        if period
            .expect("braces don't get unbalanced. Otherwise just proceed.")
            .is_some()
        {
            help = period;
        } else {
            help.expect("braces don't get unbalanced. Otherwise just proceed")?;
        }
        help.expect(r"Acceptable delimiters are characters whose \delcode is")?;
        help.expect(r"nonnegative, or you can use `\delimiter <delimiter code>'.")
    })();
    Some(if complete.is_some() {
        end + help.index
    } else {
        end
    })
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
    indirect: &[crate::parser::syntax::Command],
) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    if shortened.is_some_and(|suffix| !unique_context_on_line(text, start, suffix)) {
        return None;
    }
    let syntax = crate::parser::syntax::Syntax::latex(text);
    let fancyverb = crate::parser::fancyvrb::is_input(command_name);
    if crate::parser::syntax::code_input_command(command_name)
        && !crate::project::code_inputs::safe_syntax(&text.source, &syntax, command_name)
    {
        return None;
    }
    let mut matches = syntax
        .commands
        .iter()
        .chain(indirect)
        .filter_map(|command| {
            if !(if fancyverb {
                crate::parser::fancyvrb::is_input(&command.name)
            } else {
                command.name == command_name
            }) {
                return None;
            }
            let argument = crate::parser::syntax::file_argument(&text.source, command)?;
            if crate::parser::syntax::code_input_command(command_name)
                && !crate::project::code_inputs::safe_call(&text.source, command)
            {
                return None;
            }
            let span = crate::parser::syntax::trimmed_span(&text.source, argument.span.clone());
            let value = if command_name == "includegraphics" {
                argument.value.as_str()
            } else {
                &text.source[span.clone()]
            };
            let matches_report = if command_name == "includegraphics" {
                crate::project::graphics::unquote(name).is_some_and(|reported| value == reported)
            } else if command_name == "lstinputlisting" {
                crate::parser::listings::missing_report(value) == name
            } else if fancyverb || command_name == "inputminted" {
                value == name
            } else {
                crate::parser::syntax::file_lookup_names(command_name, value)
                    .any(|candidate| candidate == name)
            };
            if !crate::parser::syntax::is_literal_file_path(value)
                || !matches_report
                || text.position(argument.outer_span.end).line != line
                || text.source[argument.outer_span.clone()].starts_with('"')
                    && !text.source[argument.outer_span.clone()].ends_with('"')
            {
                return None;
            }
            let consumed = &text.source[start..argument.outer_span.end];
            let matched = shortened.map_or(consumed == prefix, |suffix| {
                ((fancyverb || command_name == "inputminted") && !suffix.is_empty()
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
        let parsed = crate::parser::syntax::Syntax::latex(text);
        local = file_error_commands(
            &text.source,
            &parsed,
            &crate::symbols::commands::CommandSignatures::new([(text.source.as_str(), &parsed)]),
        );
        &local
    };
    missing_path_range(text, line, context, name, "input", commands)
}

fn package_option_clash_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("LaTeX Error: Option clash for package ")?
        .strip_suffix('.')?;
    (crate::parser::syntax::is_literal_file_path(name) && !name.contains([' ', '\t', '\r', '\n']))
        .then_some(name)
}

fn loader_failure_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
    indirect: &[crate::parser::syntax::Command],
) -> Option<(Range, &'static str)> {
    if !name.ends_with(".sty") && !name.ends_with(".cls") {
        return None;
    }
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::parser::syntax::Syntax::latex(text);
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
        let Some(argument) = crate::parser::syntax::file_argument(&text.source, command) else {
            continue;
        };
        let next = crate::parser::syntax::skip_space(&text.source, argument.outer_span.end);
        // LaTeX reads the optional minimum version, or one lookahead token,
        // before the loader reports a failure. That token may be on
        // the next source line, while the path being diagnosed is above it.
        let tail = if text.source.as_bytes().get(next) == Some(&b'[') {
            crate::parser::syntax::optional_note_end(&text.source, next)
        } else if text.source.as_bytes().get(next) == Some(&b'\\') {
            crate::parser::syntax::control_word(&text.source, next)
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
        let spans = if crate::parser::syntax::file_list(command_name) {
            let Some(spans) =
                crate::parser::syntax::complete_file_list_spans(&text.source, command)
            else {
                continue;
            };
            spans
        } else {
            let span = crate::parser::syntax::trimmed_span(&text.source, argument.span.clone());
            if !crate::parser::syntax::is_literal_file_path(&text.source[span.clone()]) {
                continue;
            }
            vec![span]
        };
        for span in spans {
            if crate::parser::syntax::file_lookup_names(command_name, &text.source[span.clone()])
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
    crate::parser::syntax::is_static_key(name).then_some(name)
}

pub(crate) fn undefined_counter_name(message: &str) -> Option<&str> {
    let tail = message.strip_prefix("LaTeX Error: No counter ")?;
    let name = tail
        .strip_prefix('\'')
        .or_else(|| tail.strip_prefix('`'))?
        .strip_suffix("' defined.")?;
    crate::parser::syntax::is_static_key(name).then_some(name)
}

pub(crate) fn undefined_length_name(message: &str) -> Option<&str> {
    let control = message
        .strip_prefix("Package etoolbox Error: Length '")?
        .strip_suffix("' undefined.")?;
    let span = crate::parser::syntax::control_word(control, 0)?;
    (span.end == control.len()).then(|| &control[1..])
}

pub(crate) fn undefined_toggle_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package etoolbox Error: Toggle '")
        .or_else(|| {
            message
                .strip_prefix(BOOLEAN_EXPRESSION_ERROR)?
                .strip_prefix("\nToggle '")
        })?
        .strip_suffix("' undefined.")?;
    crate::parser::syntax::is_static_key(name).then_some(name)
}

pub(crate) fn undefined_boolean_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package etoolbox Error: Boolean '\\if")
        .or_else(|| {
            message
                .strip_prefix(BOOLEAN_EXPRESSION_ERROR)?
                .strip_prefix("\nBoolean '")
        })?
        .strip_suffix("' undefined.")?;
    crate::parser::syntax::is_static_key(name).then_some(name)
}

pub(crate) const BOOLEAN_EXPRESSION_ERROR: &str =
    "Package etoolbox Error: Invalid boolean expression.";

/// TeX prints the explanation after both rows of the source echo. Keep it
/// attached to this error, and never scan across another diagnostic or input.
fn boolean_expression_details(
    lines: &[&str],
    start: usize,
    number: u32,
    message: &str,
) -> Option<(String, usize)> {
    if message != BOOLEAN_EXPRESSION_ERROR {
        return None;
    }
    for (index, line) in lines.iter().enumerate().skip(start).take(16) {
        if let Some(tail) = line.strip_prefix("l.") {
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            if tail[..digits].parse::<u32>().ok()? != number || !tail[digits..].starts_with(' ') {
                return None;
            }
            let continuation = *lines.get(index + 1)?;
            if !continuation.is_empty() && !continuation.starts_with(char::is_whitespace) {
                return None;
            }
            // The unconsumed source row also wraps at max_print_line.
            // Advance only past full physical rows, and require the actual
            // explanation to end before the following blank separator.
            for detail_start in index + 2..index + 6 {
                if let Some(details) = boolean_expression_explanation(lines, detail_start, message)
                {
                    return Some(details);
                }
                if lines.get(detail_start - 1)?.len() < 60 {
                    break;
                }
            }
            return None;
        }
        if message_boundary(line) || line.starts_with('(') {
            return None;
        }
    }
    None
}

fn boolean_expression_explanation(
    lines: &[&str],
    start: usize,
    message: &str,
) -> Option<(String, usize)> {
    let first = *lines.get(start)?;
    if !first.starts_with("Toggle '") && !first.starts_with("Boolean '") {
        return None;
    }
    let mut detail = String::new();
    for (end, part) in lines.iter().enumerate().skip(start).take(8) {
        if part.trim().is_empty()
            || message_boundary(part) && !part.starts_with(')')
            || detail.len() + part.len() > 4096
        {
            return None;
        }
        detail.push_str(part);
        let explained = detail
            .strip_prefix("Toggle '")
            .or_else(|| detail.strip_prefix("Boolean '"))
            .and_then(|tail| tail.strip_suffix("' undefined."))
            .is_some_and(|name| !name.is_empty());
        if explained && lines.get(end + 1).is_none_or(|next| next.trim().is_empty()) {
            return Some((format!("{message}\n{detail}"), end));
        }
        if part.len() < 60 {
            return None;
        }
    }
    None
}

pub(crate) fn undefined_color_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package xcolor Error: Undefined color `")?
        .strip_suffix("'.")?;
    crate::symbols::named_colors::literal_name(name).then_some(name)
}

pub(crate) fn undefined_color_model_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package xcolor Error: Undefined color model `")?
        .strip_suffix("'.")?;
    crate::symbols::named_colors::literal_name(name).then_some(name)
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
        let syntax = crate::parser::syntax::Syntax::latex(text);
        local = kind.invocations(
            &text.source,
            &syntax,
            &crate::symbols::commands::CommandSignatures::new([(text.source.as_str(), &syntax)]),
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
            let expression_name = matches!(
                kind,
                InvocationKind::BooleanExpression | InvocationKind::ToggleExpression
            ) && tail
                .end
                .checked_sub(suffix.len())
                .is_some_and(|suffix_start| suffix_start < span.end);
            (suffix.ends_with(&text.source[tail.clone()]) || expression_name)
                && consumed.ends_with(suffix)
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
    (crate::parser::syntax::is_static_key(name) && crate::parser::syntax::is_static_key(expected))
        .then_some((name, expected))
}

pub(crate) fn environment_error_invocations_with_signatures(
    source: &str,
    parsed: &crate::parser::syntax::Syntax,
    signatures: &crate::symbols::commands::CommandSignatures<'_>,
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
    parsed: &crate::parser::syntax::Syntax,
    signatures: &crate::symbols::commands::CommandSignatures<'_>,
) -> Vec<crate::parser::syntax::Command> {
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
    parsed: &crate::parser::syntax::Syntax,
    signatures: &crate::symbols::commands::CommandSignatures<'_>,
) -> Vec<(ByteRange<usize>, ByteRange<usize>)> {
    let loaders_redefined = signatures
        .names()
        .any(|name| name == "input" || crate::parser::syntax::loader_extension(name).is_some());
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
                && crate::parser::syntax::loader_extension(&command.name).is_none())
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
            let argument = crate::parser::syntax::file_argument(source, command)?;
            let spans = if crate::parser::syntax::file_list(&command.name) {
                crate::parser::syntax::complete_file_list_spans(source, command)?
            } else {
                vec![crate::parser::syntax::trimmed_span(
                    source,
                    argument.span.clone(),
                )]
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
        let parsed = crate::parser::syntax::Syntax::latex(text);
        local = file_error_invocations_with_signatures(
            &text.source,
            &parsed,
            &crate::symbols::commands::CommandSignatures::new([(text.source.as_str(), &parsed)]),
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
    let syntax = crate::parser::syntax::Syntax::latex(text);
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
    command: &crate::parser::syntax::Command,
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
            crate::parser::syntax::reference_argument_start(
                source,
                &command.name,
                command.span.end,
            )?,
            b'{',
        ),
        "cite" | "nocite" | "citet" | "citep" | "citeauthor" | "citeyear" | "citenum"
        | "bibentry" | "citeyearpar" | "citealt" | "citealp" | "citefullauthor" | "Citet"
        | "Citep" | "Citealt" | "Citealp" | "Citeauthor" | "citetalias" | "citepalias" => (
            crate::parser::syntax::citation_argument_start(
                source,
                &command.name,
                command.span.end,
            )?,
            b'{',
        ),
        "hyperref" => {
            crate::parser::syntax::separate_label_argument_start(source, command)?;
            (
                crate::parser::syntax::skip_space(source, command.span.end),
                b'[',
            )
        }
        "setcounterref" | "addtocounterref" | "setcounterpageref" | "addtocounterpageref" => {
            let (counter, end) = crate::parser::syntax::group(
                source,
                crate::parser::syntax::skip_space(source, command.span.end),
                b'{',
                b'}',
            )?;
            if !crate::parser::syntax::is_static_key(&counter.value) {
                return None;
            }
            (crate::parser::syntax::skip_space(source, end), b'{')
        }
        _ => return None,
    };
    let close = if open == b'[' { b']' } else { b'}' };
    let (mut argument, _) = crate::parser::syntax::group(source, start, open, close)?;
    let mut tail = argument.outer_span.clone();
    if command.name == "getrefbykeydefault" {
        // refcount consumes the selector before expanding the key, but has
        // not read the default yet. Dynamic selectors cannot identify a peer.
        let (selector, end) = crate::parser::syntax::group(
            source,
            crate::parser::syntax::skip_space(source, tail.end),
            b'{',
            b'}',
        )?;
        if !selector.value.is_empty() && !crate::parser::syntax::is_static_key(&selector.value) {
            return None;
        }
        tail.end = end;
    }
    if command.name == "hyperref" && argument.value.starts_with('{') {
        let (inner, end) = crate::parser::syntax::group(source, argument.span.start, b'{', b'}')?;
        if end != argument.span.end {
            return None;
        }
        argument = inner;
    }
    if argument.value != frame
        || crate::parser::syntax::control_word(&argument.value, 0)? != (0..argument.value.len())
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
    let syntax = crate::parser::syntax::Syntax::latex(text);
    let mut matches = syntax.commands.iter().filter_map(|command| {
        let (span, tail) =
            crate::symbols::saved_text::name_error_invocation(&text.source, command, frame)
                .or_else(|| {
                    crate::symbols::named_commands::name_error_invocation(
                        &text.source,
                        command,
                        frame,
                    )
                })
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
            let syntax = crate::parser::syntax::Syntax::latex(&text);
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
        .and_then(|context| crate::symbols::special_characters::error_character(&message, context))
        .filter(|_| severity == DiagnosticSeverity::ERROR);
    let special_range = special_character.and_then(|character| {
        if project_stale {
            return None;
        }
        crate::symbols::special_characters::source_range(
            &text,
            &crate::parser::syntax::Syntax::latex(&text),
            line,
            context?,
            character,
            macro_context.map(|context| &context.macros.signatures),
        )
    });
    ranges.extend(special_range);
    let right_delimiter = (message == crate::parser::right_delimiters::ERROR
        && severity == DiagnosticSeverity::ERROR
        && !project_stale)
        .then(|| {
            crate::parser::right_delimiters::invocation(
                &text,
                &crate::parser::syntax::Syntax::latex(&text),
                line,
                context?,
                error.as_ref().and_then(|error| error.right_delimiter_frame),
                macro_context.map(|context| &context.macros.signatures),
            )
        })
        .flatten();
    let right_range = right_delimiter
        .as_ref()
        .map(|invocation| text.range(invocation.command.start, invocation.command.end));
    ranges.extend(right_range);
    let missing_right = (message == crate::parser::right_delimiters::MISSING_ERROR
        && severity == DiagnosticSeverity::ERROR
        && !project_stale)
        .then(|| {
            crate::parser::right_delimiters::missing_boundary(
                &text,
                &crate::parser::syntax::Syntax::latex(&text),
                line,
                context?,
                macro_context.map(|context| &context.macros.signatures),
            )
        })
        .flatten();
    let missing_right_range = missing_right
        .as_ref()
        .map(|boundary| text.range(boundary.start, boundary.end));
    ranges.extend(missing_right_range);
    let delimiter_operand = (message == crate::parser::delimiter_operands::ERROR
        && severity == DiagnosticSeverity::ERROR
        && !project_stale)
        .then(|| {
            crate::parser::delimiter_operands::invocation(
                &text,
                &crate::parser::syntax::Syntax::latex(&text),
                line,
                context?,
                error.as_ref()?.delimiter_operand_token?,
                macro_context.map(|context| &context.macros.signatures),
            )
        })
        .flatten();
    let delimiter_operand_range = delimiter_operand
        .as_ref()
        .map(|invocation| text.range(invocation.command.start, invocation.command.end));
    ranges.extend(delimiter_operand_range);
    let unsupported_unicode =
        unsupported_unicode_character(&message).filter(|_| severity == DiagnosticSeverity::ERROR);
    ranges.extend(unsupported_unicode.and_then(|character| {
        crate::symbols::special_characters::source_range(
            &text,
            &crate::parser::syntax::Syntax::latex(&text),
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
    let toggle_name = undefined_toggle_name(&message);
    let toggle_range = toggle_name
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| {
            invocation_error_range(
                &text,
                line,
                context?,
                name,
                if message.starts_with(BOOLEAN_EXPRESSION_ERROR) {
                    InvocationKind::ToggleExpression
                } else {
                    InvocationKind::Toggle
                },
                cached_calls,
            )
        });
    ranges.extend(toggle_range);
    let boolean_name = undefined_boolean_name(&message);
    let boolean_range = boolean_name
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| {
            invocation_error_range(
                &text,
                line,
                context?,
                name,
                if message.starts_with(BOOLEAN_EXPRESSION_ERROR) {
                    InvocationKind::BooleanExpression
                } else {
                    InvocationKind::Boolean
                },
                cached_calls,
            )
        });
    ranges.extend(boolean_range);
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
    let minted_range = missing_minted_name(&message)
        .or_else(|| missing_fancyverb_name(&message))
        .filter(|_| !project_stale)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_path_range(&text, line, context?, name, "inputminted", &[]));
    ranges.extend(minted_range);
    let loader_range = missing_file
        .filter(|_| !project_stale)
        .and_then(|name| {
            let parsed = crate::parser::syntax::Syntax::latex(&text);
            let indirect = file_commands_for_text(&text, &parsed, cached_calls, project_stale);
            loader_failure_range(&text, line, context?, name, &indirect)
        })
        .filter(|(range, _)| confirmed_file_invocation(&text, *range, cached_calls));
    ranges.extend(loader_range.map(|(range, _)| range));
    let package_clash = package_option_clash_name(&message);
    let clash_range = package_clash
        .filter(|_| !project_stale)
        .and_then(|name| {
            let parsed = crate::parser::syntax::Syntax::latex(&text);
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
            let parsed = crate::parser::syntax::Syntax::latex(&text);
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
                    .or_else(|| special_range.map(|_| crate::symbols::special_characters::diagnostic_code(&message)))
                    .or_else(|| right_range.map(|_| crate::parser::right_delimiters::CODE))
                    .or_else(|| missing_right_range.map(|_| crate::parser::right_delimiters::MISSING_CODE))
                    .or_else(|| delimiter_operand_range.map(|_| crate::parser::delimiter_operands::CODE))
                    .or_else(|| character.map(|_| "missing-character"))
                    .or_else(|| unsupported_unicode.map(|_| "unsupported-unicode-character"))
                    .or_else(|| destination.as_ref().filter(|_| precise_destination).map(|(kind, _)| kind.code()))
                    .or_else(|| missing_include_key(&message).map(|_| "missing-include"))
                    .or_else(|| input_range.map(|_| "missing-input"))
                    .or_else(|| listing_range.map(|_| "missing-listing"))
                    .or_else(|| fancyverb_range.map(|_| "missing-fancyverb-input"))
                    .or_else(|| minted_range.map(|_| "missing-minted-input"))
                    .or_else(|| loader_range.map(|(_, command)| if crate::parser::syntax::loader_extension(command) == Some("sty") { "missing-package" } else { "missing-class" }))
                    .or_else(|| clash_range.map(|_| "package-option-clash"))
                    .or_else(|| graphic_range.map(|_| "missing-image"))
                    .or_else(|| command_range.map(|_| "undefined-control-sequence"))
                    .or_else(|| environment_range.and(environment.as_ref()).map(|error| error.code))
                    .or_else(|| counter_range.map(|_| "undefined-counter"))
                    .or_else(|| length_range.map(|_| "undefined-length"))
                    .or_else(|| toggle_range.map(|_| "undefined-toggle"))
                    .or_else(|| boolean_range.map(|_| "undefined-boolean"))
                    .or_else(|| color_range.map(|_| "undefined-color"))
                    .or_else(|| model_range.map(|_| "undefined-color-model"))
                    .map(|code| NumberOrString::String(code.into())),
                message: displayed_message.clone(),
                data: precise_destination.then(|| serde_json::json!({"destination":destination.as_ref().map(|(_, name)| name)}))
                    .or_else(|| float_excess.map(|excess| serde_json::json!({"excessPt":excess})))
                    .or_else(|| special_range.and(special_character).map(|character| serde_json::json!({"character":character.to_string(),"sourceContext":context})))
                    .or_else(|| right_delimiter.as_ref().map(|invocation| serde_json::json!({"delimiter":&text.source[invocation.delimiter.clone()],"delimiterFrame":error.as_ref().and_then(|error| error.right_delimiter_frame),"sourceLine":line,"sourceContext":context})))
                    .or_else(|| missing_right.as_ref().map(|boundary| serde_json::json!({"closingToken":&text.source[boundary.clone()],"sourceLine":line,"sourceContext":context})))
                    .or_else(|| delimiter_operand.as_ref().map(|invocation| serde_json::json!({"delimiterCommand":&text.source[invocation.command.clone()],"recoveryToken":&text.source[invocation.operand.clone()],"sourceLine":line,"sourceContext":context})))
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
                    .or_else(|| toggle_range.and(toggle_name).map(|name| serde_json::json!({"toggle":name})))
                    .or_else(|| boolean_range.and(boolean_name).map(|name| serde_json::json!({"boolean":name})))
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
                    .or_else(|| minted_range.map(|range| {
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
                        if special_range.is_some() || right_range.is_some() || missing_right_range.is_some() || delimiter_operand_range.is_some() || missing_include_key(&message).is_some() || color_range.is_some() || model_range.is_some() || counter_range.is_some() || length_range.is_some() || toggle_range.is_some() || boolean_range.is_some() || environment_range.is_some() || input_range.is_some() || loader_range.is_some() || clash_range.is_some() {
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
    line == crate::process::output::TRUNCATED
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

// A full TeX print line followed by an explicit MessageBreak leaves one
// empty physical line before the package's continuation prefix.
fn package_warning_continuation(lines: &[&str], at: usize, prefix: &str) -> Option<usize> {
    let mut next = at + 1;
    if lines.get(next)?.trim().is_empty() && lines[at].len() >= 60 {
        next += 1;
    }
    let line = lines.get(next)?;
    let owned = if prefix.starts_with(' ') {
        // The kernel uses fifteen spaces instead of a package name.
        line.starts_with(prefix)
    } else {
        line.trim_start()
            .strip_prefix(prefix)?
            .starts_with(char::is_whitespace)
    };
    owned.then_some(next)
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
    let standard_rerun = standard_rerun_prefix(initial);
    let continuation_prefix = if rerunfilecheck {
        Some("(rerunfilecheck)")
    } else {
        standard_rerun
    };
    let continuation =
        |at| continuation_prefix.and_then(|prefix| package_warning_continuation(lines, at, prefix));
    // TeX byte-wraps long warning lines, including inside a literal key.
    // Keep blanks at those boundaries instead of inventing a separator.
    let mut message = if rerunfilecheck {
        // A filename can continue with parentheses that look like a source
        // announcement. Only consume them when the full warning is confirmed.
        byte_wrapped_literal_message::<60>(root, lines, separator_lines, i, initial, |message| {
            rerunfilecheck_request(message).is_some()
        })
        .unwrap_or_else(|| initial.to_owned())
    } else if reference || standard_rerun.is_some() {
        initial.to_owned()
    } else {
        lines[*i].trim().to_owned()
    };
    while (!message.ends_with(['.', '!'])
        || (reference && lines[*i].len() >= 60 && undefined_reference_key(&message).is_none())
        || ((rerunfilecheck || standard_rerun.is_some()) && lines[*i].len() >= 60)
        || standard_rerun.is_some()
            && lines.get(*i + 1).is_some_and(|next| rerun_sentence_continuation(&message, next))
        // biblatex puts explanatory sentences after a complete first sentence.
        // Keep its own continuation lines with the original warning.
        || initial.starts_with("Package biblatex Warning:")
            && lines.get(*i + 1).is_some_and(|line| {
                line.trim_start()
                    .strip_prefix("(biblatex)")
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
            })
        || continuation(*i).is_some())
        && *i + 1 < lines.len()
        && message.len() < 4096
    {
        let continuation_at = continuation(*i);
        let next_at = continuation_at.unwrap_or(*i + 1);
        let next = lines[next_at];
        // Confirm a closing page-count parenthesis before treating it as
        // warning text. A real file close on a separate line stays a boundary.
        let closes_page_counts = lines[*i].len() >= 60
            && next.starts_with(").")
            && last_page_hook_rerun_tail(&format!("{message}{next}")).is_some();
        if next.trim().is_empty() || (message_boundary(next) && !closes_page_counts) {
            break;
        }
        let mut next_index = next_at;
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
            let mut next_index = next_at;
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
        let byte_wrapped = (reference || rerunfilecheck || standard_rerun.is_some())
            && lines[*i].len() >= 60
            && continuation_at.is_none()
            && !(standard_rerun.is_some() && rerun_sentence_continuation(&message, next))
            // A kernel warning can byte-wrap immediately before its page
            // counts. Real file announcements were excluded above.
            && (!next.trim_start().starts_with('(') || standard_rerun == Some("               "));
        *i = next_at;
        if byte_wrapped {
            message.push_str(next);
        } else {
            if !message.ends_with(char::is_whitespace) {
                message.push(' ');
            }
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
                    if sources.len() < 4096 && crate::project::workspace::is_source(&file) {
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
                    || *next == crate::process::output::TRUNCATED
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
        if driver_message(line).is_some() || line == crate::process::output::TRUNCATED {
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
    signatures: crate::symbols::commands::CommandSignatures<'a>,
    sources: Vec<Url>,
    paused_sources: Vec<Url>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InvocationKind {
    Color,
    ColorModel,
    Counter,
    Length,
    Toggle,
    Boolean,
    ToggleExpression,
    BooleanExpression,
    EnvironmentBegin,
    EnvironmentEnd,
    File,
    Literal,
}

impl InvocationKind {
    fn for_message(message: &str) -> Option<Self> {
        if crate::symbols::special_characters::error_character(message, "").is_some()
            || message == "Missing $ inserted."
            || message == crate::parser::right_delimiters::ERROR
            || message == crate::parser::right_delimiters::MISSING_ERROR
            || message == crate::parser::delimiter_operands::ERROR
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
        } else if undefined_toggle_name(message).is_some() {
            Some(if message.starts_with(BOOLEAN_EXPRESSION_ERROR) {
                Self::ToggleExpression
            } else {
                Self::Toggle
            })
        } else if undefined_boolean_name(message).is_some() {
            Some(if message.starts_with(BOOLEAN_EXPRESSION_ERROR) {
                Self::BooleanExpression
            } else {
                Self::Boolean
            })
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
        syntax: &crate::parser::syntax::Syntax,
        signatures: &crate::symbols::commands::CommandSignatures<'_>,
    ) -> Vec<(ByteRange<usize>, ByteRange<usize>)> {
        match self {
            Self::Literal => vec![],
            Self::Color => crate::symbols::named_colors::error_invocations_with_signatures(
                source, syntax, signatures,
            ),
            Self::ColorModel => {
                crate::symbols::named_colors::model_error_invocations_with_signatures(
                    source, syntax, signatures,
                )
            }
            Self::Counter => crate::symbols::counters::error_invocations_with_signatures(
                source, syntax, signatures,
            ),
            Self::Length => crate::symbols::lengths::error_invocations_with_signatures(
                source, syntax, signatures,
            ),
            Self::Toggle | Self::Boolean | Self::ToggleExpression | Self::BooleanExpression => {
                crate::diagnostics::switch_diagnostics::error_invocations_with_signatures(
                    source,
                    syntax,
                    signatures,
                    if matches!(self, Self::Toggle | Self::ToggleExpression) {
                        crate::diagnostics::switch_diagnostics::Kind::Toggle
                    } else {
                        crate::diagnostics::switch_diagnostics::Kind::Boolean
                    },
                    matches!(self, Self::ToggleExpression | Self::BooleanExpression),
                )
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
    file_commands: Vec<crate::parser::syntax::Command>,
}

struct InvocationContext<'a> {
    macros: Rc<MacroContext<'a>>,
    calls: Rc<InvocationCalls<'a>>,
}

/// Saved explicit source dependencies, captured before TeX starts. No directory
/// scan or distribution-package lookup participates in this snapshot.
pub(crate) struct ProjectSources {
    documents: BTreeMap<Url, crate::project::workspace::Document>,
    dynamic_sources: BTreeSet<Url>,
}

impl ProjectSources {
    pub(crate) fn capture(root: &Path) -> Self {
        let mut workspace = crate::project::workspace::Workspace::default();
        let mut sources = BTreeSet::new();
        let mut source_calls = crate::project::source_inputs::Index::default();
        if let Ok(uri) = Url::from_file_path(root.canonicalize().unwrap_or_else(|_| root.into())) {
            workspace.reload(&uri);
            workspace.load_dependencies(&uri, 0);
            sources = workspace.source_documents_from(&uri, &uri);
            source_calls = crate::project::source_inputs::Index::new(&workspace, &uri);
        }
        let documents: BTreeMap<_, _> = workspace
            .documents
            .into_iter()
            .filter(|(uri, doc)| {
                sources.contains(uri) && doc.kind == crate::project::workspace::DocumentKind::Latex
            })
            .collect();
        // An unresolved source load can redefine previously known commands,
        // even after it has returned to a file included in the snapshot.
        let dynamic_sources = documents
            .iter()
            .filter(|(uri, doc)| {
                let alias_bodies = crate::symbols::commands::file_load_alias_body_positions(
                    &doc.text.source,
                    &doc.syntax,
                );
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
                            ) && crate::parser::syntax::loader_extension(&command.name).is_none()
                            {
                                return false;
                            }
                            if crate::parser::syntax::file_list(&command.name) {
                                return crate::parser::syntax::complete_file_list_spans(
                                    &doc.text.source,
                                    command,
                                )
                                .is_none();
                            }
                            crate::parser::syntax::file_argument(&doc.text.source, command)
                                .is_none_or(|argument| {
                                    let mut name = argument.value.trim().to_owned();
                                    for character in ['_', '%', '#', '&', '$', ' '] {
                                        name = name.replace(&format!("\\{character}"), "_");
                                    }
                                    !crate::parser::syntax::is_literal_file_path(&name)
                                })
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
    calls: [Option<(Url, Rc<InvocationCalls<'a>>)>; 12],
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
                signatures:
                    crate::symbols::commands::CommandSignatures::with_ambiguous_environments(
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
        if line == crate::process::output::TRUNCATED {
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
            let expression_details = boolean_expression_details(&lines, i + 1, number, &message);
            let (message, expression_end) =
                expression_details.map_or((message, None), |(message, end)| (message, Some(end)));
            // An unresolved reported source falls back to the root for display.
            // Its context must not identify coincidentally matching root text.
            let context = confirmed_path
                .then(|| error_context(&lines, i + 1, number, &message))
                .flatten()
                .map(|context| {
                    context.with_project(&message, analysis.as_mut(), &sources, &stack, &path)
                });
            let delimiter_end = (message == crate::parser::delimiter_operands::ERROR)
                .then(|| delimiter_error_tail(&lines, i + 1, number))
                .flatten();
            emit(
                &mut result,
                &path,
                number,
                message,
                severity,
                context,
                confirmed_path,
            );
            if let Some(end) = expression_end {
                i = end;
            }
            if let Some(end) = delimiter_end {
                i = i.max(end);
            }
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
                let expression_details = boolean_expression_details(&lines, i, number, &message);
                let (message, expression_end) = expression_details
                    .map_or((message, None), |(message, end)| (message, Some(end)));
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
                let delimiter_end = (message == crate::parser::delimiter_operands::ERROR)
                    .then(|| delimiter_error_tail(&lines, i, number))
                    .flatten();
                emit(
                    &mut result,
                    &path.path,
                    number,
                    message,
                    DiagnosticSeverity::ERROR,
                    context,
                    path.confirmed,
                );
                if let Some(end) = expression_end {
                    i = end;
                }
                if let Some(end) = delimiter_end {
                    i = i.max(end);
                }
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
#[cfg(test)]
mod tests;
