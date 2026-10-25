use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///volume-citations/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(6)));
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
fn volume_citations_navigate_and_join_bibliography_renames_without_editing_volumes_or_notes() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{alpha,title={Literal alpha}}\n@book{beta,title={Beta}}\n",
    );
    let source = "😀 \\volcite[alpha]{alpha}[alpha]{alpha,beta}\\volcites(alpha)(alpha)[alpha]{alpha}[alpha]{beta}{2}{alpha}\\relax{alpha}";
    let main = insert(&mut workspace, "main.tex", source);
    let position = workspace.documents[&main]
        .text
        .position(source.rfind("{2}{alpha}").unwrap() + 6);
    let definitions = features::definitions(&workspace, &main, position);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, bib);
    assert!(features::hover(&workspace, &main, position).is_some());
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        3
    );
    let changes = features::rename(&workspace, &main, position, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&main].len(), 2);
    assert_eq!(changes[&bib].len(), 1);
    assert_eq!(apply(source, &changes[&main]), "😀 \\volcite[alpha]{alpha}[alpha]{renamed,beta}\\volcites(alpha)(alpha)[alpha]{alpha}[alpha]{beta}{2}{renamed}\\relax{alpha}");
}

fn commands() -> Vec<(String, bool)> {
    [
        "volcite",
        "pvolcite",
        "fvolcite",
        "ftvolcite",
        "svolcite",
        "tvolcite",
        "avolcite",
    ]
    .into_iter()
    .flat_map(|base| {
        [false, true].into_iter().flat_map(move |repeated| {
            let name = format!("{base}{}", if repeated { "s" } else { "" });
            [
                (name.clone(), repeated),
                (
                    format!("{}{}", name[..1].to_uppercase(), &name[1..]),
                    repeated,
                ),
            ]
        })
    })
    .collect()
}

#[test]
fn all_volume_commands_and_stars_index_only_key_lists_and_preserve_notes_and_trailing_text() {
    for (name, repeated) in commands() {
        for star in ["", "*"] {
            let mut workspace = Workspace::default();
            let bib = insert(
                &mut workspace,
                "refs.bib",
                "@book{alpha,title={alpha}}\n@book{beta,title={beta}}",
            );
            let arguments = if repeated {
                "(alpha {protected )})(alpha)[alpha]2[alpha]{alpha,beta}% comment\r\n{alpha}[alpha]{alpha} text {alpha}"
            } else {
                "[alpha]2[alpha]{alpha,beta}{alpha}{alpha}"
            };
            let source = format!("😀 \\{name}{star}{arguments}");
            let main = insert(&mut workspace, "main.tex", &source);
            let doc = &workspace.documents[&main];
            assert_eq!(
                doc.syntax
                    .citation_references
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                if repeated {
                    vec!["alpha", "beta", "alpha"]
                } else {
                    vec!["alpha", "beta"]
                },
                "{source}"
            );
            for entry in &doc.syntax.citation_references {
                let position = doc.text.position(entry.span.start + 1);
                assert_eq!(
                    features::definitions(&workspace, &main, position).len(),
                    1,
                    "{source}"
                );
                let changes = features::rename(&workspace, &main, position, "renamed")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes[&bib].len(), 1);
                let count = if repeated && entry.name == "alpha" {
                    2
                } else {
                    1
                };
                assert_eq!(changes[&main].len(), count, "{source}");
                assert_eq!(
                    latex_lsp::editing::highlights(&workspace, &main, position).len(),
                    count
                );
                let renamed = apply(&source, &changes[&main]);
                if entry.name == "alpha" {
                    assert!(renamed.contains("[alpha]2[alpha]{renamed,beta}"));
                    assert!(renamed.ends_with(if repeated {
                        "{alpha}[alpha]{renamed} text {alpha}"
                    } else {
                        "{alpha}{alpha}"
                    }));
                } else {
                    assert!(renamed.contains("{alpha,renamed}"));
                }
            }
            assert_eq!(
                latex_lsp::semantic::tokens(doc, None)
                    .unwrap()
                    .data
                    .iter()
                    .filter(|token| token.token_type == 3)
                    .count(),
                if repeated { 3 } else { 2 }
            );
        }
    }
}

