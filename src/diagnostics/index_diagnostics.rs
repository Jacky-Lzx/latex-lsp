//! makeindex processing diagnostics refer to generated inputs or real styles.
use std::{collections::BTreeMap, io::Read, path::Path};
use tower_lsp::lsp_types::*;

#[derive(Default)]
pub(crate) struct Report {
    pub diagnostics: BTreeMap<Url, Vec<Diagnostic>>,
    pub truncated: bool,
}
impl Report {
    pub(crate) fn first_error(&self) -> Option<&Diagnostic> {
        self.diagnostics
            .values()
            .flatten()
            .find(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Entry,
    Style,
    Warning,
}
struct Header<'a> {
    kind: Kind,
    file: Option<&'a str>,
    line: Option<u32>,
}
fn header(line: &str) -> Option<Header<'_>> {
    let (kind, tail) = if let Some(tail) = line.strip_prefix("!! Input index error") {
        (Kind::Entry, tail)
    } else if let Some(tail) = line.strip_prefix("** Input style error") {
        (Kind::Style, tail)
    } else {
        (Kind::Warning, line.strip_prefix("## Warning")?)
    };
    let body = tail
        .strip_prefix(match kind {
            Kind::Warning => " (input = ",
            _ => " (file = ",
        })
        .and_then(|tail| tail.strip_suffix("):"));
    let location = body.and_then(|body| {
        let input = if matches!(kind, Kind::Warning) {
            body.rsplit_once("; output = ")?.0
        } else {
            body
        };
        input.rsplit_once(", line = ")
    });
    let (file, line) = location.map_or((None, None), |(file, line)| {
        (
            Some(file),
            line.parse::<u32>().ok().filter(|line| *line > 0),
        )
    });
    Some(Header { kind, file, line })
}
#[derive(Default)]
struct Sources {
    texts: BTreeMap<std::path::PathBuf, Option<crate::text::Text>>,
    ranges: BTreeMap<(std::path::PathBuf, u32), Option<Range>>,
    bytes: usize,
}
impl Sources {
    fn range(&mut self, path: &Path, line: u32) -> Option<Range> {
        let key = crate::project::workspace::normalize_path(path);
        if let Some(range) = self.ranges.get(&(key.clone(), line)) {
            return *range;
        }
        if !self.texts.contains_key(&key) {
            if self.texts.len() >= 32 {
                return None;
            }
            let source = (|| {
                let metadata = std::fs::metadata(path).ok()?;
                if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
                    return None;
                }
                let mut source = String::new();
                std::fs::File::open(path)
                    .ok()?
                    .take(2 * 1024 * 1024 + 1)
                    .read_to_string(&mut source)
                    .ok()?;
                if source.len() > 2 * 1024 * 1024 || self.bytes + source.len() > 16 * 1024 * 1024 {
                    return None;
                }
                self.bytes += source.len();
                Some(crate::text::Text::new(source))
            })();
            self.texts.insert(key.clone(), source);
        }
        let range = self
            .texts
            .get(&key)
            .and_then(Option::as_ref)
            .and_then(|text| {
                let row = line.checked_sub(1)?;
                let start = text.offset(Position::new(row, 0))?;
                if start == text.source.len() {
                    return None;
                }
                let contents = text.source[start..].split('\n').next()?;
                let contents = contents.strip_suffix('\r').unwrap_or(contents);
                Some(Range::new(
                    Position::new(row, 0),
                    Position::new(row, contents.encode_utf16().count() as u32),
                ))
            });
        self.ranges.insert((key, line), range);
        range
    }
}

