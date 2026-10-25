use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///biblatex-commands/{name}")).unwrap();
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
fn capitalized_and_text_citations_navigate_and_join_cross_file_key_renames() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{alpha,title={Literal alpha}}\n",
    );
    let source = "😀 \\Parencite[alpha]{alpha} \\citetitle*{alpha}\\citedate[alpha]{alpha}\\label{alpha}\\verb|\\Parencite{alpha}|% \\Cite{alpha}\n";
    let main = insert(&mut workspace, "main.tex", source);
    let position = workspace.documents[&main]
        .text
        .position(source.find("{alpha}").unwrap() + 2);
    assert_eq!(features::definitions(&workspace, &main, position).len(), 1);
    assert!(features::hover(&workspace, &main, position).is_some());
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        4
    );
    let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&main].len(), 3);
    assert_eq!(apply(source,&changes[&main]), "😀 \\Parencite[alpha]{renamed} \\citetitle*{renamed}\\citedate[alpha]{renamed}\\label{alpha}\\verb|\\Parencite{alpha}|% \\Cite{alpha}\n");
}

const COMMANDS: &[&str] = &[
    "Cite",
    "Parencite",
    "Footcite",
    "footcitetext",
    "Footcitetext",
    "smartcite",
    "Smartcite",
    "supercite",
    "Textcite",
    "Autocite",
    "citetitle",
    "Citetitle",
    "citedate",
    "citeurl",
    "notecite",
    "pnotecite",
    "fnotecite",
    "Notecite",
    "Pnotecite",
    "Fnotecite",
    "footcite",
    "fullcite",
    "footfullcite",
];

#[test]
fn all_commands_and_stars_share_key_navigation_highlights_and_safe_renames() {
    for name in COMMANDS {
        for star in ["", " % before star\r\n*% after star\r\n"] {
            let mut workspace = Workspace::default();
            let bibliography = "@book{alpha,title={Literal alpha},url={https://example.org/alpha}}\n@book{beta,title={Beta}}";
            let bib = insert(&mut workspace, "refs.bib", bibliography);
            let source = format!("😀 \\{name}{star}[alpha][alpha]{{% alpha, ignored\r\n alpha,\r\n beta}}{{alpha}}\\label{{alpha}}\\verb|\\{name}{{alpha}}|% \\{name}{{alpha}}\r\n");
            let main = insert(&mut workspace, "main.tex", &source);
            let doc = &workspace.documents[&main];
            let alpha_start = source.find(" alpha,\r\n").unwrap() + 1;
            let beta_start = source.find(" beta}").unwrap() + 1;
            for (key, start) in [("alpha", alpha_start), ("beta", beta_start)] {
                let position = doc.text.position(start + 1);
                let definitions = features::definitions(&workspace, &main, position);
                assert_eq!(definitions.len(), 1, "{source}");
                assert_eq!(definitions[0].uri, bib);
                assert!(features::hover(&workspace, &main, position).is_some());
                assert!(features::prepare_rename(&workspace, &main, position)
                    .unwrap()
                    .is_some());
                assert_eq!(
                    features::references(&workspace, &main, position, true).len(),
                    2
                );
                assert_eq!(
                    latex_lsp::editing::highlights(&workspace, &main, position).len(),
                    1
                );
                let changes = features::rename(&workspace, &main, position, "renamed")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes[&main].len(), 1);
                assert_eq!(changes[&bib].len(), 1);
                let mut expected = source.clone();
                expected.replace_range(start..start + key.len(), "renamed");
                assert_eq!(apply(&source, &changes[&main]), expected);
                if key == "alpha" {
                    assert_eq!(
                        apply(bibliography, &changes[&bib]),
                        bibliography.replacen("@book{alpha,", "@book{renamed,", 1)
                    );
                }
            }
            assert_eq!(
                latex_lsp::semantic::tokens(doc, None)
                    .unwrap()
                    .data
                    .iter()
                    .filter(|token| token.token_type == 3 && token.token_modifiers_bitset == 0)
                    .count(),
                2
            );
        }
    }
}