#[test]
fn volume_boundaries_accept_bare_ascii_operands_and_stop_before_unproven_arguments() {
    for source in [
        r"\volcite2{alpha}",
        r"\volcite\volume{alpha}",
        "\\volcite% comment\r\n[pre]{2}% comment\r\n[pages]{alpha}",
        r"\volcites2{alpha}[pre]\volume[pages]{beta}",
    ] {
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", source);
        assert_eq!(
            workspace.documents[&main].syntax.citation_references.len(),
            if source.ends_with("{beta}") { 2 } else { 1 },
            "{source}"
        );
    }
    for source in [
        r"\volcite[pre][extra]{2}{alpha}",
        r"\volcite{2}[pages][extra]{alpha}",
        r"\volcite{2}{\dynamic}",
        r"\volcite{unclosed [pages]{alpha}",
        r"\volcite二{alpha}",
        r"\volcite^^32{alpha}",
        r"\volcite\volume中文{alpha}",
        r"\volcite(global){2}{alpha}",
    ] {
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", source);
        assert!(
            workspace.documents[&main]
                .syntax
                .citation_references
                .is_empty(),
            "{source}"
        );
    }
    for tail in [
        " X{2}{beta}",
        "\\relax{2}{beta}",
        "\n\n{2}{beta}",
        "\r\n \r\n[pre]{2}{beta}",
        " 2{beta}",
    ] {
        let mut workspace = Workspace::default();
        let source = format!("\\volcites{{1}}{{alpha}}{tail}");
        let main = insert(&mut workspace, "main.tex", &source);
        assert_eq!(
            workspace.documents[&main]
                .syntax
                .citation_references
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha"],
            "{source}"
        );
    }
}

#[test]
fn volume_key_completion_preserves_other_arguments_and_never_suggests_keys_in_notes_or_volumes() {
    for marked in [
        r"\volcite[pre]{2}[pages]{al¦pha,beta}",
        r"\volcites(overall){1}{beta}[pre]{2}[pages]{al¦",
        "😀 \\volcites{1}{beta}% comment\r\n[pre]2[pages]{beta, al¦pha}",
    ] {
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
    for marked in [
        r"\volcite[al¦pha]{2}{beta}",
        r"\volcite{al¦pha}{beta}",
        r"\volcite{2}[al¦pha]{beta}",
        r"\volcites{1}{beta}[pre]{al¦pha}{beta}",
        r"\volcites{1}{beta}\relax{al¦pha}",
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        insert(&mut workspace, "refs.bib", "@book{alpha,title={Alpha}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let position = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        assert!(
            !features::completion(&workspace, &main, position)
                .iter()
                .any(|item| item.label == "alpha"),
            "{marked}"
        );
        assert!(
            features::definitions(&workspace, &main, position).is_empty(),
            "{marked}"
        );
    }
}

#[test]
fn volume_signatures_track_each_argument_and_wait_for_missing_required_fields() {
    for (marked, active, suffix) in [
        (
            r"\volcite[pr¦e]{2}[12]{alpha}",
            0,
            "[prenote]{volume}[pages]{keys}",
        ),
        (
            r"\volcite[pre]{¦2}[12]{alpha}",
            1,
            "[prenote]{volume}[pages]{keys}",
        ),
        (
            r"\volcite[pre]{2}[1¦2]{alpha}",
            2,
            "[prenote]{volume}[pages]{keys}",
        ),
        (
            r"\volcite[pre]{2}[12]{al¦pha}",
            3,
            "[prenote]{volume}[pages]{keys}",
        ),
        (
            r"\volcites(ov¦erall)(post){1}{alpha}[pre]{2}[12]{beta}",
            0,
            "[prenote 2]{volume 2}[pages 2]{keys 2}",
        ),
        (
            r"\volcites(overall)(po¦st){1}{alpha}[pre]{2}[12]{beta}",
            1,
            "[prenote 2]{volume 2}[pages 2]{keys 2}",
        ),
        (
            r"\volcites{1}{alpha}[pr¦e]{2}[12]{beta}",
            6,
            "[prenote 2]{volume 2}[pages 2]{keys 2}",
        ),
        (
            r"\volcites{1}{alpha}[pre]{¦2}[12]{beta}",
            7,
            "[prenote 2]{volume 2}[pages 2]{keys 2}",
        ),
        (
            r"\volcites{1}{alpha}[pre]{2}[1¦2]{beta}",
            8,
            "[prenote 2]{volume 2}[pages 2]{keys 2}",
        ),
        (
            r"\volcites{1}{alpha}[pre]{2}[12]{be¦ta}",
            9,
            "[prenote 2]{volume 2}[pages 2]{keys 2}",
        ),
        (r"\volcite¦", 1, "[prenote]{volume}[pages]{keys}"),
        (r"\volcite*[pre] ¦", 1, "[prenote]{volume}[pages]{keys}"),
        (r"\volcite{2}[pages]¦", 3, "[prenote]{volume}[pages]{keys}"),
        (
            r"\volcites{1}{alpha}[pre]¦",
            7,
            "[prenote 2]{volume 2}[pages 2]{keys 2}",
        ),
        (
            r"\volcites(overall)¦",
            3,
            "[prenote 1]{volume 1}[pages 1]{keys 1}",
        ),
        (
            r"\parencites[pre]¦",
            4,
            "[note / prenote 1][postnote 1]{keys 1}",
        ),
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", &source);
        let position = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        let help = latex_lsp::signature::help(&workspace, &main, position)
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.active_parameter, Some(active), "{marked}: {help:?}");
        assert!(
            help.signatures[0].label.ends_with(suffix),
            "{marked}: {help:?}"
        );
    }
    for marked in [
        r"\volcite{2}{alpha} te¦xt",
        r"\volcites{1}{alpha}2{be¦ta}",
        "\\volcite[pre]\n\n{al¦pha}",
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", &source);
        let position = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        assert!(
            latex_lsp::signature::help(&workspace, &main, position).is_none(),
            "{marked}"
        );
    }
}