pub(crate) fn parse(root: &Path, input: &Path, transcript: &Path, log: &str) -> Report {
    let mut report = Report {
        truncated: crate::server::output::truncated(log),
        ..Default::default()
    };
    let lines: Vec<_> = log.lines().take(32769).collect();
    report.truncated |= lines.len() > 32768;
    let mut records = 0;
    let mut sources = Sources::default();
    for (at, line) in lines.iter().take(32768).enumerate() {
        let Some(header) = header(line) else {
            continue;
        };
        if records >= 1024 {
            report.truncated = true;
            break;
        }
        let body = lines
            .get(at + 1)
            .and_then(|line| line.strip_prefix("   -- "))
            .filter(|body| !body.trim().is_empty());
        if body.is_none() && matches!(header.kind, Kind::Warning) {
            continue;
        }
        let message = body
            .map(|body| body.chars().take(8192).collect::<String>())
            .unwrap_or_else(|| format!("Incomplete processing error: {line}"));
        let candidate = header
            .file
            .map(|name| input.parent().unwrap_or_else(|| Path::new(".")).join(name));
        let path = candidate.filter(|path| {
            matches!(header.kind, Kind::Style)
                || crate::project::workspace::normalize_path(path)
                    == crate::project::workspace::normalize_path(input)
        });
        let location = path
            .as_ref()
            .zip(header.line)
            .and_then(|(path, line)| sources.range(path, line).map(|range| (path.clone(), range)));
        let unresolved = location.is_none();
        let (path, range) = location.unwrap_or_else(|| (root.to_owned(), Range::default()));
        let Ok(uri) = Url::from_file_path(path) else {
            continue;
        };
        let code = match header.kind {
            Kind::Entry => "invalid-index-entry",
            Kind::Style => "invalid-index-style",
            Kind::Warning => "index-entry-warning",
        };
        report.diagnostics.entry(uri).or_default().push(Diagnostic {
            range, severity: Some(if matches!(header.kind, Kind::Warning) { DiagnosticSeverity::WARNING } else { DiagnosticSeverity::ERROR }),
            source: Some("makeindex".into()), code: Some(NumberOrString::String(code.into())),
            message: format!("makeindex: {message}"),
            data: Some(serde_json::json!({"inputUri":Url::from_file_path(input).ok(),"transcriptUri":Url::from_file_path(transcript).ok(),"reportedFile":header.file,"reportedLine":header.line,"unresolvedLocation":unresolved})),
            ..Default::default()
        });
        records += 1;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Fixture {
        directory: tempfile::TempDir,
        root: PathBuf,
        input: PathBuf,
        log: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("main.tex");
            let input = directory.path().join("index, line = data.idx");
            let log = directory.path().join("main.ilg");
            std::fs::write(&root, "Source\n").unwrap();
            std::fs::write(&input, "\\indexentry{😀}{1}\r\n\\indexentry{Broken}\r\n").unwrap();
            Self {
                directory,
                root,
                input,
                log,
            }
        }
        fn parse(&self, log: &str) -> Report {
            parse(&self.root, &self.input, &self.log, log)
        }
    }
    #[test]
    fn input_errors_and_warnings_use_exact_utf16_crlf_lines() {
        let fixture = Fixture::new();
        let result = fixture.parse("!! Input index error (file = ./index, line = data.idx, line = 2):\n   -- Missing arguments.\n## Warning (input = ./index, line = data.idx, line = 1; output = temporary.ind, line = 3):\n   -- Conflicting encaps.\n");
        let items = &result.diagnostics[&Url::from_file_path(&fixture.input).unwrap()];
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].range.start, Position::new(1, 0));
        assert_eq!(
            items[1].range.end.character,
            "\\indexentry{😀}{1}".encode_utf16().count() as u32
        );
        assert_eq!(items[1].severity, Some(DiagnosticSeverity::WARNING));
        assert!(result.first_error().is_some());
    }
    #[test]
    fn style_errors_use_actual_paths_and_missing_sources_fall_back_to_the_root() {
        let fixture = Fixture::new();
        let style = fixture.directory.path().join("style.ist");
        std::fs::write(&style, "unknown_option \"x\"\n").unwrap();
        let result = fixture.parse("** Input style error (file = ./style.ist, line = 1):\n   -- Unknown specifier.\n** Input style error (file = /missing/style.ist, line = 4):\n   -- Unreadable style.\n");
        assert_eq!(
            result.diagnostics[&Url::from_file_path(style).unwrap()][0].code,
            Some(NumberOrString::String("invalid-index-style".into()))
        );
        assert_eq!(
            result.diagnostics[&Url::from_file_path(&fixture.root).unwrap()][0]
                .data
                .as_ref()
                .unwrap()["unresolvedLocation"],
            true
        );
    }
    #[test]
    fn foreign_input_paths_invalid_rows_and_incomplete_errors_do_not_invent_locations() {
        let fixture = Fixture::new();
        std::fs::write(fixture.directory.path().join("other.idx"), "Other data\n").unwrap();
        let result = fixture.parse("!! Input index error (file = other.idx, line = 1):\n   -- Foreign entry.\n!! Input index error (file = ./index, line = data.idx, line = 99):\n   -- Missing row.\n** Input style error malformed header\n## Warning incomplete\n");
        assert_eq!(result.diagnostics.len(), 1);
        let items = &result.diagnostics[&Url::from_file_path(&fixture.root).unwrap()];
        assert_eq!(items.len(), 3);
        assert!(items
            .iter()
            .all(|item| item.data.as_ref().unwrap()["unresolvedLocation"] == true));
        assert!(result.first_error().is_some());
    }
    #[test]
    fn ordinary_log_prose_and_filename_lookalikes_do_not_create_diagnostics() {
        let fixture = Fixture::new();
        let result = fixture.parse("Scanning input file ./!! Input index error.idx....done (1 entries accepted, 0 rejected).\n   -- Unassociated text.\n ## Warning (input = other.idx, line = 1; output = x, line = 1):\n   -- Indented example.\n");
        assert!(result.diagnostics.is_empty());
    }
    #[test]
    fn record_and_log_limits_are_reported() {
        let fixture = Fixture::new();
        let log = "** Input style error malformed header\n   -- Broken style.\n".repeat(1025);
        let result = fixture.parse(&log);
        assert!(result.truncated);
        assert_eq!(
            result.diagnostics.values().map(Vec::len).sum::<usize>(),
            1024
        );
        let result = fixture.parse(&"ordinary line\n".repeat(32769));
        assert!(result.truncated);
    }
}
