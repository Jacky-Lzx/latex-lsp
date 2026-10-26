use latex_lsp::{
    language::features,
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

const COMMANDS: &[(&str, &str)] = &[
    ("citefield", "field"),
    ("citelist", "literal list"),
    ("citename", "name list"),
];
fn insert(workspace: &mut Workspace, file: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///biblatex-data/{file}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(5)));
    uri
}
fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
        .unwrap()
        .source
}

#[test]
fn data_citations_index_only_keys_and_preserve_formats_fields_and_notes() {
    for (name, _) in COMMANDS {
        let mut workspace = Workspace::default();
        let bib = insert(
            &mut workspace,
            "refs.bib",
            "@book{alpha,title={Literal alpha}}\n@book{beta,title={Beta}}\n",
        );
        let source=format!("😀 \\{name}[alpha][alpha]{{% alpha, ignored\r\n alpha,\r\n beta}}[alpha]{{alpha}}\\label{{alpha}}\\verb|\\{name}{{alpha}}{{alpha}}|% \\{name}{{alpha}}{{alpha}}\r\n");
        let main = insert(&mut workspace, "main.tex", &source);
        let start = source.find(" alpha,\r\n").unwrap() + 1;
        let at = workspace.documents[&main].text.position(start + 1);
        assert_eq!(
            features::definitions(&workspace, &main, at)[0].uri,
            bib,
            "{name}"
        );
        assert!(features::hover(&workspace, &main, at).is_some());
        assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
        assert_eq!(
            latex_lsp::editing::highlights(&workspace, &main, at).len(),
            1
        );
        let semantic =
            latex_lsp::language::semantic::tokens(&workspace.documents[&main], None).unwrap();
        let mut line = 0;
        let mut character = 0;
        let mut keys = Vec::new();
        for token in semantic.data {
            character = if token.delta_line == 0 {
                character + token.delta_start
            } else {
                token.delta_start
            };
            line += token.delta_line;
            if token.token_type == 3 && token.token_modifiers_bitset == 0 {
                keys.push(Position::new(line, character));
            }
        }
        assert_eq!(keys.len(), 2, "{name}: {keys:?}");
        assert!(keys.contains(&workspace.documents[&main].text.position(start)));
        let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes[&main].len(), 1);
        let mut expected = source.clone();
        expected.replace_range(start..start + 5, "renamed");
        assert_eq!(apply(&source, &changes[&main]), expected);
        assert_eq!(
            apply(
                "@book{alpha,title={Literal alpha}}\n@book{beta,title={Beta}}\n",
                &changes[&bib]
            ),
            "@book{renamed,title={Literal alpha}}\n@book{beta,title={Beta}}\n"
        );
        for marker in ["[alpha]", "{alpha}\\label"] {
            let at = workspace.documents[&main]
                .text
                .position(source.find(marker).unwrap() + 2);
            assert!(
                features::definitions(&workspace, &main, at).is_empty(),
                "{name}/{marker}"
            );
            assert!(!features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == "alpha"));
        }
    }
}

#[test]
fn data_citation_completion_and_signatures_follow_each_argument_role() {
    for (name, selector) in COMMANDS {
        for (tail, expected) in [
            ("[al¦pha][post]{alpha}[fmt]{field}", 0),
            ("[pre][po¦st]{alpha}[fmt]{field}", 1),
            ("[pre][post]{al¦pha}[fmt]{field}", 2),
            ("[pre][post]{alpha}[fm¦t]{field}", 3),
            ("[pre][post]{alpha}[fmt]{fi¦eld}", 4),
            ("[pre][post]{alpha}¦", 4),
            ("[pre][post]{alpha}[fmt] ¦", 4),
        ] {
            let marked = format!("\\{name}{tail}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            let help = latex_lsp::language::signature::help(&workspace, &main, at)
                .unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(help.active_parameter, Some(expected), "{marked}");
            assert_eq!(
                help.signatures[0].label,
                format!("\\{name}[note / prenote][postnote]{{keys}}[format]{{{selector}}}")
            );
        }
        for tail in [
            "[pre][post]{beta, al¦pha}[fmt]{field}",
            "[pre][post]{beta,% comment\r\n al¦",
        ] {
            let marked = format!("😀 \\{name}{tail}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            insert(
                &mut workspace,
                "refs.bib",
                "@book{alpha,title={Alpha}}\n@book{beta,title={Beta}}",
            );
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            let items = features::completion(&workspace, &main, at);
            let item = items
                .iter()
                .find(|item| item.label == "alpha")
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                marked.replace("al¦pha", "alpha").replace("al¦", "alpha")
            );
        }
        for following in ["", "{alpha}{field}", "[pre]{alpha}[fmt]{field}"] {
            let source = format!("\\{name}{following}");
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    snippets: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main].text.position(name.len() + 1);
            let items = features::completion(&workspace, &main, at);
            let item = items.iter().find(|item| item.label == *name).unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                edit.new_text,
                if following.is_empty() {
                    format!("{name}{{${{1:keys}}}}{{${{2:{selector}}}}}")
                } else {
                    name.to_string()
                }
            );
        }
        let mut workspace = Workspace::default();
        let source = format!("\\newcommand\\{name}[1]{{}}\\{name}{{custom}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(source.find("custom").unwrap() + 2);
        assert_eq!(
            latex_lsp::language::signature::help(&workspace, &main, at)
                .unwrap()
                .signatures[0]
                .label,
            format!("\\{name}{{#1}}")
        );
    }
}

