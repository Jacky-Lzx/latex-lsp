use latex_lsp::{
    features, file_operations,
    text::Text,
    workspace::{DocumentKind, Workspace},
};
use tower_lsp::lsp_types::*;

#[test]
fn supporting_sources_are_discovered_and_explicit_inputs_keep_project_scope_and_versioned_moves() {
    for extension in ["bbx", "cbx", "lbx", "dbx", "fd", "ldf"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let uri = |name: &str| Url::from_file_path(root.join(name)).unwrap();
        let local_name = format!("local.{extension}");
        let other_name = format!("other.{extension}");
        let unused_name = format!("unused.{extension}");
        let source = format!("\\documentclass{{article}}\n\\input{{{local_name}}}\n\\shared");
        for (name, text) in [
            ("a.tex", source.as_str()),
            (
                "b.tex",
                &format!("\\documentclass{{article}}\\input{{{other_name}}}\\shared"),
            ),
            (local_name.as_str(), "\\newcommand\\shared[1]{#1}"),
            (other_name.as_str(), "\\newcommand\\shared[2]{#1#2}"),
            (unused_name.as_str(), "\\newcommand\\unusedstyle{}"),
        ] {
            std::fs::write(root.join(name), text).unwrap();
        }
        let mut workspace = Workspace {
            versioned_edits: true,
            snippets: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        assert_eq!(
            workspace.documents[&uri(&unused_name)].kind,
            DocumentKind::Latex
        );
        let symbols = latex_lsp::navigation::workspace_symbols(&workspace, "unusedstyle");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].location.uri, uri(&unused_name));
        let main = uri("a.tex");
        workspace.open(main.clone(), source.clone(), 8);
        let text = Text::new(source.clone());
        let position = text.position(source.len() - 2);
        let definitions = features::definitions(&workspace, &main, position);
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].uri, uri(&local_name));
        let item = features::completion(&workspace, &main, text.position(source.len()))
            .into_iter()
            .find(|item| item.label == "shared")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, "shared{${1:arg1}}");
        assert_eq!(
            latex_lsp::signature::help(&workspace, &main, text.position(source.len()))
                .unwrap()
                .signatures[0]
                .label,
            "\\shared{#1}"
        );
        let new_name = format!("renamed.{extension}");
        let edit = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(&local_name).into(),
                new_uri: uri(&new_name).into(),
            }],
        )
        .unwrap()
        .unwrap();
        let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
            panic!()
        };
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].text_document.uri, main);
        assert_eq!(edits[0].text_document.version, Some(8));
        assert_eq!(edits[0].edits.len(), 1);
        let OneOf::Left(edit) = &edits[0].edits[0] else {
            panic!()
        };
        assert_eq!(edit.new_text, new_name);
        assert_eq!(
            &source[text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap()],
            local_name
        );
    }
}
