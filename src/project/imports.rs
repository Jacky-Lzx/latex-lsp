//! Resolve static import-package paths while keeping imported directories scoped.
use crate::{
    language::signature::CommandSignatures,
    parser::syntax,
    project::workspace::{DocumentKind, Workspace},
};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub(crate) fn command(name: &str) -> bool {
    matches!(
        name,
        "import" | "subimport" | "inputfrom" | "subinputfrom" | "includefrom" | "subincludefrom"
    )
}
fn arguments(source: &str, cmd: &syntax::Command) -> Option<(syntax::Argument, syntax::Argument)> {
    let mut at = syntax::skip_space(source, cmd.span.end);
    if source.as_bytes().get(at) == Some(&b'*') {
        at = syntax::skip_space(source, at + 1);
    }
    let (directory, end) = syntax::group(source, at, b'{', b'}')?;
    let at = syntax::skip_space(source, end);
    let file = syntax::group(source, at, b'{', b'}')
        .map(|(arg, _)| arg)
        .or_else(|| {
            (source.as_bytes().get(at) == Some(&b'{')).then(|| syntax::Argument {
                value: source[at + 1..].into(),
                span: at + 1..source.len(),
                outer_span: at..source.len(),
            })
        })?;
    Some((directory, file))
}
#[derive(Clone)]
struct Call {
    argument: syntax::Argument,
    directory: PathBuf,
    target: Option<Url>,
}
#[derive(Default)]
struct Index {
    calls: BTreeMap<(Url, usize), Vec<Call>>,
}
impl Index {
    fn new(ws: &Workspace, root: &Url) -> Self {
        let mut result = Self::default();
        if !ws
            .documents
            .values()
            .any(|doc| doc.syntax.commands.iter().any(|cmd| command(&cmd.name)))
        {
            return result;
        }
        let Some(base) = root
            .to_file_path()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
        else {
            return result;
        };
        let mut pending = vec![(root.clone(), base.clone(), false, 0)];
        let mut seen = BTreeSet::new();
        while let Some((uri, directory, imported, depth)) = pending.pop() {
            if seen.len() >= 256
                || depth >= ws.config.indexing.max_depth
                || !seen.insert((uri.clone(), directory.clone(), imported))
            {
                continue;
            }
            let Some(doc) = ws
                .documents
                .get(&uri)
                .filter(|doc| doc.kind == DocumentKind::Latex)
            else {
                continue;
            };
            let positions = CommandSignatures::source_context(std::iter::once((
                doc.text.source.as_str(),
                &doc.syntax,
            )))
            .source_command_positions(&doc.text.source, &doc.syntax);
            for cmd in doc
                .syntax
                .commands
                .iter()
                .filter(|cmd| positions.contains(&cmd.span.start))
            {
                let call = if command(&cmd.name) {
                    let Some((dir, arg)) = arguments(&doc.text.source, cmd) else {
                        continue;
                    };
                    if !syntax::is_literal_file_path(&dir.value) && !dir.value.is_empty() {
                        continue;
                    }
                    if !syntax::is_literal_file_path(&arg.value) {
                        continue;
                    }
                    let origin = if cmd.name.starts_with("sub") {
                        &directory
                    } else {
                        &base
                    };
                    let directory =
                        crate::project::workspace::normalize_path(&origin.join(&dir.value));
                    let target =
                        resolve(&directory, &arg.value).or_else(|| resolve(&base, &arg.value));
                    Some((
                        Call {
                            argument: arg,
                            directory,
                            target,
                        },
                        true,
                    ))
                } else if matches!(cmd.name.as_str(), "input" | "include" | "subfile") {
                    let Some(arg) = syntax::file_argument(&doc.text.source, cmd) else {
                        continue;
                    };
                    let target = resolve(&directory, &arg.value)
                        .or_else(|| resolve(&base, &arg.value))
                        .or_else(|| {
                            crate::project::workspace::resolve_file_path(
                                &uri,
                                &cmd.name,
                                &arg.value,
                                &["tex"],
                            )
                        });
                    Some((
                        Call {
                            argument: arg.into_owned(),
                            directory: directory.clone(),
                            target,
                        },
                        imported,
                    ))
                } else {
                    None
                };
                if let Some((call, next_imported)) = call {
                    if let Some(target) = &call.target {
                        pending.push((
                            target.clone(),
                            call.directory.clone(),
                            next_imported,
                            depth + 1,
                        ));
                    }
                    if command(&cmd.name) || imported {
                        result
                            .calls
                            .entry((uri.clone(), cmd.span.start))
                            .or_default()
                            .push(call);
                    }
                }
            }
        }
        result
    }
}
fn resolve(directory: &std::path::Path, value: &str) -> Option<Url> {
    let origin = Url::from_file_path(directory.join("__latex_lsp_import__.tex")).ok()?;
    crate::project::workspace::resolve_file_path(&origin, "input", value, &["tex"])
}