#[test]
fn unsupported_data_citation_forms_do_not_guess_keys_from_notes_formats_or_prose() {
    for (name, _) in COMMANDS {
        for tail in [
            "*[pre]{al¦pha}{field}",
            "[one][two][three]{al¦pha}{field}",
            "{\\dyn¦amic}{field}",
            "{beta}[al¦pha]{field}",
            "{beta}[format]{al¦pha}",
            "{beta}{field} text {al¦pha}",
        ] {
            let marked = format!("\\{name}{tail}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            insert(
                &mut workspace,
                "refs.bib",
                "@book{alpha,title={Alpha}}\n@book{beta,title={Beta}}",
            );
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            assert!(
                features::definitions(&workspace, &main, at).is_empty(),
                "{marked}"
            );
            assert!(
                features::prepare_rename(&workspace, &main, at)
                    .unwrap()
                    .is_none(),
                "{marked}"
            );
            assert!(
                !features::completion(&workspace, &main, at)
                    .iter()
                    .any(|item| item.label == "alpha"),
                "{marked}"
            );
        }
    }
    for name in ["Citefield", "Citelist", "Citename"] {
        let mut workspace = Workspace::default();
        insert(&mut workspace, "refs.bib", "@book{alpha,title={Alpha}}");
        let source = format!("\\{name}{{alpha}}{{title}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(source.find("alpha").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &main, at).is_empty(),
            "{name}"
        );
    }
}

#[test]
fn multiline_data_citation_warnings_and_spelling_fixes_target_keys_before_the_selector() {
    for (name, _) in COMMANDS {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source=format!("\\addbibresource{{refs.bib}}\n\\{name}[alphaa][alphaa]\n{{alphaa}}\n[alphaa]\n{{alphaa}}\n\\relax\n");
        std::fs::write(&main, &source).unwrap();
        std::fs::write(
            directory.path().join("refs.bib"),
            "@book{alpha,title={Alpha}}",
        )
        .unwrap();
        let uri = Url::from_file_path(&main).unwrap();
        for line in [5, 6] {
            let log=format!("Package biblatex Warning: Citation 'alphaa' on page 1 undefined on input line {line}.\n");
            let diagnostics = latex_lsp::compilation::build_log::parse(&main, &log);
            let items = &diagnostics[&uri];
            let text = Text::new(source.clone());
            let start = source.find("{alphaa}").unwrap() + 1;
            assert_eq!(items.len(), 1, "{name}/{line}: {items:?}");
            assert_eq!(
                items[0].range,
                text.range(start, start + 6),
                "{name}/{line}"
            );
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(uri.clone(), source.clone(), 5);
            workspace.open(
                Url::from_file_path(directory.path().join("refs.bib")).unwrap(),
                "@book{alpha,title={Alpha}}".into(),
                5,
            );
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), diagnostics.clone());
            let actions = latex_lsp::editing::code_actions(
                &workspace,
                &uri,
                items[0].range,
                &CodeActionContext {
                    diagnostics: items.clone(),
                    only: None,
                    trigger_kind: None,
                },
            );
            let edit = actions
                .iter()
                .find_map(|action| match action {
                    CodeActionOrCommand::CodeAction(action) => {
                        action.edit.as_ref()?.changes.as_ref()?[&uri]
                            .iter()
                            .find(|edit| edit.new_text == "alpha")
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name}/{line}: {actions:?}"));
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                source.replacen("{alphaa}", "{alpha}", 1)
            );
        }
    }
}

