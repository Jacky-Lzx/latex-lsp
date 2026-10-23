//! BibTeX diagnostics retain database roles, token ranges and input order.
pub(crate) mod biber_fields;
pub(crate) mod biber_keys;
pub(crate) mod biber_nameparts;
pub(crate) mod biber_names;
pub(crate) mod crossref;
pub(crate) mod fields;

use crate::{bib_strings, recorder::BibliographyPaths, syntax, text::Text, workspace::Workspace};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tower_lsp::lsp_types::*;

pub(crate) const CODE: &str = "undefined-bibliography-string";
const TYPE_CODE: &str = "undefined-bibliography-entry-type";
const REPEATED_CODE: &str = "repeated-bibliography-entry";
const MAX_INPUTS: usize = 1024;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_SIZE: usize = 16 * 1024 * 1024;

fn name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("BibTeX: string name \"")?
        .strip_suffix("\" is undefined")?;
    bib_strings::identifier(name).then_some(name)
}

#[derive(Clone, Copy)]
pub(crate) enum Warning<'a> {
    String(&'a str),
    EntryType(&'a str),
}

impl Warning<'_> {
    fn code(self) -> &'static str {
        match self {
            Self::String(_) => CODE,
            Self::EntryType(_) => TYPE_CODE,
        }
    }

    fn message(self) -> String {
        match self {
            Self::String(name) => format!("BibTeX: string name \"{name}\" is undefined"),
            Self::EntryType(key) => {
                format!("BibTeX: entry type for \"{key}\" isn't style-file defined")
            }
        }
    }
}

pub(crate) fn warning(line: &str) -> Option<Warning<'_>> {
    if let Some(name) = line
        .strip_prefix("Warning--string name \"")
        .and_then(|tail| tail.strip_suffix("\" is undefined"))
    {
        return bib_strings::identifier(name).then_some(Warning::String(name));
    }
    let key = line
        .strip_prefix("Warning--entry type for \"")?
        .strip_suffix("\" isn't style-file defined")?;
    (!key.is_empty() && !key.chars().any(char::is_control)).then_some(Warning::EntryType(key))
}

pub(crate) fn position(line: &str) -> Option<(u32, &str)> {
    let (number, path) = line.strip_prefix("--line ")?.split_once(" of file ")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) || path.is_empty() {
        return None;
    }
    Some((number.parse().ok()?, path))
}

