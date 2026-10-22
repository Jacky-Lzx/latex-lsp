use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

const ERROR: &str = "Package etoolbox Error: Invalid boolean expression.";
const HELP: &str = "\nSee the etoolbox package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\n";

fn log(line: usize, echo: &str, detail: &str, numbered: bool) -> String {
    let header = if numbered {
        format!("./main.tex:{line}:")
    } else {
        "!".into()
    };
    format!(
        "(./main.tex\n{header} {ERROR}\n{HELP}l.{line} {echo}\n    {{Yes}}{{No}}\n{detail}\n\n)\n"
    )
}

fn actions(workspace: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}

#[test]
fn mixed_expression_errors_keep_distinct_namespaces_and_exact_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let call = r"\ifboolexpr{bool {darft} or togl {darft}}";
    let source = format!("\\documentclass{{article}}\n{call}{{Yes}}{{No}}\n");
    std::fs::write(&main, &source).unwrap();
    for numbered in [false, true] {
        let result = build_log::parse(
            &main,
            &(log(2, call, "Boolean 'darft' undefined.", numbered)
                + &log(2, call, "Toggle 'darft' undefined.", numbered)),
        );
        let errors = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 2, "{errors:?}");
        for ((error, (start, _)), kind) in errors
            .iter()
            .zip(source.match_indices("darft"))
            .zip(["boolean", "toggle"])
        {
            assert_eq!(
                error.range,
                Text::new(source.clone()).range(start, start + 5)
            );
            assert_eq!(
                error.code,
                Some(NumberOrString::String(format!("undefined-{kind}")))
            );
            assert_eq!(error.data.as_ref().unwrap()[kind], "darft");
            assert!(error.message.starts_with(&format!("{ERROR}\n")));
        }
    }
}

#[test]
fn expression_repairs_preserve_operators_branches_and_the_other_namespace() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let call = r"\ifboolexpr{not (bool {darft} and togl {darft})}";
    let source = format!("\\newbool{{draft}}\\newtoggle{{draft}}\n{call}{{Yes}}{{No}}\n");
    std::fs::write(&main, &source).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    for (kind, detail) in [
        ("boolean", "Boolean 'darft' undefined."),
        ("toggle", "Toggle 'darft' undefined."),
    ] {
        let diagnostics = build_log::parse(&main, &log(2, call, detail, true));
        let error = diagnostics[&uri][0].clone();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace.build_diagnostics.insert(uri.clone(), diagnostics);
        let edit = actions(&workspace, &uri, &error)
            .into_iter()
            .find_map(|item| match item {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == format!("Replace with {kind} `draft`") =>
                {
                    action.edit
                }
                _ => None,
            })
            .expect("missing expression spelling repair");
        let edits = &edit.changes.unwrap()[&uri];
        assert_eq!(edits.len(), 1);
        let repaired = workspace.documents[&uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edits[0].range),
                range_length: None,
                text: edits[0].new_text.clone(),
            }])
            .unwrap()
            .source;
        let marker = if kind == "boolean" {
            "bool {darft}"
        } else {
            "togl {darft}"
        };
        assert_eq!(
            repaired,
            source.replace(marker, &marker.replace("darft", "draft"))
        );
    }
}