#[test]
fn volume_command_snippets_supply_volume_and_keys_and_preserve_existing_calls_and_overrides() {
    for (name, _) in commands() {
        for following in ["", "{2}{alpha}", "[pre]{2}[12]{alpha}", "2{alpha}"] {
            let mut workspace = Workspace {
                snippets: true,
                ..Workspace::default()
            };
            let source = format!("\\{name}{following}");
            let main = insert(&mut workspace, "main.tex", &source);
            let items = features::completion(&workspace, &main, Position::new(0, 3));
            let item = items
                .iter()
                .find(|item| item.label == name)
                .unwrap_or_else(|| panic!("{name}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                edit.new_text,
                if following.is_empty() {
                    format!("{name}{{${{1:volume}}}}{{${{2:keys}}}}")
                } else {
                    name.clone()
                },
                "{source}"
            );
        }
    }
    let mut workspace = Workspace::default();
    let source = r"\newcommand\volcite[1]{}\volcite{custom}";
    let main = insert(&mut workspace, "main.tex", source);
    let position = workspace.documents[&main]
        .text
        .position(source.find("custom").unwrap() + 2);
    assert_eq!(
        latex_lsp::signature::help(&workspace, &main, position)
            .unwrap()
            .signatures[0]
            .label,
        r"\volcite{#1}"
    );
}

#[test]
fn volume_citation_warnings_and_spelling_fixes_target_keys_across_the_lookahead_line() {
    for (name, repeated) in commands() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = format!("\\addbibresource{{refs.bib}}\n\\{name}[alphaa]{{alphaa}}[alphaa]{{alphaa}}{}\n\\printbibliography\n", if repeated { "\n[alphaa]{alphaa}[alphaa]{alphaa}" } else { "" });
        std::fs::write(&main, &source).unwrap();
        let bib = directory.path().join("refs.bib");
        std::fs::write(&bib, "@book{alpha,title={Alpha}}").unwrap();
        let line = if repeated { 4 } else { 3 };
        let diagnostics = latex_lsp::build_log::parse(
            &main,
            &format!(
                "LaTeX Warning: Citation 'alphaa' on page 1 undefined on input line {line}.\n"
            ),
        );
        let uri = Url::from_file_path(&main).unwrap();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Workspace::default()
        };
        workspace.open(uri.clone(), source.clone(), 6);
        workspace.open(
            Url::from_file_path(&bib).unwrap(),
            "@book{alpha,title={Alpha}}".into(),
            6,
        );
        workspace
            .build_diagnostics
            .insert(uri.clone(), diagnostics.clone());
        assert_eq!(
            diagnostics[&uri].len(),
            if repeated { 2 } else { 1 },
            "{source}: {diagnostics:?}"
        );
        let text = Text::new(source.clone());
        for diagnostic in &diagnostics[&uri] {
            let start = text.offset(diagnostic.range.start).unwrap();
            assert_eq!(&source[start - 9..start], "[alphaa]{", "{diagnostic:?}");
            let actions = latex_lsp::editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic.clone()],
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
            assert_eq!(edit.range, diagnostic.range);
            let mut expected = source.clone();
            expected.replace_range(start..start + 6, "alpha");
            assert_eq!(apply(&source, std::slice::from_ref(edit)), expected);
        }
    }
}