pub(crate) fn metadata(line: &str) -> bool {
    [
        "Database file #",
        "The style file: ",
        "The top-level auxiliary file: ",
        "This is BibTeX, ",
        "Capacity: ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

fn read(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_SIZE {
        return None;
    }
    let mut source = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_SIZE + 1)
        .read_to_string(&mut source)
        .ok()?;
    (source.len() as u64 <= MAX_FILE_SIZE).then_some(source)
}

struct Parsed {
    text: Text,
    syntax: syntax::Syntax,
    hash: u64,
    entry_keys: BTreeMap<String, Option<std::ops::Range<usize>>>,
    biber_locations: OnceLock<biber_keys::Locations>,
    biber_field_locations: OnceLock<biber_fields::Locations>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Proof {
    uri: Url,
    hash: u64,
    database: bool,
}

pub(crate) struct Context {
    inputs: Vec<(PathBuf, bool)>,
    databases: usize,
    complete: bool,
    documents: BTreeMap<PathBuf, Option<Arc<Parsed>>>,
    bytes: usize,
    field_warnings: fields::Warnings,
    biber_sources: biber_keys::Sources,
}

impl Context {
    pub fn new(log: &str) -> Self {
        Self {
            inputs: vec![],
            databases: 0,
            complete: !crate::output::truncated(log),
            documents: BTreeMap::new(),
            bytes: 0,
            field_warnings: fields::Warnings::new(log),
            biber_sources: biber_keys::Sources::new(log),
        }
    }

    pub fn record(&mut self, paths: &BibliographyPaths<'_>, line: &str) {
        let (path, database) = if let Some(tail) = line.strip_prefix("Database file #") {
            let Some((number, path)) = tail.split_once(": ") else {
                self.complete = false;
                return;
            };
            self.databases += 1;
            if !number.bytes().all(|byte| byte.is_ascii_digit())
                || number.parse::<usize>().ok() != Some(self.databases)
            {
                self.complete = false;
            }
            if !path.trim_matches('"').ends_with(".bib") {
                self.complete = false;
                return;
            }
            (path, true)
        } else if let Some(path) = line.strip_prefix("The style file: ") {
            (path, false)
        } else {
            // Error recovery may skip declarations the editor can still index.
            // Field-like text inside a recorded path is not an error record.
            if !metadata(line) && line.contains("---line ") {
                self.complete = false;
            }
            return;
        };
        let Some(path) = paths.resolve(path) else {
            self.complete &= !database;
            return;
        };
        if self.inputs.len() >= MAX_INPUTS
            || database && self.inputs.iter().any(|(other, db)| *db && *other == path)
        {
            self.complete = false;
            return;
        }
        self.inputs.push((path, database));
    }

    fn document(&mut self, path: &Path) -> Option<Arc<Parsed>> {
        if self.documents.len() >= MAX_INPUTS && !self.documents.contains_key(path) {
            return None;
        }
        let bytes = &mut self.bytes;
        self.documents
            .entry(path.into())
            .or_insert_with(|| {
                let source = read(path)?;
                *bytes = bytes.saturating_add(source.len());
                if *bytes > MAX_TOTAL_SIZE {
                    return None;
                }
                let fingerprint = hash(&source);
                let text = Text::new(source);
                let syntax = syntax::Syntax::bibtex(&text);
                let mut entry_keys = BTreeMap::new();
                for entry in &syntax.citations {
                    entry_keys
                        .entry(entry.name.clone())
                        .and_modify(|range| *range = None)
                        .or_insert_with(|| Some(entry.span.clone()));
                }
                Some(Arc::new(Parsed {
                    text,
                    syntax,
                    hash: fingerprint,
                    entry_keys,
                    biber_locations: OnceLock::new(),
                    biber_field_locations: OnceLock::new(),
                }))
            })
            .clone()
    }

    /// A generic tool message provides a key but no declaration line. Every
    /// candidate database must be readable and the key unique across them.
    pub fn entry<'a>(
        &mut self,
        paths: impl Iterator<Item = &'a PathBuf>,
        key: &str,
    ) -> Option<(PathBuf, Range)> {
        let mut seen = BTreeSet::new();
        let mut selected = None;
        for path in paths {
            if !seen.insert(path) {
                continue;
            }
            let parsed = self.document(path)?;
            if let Some(range) = parsed.entry_keys.get(key) {
                let span = range.as_ref()?;
                if selected.is_some() {
                    return None;
                }
                selected = Some((path.clone(), parsed.text.range(span.start, span.end)));
            }
        }
        selected
    }

    pub fn is_database(&self, name: &str, path: &Path) -> bool {
        let name = name.trim().trim_matches('"');
        // The native tool reports opened names. Auxiliary/style aliases keep
        // their role even when their target is also a recorded database.
        if matches!(
            Path::new(name).extension().and_then(|ext| ext.to_str()),
            Some("aux" | "bst")
        ) {
            return false;
        }
        name.ends_with(".bib") || self.inputs.iter().any(|(input, db)| *db && *input == path)
    }

    /// Native repeated-entry errors do not contain the key. Only a unique
    /// declaration on the reported line can identify it without guessing.
    pub fn repeated(&mut self, path: Option<&Path>, line: u32, diagnostic: &mut Diagnostic) {
        diagnostic.code = Some(NumberOrString::String(REPEATED_CODE.into()));
        let mut data = diagnostic
            .data
            .take()
            .unwrap_or_else(|| serde_json::json!({}));
        let Some(parsed) = path.and_then(|path| self.document(path)) else {
            data["unresolvedSource"] = true.into();
            diagnostic.data = Some(data);
            return;
        };
        let Some(selected) = line
            .checked_sub(1)
            .filter(|line| (*line as usize) < parsed.text.text_line_count())
        else {
            diagnostic.data = Some(data);
            return;
        };
        let mut entries = parsed.syntax.citations.iter().filter(|entry| {
            parsed.text.position(entry.span.start).line == selected
                && !entry.name.chars().any(char::is_whitespace)
        });
        let Some(entry) = entries.next().filter(|_| entries.next().is_none()) else {
            data["unresolvedEntry"] = true.into();
            diagnostic.data = Some(data);
            return;
        };
        diagnostic.range = parsed.text.range(entry.span.start, entry.span.end);
        data["key"] = entry.name.clone().into();
        diagnostic.data = Some(data);
        let Some(path) = path.filter(|path| {
            self.inputs
                .iter()
                .rev()
                .find(|(_, db)| *db)
                .is_some_and(|(last, _)| last == path)
        }) else {
            return;
        };
        let inputs = self.inputs.clone();
        let mut related = Vec::new();
        let mut seen = BTreeSet::new();
        let mut examined = 0;
        for (input, database) in inputs {
            if !database || !seen.insert(input.clone()) {
                continue;
            }
            let Some(other) = self.document(&input) else {
                continue;
            };
            for earlier in &other.syntax.citations {
                examined += 1;
                if examined > 4096 || related.len() >= 16 {
                    break;
                }
                if earlier.name.eq_ignore_ascii_case(&entry.name)
                    && (input != path || earlier.span.start < entry.span.start)
                {
                    let Ok(uri) = Url::from_file_path(&input) else {
                        continue;
                    };
                    related.push(DiagnosticRelatedInformation {
                        location: Location::new(
                            uri,
                            other.text.range(earlier.span.start, earlier.span.end),
                        ),
                        message: format!("Earlier bibliography entry with key `{}`", earlier.name),
                    });
                }
            }
            if input == path || examined > 4096 || related.len() >= 16 {
                break;
            }
        }
        if !related.is_empty() {
            diagnostic.related_information = Some(related);
        }
    }

    pub fn diagnostic(
        &mut self,
        root: &Path,
        paths: &BibliographyPaths<'_>,
        warning: Warning<'_>,
        location: Option<(u32, &str)>,
    ) -> Option<(Url, Diagnostic)> {
        let location = location.and_then(|(line, name)| {
            let path = paths.resolve(name)?;
            // Auxiliary/style names must not borrow a matching bibliography token.
            let database = self.is_database(name, &path);
            database.then_some((path, line))
        });
        let mut data = match warning {
            Warning::String(abbreviation) => serde_json::json!({"abbreviation":abbreviation}),
            Warning::EntryType(key) => serde_json::json!({"key":key}),
        };
        let mut range = Range::default();
        let mut uri = Url::from_file_path(root).ok()?;
        if let Some((path, line)) = location {
            if let Some(parsed) = self.document(&path) {
                uri = Url::from_file_path(&path).ok()?;
                let valid_line = line
                    .checked_sub(1)
                    .filter(|line| (*line as usize) < parsed.text.text_line_count());
                let selected = valid_line.unwrap_or_else(|| {
                    line.saturating_sub(1)
                        .min(parsed.text.text_line_count().saturating_sub(1) as u32)
                });
                let start = parsed.text.offset(Position::new(selected, 0))?;
                let end = parsed.text.source[start..]
                    .find('\n')
                    .map_or(parsed.text.source.len(), |length| start + length);
                let end = end - usize::from(parsed.text.source[..end].ends_with('\r'));
                range = parsed.text.range(start, end);
                if valid_line.is_none() {
                    data["unresolvedLine"] = true.into();
                    data["reportedLine"] = line.into();
                } else if let Warning::EntryType(key) = warning {
                    let mut types =
                        parsed
                            .syntax
                            .bibliography_completion
                            .types
                            .iter()
                            .filter(|kind| {
                                kind.key.as_ref().is_some_and(|span| {
                                    parsed.text.source[span.clone()] == *key
                                        && parsed.text.position(span.start).line == selected
                                })
                            });
                    if let Some(kind) = types.next().filter(|_| types.next().is_none()) {
                        range = parsed.text.range(kind.span.start, kind.span.end);
                        data["entryType"] = parsed.text.source[kind.span.clone()].to_owned().into();
                    } else {
                        data["unresolvedEntry"] = true.into();
                    }
                } else {
                    let Warning::String(abbreviation) = warning else {
                        unreachable!()
                    };
                    let matches: Vec<_> = parsed
                        .syntax
                        .bibliography_strings
                        .references
                        .iter()
                        .filter(|entry| {
                            entry.name.eq_ignore_ascii_case(abbreviation)
                                && parsed.text.position(entry.span.start).line == selected
                        })
                        .collect();
                    if matches.len() == 1 {
                        let entry = matches[0];
                        range = parsed.text.range(entry.span.start, entry.span.end);
                        if self.complete
                            && self
                                .inputs
                                .iter()
                                .rev()
                                .find(|(_, db)| *db)
                                .is_some_and(|(input, _)| *input == path)
                        {
                            let inputs = self.inputs.clone();
                            let proofs: Option<Vec<_>> = inputs
                                .into_iter()
                                .map(|(path, database)| {
                                    let parsed = self.document(&path)?;
                                    Some(Proof {
                                        uri: Url::from_file_path(path).ok()?,
                                        hash: parsed.hash,
                                        database,
                                    })
                                })
                                .collect();
                            if let Some(proofs) = proofs {
                                data["bibliographyInputs"] = serde_json::to_value(proofs).ok()?;
                            }
                        }
                    } else {
                        data["unresolvedReference"] = true.into();
                    }
                }
            } else {
                data["unresolvedSource"] = true.into();
            }
        } else {
            data["unresolvedSource"] = true.into();
        }
        Some((
            uri,
            Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("latex-build".into()),
                code: Some(NumberOrString::String(warning.code().into())),
                message: warning.message(),
                data: Some(data),
                ..Default::default()
            },
        ))
    }
}

