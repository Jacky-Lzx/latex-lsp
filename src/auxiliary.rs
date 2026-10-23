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
    citations: BTreeMap<String, Option<String>>,
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
    while at < source.len() {
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
                    if matches!(name, r"\newlabel" | r"\@input" | r"\bibcite") {
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
                        if name == r"\bibcite" {
                            if syntax::is_static_citation_key(&key.value)
                                && (result.citations.len() < 4096
                                    || result.citations.contains_key(&key.value))
                            {
                                let value = plain_citation_label(&body.value);
                                match result.citations.entry(key.value) {
                                    std::collections::btree_map::Entry::Vacant(entry) => {
                                        entry.insert(value);
                                    }
                                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                                        if entry.get() != &value {
                                            entry.insert(None);
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        if !syntax::is_static_reference_key(&key.value)
                            || key.value.ends_with("@cref")
                            || result.labels.len() >= 4096
                                && !result.labels.contains_key(&key.value)
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
// A flat bibcite label is usable without executing TeX. Structured natbib
// records are deliberately excluded: identical records serve both numeric and
// author-year citation modes, so the first field is not a display label.
fn plain_citation_label(value: &str) -> Option<String> {
    if value.contains(['\\', '{', '}', '#', '$', '%', '&', '~', '^', '_'])
        || value
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
        || value.chars().count() > 256
    {
        return None;
    }
    let label = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!label.is_empty()).then_some(label)
}
fn citation_labels(ws: &Workspace, uri: &Url) -> BTreeMap<String, Option<String>> {
    let root = ws.root_document(uri).unwrap_or_else(|_| uri.clone());
    let Ok(path) = root.to_file_path() else {
        return BTreeMap::new();
    };
    let path = ws
        .build_config(&root)
        .artifact_path(&path, "aux")
        .filter(|path| path.is_file())
        .unwrap_or_else(|| path.with_extension("aux"));
    let mut result = BTreeMap::new();
    let mut pending = vec![path];
    let mut seen = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if seen.len() >= 16 || !seen.insert(path.clone()) {
            continue;
        }
        let Some(data) = read(&path) else {
            continue;
        };
        for (key, value) in &data.citations {
            if result.len() >= 8192 && !result.contains_key(key) {
                continue;
            }
            match result.entry(key.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(value.clone());
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if entry.get() != value {
                        entry.insert(None);
                    }
                }
            }
        }
        if let Some(parent) = path.parent() {
            pending.extend(data.inputs.iter().map(|input| parent.join(input)));
        }
    }
    result
}
fn hint(position: Position, value: String, tooltip: String) -> InlayHint {
    InlayHint {
        position,
        label: InlayHintLabel::String(format!("→ {value}")),
        kind: None,
        text_edits: None,
        tooltip: Some(InlayHintTooltip::String(tooltip)),
        padding_left: Some(true),
        padding_right: Some(false),
        data: None,
    }
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
    if !config.enabled || !config.numbers && !config.pages && !config.citations {
        return vec![];
    }
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let labels = if config.numbers || config.pages {
        labels(ws, uri)
    } else {
        BTreeMap::new()
    };
    let citations = if config.citations {
        citation_labels(ws, uri)
    } else {
        BTreeMap::new()
    };
    crate::reference_diagnostics::hint_occurrences(ws, uri)
        .into_iter()
        .filter_map(|(citation, span)| {
            let position = doc.text.position(span.end);
            if position < range.start || position > range.end {
                return None;
            }
            let name = &doc.text.source[span];
            if citation {
                let label = citations.get(name)?.as_ref()?;
                return Some(hint(position, label.clone(), format!("Bibliography label for `{name}`: {label}\nValues come from an auxiliary file and may be stale.")));
            }
            let info = labels.get(name)?;
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
            Some(hint(position, parts.join(", "), format!(
                "{}\nValues come from an auxiliary file and may be stale.", describe(info)
            )))
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
#[cfg(test)]
mod citation_tests {
    use super::*;
    fn workspace(dir: &Path, source: &str, auxiliary: &str) -> (Workspace, Url) {
        let uri = Url::from_file_path(dir.join("main.tex")).unwrap();
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        std::fs::write(dir.join("main.aux"), auxiliary).unwrap();
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.open(uri.clone(), source.into(), 5);
        (ws, uri)
    }
    fn values(ws: &Workspace, uri: &Url) -> Vec<String> {
        hints(
            ws,
            uri,
            Range::new(Position::new(0, 0), Position::new(u32::MAX, 0)),
        )
        .into_iter()
        .map(|hint| match hint.label {
            InlayHintLabel::String(label) => label,
            _ => panic!(),
        })
        .collect()
    }
    #[test]
    fn flat_citation_records_reject_expansion_structures_and_conflicting_labels() {
        let parsed = parse(concat!(
            "\\bibcite{paper}{Doe26}\n\\bibcite{quoted\"key}{7}\n",
            "\\bibcite{conflict}{1}\\bibcite{conflict}{2}\n",
            "\\bibcite{expanded}{\\number\\count0}\n",
            "\\bibcite{natbib}{{1}{2026}{{Doe}}{{Doe}}}\n",
            "\\@writefile{toc}{\\bibcite{hidden}{9}}\n",
            "% \\bibcite{comment}{9}\n"
        ));
        assert_eq!(parsed.citations["paper"].as_deref(), Some("Doe26"));
        assert_eq!(parsed.citations["quoted\"key"].as_deref(), Some("7"));
        for key in ["conflict", "expanded", "natbib"] {
            assert_eq!(parsed.citations[key], None);
        }
        for key in ["hidden", "comment"] {
            assert!(!parsed.citations.contains_key(key));
        }
    }
    #[test]
    fn citation_records_do_not_exhaust_label_budget_or_hide_retained_key_conflicts() {
        let mut source: String = (0..4097)
            .map(|key| format!(r"\bibcite{{paper{key}}}{{7}}"))
            .collect();
        source.push_str(r"\bibcite{paper0}{8}\newlabel{section}{{2}{5}}");
        let parsed = parse(&source);
        assert_eq!(parsed.citations.len(), 4096);
        assert_eq!(parsed.citations["paper0"], None);
        assert_eq!(parsed.labels["section"].number, "2");
        let mut source: String = (0..4097)
            .map(|key| format!(r"\newlabel{{section{key}}}{{{{2}}{{5}}}}"))
            .collect();
        source.push_str(r"\bibcite{paper}{Doe26}");
        let parsed = parse(&source);
        assert_eq!(parsed.labels.len(), 4096);
        assert_eq!(parsed.citations["paper"].as_deref(), Some("Doe26"));
    }
    #[test]
    fn citation_hints_follow_output_job_names_recursive_auxiliaries_and_disk_updates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("build output")).unwrap();
        let source = r"\documentclass{article}\bibitem{paper}\cite{paper}";
        let (mut ws, uri) = workspace(dir.path(), source, r"\bibcite{paper}{old}");
        ws.config.build.output_directory = Some("build output".into());
        ws.config.build.job_name = Some("custom".into());
        std::fs::write(
            dir.path().join("build output/custom.aux"),
            "\\@input{child.aux}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("build output/child.aux"),
            "\\bibcite{paper}{7}\\@input{custom.aux}\n",
        )
        .unwrap();
        assert_eq!(values(&ws, &uri), ["→ 7"]);
        std::fs::write(
            dir.path().join("build output/child.aux"),
            "\\bibcite{paper}{Doe26}\n",
        )
        .unwrap();
        assert_eq!(values(&ws, &uri), ["→ Doe26"]);
        std::fs::write(
            dir.path().join("build output/custom.aux"),
            "\\bibcite{paper}{8}\\@input{child.aux}\n",
        )
        .unwrap();
        assert!(values(&ws, &uri).is_empty());
    }
    #[test]
    fn citation_hints_only_describe_executable_printing_citations() {
        let dir = tempfile::tempdir().unwrap();
        for call in [
            r"\nocite{paper}",
            r"\def\hide{\nocite}\hide{paper}",
            r"\csuse{nocite}{paper}",
            r"\def\cite#1{}\cite{paper}",
            r"\def\take#1{}\take{\cite{paper}}",
            r"\newcommand\later{\cite{paper}}",
            r"\noexpand\cite{paper}",
            r"\detokenize{\cite{paper}}",
            r"\addtocategory{selected}{paper}",
            r"\ifentrycategory{paper}{selected}{}{}",
            r"\entrydata{paper}{}",
            r"\ifentryexists{paper}{}{}",
        ] {
            let source = format!(r"\documentclass{{article}}\bibitem{{paper}}{call}");
            let (ws, uri) = workspace(dir.path(), &source, r"\bibcite{paper}{7}");
            assert!(values(&ws, &uri).is_empty(), "{source}");
        }
        let source = r"\documentclass{article}\bibitem{paper}\def\link{\cite}\emph{\cite{paper}}\csuse{cite}{paper}\link{paper}\let\strong\textbf\strong{Text}\cite{paper}";
        let (ws, uri) = workspace(dir.path(), source, r"\bibcite{paper}{7}");
        assert_eq!(values(&ws, &uri), ["→ 7", "→ 7", "→ 7", "→ 7"]);
    }
    #[test]
    fn citation_hint_ranges_utf16_settings_and_response_budget_are_respected() {
        let dir = tempfile::tempdir().unwrap();
        let source = "\\documentclass{article}\r\n😀 \\bibitem{paper}\\cite{paper}\\label{target}\\ref{target}";
        let (mut ws, uri) = workspace(
            dir.path(),
            source,
            "\\bibcite{paper}{7}\n\\newlabel{target}{{2}{5}}\n",
        );
        let position = ws.documents[&uri]
            .text
            .position(source.find(r"\cite{paper}").unwrap() + 11);
        let all = hints(
            &ws,
            &uri,
            Range::new(Position::new(0, 0), Position::new(2, 0)),
        );
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].position, position);
        assert!(hints(
            &ws,
            &uri,
            Range::new(Position::new(0, 0), Position::new(1, 2))
        )
        .is_empty());
        ws.config.inlay_hints.citations = false;
        assert_eq!(values(&ws, &uri), ["→ 2, p. 5"]);
        ws.config.inlay_hints.citations = true;
        ws.config.inlay_hints.numbers = false;
        ws.config.inlay_hints.pages = false;
        assert_eq!(values(&ws, &uri), ["→ 7"]);
        ws.config.inlay_hints.enabled = false;
        assert!(values(&ws, &uri).is_empty());
        ws.config.inlay_hints.enabled = true;
        ws.open(
            uri.clone(),
            format!(
                "\\documentclass{{article}}\\bibitem{{paper}}{}",
                r"\cite{paper}".repeat(1100)
            ),
            6,
        );
        assert_eq!(values(&ws, &uri).len(), 1024);
    }
    #[test]
    fn citation_hints_do_not_import_external_document_bibliographies() {
        let dir = tempfile::tempdir().unwrap();
        let source = r"\documentclass{article}\externaldocument{other}\bibitem{paper}\cite{paper}";
        let (ws, uri) = workspace(dir.path(), source, "");
        std::fs::write(dir.path().join("other.aux"), r"\bibcite{paper}{9}").unwrap();
        assert!(values(&ws, &uri).is_empty());
    }
    #[test]
    fn citation_hints_choose_the_consuming_root_without_importing_other_projects() {
        let dir = tempfile::tempdir().unwrap();
        for (name, source) in [
            ("main.tex", r"\documentclass{article}\input{chapter}"),
            ("other.tex", r"\documentclass{article}\cite{paper}"),
            ("chapter.tex", r"\cite{paper}"),
            ("main.aux", r"\bibcite{paper}{7}"),
            ("other.aux", r"\bibcite{paper}{99}"),
        ] {
            std::fs::write(dir.path().join(name), source).unwrap();
        }
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.add_root(dir.path().into());
        let chapter = Url::from_file_path(dir.path().join("chapter.tex")).unwrap();
        let other = Url::from_file_path(dir.path().join("other.tex")).unwrap();
        assert_eq!(values(&ws, &chapter), ["→ 7"]);
        assert_eq!(values(&ws, &other), ["→ 99"]);
        ws.open(
            other.clone(),
            r"\documentclass{article}\input{chapter}".into(),
            7,
        );
        assert!(values(&ws, &chapter).is_empty());
    }
    #[test]
    fn citation_hint_labels_do_not_display_tex_syntax_or_control_characters() {
        for label in [
            r"A~B",
            "A_B",
            "A^B",
            "A&B",
            "A\u{0000}B",
            "A\u{001b}B",
            "",
            "   ",
        ] {
            assert_eq!(plain_citation_label(label), None, "{label:?}");
        }
        assert_eq!(
            plain_citation_label(" Doe  2026\r\n 中文 "),
            Some("Doe 2026 中文".into())
        );
    }
    #[test]
    #[ignore = "requires pdflatex"]
    fn real_bibitem_auxiliary_labels_appear_on_citations() {
        for (item, expected) in [
            (r"\bibitem{paper}", "→ 1"),
            (r"\bibitem[Doe26]{paper}", "→ Doe26"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let source = format!(
                r"\documentclass{{article}}\begin{{document}}\cite{{paper}}\begin{{thebibliography}}{{1}}{item}A book.\end{{thebibliography}}\end{{document}}"
            );
            let (ws, uri) = workspace(dir.path(), &source, "");
            for _ in 0..2 {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(dir.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            assert_eq!(values(&ws, &uri), [expected]);
        }
    }
}