pub(crate) fn dependencies(
    ws: &Workspace,
    uri: &Url,
    root: &Url,
    cmd: &syntax::Command,
) -> Option<Vec<Url>> {
    if !command(&cmd.name) && !matches!(cmd.name.as_str(), "input" | "include" | "subfile") {
        return None;
    }
    let index = Index::new(ws, root);
    index
        .calls
        .get(&(uri.clone(), cmd.span.start))
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| call.target.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
}

pub(crate) fn update_links(ws: &Workspace, uri: &Url, root: &Url, links: &mut Vec<DocumentLink>) {
    let Some(doc) = ws.documents.get(uri) else {
        return;
    };
    for ((source, _), calls) in Index::new(ws, root).calls {
        if source != *uri {
            continue;
        }
        let Some(call) = calls.first() else {
            continue;
        };
        let range = doc
            .text
            .range(call.argument.span.start, call.argument.span.end);
        links.retain(|link| link.range != range);
        let targets: BTreeSet<_> = calls
            .iter()
            .filter_map(|call| call.target.clone())
            .collect();
        if targets.len() == 1 && calls.iter().all(|call| call.target.is_some()) {
            links.push(DocumentLink {
                range,
                target: targets.into_iter().next(),
                tooltip: Some("Imported source".into()),
                data: None,
            });
        }
    }
}

pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let doc = ws.documents.get(uri)?;
    let cmd = doc.syntax.commands.iter().find(|cmd| {
        command(&cmd.name)
            && arguments(&doc.text.source, cmd)
                .is_some_and(|(_, arg)| arg.span.start <= offset && offset <= arg.span.end)
    })?;
    let (dir, arg) = arguments(&doc.text.source, cmd)?;
    let root = ws.root_document(uri).unwrap_or_else(|_| uri.clone());
    let index = Index::new(ws, &root);
    let mut directories: BTreeSet<_> = index
        .calls
        .get(&(uri.clone(), cmd.span.start))
        .into_iter()
        .flatten()
        .map(|call| call.directory.clone())
        .collect();
    if directories.is_empty() {
        let origin = if cmd.name.starts_with("sub") {
            uri
        } else {
            &root
        };
        directories.insert(origin.to_file_path().ok()?.parent()?.join(dir.value));
    }
    let prefix = &doc.text.source[arg.span.start..offset];
    if prefix.contains(['\\', '{', '}', '%', '\n']) {
        return Some(vec![]);
    }
    let separator = prefix.rfind('/').map_or(0, |at| at + 1);
    let parent = &prefix[..separator];
    let name_prefix = &prefix[separator..];
    let span = arg.span.start + separator..arg.span.end;
    let (range, extra) = syntax::name_completion_ranges(&doc.text, span, offset)?;
    let mut items = BTreeMap::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory.join(parent)) else {
            continue;
        };
        for entry in entries.take(4096).flatten() {
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            if !name.starts_with(name_prefix) || name.contains(['{', '}', '%', '\\', '\n', '\r']) {
                continue;
            }
            let folder = entry.path().is_dir();
            if !folder && !name.ends_with(".tex") {
                continue;
            }
            if items.len() >= ws.completion_budget()
                && items
                    .last_key_value()
                    .is_some_and(|(last, _)| last <= &name)
            {
                continue;
            }
            let new_text = if folder {
                format!("{name}/")
            } else if arg.value.ends_with(".tex") {
                name.clone()
            } else {
                name.strip_suffix(".tex").unwrap().into()
            };
            items.entry(name.clone()).or_insert(CompletionItem {
                label: name,
                kind: Some(if folder {
                    CompletionItemKind::FOLDER
                } else {
                    CompletionItemKind::FILE
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })),
                additional_text_edits: (!extra.is_empty()).then(|| extra.clone()),
                ..Default::default()
            });
            if items.len() > ws.completion_budget() {
                items.pop_last();
            }
        }
    }
    Some(items.into_values().collect())
}

pub(crate) fn is_link(ws: &Workspace, uri: &Url, root: &Url, range: Range) -> bool {
    let Some(doc) = ws.documents.get(uri) else {
        return false;
    };
    Index::new(ws, root)
        .calls
        .into_iter()
        .any(|((source, _), calls)| {
            source == *uri
                && calls.iter().any(|call| {
                    doc.text
                        .range(call.argument.span.start, call.argument.span.end)
                        == range
                })
        })
}