#[test]
fn data_citation_key_renames_respect_independent_projects_and_shared_source_ambiguity() {
    for (name, _) in COMMANDS {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for project in ["a", "b"] {
            std::fs::write(root.join(format!("{project}.tex")),format!("\\documentclass{{article}}\\addbibresource{{{project}.bib}}\\{name}{{alpha}}[alpha]{{alpha}}")).unwrap();
            std::fs::write(
                root.join(format!("{project}.bib")),
                "@book{alpha,title={Alpha}}",
            )
            .unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let bib = Url::from_file_path(root.join("a.bib")).unwrap();
        let main = Url::from_file_path(root.join("a.tex")).unwrap();
        let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[&main].len(), 1);
        assert!(changes.contains_key(&bib));
        std::fs::write(
            root.join("shared.tex"),
            format!("\\{name}{{alpha}}[alpha]{{alpha}}"),
        )
        .unwrap();
        for project in ["a", "b"] {
            std::fs::write(
                root.join(format!("{project}.tex")),
                format!(
                    "\\documentclass{{article}}\\addbibresource{{{project}.bib}}\\input{{shared}}"
                ),
            )
            .unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        assert!(features::rename(&workspace, &bib, Position::new(0, 8), "renamed").is_err());
    }
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_data_citation_warnings_and_key_renames_compile_with_formats_and_key_lists() {
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let bib = root.join("refs.bib");
    let mut source="\\documentclass{article}\n\\usepackage[backend=bibtex,style=numeric]{biblatex}\n\\addbibresource{refs.bib}\n\\begin{document}\n".to_string();
    for (name, selector) in [
        ("citefield", "title"),
        ("citelist", "location"),
        ("citename", "author"),
    ] {
        for format in [String::new(), format!("[{name}]")] {
            source.push_str(&format!(
                "\\{name}[see][12]\n{{alpha,beta}}\n{format}{{{selector}}}\\relax\\par\n"
            ));
        }
    }
    source.push_str("\\printbibliography\n\\end{document}\n");
    let bibliography="@book{alpha,author={Alpha, Alice},title={Literal alpha},location={Alpha City},year={2020}}\n@book{beta,author={Beta, Bob},title={Beta},location={Beta City},year={2021}}\n";
    fs::write(&main, &source).unwrap();
    fs::write(&bib, bibliography).unwrap();
    let run = |program: &str, args: &[&str]| {
        let output = Command::new(program)
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let tex = || {
        run(
            "pdflatex",
            &["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
        )
    };
    tex();
    let log = fs::read_to_string(root.join("main.log")).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let bib_uri = Url::from_file_path(&bib).unwrap();
    let diagnostics = latex_lsp::compilation::build_log::parse(&main, &log);
    let citations = diagnostics[&uri]
        .iter()
        .filter(|d| d.message.contains("Citation '"))
        .collect::<Vec<_>>();
    assert_eq!(citations.len(), 12, "{citations:?}");
    let text = Text::new(source.clone());
    for diagnostic in citations {
        let key = &source[text.offset(diagnostic.range.start).unwrap()
            ..text.offset(diagnostic.range.end).unwrap()];
        assert!(matches!(key, "alpha" | "beta"), "{diagnostic:?}");
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let changes = features::rename(&workspace, &bib_uri, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&uri].len(), 6);
    fs::write(&main, apply(&source, &changes[&uri])).unwrap();
    fs::write(&bib, apply(bibliography, &changes[&bib_uri])).unwrap();
    tex();
    run("bibtex", &["main"]);
    tex();
    tex();
    let log = fs::read_to_string(root.join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
    let aux = fs::read_to_string(root.join("main.aux")).unwrap();
    assert!(aux.contains("\\abx@aux@cite{0}{renamed}"));
    assert!(!aux.contains("\\abx@aux@cite{0}{alpha}"));
    assert!(fs::metadata(root.join("main.pdf")).unwrap().len() > 1000);
}
