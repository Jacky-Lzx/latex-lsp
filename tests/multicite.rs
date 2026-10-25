use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///multicite/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(4)));
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
fn repeated_citation_groups_join_navigation_references_and_bibliography_renames() {
    let mut workspace = Workspace::default();
    let bibliography = "@book{alpha,title={Literal alpha}}\n@book{beta,title={Beta}}\n";
    let bib = insert(&mut workspace, "refs.bib", bibliography);
    let source =
        "😀 \\parencites{alpha}{beta}{alpha}\n% \\parencites{alpha}\n\\verb|\\parencites{alpha}|\n";
    let main = insert(&mut workspace, "main.tex", source);
    for key in ["alpha", "beta"] {
        let at = workspace.documents[&main]
            .text
            .position(source.find(&format!("{{{key}}}")).unwrap() + 2);
        assert_eq!(
            features::definitions(&workspace, &main, at).len(),
            1,
            "{key}"
        );
        assert!(features::hover(&workspace, &main, at).is_some());
        assert_eq!(
            features::references(&workspace, &main, at, true).len(),
            if key == "alpha" { 3 } else { 2 }
        );
    }
    let position = workspace.documents[&bib]
        .text
        .position(bibliography.find("alpha").unwrap() + 1);
    let changes = features::rename(&workspace, &bib, position, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(apply(source,&changes[&main]), "😀 \\parencites{renamed}{beta}{renamed}\n% \\parencites{alpha}\n\\verb|\\parencites{alpha}|\n");
    assert_eq!(
        apply(bibliography, &changes[&bib]),
        "@book{renamed,title={Literal alpha}}\n@book{beta,title={Beta}}\n"
    );
}

const COMMANDS: &[&str] = &[
    "cites",
    "Cites",
    "parencites",
    "Parencites",
    "footcites",
    "Footcites",
    "footcitetexts",
    "Footcitetexts",
    "smartcites",
    "Smartcites",
    "supercites",
    "textcites",
    "Textcites",
    "autocites",
    "Autocites",
];

#[test]
fn every_multicite_variant_indexes_only_keys_and_renames_from_each_group() {
    for name in COMMANDS {
        for star in ["", "*"] {
            let mut workspace = Workspace::default();
            let bib = insert(
                &mut workspace,
                "refs.bib",
                "@book{alpha,title={alpha}}\n@book{beta,title={beta}}",
            );
            let source = format!("😀 \\{name}{star}(alpha {{protected )}})(alpha)[alpha][alpha]{{alpha,beta}}% comment\r\n[alpha]{{alpha}}\\relax{{alpha}}");
            let main = insert(&mut workspace, "main.tex", &source);
            let doc = &workspace.documents[&main];
            assert_eq!(
                doc.syntax
                    .citation_references
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                ["alpha", "beta", "alpha"],
                "{source}"
            );
            for entry in doc
                .syntax
                .citation_references
                .iter()
                .filter(|entry| entry.name == "alpha")
            {
                let position = doc.text.position(entry.span.start + 1);
                let edit = features::rename(&workspace, &main, position, "renamed")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(edit[&main].len(), 2, "{source}");
                assert_eq!(edit[&bib].len(), 1);
                let renamed = apply(&source, &edit[&main]);
                assert!(
                    renamed.contains("(alpha {protected )})(alpha)[alpha][alpha]{renamed,beta}")
                );
                assert!(renamed.ends_with("[alpha]{renamed}\\relax{alpha}"));
                assert_eq!(
                    latex_lsp::editing::highlights(&workspace, &main, position).len(),
                    2
                );
            }
            let tokens = latex_lsp::semantic::tokens(doc, None).unwrap();
            assert_eq!(
                tokens
                    .data
                    .iter()
                    .filter(|token| token.token_type == 3)
                    .count(),
                3,
                "{source}"
            );
        }
    }
}

#[test]
fn multicite_stops_at_paragraphs_ordinary_tokens_and_invalid_note_sequences() {
    for tail in [
        "\n\n{beta}",
        "\r\n \r\n{beta}",
        " text {beta}",
        "\\relax{beta}",
        "[one][two][three]{beta}",
        "(note){beta}",
        "[unclosed {beta}",
    ] {
        let mut workspace = Workspace::default();
        let source = format!("\\parencites{{alpha}}{tail}");
        let main = insert(&mut workspace, "main.tex", &source);
        let references = &workspace.documents[&main].syntax.citation_references;
        assert_eq!(
            references
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha"],
            "{source}"
        );
    }
    for source in [
        r"\parencites(one)(two)(three){alpha}",
        r"\parencites[note][note][extra]{alpha}",
        r"\parencites{\dynamic}",
        r"\Supercites{alpha}",
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
}

#[test]
fn later_and_unclosed_key_groups_complete_without_replacing_notes_or_suffixes() {
    for marked in [
        r"\parencites(global)[note]{beta}[alpha]{al¦pha,beta}",
        r"\parencites{beta}{al¦",
        "😀 \\parencites{beta}% comment\r\n[note]{beta, al¦pha}",
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
        let result = apply(&source, std::slice::from_ref(edit));
        assert_eq!(
            result,
            marked.replace("al¦pha", "alpha").replace("al¦", "alpha"),
            "{marked}"
        );
    }
}

#[test]
fn multicite_signatures_follow_global_notes_and_repeated_local_groups() {
    for (marked, active) in [
        (
            r"\parencites(ov¦erall)(post)[pre][post]{alpha}[note]{beta}",
            0,
        ),
        (
            r"\parencites(overall)(po¦st)[pre][post]{alpha}[note]{beta}",
            1,
        ),
        (
            r"\parencites(overall)(post)[pr¦e][post]{alpha}[note]{beta}",
            2,
        ),
        (
            r"\parencites(overall)(post)[pre][po¦st]{alpha}[note]{beta}",
            3,
        ),
        (
            r"\parencites(overall)(post)[pre][post]{al¦pha}[note]{beta}",
            4,
        ),
        (
            r"\parencites(overall)(post)[pre][post]{alpha}[no¦te]{beta}",
            5,
        ),
        (
            r"\parencites(overall)(post)[pre][post]{alpha}[note]{be¦ta}",
            7,
        ),
        (r"\parencites*{alpha}[one][two]{be¦", 7),
    ] {
        let mut workspace = Workspace::default();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let position = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        let help = latex_lsp::signature::help(&workspace, &main, position)
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.active_parameter, Some(active), "{marked}");
        assert!(help.signatures[0]
            .label
            .ends_with("[note / prenote 2][postnote 2]{keys 2}"));
    }
    for marked in [
        r"\parencites{alpha} te¦xt",
        "\\parencites{alpha}\n\n{be¦ta}",
    ] {
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", &marked.replace('¦', ""));
        let position = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        assert!(
            latex_lsp::signature::help(&workspace, &main, position).is_none(),
            "{marked}"
        );
    }
    let mut workspace = Workspace::default();
    let main = insert(
        &mut workspace,
        "main.tex",
        r"\newcommand\parencites[1]{}\parencites{custom}",
    );
    let position = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("custom")
            .unwrap()
            + 2,
    );
    let help = latex_lsp::signature::help(&workspace, &main, position).unwrap();
    assert_eq!(help.signatures[0].label, r"\parencites{#1}");
}