#[test]
fn completion_replaces_only_current_keys_in_finished_and_unfinished_lists() {
    for name in COMMANDS {
        for tail in ["[alpha]{beta, al¦pha}", "*[alpha][alpha]{beta,\r\n al¦"] {
            let marked = format!("😀 \\{name}{tail}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            insert(
                &mut workspace,
                "refs.bib",
                "@book{alpha,title={Alpha}}\n@book{beta,title={Beta}}",
            );
            let main = insert(&mut workspace, "main.tex", &source);
            let position = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            let items = features::completion(&workspace, &main, position);
            let item = items
                .iter()
                .find(|item| item.label == "alpha")
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                marked.replace("al¦pha", "alpha").replace("al¦", "alpha"),
                "{marked}"
            );
        }
    }
}

#[test]
fn command_snippets_preserve_existing_arguments_and_user_signatures_win() {
    for name in COMMANDS {
        for following in ["", "{alpha}", "[pre][post]{alpha}"] {
            let mut workspace = Workspace {
                snippets: true,
                ..Workspace::default()
            };
            let source = format!("\\{name}{following}");
            let main = insert(&mut workspace, "main.tex", &source);
            let items = features::completion(&workspace, &main, Position::new(0, 3));
            let item = items
                .iter()
                .find(|item| item.label == *name)
                .unwrap_or_else(|| panic!("{source}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                edit.new_text,
                if following.is_empty() {
                    format!("{name}{{${{1:key}}}}")
                } else {
                    name.to_string()
                },
                "{source}"
            );
        }
        let mut workspace = Workspace::default();
        let source = format!("\\newcommand\\{name}[1]{{}}\\{name}{{custom}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let position = workspace.documents[&main]
            .text
            .position(source.find("custom").unwrap() + 2);
        assert_eq!(
            latex_lsp::signature::help(&workspace, &main, position)
                .unwrap()
                .signatures[0]
                .label,
            format!("\\{name}{{#1}}")
        );
    }
}

#[test]
fn signature_help_distinguishes_notes_and_keys_for_regular_and_starred_commands() {
    for name in COMMANDS {
        for star in ["", "*"] {
            for (arguments, active) in [
                ("[pr¦e][post]{alpha}", 0),
                ("[pre][po¦st]{alpha}", 1),
                ("[pre][post]{al¦pha}", 2),
            ] {
                let marked = format!("\\{name}{star}{arguments}");
                let source = marked.replace('¦', "");
                let mut workspace = Workspace::default();
                let main = insert(&mut workspace, "main.tex", &source);
                let position = workspace.documents[&main]
                    .text
                    .position(marked.find('¦').unwrap());
                let help = latex_lsp::signature::help(&workspace, &main, position)
                    .unwrap_or_else(|| panic!("{marked}"));
                assert_eq!(help.active_parameter, Some(active), "{marked}");
                assert!(
                    help.signatures[0]
                        .label
                        .ends_with("[note / prenote][postnote]{keys}"),
                    "{marked}: {help:?}"
                );
            }
        }
    }
}

#[test]
fn notes_extra_arguments_dynamic_keys_and_unknown_capitalizations_are_not_guessed() {
    for name in COMMANDS {
        for tail in [
            "[al¦pha]{beta}",
            "[pre][post][extra]{al¦pha}",
            "{\\dynamic}{al¦pha}",
            "{beta} text {al¦pha}",
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
            let position = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            assert!(
                features::definitions(&workspace, &main, position).is_empty(),
                "{marked}"
            );
            assert!(
                features::prepare_rename(&workspace, &main, position)
                    .unwrap()
                    .is_none(),
                "{marked}"
            );
            assert!(
                !features::completion(&workspace, &main, position)
                    .iter()
                    .any(|item| item.label == "alpha"),
                "{marked}"
            );
        }
    }
    for name in ["Supercite", "Citeurl", "Citedate", "PARencite"] {
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", &format!("\\{name}{{alpha}}"));
        insert(&mut workspace, "refs.bib", "@book{alpha,title={Alpha}}");
        let at = workspace.documents[&main].text.position(name.len() + 3);
        assert!(
            features::definitions(&workspace, &main, at).is_empty(),
            "{name}"
        );
        assert!(
            !features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == "alpha"),
            "{name}"
        );
    }
}

#[test]
fn missing_key_warnings_and_spelling_fixes_preserve_notes_and_prose() {
    for name in COMMANDS {
        for star in ["", "*"] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let source = format!("\\addbibresource{{refs.bib}}\n😀 \\{name}{star}[alphaa][alphaa]{{alphaa}}{{alphaa}}% alphaa\n");
            std::fs::write(&main, &source).unwrap();
            std::fs::write(
                directory.path().join("refs.bib"),
                "@book{alpha,title={Alpha}}",
            )
            .unwrap();
            let uri = Url::from_file_path(&main).unwrap();
            let diagnostics = latex_lsp::build_log::parse(&main, "Package biblatex Warning: Citation 'alphaa' on page 1 undefined on input line 2.\n");
            let items = &diagnostics[&uri];
            assert_eq!(items.len(), 1, "{source}: {items:?}");
            let text = Text::new(source.clone());
            let start = source.find("{alphaa}").unwrap() + 1;
            assert_eq!(items[0].range, text.range(start, start + 6), "{source}");
            let mut workspace = Workspace {
                code_action_literals: true,
                ..Workspace::default()
            };
            workspace.open(uri.clone(), source.clone(), 5);
            workspace.open(
                Url::from_file_path(directory.path().join("refs.bib")).unwrap(),
                "@book{alpha,title={Alpha}}".into(),
                5,
            );
            workspace
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
                .unwrap_or_else(|| panic!("{source}: {actions:?}"));
            assert_eq!(edit.range, items[0].range);
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                source.replacen("{alphaa}", "{alpha}", 1)
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_biblatex_single_citations_resolve_after_bibliography_key_rename() {
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("refs.bib");
    let mut source = "\\documentclass{article}\n\\usepackage[backend=bibtex,style=numeric]{biblatex}\n\\addbibresource{refs.bib}\n\\begin{document}\n".to_string();
    for name in COMMANDS {
        for star in ["", "*"] {
            source.push_str(&format!(
                "\\{name}{star}[see][12]{{alpha,beta}}\\relax\\par\n"
            ));
        }
    }
    source.push_str("\\printbibliography\n\\end{document}\n");
    let bibliography = "@book{alpha,author={Alpha, Alice},title={Literal alpha},year={2020},url={https://example.org/alpha}}\n@book{beta,author={Beta, Bob},title={Second},year={2021}}\n";
    fs::write(&main, &source).unwrap();
    fs::write(&bib, bibliography).unwrap();
    let run = |program: &str, arguments: &[&str]| {
        let output = Command::new(program)
            .args(arguments)
            .current_dir(directory.path())
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
    let log = fs::read_to_string(directory.path().join("main.log")).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let diagnostics = latex_lsp::build_log::parse(&main, &log);
    let citations = diagnostics[&uri]
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("Citation '"))
        .collect::<Vec<_>>();
    assert_eq!(citations.len(), COMMANDS.len() * 4, "{citations:?}");
    let text = Text::new(source.clone());
    for diagnostic in citations {
        let key = &source[text.offset(diagnostic.range.start).unwrap()
            ..text.offset(diagnostic.range.end).unwrap()];
        assert!(matches!(key, "alpha" | "beta"), "{diagnostic:?}");
    }
    let mut workspace = Workspace::default();
    workspace.open(uri.clone(), source.clone(), 5);
    let bib_uri = Url::from_file_path(&bib).unwrap();
    workspace.open(bib_uri.clone(), bibliography.into(), 5);
    let changes = features::rename(&workspace, &bib_uri, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&uri].len(), COMMANDS.len() * 2);
    assert_eq!(changes[&bib_uri].len(), 1);
    let renamed_bib = apply(bibliography, &changes[&bib_uri]);
    assert!(renamed_bib.contains("title={Literal alpha}"));
    assert!(renamed_bib.contains("url={https://example.org/alpha}"));
    fs::write(&main, apply(&source, &changes[&uri])).unwrap();
    fs::write(&bib, renamed_bib).unwrap();
    tex();
    run("bibtex", &["main"]);
    tex();
    tex();
    let log = fs::read_to_string(directory.path().join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
    let aux = fs::read_to_string(directory.path().join("main.aux")).unwrap();
    assert!(aux.contains("\\abx@aux@cite{0}{renamed}"));
    assert!(!aux.contains("\\abx@aux@cite{0}{alpha}"));
    assert!(
        fs::metadata(directory.path().join("main.pdf"))
            .unwrap()
            .len()
            > 1000
    );
}
