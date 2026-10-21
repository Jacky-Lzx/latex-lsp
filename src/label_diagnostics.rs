//! Locate compiler-confirmed duplicate labels in the inputs of this build.
use crate::{build_log, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    Position, Range, Url,
};

fn reported_key(diagnostic: &Diagnostic) -> Option<&str> {
    if diagnostic.source.as_deref() != Some("latex-build")
        || diagnostic.severity != Some(DiagnosticSeverity::WARNING)
        || build_log::unresolved_location(diagnostic)
    {
        return None;
    }
    build_log::duplicate_label_key(&diagnostic.message)
}

pub(crate) fn resolve_diagnostics(
    workspace: &Workspace,
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    let names: BTreeSet<_> = diagnostics
        .values()
        .flatten()
        .filter_map(reported_key)
        .map(str::to_owned)
        .collect();
    if names.is_empty() {
        return;
    }
    let mut locations = BTreeMap::<String, BTreeSet<(Url, Position, Position)>>::new();
    for uri in sources {
        let Some(doc) = workspace.documents.get(uri) else {
            continue;
        };
        for entry in &doc.syntax.labels {
            if names.contains(entry.name.as_str()) {
                let range = doc.text.range(entry.span.start, entry.span.end);
                locations.entry(entry.name.clone()).or_default().insert((
                    uri.clone(),
                    range.start,
                    range.end,
                ));
            }
        }
    }
    let mut result = BTreeMap::<Url, Vec<Diagnostic>>::new();
    let mut seen = BTreeSet::new();
    for (original_uri, items) in std::mem::take(diagnostics) {
        for diagnostic in items {
            let mapped = reported_key(&diagnostic)
                .and_then(|key| locations.get_key_value(key))
                // TeX can still report the previous build's duplicate from the old aux
                // on the first pass after a repair. One current declaration
                // alone is insufficient evidence for a precise duplicate.
                .filter(|(_, locations)| locations.len() >= 2);
            if let Some((key, locations)) = mapped {
                for (uri, start, end) in locations {
                    if !seen.insert((uri.clone(), *start, *end, diagnostic.message.clone())) {
                        continue;
                    }
                    let mut precise = diagnostic.clone();
                    precise.range = Range::new(*start, *end);
                    precise.code = Some(NumberOrString::String("duplicate-label".into()));
                    precise.data = Some(serde_json::json!({"label":key}));
                    precise.related_information = Some(
                        locations
                            .iter()
                            .filter(|(other, from, to)| other != uri || from != start || to != end)
                            .take(16)
                            .map(|(other, from, to)| DiagnosticRelatedInformation {
                                location: Location::new(other.clone(), Range::new(*from, *to)),
                                message: format!("Other explicit label `{key}` in this build"),
                            })
                            .collect(),
                    );
                    result.entry(uri.clone()).or_default().push(precise);
                }
            } else {
                result
                    .entry(original_uri.clone())
                    .or_default()
                    .push(diagnostic);
            }
        }
    }
    *diagnostics = result;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Document;
    use serde_json::json;

    fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
        let uri = Url::parse(&format!("file:///label-diagnostics/{name}")).unwrap();
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.into(), Some(3)));
        uri
    }
    fn warning(key: &str) -> Diagnostic {
        Diagnostic {
            source: Some("latex-build".into()),
            severity: Some(DiagnosticSeverity::WARNING),
            message: format!("LaTeX Warning: Label `{key}' multiply defined."),
            ..Default::default()
        }
    }
    fn aux() -> Url {
        Url::parse("file:///label-diagnostics/build/main.aux").unwrap()
    }
    fn range(workspace: &Workspace, uri: &Url, key: &str) -> Range {
        let doc = &workspace.documents[uri];
        let start = doc.text.source.find(key).unwrap();
        doc.text.range(start, start + key.len())
    }
    fn fixture() -> (Workspace, Url, Url) {
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", r"\label{shared}");
        let child = insert(&mut workspace, "child.tex", r"\label{shared}");
        (workspace, main, child)
    }

    #[test]
    fn compiler_label_warnings_map_to_exact_utf16_declarations_and_counterparts() {
        let mut workspace = Workspace::default();
        let key = "章节😀";
        let main = insert(
            &mut workspace,
            "main.tex",
            &format!("😀 \\label{{{key}}}\\ref{{{key}}}\\hypertarget{{{key}}}{{Target}}"),
        );
        let child = insert(
            &mut workspace,
            "child.tex",
            &format!("\\hyperdef[{{{key}}}]{{category}}{{destination}}{{Body}}"),
        );
        let reported = warning(key);
        let ordinary = Diagnostic {
            message: "LaTeX Warning: Keep.".into(),
            ..reported.clone()
        };
        let mut diagnostics: BTreeMap<_, _> =
            [(aux(), vec![reported.clone(), reported, ordinary.clone()])].into();
        let analysis = build_log::compilation_analysis(&diagnostics, false);
        resolve_diagnostics(
            &workspace,
            &[main.clone(), child.clone()].into(),
            &mut diagnostics,
        );
        assert_eq!(diagnostics[&aux()], vec![ordinary]);
        for (uri, other) in [(&main, &child), (&child, &main)] {
            let items = &diagnostics[uri];
            assert_eq!(items.len(), 1);
            let item = &items[0];
            assert_eq!(item.range, range(&workspace, uri, key));
            assert_eq!(
                item.code,
                Some(NumberOrString::String("duplicate-label".into()))
            );
            assert_eq!(item.data, Some(json!({"label":key})));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            let related = item.related_information.as_ref().unwrap();
            assert_eq!(related.len(), 1);
            assert_eq!(
                related[0].location,
                Location::new(other.clone(), range(&workspace, other, key))
            );
        }
        assert_eq!(
            build_log::compilation_analysis(&diagnostics, false),
            analysis
        );
    }

    #[test]
    fn compiler_label_locations_are_limited_to_confirmed_build_inputs() {
        let (mut workspace, main, child) = fixture();
        let excluded = insert(&mut workspace, "excluded.tex", r"\label{shared}");
        let mut diagnostics = [(aux(), vec![warning("shared")])].into();
        resolve_diagnostics(
            &workspace,
            &[main.clone(), child.clone()].into(),
            &mut diagnostics,
        );
        assert!(!diagnostics.contains_key(&excluded));
        assert_eq!(diagnostics.len(), 2);
        for items in diagnostics.values() {
            for item in items {
                assert!(item
                    .related_information
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|other| other.location.uri != excluded));
            }
        }
    }

    #[test]
    fn old_aux_warnings_need_two_distinct_current_declarations_before_retargeting() {
        let (mut workspace, main, _child) = fixture();
        let entry = workspace.documents[&main].syntax.labels[0].clone();
        workspace
            .documents
            .get_mut(&main)
            .unwrap()
            .syntax
            .labels
            .push(entry);
        for sources in [
            BTreeSet::new(),
            [main.clone()].into(),
            [
                main.clone(),
                Url::parse("file:///label-diagnostics/not-loaded.tex").unwrap(),
            ]
            .into(),
        ] {
            let mut diagnostics: BTreeMap<_, _> = [(aux(), vec![warning("shared")])].into();
            let before = diagnostics.clone();
            resolve_diagnostics(&workspace, &sources, &mut diagnostics);
            assert_eq!(diagnostics, before);
        }
        workspace
            .documents
            .get_mut(&main)
            .unwrap()
            .syntax
            .labels
            .clear();
        let mut diagnostics: BTreeMap<_, _> = [(aux(), vec![warning("shared")])].into();
        let before = diagnostics.clone();
        resolve_diagnostics(&workspace, &[main].into(), &mut diagnostics);
        assert_eq!(diagnostics, before);
    }

    #[test]
    fn duplicate_labels_in_one_file_use_distinct_spans_without_borrowing_other_namespaces() {
        let mut workspace = Workspace::default();
        let source =
            r"\hypertarget{shared}{Target}\label{shared}\bibitem{shared}\label{shared}\ref{shared}";
        let main = insert(&mut workspace, "main.tex", source);
        let mut diagnostics = [(aux(), vec![warning("shared")])].into();
        resolve_diagnostics(&workspace, &[main.clone()].into(), &mut diagnostics);
        let expected: Vec<_> = source
            .match_indices(r"\label{shared}")
            .map(|(at, _)| workspace.documents[&main].text.range(at + 7, at + 13))
            .collect();
        let items = &diagnostics[&main];
        assert_eq!(items.len(), 2);
        for (index, item) in items.iter().enumerate() {
            assert_eq!(item.range, expected[index]);
            let related = item.related_information.as_ref().unwrap();
            assert_eq!(related.len(), 1);
            assert_eq!(
                related[0].location,
                Location::new(main.clone(), expected[1 - index])
            );
        }
    }

    #[test]
    fn uncertain_sources_lines_severities_and_noncompiler_messages_keep_original_locations() {
        let (workspace, main, child) = fixture();
        let mut controls = vec![];
        for flag in ["unresolvedSource", "unresolvedLine"] {
            controls.push(Diagnostic {
                data: Some(json!({flag:true})),
                ..warning("shared")
            });
        }
        for severity in [
            None,
            Some(DiagnosticSeverity::ERROR),
            Some(DiagnosticSeverity::INFORMATION),
        ] {
            controls.push(Diagnostic {
                severity,
                ..warning("shared")
            });
        }
        controls.push(Diagnostic {
            source: Some("latex-lsp".into()),
            ..warning("shared")
        });
        controls.push(Diagnostic {
            message: "Package local Warning: Label `shared' multiply defined.".into(),
            ..warning("shared")
        });
        for item in controls {
            let mut diagnostics: BTreeMap<_, _> =
                [(aux(), vec![item.clone(), warning("shared")])].into();
            resolve_diagnostics(
                &workspace,
                &[main.clone(), child.clone()].into(),
                &mut diagnostics,
            );
            assert_eq!(diagnostics[&aux()], vec![item]);
            assert_eq!(diagnostics[&main].len(), 1);
            assert_eq!(diagnostics[&child].len(), 1);
        }
    }

    #[test]
    fn repeated_compiler_rows_are_deduplicated_and_counterparts_have_a_fixed_budget() {
        let mut workspace = Workspace::default();
        let source = r"\label{shared}".repeat(20);
        let main = insert(&mut workspace, "main.tex", &source);
        let mut diagnostics = [(aux(), vec![warning("shared"); 3])].into();
        resolve_diagnostics(&workspace, &[main.clone()].into(), &mut diagnostics);
        assert_eq!(diagnostics.len(), 1);
        let items = &diagnostics[&main];
        assert_eq!(items.len(), 20);
        let sites: BTreeSet<_> = items
            .iter()
            .map(|item| (item.range.start, item.range.end))
            .collect();
        assert_eq!(sites.len(), 20);
        for item in items {
            let related = item.related_information.as_ref().unwrap();
            assert_eq!(related.len(), 16);
            assert!(related
                .iter()
                .all(|other| other.location.uri == main && other.location.range != item.range));
        }
    }

    #[test]
    fn label_mapping_is_idempotent_and_does_not_rewrite_unmatched_warnings() {
        let (workspace, main, child) = fixture();
        let unknown = warning("unrecorded");
        let mut diagnostics = [(aux(), vec![warning("shared"), unknown.clone()])].into();
        let sources = [main, child].into();
        resolve_diagnostics(&workspace, &sources, &mut diagnostics);
        assert_eq!(diagnostics[&aux()], vec![unknown]);
        let once = diagnostics.clone();
        resolve_diagnostics(&workspace, &sources, &mut diagnostics);
        assert_eq!(diagnostics, once);
    }
}
