//! Resolve only documentation, retaining the completion's original edits.
use crate::{command_docs, syntax, workspace::Workspace};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    hash::{Hash, Hasher},
};
use tower_lsp::{
    jsonrpc::{Error, ErrorCode, Result},
    lsp_types::*,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    origin: Url,
    definition: Url,
    name: String,
    fingerprint: String,
}

fn fingerprint(ws: &Workspace, scope: &BTreeSet<Url>) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    ws.index_truncated.hash(&mut hash);
    for uri in scope {
        uri.hash(&mut hash);
        if let Some(doc) = ws.documents.get(uri) {
            doc.text.source.hash(&mut hash);
            doc.version.hash(&mut hash);
        }
    }
    format!("{:016x}", hash.finish())
}

pub(crate) struct Commands<'a> {
    origin: &'a Url,
    index: command_docs::Index<'a>,
    fingerprint: Option<String>,
}

impl<'a> Commands<'a> {
    pub(crate) fn new(ws: &'a Workspace, origin: &'a Url, scope: &BTreeSet<Url>) -> Self {
        Self {
            origin,
            index: command_docs::Index::new(ws, scope),
            fingerprint: ws
                .completion_resolve_documentation
                .then(|| fingerprint(ws, scope)),
        }
    }

    pub(crate) fn apply(&self, item: &mut CompletionItem, entry: &syntax::Entry) {
        let Some(declaration) = self.index.get(&entry.name) else {
            return;
        };
        if let Some(fingerprint) = &self.fingerprint {
            item.documentation = None;
            item.data = Some(
                serde_json::to_value(Identity {
                    origin: self.origin.clone(),
                    definition: declaration.uri.clone(),
                    name: entry.name.clone(),
                    fingerprint: fingerprint.clone(),
                })
                .unwrap(),
            );
        } else {
            item.documentation = self
                .index
                .documentation(&entry.name)
                .map(Documentation::String);
        }
    }
}

fn modified() -> Error {
    Error {
        code: ErrorCode::ServerError(-32801),
        message: "Completion item is outdated; request completions again".into(),
        data: None,
    }
}

