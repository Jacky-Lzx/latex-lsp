//! Read bounded auxiliary metadata without executing TeX.
use crate::{
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
use tower_lsp::lsp_types::*;
#[derive(Clone, Debug)]
pub(crate) struct Reference {
    pub key: String,
    pub number: String,
    pub page: String,
    pub title: String,
    pub external: bool,
    pub source: Option<Url>,
}
#[derive(Default)]
struct Parsed {
    labels: BTreeMap<String, Reference>,
    inputs: Vec<String>,
}
type Stamp = (Option<SystemTime>, u64);
type Cache = BTreeMap<PathBuf, (Stamp, Arc<Parsed>)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
fn read(path: &Path) -> Option<Arc<Parsed>> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 2 * 1024 * 1024 {
        return None;
    }
    let stamp = (meta.modified().ok(), meta.len());
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some((_, data)) = cache.get(path).filter(|(old, _)| *old == stamp) {
        return Some(data.clone());
    }
    let mut source = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(2 * 1024 * 1024 + 1)
        .read_to_string(&mut source)
        .ok()?;
    if source.len() > 2 * 1024 * 1024 {
        return None;
    }
    let value = Arc::new(parse(&source));
    if cache.len() >= 32 {
        cache.clear();
    }
    cache.insert(path.into(), (stamp, value.clone()));
    Some(value)
}
fn preview(value: &str) -> String {
    value.chars().take(256).collect()
}
fn parse(source: &str) -> Parsed {
    let mut result = Parsed::default();
    let mut at = 0;
    while at < source.len() && result.labels.len() < 4096 {
        match source.as_bytes()[at] {
            b'%' => {
                at = source[at..]
                    .find('\n')
                    .map_or(source.len(), |len| at + len + 1);
                continue;
            }
            b'{' => {
                if let Some((_, end)) = syntax::group(source, at, b'{', b'}') {
                    at = end;
                    continue;
                }
            }
            b'\\' => {
                if let Some(word) = syntax::control_word(source, at) {
                    let name = &source[word.clone()];
                    at = word.end;
                    if name == r"\newlabel" || name == r"\@input" {
                        let Some((key, end)) =
                            syntax::group(source, syntax::skip_space(source, at), b'{', b'}')
                        else {
                            continue;
                        };
                        at = end;
                        if name == r"\@input" {
                            if key.value.ends_with(".aux")
                                && syntax::is_literal_file_path(&key.value)
                            {
                                result.inputs.push(key.value);
                            }
                            continue;
                        }
                        let Some((body, end)) =
                            syntax::group(source, syntax::skip_space(source, at), b'{', b'}')
                        else {
                            continue;
                        };
                        at = end;
                        if !syntax::is_static_reference_key(&key.value)
                            || key.value.ends_with("@cref")
                        {
                            continue;
                        }
                        let mut fields = Vec::new();
                        let mut field = body.span.start;
                        for _ in 0..3 {
                            let Some((arg, end)) = syntax::group(
                                source,
                                syntax::skip_space(source, field),
                                b'{',
                                b'}',
                            ) else {
                                break;
                            };
                            if end > body.span.end {
                                break;
                            }
                            fields.push(preview(&arg.value));
                            field = end;
                        }
                        if fields.len() >= 2 {
                            result.labels.insert(
                                key.value.clone(),
                                Reference {
                                    key: key.value.clone(),
                                    number: fields[0].clone(),
                                    page: fields[1].clone(),
                                    title: fields.get(2).cloned().unwrap_or_default(),
                                    external: false,
                                    source: None,
                                },
                            );
                        }
                    }
                    continue;
                }
            }
            _ => {}
        }
        at += source[at..].chars().next().unwrap().len_utf8();
    }
    result
}
fn collect(
    path: PathBuf,
    prefix: &str,
    external: bool,
    source: Option<Url>,
    result: &mut BTreeMap<String, Reference>,
) {
    let mut pending = vec![(path, source)];
    let mut seen = BTreeSet::new();
    while let Some((path, source)) = pending.pop() {
        if seen.len() >= 16 || !seen.insert(path.clone()) {
            continue;
        }
        let Some(data) = read(&path) else {
            continue;
        };
        for (name, info) in &data.labels {
            if result.len() >= 8192 {
                return;
            }
            let mut info = info.clone();
            info.external = external;
            info.source = source.clone();
            result.entry(format!("{prefix}{name}")).or_insert(info);
        }
        for input in &data.inputs {
            let Some(parent) = path.parent() else {
                continue;
            };
            let child = parent.join(input);
            let tex = child.with_extension("tex");
            let child_source = if tex.is_file() {
                Url::from_file_path(tex).ok()
            } else {
                source.clone()
            };
            pending.push((child, child_source));
        }
    }
}
pub(crate) fn labels(ws: &Workspace, uri: &Url) -> BTreeMap<String, Reference> {
    let root = ws.root_document(uri).unwrap_or_else(|_| uri.clone());
    let mut result = BTreeMap::new();
    let Ok(path) = root.to_file_path() else {
        return result;
    };
    let config = ws.build_config(&root);
    let auxiliary = config
        .artifact_path(&path, "aux")
        .filter(|path| path.is_file())
        .unwrap_or_else(|| path.with_extension("aux"));
    collect(auxiliary, "", false, None, &mut result);
    let scope = ws.symbol_scope(uri);
    for (source, doc) in &ws.documents {
        if !scope.contains(source) || doc.kind != DocumentKind::Latex {
            continue;
        }
        let positions = crate::signature::CommandSignatures::source_context(std::iter::once((
            doc.text.source.as_str(),
            &doc.syntax,
        )))
        .source_command_positions(&doc.text.source, &doc.syntax);
        for cmd in doc
            .syntax
            .commands
            .iter()
            .filter(|cmd| cmd.name == "externaldocument" && positions.contains(&cmd.span.start))
        {
            let mut at = syntax::skip_space(&doc.text.source, cmd.span.end);
            let mut prefix = String::new();
            if let Some((arg, end)) = syntax::bracket_argument(&doc.text.source, at) {
                prefix = arg.value;
                at = syntax::skip_space(&doc.text.source, end);
            }
            let Some((arg, _)) = syntax::group(&doc.text.source, at, b'{', b'}') else {
                continue;
            };
            if !syntax::is_literal_file_path(&arg.value)
                || !prefix.is_empty() && !syntax::is_static_reference_key(&prefix)
            {
                continue;
            }
            let Some(parent) = path.parent() else {
                continue;
            };
            let base = parent.join(arg.value);
            let tex = base.with_extension("tex");
            let source = tex
                .is_file()
                .then(|| Url::from_file_path(tex).ok())
                .flatten();
            let aux = if base.extension().is_some_and(|extension| extension == "aux") {
                base
            } else {
                PathBuf::from(format!("{}.aux", base.display()))
            };
            collect(aux, &prefix, true, source, &mut result);
        }
    }
    result
}
pub(crate) fn describe(info: &Reference) -> String {
    format!(
        "Last compiled reference: {}\nPage: {}{}{}",
        info.number,
        info.page,
        if info.title.is_empty() {
            ""
        } else {
            "\nTitle: "
        },
        info.title
    )
}
pub(crate) fn definition(ws: &Workspace, uri: &Url, name: &str) -> Option<Location> {
    let info = labels(ws, uri).remove(name)?;
    if !info.external {
        return None;
    }
    let source = info.source?;
    let path = source.to_file_path().ok()?;
    if std::fs::metadata(&path).ok()?.len() > 2 * 1024 * 1024 {
        return None;
    }
    let doc = Document::new(&source, std::fs::read_to_string(path).ok()?, None);
    let entry = doc
        .syntax
        .labels
        .iter()
        .find(|entry| entry.name == info.key);
    Some(Location {
        uri: source,
        range: entry
            .map(|entry| doc.text.range(entry.span.start, entry.span.end))
            .unwrap_or_default(),
    })
}

