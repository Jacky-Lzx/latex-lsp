use lsp_types::*;
use texquill::{
    compilation::build,
    config::BuildConfig,
    editing,
    language::{features, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///standard-math/main.tex").unwrap();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            snippets: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    (workspace, uri)
}

#[test]
fn standard_math_completions_keep_delimiters_bare_and_hint_accent_expressions() {
    for (source, name, replacement) in [
        (r"$\wideh", "widehat", "widehat{${1:expression}}"),
        (r"$\lang", "langle", "langle"),
        (r"$\Vert", "Vert", "Vert"),
        (r"$\eps", "epsilon", "epsilon"),
        (r"$\Gam", "Gamma", "Gamma"),
        (r"$\sin", "sin", "sin"),
    ] {
        let (workspace, uri) = setup(source);
        let doc = &workspace.documents[&uri];
        let items = features::completion(&workspace, &uri, doc.text.position(source.len()));
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.new_text, replacement);
        let hover = features::hover(&workspace, &uri, doc.text.position(source.len() - 1));
        if source.ends_with(name) {
            assert!(hover.is_some());
        }
    }
    for name in ["widehat", "vec", "overline", "underline", "sqrtsign"] {
        let source = format!("$\\{name}{{\\alpha +x}}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        let cursor = doc.text.position(source.find("+x").unwrap() + 1);
        let help = signature::help(&workspace, &uri, cursor).unwrap();
        assert_eq!(help.signatures[0].label, format!("\\{name}{{expression}}"));
        assert_eq!(help.active_parameter, Some(0));
        assert!(features::hover(&workspace, &uri, doc.text.position(3)).is_some());
    }
}

#[test]
fn math_builtins_cannot_be_overwritten_by_macro_or_environment_renames() {
    let source = r"\newcommand{\localmacro}{Value}\newenvironment{localenv}{}{}\csdef{namedmacro}{Value}\label{localkey}";
    let (workspace, uri) = setup(source);
    let doc = &workspace.documents[&uri];
    for target in ["localmacro", "localenv", "namedmacro"] {
        let position = doc.text.position(source.find(target).unwrap() + 2);
        for name in [
            "hat",
            "widehat",
            "bar",
            "vec",
            "sqrtsign",
            "langle",
            "Vert",
            "Uparrow",
            "epsilon",
            "varepsilon",
            "Gamma",
            "sin",
            "exp",
            "max",
        ] {
            let error = features::rename(&workspace, &uri, position, name).unwrap_err();
            assert!(
                error.message.contains("already defined"),
                "{target} -> {name}: {error:?}"
            );
        }
    }
    // Labels use an independent namespace and may retain mathematical names.
    assert!(features::rename(
        &workspace,
        &uri,
        doc.text.position(source.find("localkey").unwrap() + 2),
        "hat"
    )
    .unwrap()
    .is_some());
}

#[test]
#[ignore = "requires pdflatex"]
fn real_standard_math_typos_offer_a_fix_that_compiles_with_existing_accents_and_delimiters() {
    for (typo, replacement, body) in [
        (
            "widehaat",
            "widehat",
            r"\left\langle\widehaat{x}+\vec{y}\right\rangle",
        ),
        (
            "epsilonn",
            "epsilon",
            r"\epsilonn+\varepsilon+\Gamma+\sin x+\localop_x x+\operatorname*{arg\,max}_{x}f(x)",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        let source = format!("\\documentclass{{article}}\n\\usepackage{{amsmath}}\n\\DeclareMathOperator*{{\\localop}}{{local}}\n\\begin{{document}}\n\\hbox{{${body}$}}\n\\end{{document}}\n");
        std::fs::write(&main, &source).unwrap();
        let config = BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let before = build::run(&main, &config, &Default::default());
        assert_eq!(before.status, "error", "{}", before.output);
        let diagnostic = before.diagnostics[&uri]
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("undefined-control-sequence".into()))
            })
            .unwrap()
            .clone();
        let text = Text::new(source.clone());
        let span = text.offset(diagnostic.range.start).unwrap()
            ..text.offset(diagnostic.range.end).unwrap();
        assert_eq!(&source[span], format!("\\{typo}"));
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace
            .project
            .build_diagnostics
            .insert(uri.clone(), before.diagnostics);
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                only: Some(vec![CodeActionKind::QUICKFIX]),
                trigger_kind: None,
            },
        );
        let edit = actions
            .into_iter()
            .find_map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => action
                    .edit
                    .and_then(|edit| edit.changes)
                    .and_then(|changes| {
                        changes.get(&uri).and_then(|edits| {
                            edits
                                .iter()
                                .find(|edit| edit.new_text == format!("\\{replacement}"))
                                .cloned()
                        })
                    }),
                _ => None,
            })
            .expect("missing standard math spelling fix");
        let fixed = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap()
            .source;
        assert_eq!(
            fixed,
            source.replace(&format!("\\{typo}"), &format!("\\{replacement}"))
        );
        std::fs::write(&main, fixed).unwrap();
        let after = build::run(&main, &config, &Default::default());
        assert_eq!(after.status, "success", "{}", after.output);
        assert_eq!(after.pages, Some(1));
    }
}
