//! Resolve edits from a current canonical action; never accept edits from data.
use crate::{editing, project::workspace::Workspace};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use tower_lsp::{
    jsonrpc::{Error, ErrorCode, Result},
    lsp_types::*,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    uri: Url,
    range: Range,
    context: CodeActionContext,
    fingerprint: String,
    action: String,
}

fn hash(value: &impl Serialize) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(value).unwrap().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn fingerprint(ws: &Workspace, uri: &Url) -> String {
    let scope = ws.symbol_scope(uri);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    ws.index_truncated.hash(&mut hash);
    ws.versioned_edits.hash(&mut hash);
    serde_json::to_vec(&ws.config).unwrap().hash(&mut hash);
    for uri in scope {
        uri.hash(&mut hash);
        if let Some(doc) = ws.documents.get(&uri) {
            doc.text.source.hash(&mut hash);
            doc.version.hash(&mut hash);
        }
    }
    format!("{:016x}", hash.finish())
}

pub(crate) fn actions(
    ws: &Workspace,
    uri: &Url,
    range: Range,
    context: &CodeActionContext,
) -> CodeActionResponse {
    let mut actions = editing::code_actions(ws, uri, range, context);
    if !ws.code_action_resolve_edits
        || !ws.code_action_literals
        || !actions.iter().any(|action| {
            matches!(action,
            CodeActionOrCommand::CodeAction(action)
                if action.edit.is_some() && action.command.is_none() && action.data.is_none())
        })
    {
        return actions;
    }
    let fingerprint = fingerprint(ws, uri);
    for action in &mut actions {
        let CodeActionOrCommand::CodeAction(action) = action else {
            continue;
        };
        if action.edit.is_none() || action.command.is_some() || action.data.is_some() {
            continue;
        }
        let identity = Identity {
            uri: uri.clone(),
            range,
            context: context.clone(),
            fingerprint: fingerprint.clone(),
            action: hash(action),
        };
        action.edit = None;
        action.data = Some(serde_json::to_value(identity).unwrap());
    }
    actions
}

fn modified() -> Error {
    Error {
        code: ErrorCode::ServerError(-32801),
        message: "Code action is outdated; request code actions again".into(),
        data: None,
    }
}