pub(crate) fn hints(ws: &Workspace, uri: &Url, range: Range) -> Vec<InlayHint> {
    let config = ws.settings(uri).inlay_hints;
    if !config.enabled || !config.numbers && !config.pages {
        return vec![];
    }
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let labels = labels(ws, uri);
    crate::reference_diagnostics::occurrences(doc)
        .into_iter()
        .filter(|(citation, _)| !*citation)
        .filter_map(|(_, span)| {
            let position = doc.text.position(span.end);
            if position < range.start || position > range.end {
                return None;
            }
            let info = labels.get(&doc.text.source[span])?;
            let mut parts = Vec::new();
            if config.numbers && !info.number.is_empty() {
                parts.push(info.number.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            if config.pages && !info.page.is_empty() {
                parts.push(format!(
                    "p. {}",
                    info.page.split_whitespace().collect::<Vec<_>>().join(" ")
                ));
            }
            if parts.is_empty() {
                return None;
            }
            Some(InlayHint {
                position,
                label: InlayHintLabel::String(format!("→ {}", parts.join(", "))),
                kind: None,
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String(format!(
                    "{}\nValues come from an auxiliary file and may be stale.",
                    describe(info)
                ))),
                padding_left: Some(true),
                padding_right: Some(false),
                data: None,
            })
        })
        .take(1024)
        .collect()
}

pub(crate) fn document_links(ws: &Workspace, uri: &Url, root: &Url) -> Vec<DocumentLink> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let Ok(path) = root.to_file_path() else {
        return vec![];
    };
    let Some(parent) = path.parent() else {
        return vec![];
    };
    let positions = crate::signature::CommandSignatures::source_context(std::iter::once((
        doc.text.source.as_str(),
        &doc.syntax,
    )))
    .source_command_positions(&doc.text.source, &doc.syntax);
    doc.syntax
        .commands
        .iter()
        .filter(|cmd| cmd.name == "externaldocument" && positions.contains(&cmd.span.start))
        .filter_map(|cmd| {
            let mut at = syntax::skip_space(&doc.text.source, cmd.span.end);
            if let Some((_, end)) = syntax::bracket_argument(&doc.text.source, at) {
                at = syntax::skip_space(&doc.text.source, end);
            }
            let (arg, _) = syntax::group(&doc.text.source, at, b'{', b'}')?;
            if !syntax::is_literal_file_path(&arg.value) {
                return None;
            }
            let base = parent.join(arg.value);
            let tex = base.with_extension("tex");
            let aux = PathBuf::from(format!("{}.aux", base.display()));
            let target = if tex.is_file() {
                tex
            } else if aux.is_file() {
                aux
            } else {
                return None;
            };
            Some(DocumentLink {
                range: doc.text.range(arg.span.start, arg.span.end),
                target: Url::from_file_path(target).ok(),
                tooltip: Some("External document".into()),
                data: None,
            })
        })
        .collect()
}