pub(crate) fn resolve(ws: &Workspace, mut item: CompletionItem) -> Result<CompletionItem> {
    let Some(data) = &item.data else {
        return Ok(item);
    };
    let identity: Identity = serde_json::from_value(data.clone())
        .map_err(|_| Error::invalid_params("Invalid completion item identity"))?;
    if item.label != identity.name || !ws.documents.contains_key(&identity.origin) {
        return Err(modified());
    }
    let scope = ws.symbol_scope(&identity.origin);
    if fingerprint(ws, &scope) != identity.fingerprint {
        return Err(modified());
    }
    let index = command_docs::Index::new(ws, &scope);
    if index
        .get(&identity.name)
        .is_none_or(|declaration| *declaration.uri != identity.definition)
    {
        return Err(modified());
    }
    item.documentation = index
        .documentation(&identity.name)
        .map(Documentation::String);
    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(deferred: bool) -> (Workspace, Url) {
        let uri = Url::parse("untitled:completion.tex").unwrap();
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.snippets = true;
        ws.completion_resolve_documentation = deferred;
        ws.open(
            uri.clone(),
            "% 中文😀 **plain** <tag>\r\n\\newcommand{\\example}[1]{#1}\r\n😀 \\exaBAD".into(),
            1,
        );
        (ws, uri)
    }

    fn candidate(ws: &Workspace, uri: &Url) -> CompletionItem {
        let doc = &ws.documents[uri];
        let offset = doc.text.source.rfind("\\exaBAD").unwrap() + 4;
        crate::features::completion(ws, uri, doc.text.position(offset))
            .into_iter()
            .find(|item| item.label == "example")
            .unwrap()
    }

    #[test]
    fn inline_and_deferred_documentation_agree_and_resolution_preserves_all_other_fields() {
        let (mut ws, uri) = fixture(false);
        let inline = candidate(&ws, &uri);
        assert!(inline.data.is_none());
        let Documentation::String(text) = inline.documentation.as_ref().unwrap() else {
            panic!()
        };
        assert!(text.ends_with("中文😀 **plain** <tag>"));
        ws.completion_resolve_documentation = true;
        let deferred = candidate(&ws, &uri);
        assert!(deferred.documentation.is_none());
        let resolved = resolve(&ws, deferred.clone()).unwrap();
        assert_eq!(resolved.documentation, inline.documentation);
        let mut expected = deferred;
        expected.documentation = inline.documentation;
        assert_eq!(resolved, expected);
        assert_eq!(resolve(&ws, resolved.clone()).unwrap(), resolved);
        assert_eq!(resolved.text_edit, inline.text_edit);
        assert_eq!(resolved.insert_text_format, Some(InsertTextFormat::SNIPPET));
    }

    #[test]
    fn changed_source_versions_ambiguities_and_forged_identity_are_rejected() {
        let (mut ws, uri) = fixture(true);
        let item = candidate(&ws, &uri);
        for field in ["label", "fingerprint", "definition", "origin"] {
            let mut forged = serde_json::to_value(&item).unwrap();
            if field == "label" {
                forged["label"] = serde_json::json!("other");
            } else {
                forged["data"][field] = serde_json::json!(if field == "fingerprint" {
                    "invalid"
                } else {
                    "untitled:elsewhere.tex"
                });
            }
            assert_eq!(
                resolve(&ws, serde_json::from_value(forged).unwrap())
                    .unwrap_err()
                    .code,
                ErrorCode::ServerError(-32801)
            );
        }
        let source = ws.documents[&uri].text.source.clone();
        ws.open(uri.clone(), source.clone(), 2);
        assert_eq!(
            resolve(&ws, item).unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
        let item = candidate(&ws, &uri);
        ws.open(
            uri.clone(),
            format!("\\def\\example{{duplicate}}\n{source}"),
            3,
        );
        assert!(resolve(&ws, item).is_err());
        let ambiguous = candidate(&ws, &uri);
        assert!(ambiguous.data.is_none());
        let Documentation::String(text) = ambiguous.documentation.unwrap() else {
            panic!()
        };
        assert!(!text.contains("**plain**"));
    }

    #[test]
    fn unknown_item_without_identity_is_unchanged_and_malformed_identity_is_invalid() {
        let (ws, _) = fixture(true);
        let mut item = CompletionItem {
            label: "section".into(),
            ..Default::default()
        };
        assert_eq!(resolve(&ws, item.clone()).unwrap(), item);
        item.data = Some(serde_json::json!({"origin":"untitled:completion.tex"}));
        assert_eq!(
            resolve(&ws, item).unwrap_err().code,
            ErrorCode::InvalidParams
        );
    }

    #[test]
    fn project_scope_changes_invalidate_items_and_unrelated_projects_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = Url::from_file_path(dir.path().join("main.tex")).unwrap();
        let child = Url::from_file_path(dir.path().join("macros.tex")).unwrap();
        let other = Url::from_file_path(dir.path().join("other.tex")).unwrap();
        let source = "\\documentclass{article}\n\\input{macros}\n\\exaBAD";
        let definitions = "% A project macro\n\\newcommand{\\example}{x}";
        for (uri, source) in [
            (&root, source),
            (&child, definitions),
            (
                &other,
                "\\documentclass{article}\n% Other\n\\def\\example{other}",
            ),
        ] {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.completion_resolve_documentation = true;
        ws.open(root.clone(), source.into(), 1);
        ws.open(child.clone(), definitions.into(), 1);
        ws.open(
            other.clone(),
            std::fs::read_to_string(other.to_file_path().unwrap()).unwrap(),
            1,
        );
        let item = candidate(&ws, &root);
        ws.open(
            other.clone(),
            "\\documentclass{article}\n\\def\\example{changed}".into(),
            2,
        );
        let resolved = resolve(&ws, item.clone()).unwrap();
        assert!(
            matches!(resolved.documentation, Some(Documentation::String(ref text)) if text.contains("A project macro"))
        );
        ws.open(
            child,
            "% Updated project macro\n\\newcommand{\\example}{x}".into(),
            2,
        );
        assert!(resolve(&ws, item).is_err());
    }
}
