//! Environment pairing interpreted separately in every consuming project.
use crate::{
    environment_calls::project_contexts,
    signature::{definition_body_spans, CommandSignatures},
    syntax::{self, QuickFix},
    workspace::{Document, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::{Diagnostic, Url};

type Interpretations = Vec<(bool, Vec<QuickFix>)>;

#[derive(Default)]
pub(crate) struct Index {
    replacements: BTreeMap<Url, Vec<QuickFix>>,
}

impl Index {
    pub(crate) fn new(workspace: &Workspace, requested: &BTreeSet<Url>) -> Self {
        let mut interpretations: BTreeMap<Url, Interpretations> = BTreeMap::new();
        for (scope, consumers) in project_contexts(workspace) {
            if consumers.is_disjoint(requested) {
                continue;
            }
            let signatures = CommandSignatures::new(
                workspace
                    .documents
                    .iter()
                    .filter(|(uri, doc)| scope.contains(*uri) && doc.kind == DocumentKind::Latex)
                    .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
            );
            for uri in consumers.intersection(requested) {
                let Some(doc) = workspace
                    .documents
                    .get(uri)
                    .filter(|doc| doc.kind == DocumentKind::Latex)
                else {
                    continue;
                };
                let calls = signatures.environment_invocations(&doc.text.source, &doc.syntax);
                let bodies = definition_body_spans(&doc.text.source, &doc.syntax);
                let needs_project_matching = calls.iter().any(|call| !call.direct)
                    || doc.syntax.environment_fixes.iter().any(|fix| {
                        doc.text
                            .offset(fix.diagnostic.range.start)
                            .is_some_and(|offset| bodies.iter().any(|span| span.contains(&offset)))
                    });
                let uncertain: BTreeSet<_> = calls
                    .iter()
                    .filter(|call| call.operand || call.deferred)
                    .map(|call| call.argument.value.clone())
                    .collect();
                let fixes = if signatures.environment_primitives_unchanged() {
                    syntax::environment_stack_fixes(
                        &doc.text,
                        calls
                            .into_iter()
                            .filter(|call| {
                                !uncertain.contains(&call.argument.value)
                                    && !syntax::is_verbatim_environment(&call.argument.value)
                            })
                            .map(|call| (call.begin, call.argument, call.span)),
                    )
                } else {
                    vec![]
                };
                interpretations
                    .entry(uri.clone())
                    .or_default()
                    .push((needs_project_matching, fixes));
            }
        }
        let mut index = Self::default();
        for (uri, mut contexts) in interpretations {
            if !contexts
                .iter()
                .any(|(needs_project_matching, _)| *needs_project_matching)
            {
                continue;
            }
            let (_, mut agreed) = contexts.pop().unwrap();
            agreed.retain(|fix| {
                contexts
                    .iter()
                    .all(|(_, fixes)| fixes.iter().any(|other| same_fix(fix, other)))
            });
            index.replacements.insert(uri, agreed);
        }
        index
    }

    pub(crate) fn replace_diagnostics(
        &self,
        uri: &Url,
        doc: &Document,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if let Some(fixes) = self.replacements.get(uri) {
            diagnostics.retain(|diagnostic| {
                !doc.syntax
                    .environment_fixes
                    .iter()
                    .any(|fix| fix.diagnostic == *diagnostic)
            });
            diagnostics.extend(fixes.iter().map(|fix| fix.diagnostic.clone()));
        }
    }

    pub(crate) fn fixes(&self, uri: &Url, doc: &Document) -> Vec<QuickFix> {
        let Some(replacements) = self.replacements.get(uri) else {
            return doc.syntax.fixes.clone();
        };
        let mut fixes: Vec<_> = doc
            .syntax
            .fixes
            .iter()
            .filter(|fix| {
                !doc.syntax
                    .environment_fixes
                    .iter()
                    .any(|old| same_fix(fix, old))
            })
            .cloned()
            .collect();
        fixes.extend(replacements.iter().cloned());
        fixes
    }
}

pub(crate) fn fixes(workspace: &Workspace, uri: &Url, doc: &Document) -> Vec<QuickFix> {
    Index::new(workspace, &BTreeSet::from([uri.clone()])).fixes(uri, doc)
}

fn same_fix(a: &QuickFix, b: &QuickFix) -> bool {
    a.diagnostic == b.diagnostic && a.edit == b.edit && a.additional_edits == b.additional_edits
}
