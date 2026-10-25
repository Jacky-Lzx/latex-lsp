use latex_lsp::{features, workspace::Workspace};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const LOADERS: [&str; 5] = [
    "verbatiminput",
    "lstinputlisting",
    "VerbatimInput",
    "LVerbatimInput",
    "BVerbatimInput",
];
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn workspace(root: &Path, source: &str) -> Workspace {
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join("notes.tex"),
        r"\newcommand{\fromnotes}{CodeUnique}",
    )
    .unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace
        .documents
        .get_mut(&uri(root, "main.tex"))
        .unwrap()
        .version = Some(7);
    workspace
        .documents
        .get_mut(&uri(root, "notes.tex"))
        .unwrap()
        .version = Some(3);
    workspace.recorded_inputs.insert(
        uri(root, "main.tex"),
        BTreeSet::from([uri(root, "notes.tex")]),
    );
    workspace
}
fn apply(workspace: &Workspace, edit: WorkspaceEdit) -> BTreeMap<Url, String> {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edits")
    };
    documents
        .into_iter()
        .map(|doc| {
            let source = &workspace.documents[&doc.text_document.uri];
            assert_eq!(doc.text_document.version, source.version);
            let mut edits: Vec<_> = doc
                .edits
                .into_iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    _ => panic!(),
                })
                .collect();
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let updated = source
                .text
                .changed(
                    edits
                        .into_iter()
                        .map(|edit| TextDocumentContentChangeEvent {
                            range: Some(edit.range),
                            range_length: None,
                            text: edit.new_text,
                        })
                        .collect(),
                )
                .unwrap();
            (doc.text_document.uri, updated.source)
        })
        .collect()
}

#[test]
fn inactive_display_calls_preserve_recorded_source_definitions_and_versioned_macro_renames() {
    for loader in LOADERS {
        let command = format!("\\{loader}{{notes.tex}}");
        for kind in [
            "plain",
            "unused-body",
            "discarded-group",
            "discarded-token",
            "active",
            "after-completed-argument",
        ] {
            let body = match kind {
                "plain" => String::new(),
                "unused-body" => format!("\\newcommand{{\\unused}}{{{command}}}"),
                "discarded-group" => format!("\\hold{{{command}}}"),
                "discarded-token" => format!("\\hold{command}"),
                "active" => command.clone(),
                _ => format!("\\hold{{ignored}}{command}"),
            };
            let displayed = matches!(kind, "active" | "after-completed-argument");
            let actual_input = if displayed { "" } else { r"\input{\picked}" };
            let source = format!(
                r"\documentclass{{article}}\newcommand{{\hold}}[1]{{}}\def\picked{{notes}}\begin{{document}}{body}{actual_input}\ifdefined\fromnotes\fromnotes\fi\end{{document}}"
            );
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let workspace = workspace(root, &source);
            let main = uri(root, "main.tex");
            let notes = uri(root, "notes.tex");
            assert_eq!(
                workspace.project_documents(&main).contains(&notes),
                !displayed,
                "{loader}/{kind}"
            );
            let at = workspace.documents[&main]
                .text
                .position(source.rfind(r"\fromnotes").unwrap() + 3);
            let definitions = features::definitions(&workspace, &main, at);
            assert_eq!(
                definitions.len(),
                usize::from(!displayed),
                "{loader}/{kind}: {definitions:?}"
            );
            assert_eq!(
                features::prepare_rename(&workspace, &main, at)
                    .unwrap()
                    .is_some(),
                !displayed,
                "{loader}/{kind}"
            );
            if displayed {
                continue;
            }
            assert_eq!(definitions[0].uri, notes);
            let edit = features::rename(&workspace, &main, at, "renamednotes")
                .unwrap()
                .unwrap();
            let updated = apply(&workspace, edit);
            assert_eq!(updated.len(), 2);
            for target in [&main, &notes] {
                assert_eq!(
                    updated[target],
                    workspace.documents[target]
                        .text
                        .source
                        .replace(r"\fromnotes", r"\renamednotes"),
                    "{loader}/{kind}"
                );
            }
        }
    }
}

#[test]
fn recorder_macro_providers_decide_whether_a_following_code_command_is_displayed() {
    for loader in LOADERS {
        for arguments in [0, 1] {
            let source = format!(
                r"\documentclass{{article}}\hold\{loader}{{notes.tex}}\ifdefined\fromnotes\fromnotes\fi"
            );
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let mut workspace = workspace(root, &source);
            let main = uri(root, "main.tex");
            let notes = uri(root, "notes.tex");
            // Only the recorder connects this late-arriving signature provider.
            let provider = uri(root, "provider.tex");
            let text = format!("\\newcommand{{\\hold}}[{arguments}]{{}}");
            std::fs::write(root.join("provider.tex"), &text).unwrap();
            workspace.documents.insert(
                provider.clone(),
                latex_lsp::workspace::Document::new(&provider, text, Some(5)),
            );
            workspace
                .recorded_inputs
                .get_mut(&main)
                .unwrap()
                .insert(provider.clone());
            let project = workspace.project_documents(&main);
            assert!(project.contains(&provider));
            assert_eq!(
                project.contains(&notes),
                arguments == 1,
                "{loader}/{arguments}"
            );
            let at = workspace.documents[&main]
                .text
                .position(source.rfind(r"\fromnotes").unwrap() + 3);
            let definitions = features::definitions(&workspace, &main, at);
            assert_eq!(
                definitions.len(),
                arguments,
                "{loader}/{arguments}: {definitions:?}"
            );
        }
    }
}

#[test]
fn an_active_redefined_display_command_keeps_uncertain_recorded_execution_as_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\documentclass{article}\renewcommand{\lstinputlisting}[1]{\input{#1}}\verbatiminput{notes.tex}\lstinputlisting{notes.tex}\fromnotes";
    let workspace = workspace(root, source);
    let main = uri(root, "main.tex");
    let notes = uri(root, "notes.tex");
    assert!(workspace.project_documents(&main).contains(&notes));
    let at = workspace.documents[&main]
        .text
        .position(source.rfind(r"\fromnotes").unwrap() + 3);
    assert_eq!(features::definitions(&workspace, &main, at)[0].uri, notes);
}
