//! Find conflicting definitions only where they coexist in a saved project.
use crate::{syntax, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::*;

type Definition = (Url, Position, Position);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Label,
    Citation,
    Anchor,
}

impl Kind {
    fn description(self) -> &'static str {
        match self {
            Self::Label => "label",
            Self::Citation => "bibliography key",
            Self::Anchor => "PDF destination",
        }
    }
}

pub(crate) fn duplicates(
    workspace: &Workspace,
    requested: &BTreeSet<Url>,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut projects: Vec<_> = workspace.saved_projects().into_values().collect();
    if projects.is_empty() {
        projects.push(workspace.documents.keys().cloned().collect());
    }
    // A shared declaration can conflict in several consumers. Merge locations
    // before emitting so one occurrence receives one diagnostic.
    let mut conflicts = BTreeMap::<(Kind, String, Definition), BTreeSet<Definition>>::new();
    for project in projects {
        if project.is_disjoint(requested) {
            continue;
        }
        let mut index = BTreeMap::<(Kind, &str), Vec<Definition>>::new();
        for uri in &project {
            let Some(doc) = workspace.documents.get(uri) else {
                continue;
            };
            for (kind, entries) in [
                (Kind::Label, &doc.syntax.labels),
                (Kind::Citation, &doc.syntax.citations),
                (Kind::Anchor, &doc.syntax.anchors),
            ] {
                for entry in entries {
                    if !syntax::is_static_key(&entry.name) {
                        continue;
                    }
                    let range = doc.text.range(entry.span.start, entry.span.end);
                    index.entry((kind, &entry.name)).or_default().push((
                        uri.clone(),
                        range.start,
                        range.end,
                    ));
                }
            }
        }
        for ((kind, name), definitions) in index {
            if !definitions
                .iter()
                .skip(1)
                .any(|definition| definition.0 != definitions[0].0)
            {
                continue;
            }
            for definition in &definitions {
                if !requested.contains(&definition.0) {
                    continue;
                }
                let related = conflicts
                    .entry((kind, name.into(), definition.clone()))
                    .or_default();
                related.extend(
                    definitions
                        .iter()
                        .filter(|other| *other != definition)
                        .take(16)
                        .cloned(),
                );
            }
        }
    }
    let mut result = BTreeMap::<Url, Vec<Diagnostic>>::new();
    for ((kind, name, (uri, start, end)), related) in conflicts {
        let kind = kind.description();
        result.entry(uri).or_default().push(Diagnostic {
            range: Range::new(start, end),
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String("duplicate-definition".into())),
            source: Some("latex-lsp".into()),
            message: format!("Duplicate {kind} `{name}` across project files"),
            related_information: Some(
                related
                    .into_iter()
                    .take(16)
                    .map(|(uri, start, end)| DiagnosticRelatedInformation {
                        location: Location::new(uri, Range::new(start, end)),
                        message: format!("Other definition of {kind} `{name}`"),
                    })
                    .collect(),
            ),
            ..Default::default()
        });
    }
    result
}