#[test]
fn volume_citation_renames_update_all_shared_projects_and_reject_ambiguous_shared_calls() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b", "other"] {
        let database = if name == "other" { "other" } else { "shared" };
        std::fs::write(root.join(format!("{name}.tex")), format!("\\documentclass{{article}}\\addbibresource{{{database}.bib}}\\volcites{{1}}{{alpha}}{{2}}{{alpha}}" )).unwrap();
    }
    for name in ["shared", "other"] {
        std::fs::write(
            root.join(format!("{name}.bib")),
            "@book{alpha,title={Alpha}}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let bib = Url::from_file_path(root.join("shared.bib")).unwrap();
    let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    assert_eq!(changes[&bib].len(), 1);
    for name in ["a.tex", "b.tex"] {
        let uri = Url::from_file_path(root.join(name)).unwrap();
        assert_eq!(changes[&uri].len(), 2);
    }
    assert!(!changes.keys().any(|uri| uri.path().contains("other")));
    std::fs::write(root.join("common.tex"), r"\volcite{2}{alpha}").unwrap();
    for name in ["a", "other"] {
        std::fs::write(
            root.join(format!("{name}.tex")),
            format!(
                "\\documentclass{{article}}\\addbibresource{{{}.bib}}\\input{{common}}",
                if name == "a" { "shared" } else { "other" }
            ),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let shared = Url::from_file_path(root.join("common.tex")).unwrap();
    assert!(features::prepare_rename(&workspace, &shared, Position::new(0, 14)).is_err());
    assert!(features::rename(&workspace, &shared, Position::new(0, 14), "renamed").is_err());
}

#[test]
fn volume_signature_aliases_keep_their_reader_and_independent_volume_macro_navigation() {
    let mut workspace = Workspace::default();
    let source = r"\newcommand\volume{2}\newcommand\alias{\volcite}\alias[pre]\volume[12]{alpha}";
    let main = insert(&mut workspace, "main.tex", source);
    let position = workspace.documents[&main]
        .text
        .position(source.rfind("alpha").unwrap() + 2);
    let help = latex_lsp::signature::help(&workspace, &main, position).unwrap();
    assert_eq!(
        help.signatures[0].label,
        r"\alias[prenote]{volume}[pages]{keys}"
    );
    assert_eq!(help.active_parameter, Some(3));
    let source = r"\newcommand\volume{2}\volcite\volume{alpha}";
    let main = insert(&mut workspace, "main.tex", source);
    let position = workspace.documents[&main]
        .text
        .position(source.rfind(r"\volume").unwrap() + 3);
    let definitions = features::definitions(&workspace, &main, position);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].range.start.character, 11);
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_volume_citations_all_variants_resolve_after_a_bibliography_key_rename() {
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("refs.bib");
    let mut source = "\\documentclass{article}\n\\usepackage[backend=bibtex,style=authoryear]{biblatex}\n\\addbibresource{refs.bib}\n\\newcommand\\volume{2}\n\\begin{document}\n".to_string();
    for (name, repeated) in commands() {
        for star in ["", "*"] {
            let arguments = if repeated {
                "(overall)(end)[see]2[12]{alpha}[compare]{3}[34]{beta}"
            } else {
                "[see]\\volume[12]{alpha}"
            };
            source.push_str(&format!("\\{name}{star}{arguments}\\relax\\par\n"));
        }
    }
    source.push_str("\\printbibliography\n\\end{document}\n");
    let bibliography = "@book{alpha,author={Alpha, Alice},title={First},year={2020}}\n@book{beta,author={Beta, Bob},title={Second},year={2021}}\n";
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
    assert_eq!(citations.len(), 84, "{citations:?}");
    let text = Text::new(source.clone());
    for diagnostic in citations {
        let key = &source[text.offset(diagnostic.range.start).unwrap()
            ..text.offset(diagnostic.range.end).unwrap()];
        assert!(matches!(key, "alpha" | "beta"), "{diagnostic:?}");
    }
    let mut workspace = Workspace::default();
    workspace.open(uri.clone(), source.clone(), 6);
    let bib_uri = Url::from_file_path(&bib).unwrap();
    workspace.open(bib_uri.clone(), bibliography.into(), 6);
    let changes = features::rename(&workspace, &bib_uri, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&uri].len(), 56);
    fs::write(&main, apply(&source, &changes[&uri])).unwrap();
    fs::write(&bib, apply(bibliography, &changes[&bib_uri])).unwrap();
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