#[test]
fn multicite_command_snippets_keep_existing_parenthesized_notes() {
    for following in ["", "(overall){alpha}{beta}"] {
        let mut workspace = Workspace {
            snippets: true,
            ..Workspace::default()
        };
        let source = format!("\\parencites{following}");
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(&workspace, &main, Position::new(0, 5));
        let item = items
            .iter()
            .find(|item| item.label == "parencites")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(
            edit.new_text,
            if following.is_empty() {
                "parencites{${1:keys}}"
            } else {
                "parencites"
            }
        );
    }
}

#[test]
fn multicite_warnings_locate_repeated_keys_on_a_shared_engine_line() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "😀 \\parencites(note){typo,known}\n[typo]{typo}\n";
    std::fs::write(&main, source).unwrap();
    let diagnostics = latex_lsp::build_log::parse(
        &main,
        "Package biblatex Warning: Citation 'typo' on page 1 undefined on input line 2.\n",
    );
    let uri = Url::from_file_path(&main).unwrap();
    let text = Text::new(source.into());
    let ranges = &diagnostics[&uri];
    assert_eq!(ranges.len(), 2, "{ranges:?}");
    for diagnostic in ranges {
        let start = text.offset(diagnostic.range.start).unwrap();
        let end = text.offset(diagnostic.range.end).unwrap();
        assert_eq!(&source[start..end], "typo");
        assert_ne!(start, source.find("[typo]").unwrap() + 1);
    }
}

