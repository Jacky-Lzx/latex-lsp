use latex_lsp::{build, build_log, config::BuildConfig, editing, text::Text, workspace::Workspace};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::*;

const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 96, 96, 96, 0, 0, 0, 4, 0,
    1, 246, 23, 56, 85, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn path_error(diagnostic: &Diagnostic) -> bool {
    diagnostic.message.contains("not found") || diagnostic.message.contains("No verbatim file")
}

fn parse(source: &str, command: &str, name: &str, echo: &str, numbered: bool) -> Diagnostic {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let message = match command {
        "input" => format!("LaTeX Error: File `{name}' not found."),
        "includegraphics" => {
            format!("Package pdftex.def Error: File `{name}' not found: using draft setting.")
        }
        "lstinputlisting" => format!(
            "Package Listings Error: File `{}(.txt)' not found.",
            name.strip_suffix(".txt").unwrap()
        ),
        _ => format!("FancyVerb Error:\n  No verbatim file {name}\n."),
    };
    let header = if numbered && !matches!(command, "input" | "lstinputlisting") {
        "./main.tex:1:"
    } else {
        "!"
    };
    let emergency = if numbered { "./main.tex:1:" } else { "!" };
    let prompt = format!("Enter file name: \n{emergency} Emergency stop.\n<read *>\n");
    let frames = match command {
        "input" | "lstinputlisting" => &prompt,
        "includegraphics" => "See the pdftex.def package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n",
        _ if numbered => "\\FV@Error ... {FancyVerb Error:\n\\space \\space #1\n}\n",
        _ => "\\FV@Error ...ncyVerb Error:^^J\\space \\space #1^^J}\n",
    };
    let log = format!("(./main.tex\n{header} {message}\n{frames}l.1 {echo}\n)\n");
    build_log::parse(&main, &log)[&Url::from_file_path(main).unwrap()]
        .iter()
        .find(|diagnostic| path_error(diagnostic))
        .unwrap()
        .clone()
}