pub(crate) fn resolve(ws: &Workspace, mut action: CodeAction) -> Result<CodeAction> {
    let Some(data) = &action.data else {
        return Ok(action);
    };
    let identity: Identity = serde_json::from_value(data.clone())
        .map_err(|_| Error::invalid_params("Invalid code action identity"))?;
    if !ws.documents.contains_key(&identity.uri)
        || fingerprint(ws, &identity.uri) != identity.fingerprint
    {
        return Err(modified());
    }
    let canonical = editing::code_actions(ws, &identity.uri, identity.range, &identity.context)
        .into_iter()
        .filter_map(|candidate| match candidate {
            CodeActionOrCommand::CodeAction(action)
                if action.edit.is_some() && action.command.is_none() =>
            {
                Some(action)
            }
            _ => None,
        })
        .find(|candidate| hash(candidate) == identity.action)
        .ok_or_else(modified)?;
    let mut metadata = action.clone();
    metadata.data = None;
    metadata.edit = canonical.edit.clone();
    if metadata != canonical
        || action
            .edit
            .as_ref()
            .is_some_and(|edit| Some(edit) != canonical.edit.as_ref())
    {
        return Err(modified());
    }
    action.edit = canonical.edit;
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{project::workspace::Document, text::Text};

    fn fixture() -> (Workspace, Url, Range) {
        let uri = Url::parse("untitled:actions.tex").unwrap();
        let source = "\\begin{document}\r\n😀 Hello world\r\n\\end{document}";
        let mut ws = Workspace {
            code_action_literals: true,
            code_action_resolve_edits: true,
            versioned_edits: true,
            ..Default::default()
        };
        ws.documents
            .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
        let text = Text::new(source.into());
        let start = source.find("Hello").unwrap();
        let range = text.range(start, start + "Hello world".len());
        (ws, uri, range)
    }

    fn context(kind: CodeActionKind) -> CodeActionContext {
        CodeActionContext {
            diagnostics: vec![],
            only: Some(vec![kind]),
            trigger_kind: None,
        }
    }

    fn first(ws: &Workspace, uri: &Url, range: Range, context: &CodeActionContext) -> CodeAction {
        actions(ws, uri, range, context)
            .into_iter()
            .find_map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => Some(action),
                _ => None,
            })
            .unwrap()
    }

    fn apply(doc: &Document, edit: &WorkspaceEdit) -> String {
        let Some(DocumentChanges::Edits(changes)) = &edit.document_changes else {
            panic!()
        };
        let mut edits = Vec::new();
        for change in changes {
            assert_eq!(change.text_document.version, doc.version);
            for edit in &change.edits {
                let OneOf::Left(edit) = edit else { panic!() };
                edits.push((
                    doc.text.offset(edit.range.start).unwrap(),
                    doc.text.offset(edit.range.end).unwrap(),
                    edit.new_text.clone(),
                ));
            }
        }
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));
        let mut source = doc.text.source.clone();
        for (start, end, replacement) in edits {
            source.replace_range(start..end, &replacement);
        }
        source
    }

    #[test]
    fn refactor_resolution_matches_eager_edits_and_preserves_metadata() {
        let (mut ws, uri, range) = fixture();
        let context = context(CodeActionKind::REFACTOR);
        let deferred = actions(&ws, &uri, range, &context);
        ws.code_action_resolve_edits = false;
        let eager = actions(&ws, &uri, range, &context);
        assert_eq!(deferred.len(), 4);
        for (deferred, eager) in deferred.into_iter().zip(eager) {
            let (CodeActionOrCommand::CodeAction(deferred), CodeActionOrCommand::CodeAction(eager)) =
                (deferred, eager)
            else {
                panic!()
            };
            assert!(deferred.edit.is_none() && deferred.data.is_some());
            let resolved = resolve(&ws, deferred.clone()).unwrap();
            assert_eq!(resolved.edit, eager.edit);
            let mut expected = deferred;
            expected.edit = eager.edit;
            assert_eq!(resolved, expected);
            assert_eq!(resolve(&ws, resolved.clone()).unwrap(), resolved);
            let applied = apply(&ws.documents[&uri], resolved.edit.as_ref().unwrap());
            if resolved.kind == Some(CodeActionKind::REFACTOR_EXTRACT) {
                assert!(applied.contains("\\newcommand{\\extractedcommand}{Hello world}\r\n"));
                assert!(applied.contains("😀 \\extractedcommand{}"));
            } else {
                assert_ne!(applied, ws.documents[&uri].text.source);
                assert!(applied.contains("Hello world"));
            }
        }
    }

    #[test]
    fn quick_fixes_and_fix_all_resolve_to_edits_that_clear_syntax_errors() {
        let (mut ws, uri, _) = fixture();
        let source = "\\begin{itemize}\n\\item Hello\n";
        ws.documents
            .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
        let range = Range::new(Position::new(0, 0), Position::new(3, 0));
        for kind in [CodeActionKind::QUICKFIX, CodeActionKind::SOURCE_FIX_ALL] {
            let item = first(&ws, &uri, range, &context(kind));
            assert!(item.edit.is_none() && item.diagnostics.is_some());
            let resolved = resolve(&ws, item).unwrap();
            let applied = apply(&ws.documents[&uri], resolved.edit.as_ref().unwrap());
            let corrected = Document::new(&uri, applied, Some(2));
            assert!(crate::language::features::diagnostics(&corrected).is_empty());
        }
    }

    #[test]
    fn source_versions_settings_and_changed_metadata_are_rejected() {
        let (mut ws, uri, range) = fixture();
        let context = context(CodeActionKind::REFACTOR);
        let action = first(&ws, &uri, range, &context);
        for field in ["title", "kind", "isPreferred", "data"] {
            let mut forged = serde_json::to_value(&action).unwrap();
            match field {
                "isPreferred" => forged[field] = true.into(),
                "data" => forged[field]["action"] = "forged".into(),
                _ => forged[field] = "forged".into(),
            }
            assert!(resolve(&ws, serde_json::from_value(forged).unwrap()).is_err());
        }
        let original = ws.documents[&uri].text.source.clone();
        ws.documents.get_mut(&uri).unwrap().version = Some(2);
        assert_eq!(
            resolve(&ws, action).unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
        let action = first(&ws, &uri, range, &context);
        ws.documents.insert(
            uri.clone(),
            Document::new(&uri, original.replace("world", "WORLD"), Some(2)),
        );
        assert!(resolve(&ws, action).is_err());
        let action = first(&ws, &uri, range, &context);
        ws.config.diagnostics.math_delimiters = !ws.config.diagnostics.math_delimiters;
        assert!(resolve(&ws, action).is_err());
    }

    #[test]
    fn absent_or_malformed_data_and_legacy_actions_follow_protocol() {
        let (mut ws, uri, range) = fixture();
        let context = context(CodeActionKind::REFACTOR);
        ws.code_action_resolve_edits = false;
        let inline = first(&ws, &uri, range, &context);
        assert!(inline.edit.is_some() && inline.data.is_none());
        assert_eq!(resolve(&ws, inline.clone()).unwrap(), inline);
        let mut invalid = inline;
        invalid.data = Some(serde_json::json!({"uri":uri}));
        assert_eq!(
            resolve(&ws, invalid).unwrap_err().code,
            ErrorCode::InvalidParams
        );
        ws.code_action_literals = false;
        ws.apply_edits = true;
        ws.code_action_resolve_edits = true;
        assert!(actions(&ws, &uri, range, &context)
            .iter()
            .all(|action| matches!(action, CodeActionOrCommand::Command(_))));
    }

    #[test]
    fn related_project_changes_invalidate_actions_but_independent_projects_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
        let child = Url::from_file_path(dir.path().join("macros.tex")).unwrap();
        let other = Url::from_file_path(dir.path().join("other.tex")).unwrap();
        let source =
            "\\documentclass{article}\n\\input{macros}\n\\begin{document}\nHello\n\\end{document}";
        let declarations = "\\newcommand{\\unrelated}{}";
        for (uri, source) in [
            (&uri, source),
            (&child, declarations),
            (&other, "\\documentclass{article}\nIndependent"),
        ] {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let mut ws = Workspace {
            code_action_literals: true,
            code_action_resolve_edits: true,
            ..Default::default()
        };
        ws.config.distribution.enabled = false;
        ws.open(uri.clone(), source.into(), 1);
        ws.open(child.clone(), declarations.into(), 1);
        ws.open(
            other.clone(),
            "\\documentclass{article}\nIndependent".into(),
            1,
        );
        let range = ws.documents[&uri].text.range(
            source.find("Hello").unwrap(),
            source.find("Hello").unwrap() + 5,
        );
        let context = context(CodeActionKind::REFACTOR_EXTRACT);
        let action = first(&ws, &uri, range, &context);
        ws.open(
            other,
            "\\documentclass{article}\nChanged independently".into(),
            2,
        );
        assert!(resolve(&ws, action.clone()).is_ok());
        ws.open(
            child.clone(),
            "\\newcommand{\\extractedcommand}{}".into(),
            2,
        );
        assert_eq!(
            resolve(&ws, action).unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
        let updated = resolve(&ws, first(&ws, &uri, range, &context)).unwrap();
        let edits = updated.edit.unwrap().changes.unwrap();
        assert!(edits[&uri][0].new_text.contains("extractedcommandb"));
    }

    #[test]
    fn build_commands_keep_their_guards_and_disappeared_build_fixes_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.tex");
        let uri = Url::from_file_path(&path).unwrap();
        let source = "\\documentclass{article}\n\\begin{document}\nA & B\n\\end{document}";
        std::fs::write(&path, source).unwrap();
        let mut ws = Workspace {
            code_action_literals: true,
            code_action_resolve_edits: true,
            ..Default::default()
        };
        ws.config.distribution.enabled = false;
        ws.add_root(dir.path().into());
        ws.open(uri.clone(), source.into(), 1);
        let diagnostics = crate::compilation::build_log::parse(&path, "(./main.tex\n! Misplaced alignment tab character &.\nl.3 A &\n      B\n\nLaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.\n)\n");
        ws.build_diagnostics.insert(uri.clone(), diagnostics);
        let range = Range::new(Position::new(0, 0), Position::new(4, 0));
        let context = context(CodeActionKind::QUICKFIX);
        let deferred = actions(&ws, &uri, range, &context);
        let mut fix = None;
        let mut rebuild = false;
        for item in deferred {
            let CodeActionOrCommand::CodeAction(action) = item else {
                panic!()
            };
            if let Some(command) = &action.command {
                assert!(action.data.is_none() && action.edit.is_none());
                assert_eq!(command.command, "latex.rebuild");
                assert!(command.arguments.as_ref().unwrap()[0]
                    .get("rebuildGuard")
                    .is_some());
                assert_eq!(resolve(&ws, action.clone()).unwrap(), action);
                rebuild = true;
            } else {
                let resolved = resolve(&ws, action.clone()).unwrap();
                assert!(resolved.edit.is_some());
                fix = Some(action);
            }
        }
        assert!(rebuild);
        let fix = fix.expect("compiled alignment error must have a deferred fix");
        ws.build_diagnostics.clear();
        assert_eq!(
            resolve(&ws, fix).unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
    }
}