#[test]
fn comment_endlines_do_not_create_paragraphs_and_warning_lookahead_includes_the_next_token() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\parencites{alpha}% ignored endline\n\n{beta}\n\\printbibliography\n";
    std::fs::write(&main, source).unwrap();
    let mut workspace = Workspace::default();
    let uri = insert(&mut workspace, "main.tex", source);
    assert_eq!(
        workspace.documents[&uri].syntax.citation_references.len(),
        2
    );
    let diagnostics = latex_lsp::build_log::parse(
        &main,
        "LaTeX Warning: Citation 'beta' on page 1 undefined on input line 4.\n",
    );
    let uri = Url::from_file_path(&main).unwrap();
    assert_eq!(
        diagnostics[&uri][0].range,
        Range::new(Position::new(2, 1), Position::new(2, 5))
    );
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_biblatex_multicites_resolve_again_after_a_cross_file_key_rename() {
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("refs.bib");
    let source = "\\documentclass{article}\n\\usepackage[backend=bibtex,style=authoryear]{biblatex}\n\\addbibresource{refs.bib}\n\\begin{document}\n\\parencites(overall)(end)[see][12]{alpha}\n[compare][34]{beta}{alpha}\n\\printbibliography\n\\end{document}\n";
    let bibliography = "@book{alpha,author={Alpha, Alice},title={First},year={2020}}\n@book{beta,author={Beta, Bob},title={Second},year={2021}}\n";
    fs::write(&main, source).unwrap();
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
    let diagnostics = latex_lsp::build_log::parse(&main, &log);
    let main_uri = Url::from_file_path(&main).unwrap();
    let text = Text::new(source.into());
    let citations = diagnostics[&main_uri]
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("Citation '"))
        .collect::<Vec<_>>();
    assert_eq!(citations.len(), 3, "{citations:?}");
    for diagnostic in citations {
        let key = &source[text.offset(diagnostic.range.start).unwrap()
            ..text.offset(diagnostic.range.end).unwrap()];
        assert!(matches!(key, "alpha" | "beta"), "{diagnostic:?}");
    }
    let mut workspace = Workspace::default();
    workspace.open(main_uri.clone(), source.into(), 4);
    let bib_uri = Url::from_file_path(&bib).unwrap();
    workspace.open(bib_uri.clone(), bibliography.into(), 4);
    let position = text.position(source.rfind("{alpha}").unwrap() + 2);
    let edits = features::rename(&workspace, &main_uri, position, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    fs::write(&main, apply(source, &edits[&main_uri])).unwrap();
    fs::write(&bib, apply(bibliography, &edits[&bib_uri])).unwrap();
    tex();
    run("bibtex", &["main"]);
    tex();
    tex();
    let log = fs::read_to_string(directory.path().join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
    let aux = fs::read_to_string(directory.path().join("main.aux")).unwrap();
    assert!(aux.contains("\\abx@aux@cite{0}{renamed}"), "{aux}");
    assert!(!aux.contains("\\abx@aux@cite{0}{alpha}"), "{aux}");
    assert!(
        fs::metadata(directory.path().join("main.pdf"))
            .unwrap()
            .len()
            > 1000
    );
}

#[test]
fn multicite_citation_fixes_replace_only_the_confirmed_missing_key() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\addbibresource{refs.bib}\\parencites(overall)[alphaa]{known}[alphaa]{alphaa}\n\\printbibliography\n";
    std::fs::write(&main, source).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let diagnostics = latex_lsp::build_log::parse(
        &main,
        "LaTeX Warning: Citation 'alphaa' on page 1 undefined on input line 2.\n",
    );
    let diagnostic = &diagnostics[&uri][0];
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Workspace::default()
    };
    workspace.open(uri.clone(), source.into(), 4);
    let bib = Url::from_file_path(directory.path().join("refs.bib")).unwrap();
    std::fs::write(bib.to_file_path().unwrap(), "@book{alpha,title={Alpha}}").unwrap();
    workspace.open(bib, "@book{alpha,title={Alpha}}".into(), 4);
    workspace
        .build_diagnostics
        .insert(uri.clone(), diagnostics.clone());
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
            CodeActionOrCommand::CodeAction(action) => action.edit.as_ref()?.changes.as_ref()?
                [&uri]
                .iter()
                .find(|edit| edit.new_text == "alpha"),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "actions={actions:?} diagnostic={diagnostic:?} project={:?}",
                workspace.project_documents(&uri)
            )
        });
    assert_eq!(
        apply(source, std::slice::from_ref(edit)),
        source.replace("{alphaa}", "{alpha}")
    );
}

#[test]
fn shared_bibliography_renames_cover_multicites_without_touching_other_projects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b", "other"] {
        let database = if name == "other" { "other" } else { "shared" };
        std::fs::write(root.join(format!("{name}.tex")), format!("\\documentclass{{article}}\\addbibresource{{{database}.bib}}\\parencites{{alpha}}{{alpha}}" )).unwrap();
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
    assert_eq!(changes.len(), 3, "{changes:?}");
    assert_eq!(changes[&bib].len(), 1);
    for name in ["a.tex", "b.tex"] {
        let uri = Url::from_file_path(root.join(name)).unwrap();
        assert_eq!(changes[&uri].len(), 2);
    }
    assert!(!changes.keys().any(|uri| uri.path().contains("other")));
}