fn interfaces() -> [(&'static str, &'static str, &'static str); 6] {
    [
        ("input", "missing.tex", "missing-input"),
        ("includegraphics", "missing.png", "missing-image"),
        ("lstinputlisting", "missing.txt", "missing-listing"),
        ("VerbatimInput", "missing.txt", "missing-fancyverb-input"),
        ("BVerbatimInput", "missing.txt", "missing-fancyverb-input"),
        ("LVerbatimInput", "missing.txt", "missing-fancyverb-input"),
    ]
}

#[test]
fn shortened_path_contexts_reject_duplicate_suffixes_in_unknown_or_opaque_calls() {
    for (command, name, _) in interfaces() {
        let call = format!("\\{command}{{{name}}}");
        for source in [
            format!("\\iffalse{call}\\fi\\unknown{command}{{{name}}}"),
            format!("\\verb|{command}{{{name}}}|{call}"),
            format!("{call} %{command}{{{name}}}"),
        ] {
            for numbered in [false, true] {
                let diagnostic = parse(
                    &source,
                    command,
                    name,
                    &format!("...{command}{{{name}}}"),
                    numbered,
                );
                assert!(
                    diagnostic.code.is_none() && diagnostic.data.is_none(),
                    "{source}: {diagnostic:?}"
                );
            }
        }
    }
}

#[test]
fn full_and_unique_shortened_path_contexts_still_distinguish_repeated_filenames() {
    for (command, name, code) in interfaces() {
        let call = format!("\\{command}{{{name}}}");
        let full = format!("\\iffalse{call}\\fi{call}");
        let mut cases = vec![
            (
                format!("\\iffalse\\unknownOther{{{name}}}\\fi{call}"),
                format!("...{call}"),
            ),
            (full.clone(), full),
            (format!("😀 prefix {call}"), format!("...😀 prefix {call}")),
        ];
        if command.ends_with("VerbatimInput") {
            cases.push((call, "...ssing.txt}".into()));
        }
        for (source, echo) in cases {
            for numbered in [false, true] {
                let diagnostic = parse(&source, command, name, &echo, numbered);
                let start = source.rfind(name).unwrap();
                assert_eq!(
                    diagnostic.range,
                    Text::new(source.clone()).range(start, start + name.len()),
                    "{source}/{echo}: {diagnostic:?}"
                );
                assert_eq!(diagnostic.code, Some(NumberOrString::String(code.into())));
                assert_eq!(diagnostic.data, Some(serde_json::json!({"file":name})));
            }
        }
    }
}

fn repaired(
    workspace: &Workspace,
    uri: &Url,
    diagnostic: &Diagnostic,
    correct: &str,
) -> Option<String> {
    let edit = editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
    .into_iter()
    .find_map(|item| match item {
        CodeActionOrCommand::CodeAction(action)
            if action.title.ends_with(&format!("file `{correct}`")) =>
        {
            action.edit
        }
        _ => None,
    })?;
    let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
        panic!("missing versioned edit")
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].text_document.uri, *uri);
    assert_eq!(changes[0].text_document.version, Some(7));
    assert_eq!(changes[0].edits.len(), 1);
    let OneOf::Left(edit) = &changes[0].edits[0] else {
        panic!("unexpected annotation")
    };
    Some(
        workspace.documents[uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap()
            .source,
    )
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_shortened_path_contexts_reject_wrong_repairs_and_preserve_unique_repairs() {
    for engine in ["pdflatex", "xelatex"] {
        for (command, package, size, extension) in [
            ("input", "", 34, "tex"),
            ("includegraphics", "graphicx", 22, "png"),
            ("lstinputlisting", "listings", 22, "txt"),
            ("VerbatimInput", "fancyvrb", 30, "txt"),
        ] {
            for ambiguous in [false, true] {
                let name = format!("key{}me.{extension}", "x".repeat(size - 5));
                let correct = name.replace("me.", "ame.");
                let call = if ambiguous {
                    format!("\\iffalse\\{command}{{{name}}}\\fi\\verylongcustomwrapper{command}{{{name}}}")
                } else {
                    format!("\\iffalse\\unknownOther{{{name}}}\\fi\\{command}{{{name}}}")
                };
                let packages = if package.is_empty() {
                    String::new()
                } else {
                    format!("\\usepackage{{{package}}}\n")
                };
                // Keep the unique echo independent of an unused macro whose
                // dynamic input makes the project's source scope uncertain.
                let declaration = if command == "input" && !ambiguous {
                    "% Direct input without an unresolved source loader.\n".to_owned()
                } else {
                    format!("\\newcommand{{\\verylongcustomwrapper{command}}}[1]{{\\{command}{{#1}}}}\n")
                };
                let source = format!("\\documentclass{{article}}\n{packages}{declaration}\\begin{{document}}\n\\section{{MarkerUnique}}\n{call}\n\\end{{document}}\n");
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let uri = Url::from_file_path(&main).unwrap();
                std::fs::write(
                    directory.path().join(&correct),
                    if extension == "png" {
                        PNG
                    } else {
                        b"ImportedUnique.\n"
                    },
                )
                .unwrap();
                std::fs::write(&main, &source).unwrap();
                let config = BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                };
                let before = build::run(&main, &config, &AtomicBool::new(false));
                assert_eq!(
                    before.status, "error",
                    "{engine}/{command}/{ambiguous}: {}",
                    before.output
                );
                let diagnostic = before.diagnostics[&uri]
                    .iter()
                    .find(|d| {
                        d.severity == Some(DiagnosticSeverity::ERROR)
                            && (path_error(d) || d.message.contains("Unable to load"))
                    })
                    .unwrap();
                let start = source.rfind(&name).unwrap();
                let mut expected = source.clone();
                expected.replace_range(start..start + name.len(), &correct);
                let mut workspace = Workspace {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                };
                workspace.add_root(directory.path().into());
                workspace.open(uri.clone(), source.clone(), 7);
                workspace
                    .build_diagnostics
                    .insert(uri.clone(), before.diagnostics.clone());
                let repair = repaired(&workspace, &uri, diagnostic, &correct);
                if ambiguous {
                    assert!(
                        diagnostic.code.is_none() && diagnostic.data.is_none(),
                        "{engine}/{command}: {diagnostic:?}"
                    );
                    assert!(repair.is_none(), "{engine}/{command}: {repair:?}");
                } else {
                    assert_eq!(
                        diagnostic.range,
                        Text::new(source).range(start, start + name.len()),
                        "{engine}/{command}: {diagnostic:?}"
                    );
                    assert_eq!(repair.as_ref(), Some(&expected), "{engine}/{command}");
                }
                std::fs::write(&main, expected).unwrap();
                let after = build::run(&main, &config, &AtomicBool::new(false));
                assert_eq!(
                    after.status, "success",
                    "{engine}/{command}/{ambiguous}: {}",
                    after.output
                );
                assert!(directory.path().join("build/latex-lsp/main.pdf").is_file());
            }
        }
    }
}
