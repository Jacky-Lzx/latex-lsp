//! Bibliography tool messages and observed source locations.
use super::*;

pub fn parse_bibliography(root: &Path, log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    parse_bibliography_log(root, &root.with_extension("blg"), log)
}

pub fn parse_bibliography_log(
    root: &Path,
    log_path: &Path,
    log: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let paths = crate::compilation::recorder::BibliographyPaths::new(root, log_path, log);
    let mut result = parse_bibliography_in(root, &paths, log);
    if !crate::process::output::truncated(log) {
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
                return crate::bibliography::bib_resources::literal_path(name).then_some(name);
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
            && crate::parser::syntax::is_literal_file_path(name)
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
        let syntax = crate::parser::syntax::Syntax::latex(&text);
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
            .any(|command| crate::bibliography::bib_resources::loads(&command.name))
    });
    let signatures = (source_aware || resource_calls).then(|| {
        crate::symbols::commands::CommandSignatures::source_context(
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
                !(source_aware || crate::bibliography::bib_resources::loads(&command.name))
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
                    let Some(argument) =
                        crate::parser::syntax::file_argument(&text.source, command)
                    else {
                        return;
                    };
                    vec![argument.span.clone()]
                }
                MissingBibliographyKind::Database => {
                    if crate::bibliography::bib_resources::loads(&command.name) {
                        let Some(argument) =
                            crate::parser::syntax::file_argument(&text.source, command)
                        else {
                            return;
                        };
                        vec![argument.span.clone()]
                    } else {
                        let Some(spans) =
                            crate::parser::syntax::complete_file_list_spans(&text.source, command)
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
                        if crate::bibliography::bib_resources::loads(&command.name) {
                            value.to_owned()
                        } else {
                            let Some(name) =
                                crate::parser::syntax::file_lookup_names(&command.name, value)
                                    .next()
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
    if crate::process::output::truncated(output) {
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
    paths: &crate::compilation::recorder::BibliographyPaths<'_>,
    log: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut result = BTreeMap::new();
    let mut databases_complete = true;
    let databases: Vec<_> = log
        .lines()
        .filter_map(crate::compilation::recorder::bibliography_database_source)
        .filter_map(|name| {
            let path = paths.resolve(name);
            databases_complete &= path.is_some();
            path
        })
        .collect();
    let mut previous = "";
    let mut bibliography_context = crate::bibliography::bib_diagnostics::Context::new(log);
    let mut lines = log.lines().peekable();
    while let Some(line) = lines.next() {
        let biber = crate::compilation::recorder::biber_record(line);
        // Only this exact cycle may use the input history before its fatal
        // record. Recording the level still invalidates every later query.
        let cycle_inputs = biber
            .and_then(|(_, message)| {
                crate::bibliography::bib_diagnostics::biber_cycles::record(message)
            })
            .and_then(|_| bibliography_context.biber_cycle_history());
        if biber.is_some() {
            bibliography_context.biber_key_record(paths, line);
        }
        let native_warning = line.starts_with("Warning--");
        if biber.is_none() && !native_warning && !line.starts_with(" :") {
            bibliography_context.record(paths, line);
        }
        // Never interpret literal paths, source echoes, or an INFO/DEBUG
        // message containing an error-looking substring as another record.
        if crate::bibliography::bib_diagnostics::metadata(line)
            || line.starts_with(" :")
            || biber.is_some_and(|(level, _)| !matches!(level, "ERROR" | "WARN"))
        {
            previous = "";
            continue;
        }
        if let Some(warning) = crate::bibliography::bib_diagnostics::warning(line) {
            let location = lines
                .peek()
                .copied()
                .and_then(crate::bibliography::bib_diagnostics::position);
            if location.is_some() {
                lines.next();
            }
            if let Some((uri, diagnostic)) =
                bibliography_context.diagnostic(root, paths, warning, location)
            {
                result.entry(uri).or_insert_with(Vec::new).push(diagnostic);
            }
            previous = "";
            continue;
        }
        if let Some((key, field)) = crate::bibliography::bib_diagnostics::fields::record(line) {
            let location = lines
                .peek()
                .copied()
                .and_then(crate::bibliography::bib_diagnostics::position);
            if location.is_some() {
                lines.next();
            }
            if let Some((uri, diagnostic)) =
                bibliography_context.extra_field(root, paths, key, field, location)
            {
                result.entry(uri).or_insert_with(Vec::new).push(diagnostic);
            }
            previous = "";
            continue;
        }
        if let Some((kind, child)) = crate::bibliography::bib_diagnostics::crossref::record(line) {
            let parent = lines.peek().copied().and_then(|line| kind.parent(line));
            if parent.is_some() {
                lines.next();
            }
            if let Some((uri, diagnostic)) =
                bibliography_context.crossref(root, kind, child, parent)
            {
                result.entry(uri).or_insert_with(Vec::new).push(diagnostic);
            }
            previous = "";
            continue;
        }
        if line == crate::process::output::TRUNCATED {
            previous = "Error context omitted from a large bibliography log";
            continue;
        }
        if let Some((message, tail)) = line
            .split_once("---line ")
            .filter(|_| biber.is_none() && !native_warning)
        {
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
                    let entry_path = location
                        .clone()
                        .filter(|path| bibliography_context.is_database(name, path));
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
                    if message == "BibTeX: Repeated entry" {
                        if let Ok(uri) = Url::from_file_path(&path) {
                            if let Some(entries) = result.get_mut(&uri) {
                                for diagnostic in entries.iter_mut().filter(|diagnostic| {
                                    diagnostic.message == message
                                        && (diagnostic.range.start.line == number.saturating_sub(1)
                                            || diagnostic.data.as_ref().is_some_and(|data| {
                                                data.get("reportedLine")
                                                    .and_then(serde_json::Value::as_u64)
                                                    == Some(number.into())
                                            }))
                                }) {
                                    bibliography_context.repeated(
                                        entry_path.as_deref(),
                                        number,
                                        diagnostic,
                                    );
                                }
                            }
                        }
                    }
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
        if let Some((message, _)) = line
            .split_once("---while reading file ")
            .filter(|_| biber.is_none() && !native_warning)
        {
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
        } else if let Some(("ERROR", message)) = biber {
            (message, DiagnosticSeverity::ERROR, "Biber")
        } else if let Some(("WARN", message)) = biber {
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
        if tool == "Biber" {
            if let Some(warning) = crate::bibliography::bib_diagnostics::biber_keys::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_key_warning(root, paths, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_cycles::record(message)
            {
                if let Some((uri, diagnostic)) = bibliography_context.biber_cycle_diagnostic(
                    root,
                    warning,
                    cycle_inputs.as_deref(),
                    message,
                    severity,
                ) {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_aliases::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_alias_warning(root, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_nameparts::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_namepart_warning(root, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_names::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_name_warning(root, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_relations::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_relation_warning(root, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_xdata_fields::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_xdata_field_warning(root, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_xdata::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_xdata_warning(root, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_fields::record(message)
            {
                if let Some((uri, diagnostic)) = bibliography_context
                    .biber_field_warning(root, paths, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_required::record(message)
            {
                if let Some((uri, diagnostic)) = bibliography_context
                    .biber_required_warning(root, paths, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
                }
                continue;
            }
        }
        if tool == "Biber" {
            if let Some(warning) =
                crate::bibliography::bib_diagnostics::biber_types::record(message)
            {
                if let Some((uri, diagnostic)) =
                    bibliography_context.biber_type_warning(root, paths, warning, message, severity)
                {
                    result.entry(uri).or_default().push(diagnostic);
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
        let entry = key
            .filter(|_| explicit_path.is_some() || databases_complete)
            .and_then(|key| {
                bibliography_context.entry(explicit_path.iter().chain(databases.iter()), key)
            });
        let path = explicit_path
            .as_deref()
            .or_else(|| entry.as_ref().map(|(path, _)| path.as_path()))
            .unwrap_or(root);
        let diagnostic_message = format!("{tool}: {}", message.trim());
        if number.is_none() {
            if let Some((path, range)) = &entry {
                if let Ok(uri) = Url::from_file_path(path) {
                    result.entry(uri).or_default().push(Diagnostic {
                        range: *range,
                        severity: Some(severity),
                        source: Some("latex-build".into()),
                        message: diagnostic_message,
                        ..Default::default()
                    });
                }
                continue;
            }
        }
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
    paths: &crate::compilation::recorder::BibliographyPaths<'_>,
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
    paths: &'a crate::compilation::recorder::BibliographyPaths<'_>,
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
    paths: &crate::compilation::recorder::BibliographyPaths<'_>,
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
    paths: &crate::compilation::recorder::BibliographyPaths<'_>,
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
    paths: &crate::compilation::recorder::BibliographyPaths<'_>,
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
    paths: &crate::compilation::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<(
    usize,
    crate::compilation::recorder::RecordedBibliographySource,
)> {
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
    paths: &crate::compilation::recorder::BibliographyPaths<'_>,
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