pub(crate) fn fixes(
    workspace: &Workspace,
    root: &Url,
    uri: &Url,
    diagnostic: &Diagnostic,
    comparisons: &mut usize,
) -> Vec<syntax::QuickFix> {
    // Include TeX/configuration and opened-path changes when this diagnostic
    // came from a build, including a symlink selecting a different database.
    if [
        workspace.build_checks.get(root),
        workspace.bibliography_checks.get(root),
    ]
    .into_iter()
    .flatten()
    .any(|snapshot| snapshot.changed_input().is_some() || snapshot.changed_path().is_some())
    {
        return vec![];
    }
    let Some(doc) = workspace
        .documents
        .get(uri)
        .filter(|doc| doc.kind == crate::workspace::DocumentKind::Bibtex)
    else {
        return vec![];
    };
    let Some(reported) = name(&diagnostic.message) else {
        return vec![];
    };
    let Some(data) = diagnostic.data.as_ref() else {
        return vec![];
    };
    if diagnostic.code != Some(NumberOrString::String(CODE.into()))
        || data.get("abbreviation").and_then(|name| name.as_str()) != Some(reported)
        || data
            .get("unresolvedReference")
            .and_then(|value| value.as_bool())
            == Some(true)
        || !(2..=128).contains(&reported.chars().count())
    {
        return vec![];
    }
    let Some(reference) = doc
        .syntax
        .bibliography_strings
        .references
        .iter()
        .find(|entry| {
            entry.name.eq_ignore_ascii_case(reported)
                && doc.text.range(entry.span.start, entry.span.end) == diagnostic.range
        })
    else {
        return vec![];
    };
    let Some(proofs) = data
        .get("bibliographyInputs")
        .and_then(|value| serde_json::from_value::<Vec<Proof>>(value.clone()).ok())
    else {
        return vec![];
    };
    if proofs.is_empty()
        || proofs.len() > MAX_INPUTS
        || proofs
            .iter()
            .rev()
            .find(|proof| proof.database)
            .is_none_or(|proof| proof.uri != *uri)
    {
        return vec![];
    }
    let project = workspace.project_documents(root);
    let mut seen_inputs = BTreeSet::new();
    // Month abbreviations come from styles, not the BibTeX language itself.
    // Only database declarations confirmed by the input history are candidates.
    let mut candidates: BTreeMap<String, String> = BTreeMap::new();
    let mut entries = 0;
    let mut bytes = 0usize;
    for proof in proofs {
        let Some(source) = proof.uri.to_file_path().ok().and_then(|path| read(&path)) else {
            return vec![];
        };
        bytes = bytes.saturating_add(source.len());
        if bytes > MAX_TOTAL_SIZE {
            return vec![];
        }
        if hash(&source) != proof.hash
            || workspace
                .documents
                .get(&proof.uri)
                .is_some_and(|doc| hash(&doc.text.source) != proof.hash)
        {
            return vec![];
        }
        if !proof.database {
            continue;
        }
        if !project.contains(&proof.uri) || !seen_inputs.insert(proof.uri.clone()) {
            return vec![];
        }
        let Some(document) = workspace
            .documents
            .get(&proof.uri)
            .filter(|doc| doc.kind == crate::workspace::DocumentKind::Bibtex)
        else {
            return vec![];
        };
        for entry in &document.syntax.bibliography_strings.definitions {
            entries += 1;
            if entries > 4096 {
                return vec![];
            }
            if (proof.uri != *uri || entry.span.start < reference.span.start)
                && document
                    .syntax
                    .bibliography_strings
                    .available_definitions
                    .contains(&entry.span.start)
            {
                candidates.insert(entry.name.to_ascii_lowercase(), entry.name.clone());
            }
        }
    }
    let key = reported.to_ascii_lowercase();
    if candidates.contains_key(&key) {
        return vec![];
    }
    let mut candidates: Vec<_> = candidates
        .into_iter()
        .take(crate::spelling_fixes::MAX_COMPARISONS.saturating_sub(*comparisons))
        .filter_map(|(normalized, name)| {
            *comparisons += 1;
            crate::spelling_fixes::distance(
                &key,
                &normalized,
                if key.chars().count() < 8 { 1 } else { 2 },
            )
            .map(|score| (score, name))
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .take(5)
        .map(|(_, name)| syntax::QuickFix {
            title: format!("Replace with bibliography abbreviation `{name}`"),
            diagnostic: diagnostic.clone(),
            edit: TextEdit {
                range: diagnostic.range,
                new_text: name,
            },
            additional_edits: vec![],
        })
        .collect()
}
