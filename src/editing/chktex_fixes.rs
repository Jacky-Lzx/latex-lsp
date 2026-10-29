//! Repairs for current ChkTeX warnings, verified against project math syntax.
use crate::{
    diagnostics::math_diagnostics, parser::syntax::QuickFix, project::workspace::Workspace,
};
use lsp_types::{TextEdit, Url};

pub(super) fn fixes(workspace: &Workspace, uri: &Url) -> Vec<QuickFix> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let warnings: Vec<_> = workspace
        .project
        .lint_diagnostics
        .get(uri)
        .into_iter()
        .flatten()
        .filter(|diagnostic| math_diagnostics::inline_dollar_warning(diagnostic))
        .collect();
    if warnings.is_empty() {
        return vec![];
    }
    let pairs = math_diagnostics::inline_dollar_pairs(workspace, uri);
    warnings
        .into_iter()
        .filter_map(|diagnostic| {
            let start = doc.text.offset(diagnostic.range.start)?;
            let pair = pairs.iter().find(|pair| pair.start == start)?;
            Some(QuickFix {
                title: "Use \\( ... \\) instead of $ ... $".into(),
                diagnostic: diagnostic.clone(),
                edit: TextEdit {
                    range: doc.text.range(pair.start, pair.start + 1),
                    new_text: "\\(".into(),
                },
                additional_edits: vec![TextEdit {
                    range: doc.text.range(pair.end - 1, pair.end),
                    new_text: "\\)".into(),
                }],
            })
        })
        .collect()
}
