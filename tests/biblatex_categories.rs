use latex_lsp::{
    language::features,
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, file: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///biblatex-categories/{file}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
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
fn category_memberships_reference_keys_without_renaming_category_names() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{alpha,title={Alpha}}\n@book{beta,title={Beta}}\n",
    );
    let source="% 😀\r\n\\DeclareBibliographyCategory{alpha}\r\n\\addtocategory% boundary\r\n{alpha}{% alpha, ignored\r\n alpha,\r\n beta}\\cite{alpha}\\label{alpha}\r\n\\verb|\\addtocategory{alpha}{alpha}|% \\addtocategory{alpha}{alpha}\r\n";
    let main = insert(&mut workspace, "main.tex", source);
    let start = source.find(" alpha,\r\n").unwrap() + 1;
    let at = workspace.documents[&main].text.position(start + 1);
    assert_eq!(features::definitions(&workspace, &main, at)[0].uri, bib);
    assert!(features::hover(&workspace, &main, at).is_some());
    assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
    assert_eq!(
        latex_lsp::editing::highlights(&workspace, &main, at).len(),
        2
    );
    let tokens = latex_lsp::language::semantic::tokens(&workspace.documents[&main], None).unwrap();
    let (mut line, mut character) = (0, 0);
    let mut keys = Vec::new();
    for token in tokens.data {
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
    assert_eq!(keys.len(), 3, "{keys:?}");
    assert!(keys.contains(&workspace.documents[&main].text.position(start)));
    let rename = features::rename(&workspace, &main, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    let mut expected = source.replacen("\\cite{alpha}", "\\cite{renamed}", 1);
    expected.replace_range(start..start + 5, "renamed");
    assert_eq!(apply(source, &rename[&main]), expected);
    assert_eq!(rename[&bib][0].new_text, "renamed");
    let category = source.find("{alpha}{%").unwrap() + 2;
    let definitions = features::definitions(
        &workspace,
        &main,
        workspace.documents[&main].text.position(category),
    );
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, main);
    let declaration = source.find("{alpha}").unwrap() + 1;
    assert_eq!(
        definitions[0].range,
        workspace.documents[&main]
            .text
            .range(declaration, declaration + 5)
    );
}

#[test]
fn category_membership_completion_and_signatures_distinguish_category_from_keys() {
    for (marked, parameter) in [
        ("\\addtocategory{al¦pha}{beta}", 0),
        ("\\addtocategory{alpha}{beta, al¦pha}", 1),
        ("\\addtocategory{alpha} ¦", 1),
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "refs.bib",
            "@book{alpha,title={Alpha}}\n@book{beta,title={Beta}}\n",
        );
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        let signature = latex_lsp::language::signature::help(&workspace, &main, at).unwrap();
        assert_eq!(
            signature.signatures[0].label,
            "\\addtocategory{category}{keys}"
        );
        assert_eq!(signature.active_parameter, Some(parameter));
        let completion = features::completion(&workspace, &main, at);
        assert_eq!(
            completion.iter().any(|item| item.label == "alpha"),
            parameter == 1 && marked.contains("al¦pha}")
        );
    }
}

#[test]
fn category_key_completion_preserves_other_keys_comments_and_unclosed_lists() {
    for marked in [
        "😀 \\addtocategory{alpha}{beta,% ignored alpha\r\n al¦pha}",
        "😀 \\addtocategory{alpha}{beta,% ignored alpha\r\n al¦",
    ] {
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
        let item = items.iter().find(|item| item.label == "alpha").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(
            apply(&source, std::slice::from_ref(edit)),
            marked.replace("al¦pha", "alpha").replace("al¦", "alpha")
        );
        assert_eq!(
            latex_lsp::language::signature::help(&workspace, &main, at)
                .unwrap()
                .active_parameter,
            Some(1)
        );
    }
}

#[test]
fn unsupported_category_forms_do_not_treat_categories_or_prose_as_keys() {
    for marked in [
        "\\addtocategory*{alpha}{al¦pha}",
        "\\addtocategory[alpha]{alpha}{al¦pha}",
        "\\addtocategory{alpha}[alpha]{al¦pha}",
        "\\addtocategory{al¦pha}{beta}",
        "\\addtocategory{beta}{beta}{al¦pha}",
        "\\addtocategory{beta}{beta} text {al¦pha}",
        "\\addtocategory{beta}{\\dyn¦amic}",
        "\\addtocategory{beta}{al% removed newline\n¦pha}",
        "\\Addtocategory{beta}{al¦pha}",
    ] {
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

#[test]
fn category_commands_offer_snippets_and_respect_custom_signatures() {
    for (name, snippet) in [
        ("addtocategory", "addtocategory{${1:category}}{${2:keys}}"),
        (
            "DeclareBibliographyCategory",
            "DeclareBibliographyCategory{${1:category}}",
        ),
    ] {
        for following in ["", "{alpha}{beta}"] {
            let source = format!("\\{name}{following}");
            let mut workspace = Workspace {
                snippets: true,
                ..Default::default()
            };
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main].text.position(name.len() + 1);
            let items = features::completion(&workspace, &main, at);
            let item = items.iter().find(|item| item.label == name).unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                edit.new_text,
                if following.is_empty() { snippet } else { name }
            );
        }
    }
    for (source, expected) in [
        (
            "\\DeclareBibliographyCategory{alpha}",
            "\\DeclareBibliographyCategory{category}",
        ),
        (
            "\\newcommand\\addtocategory[1]{}\\addtocategory{alpha}",
            "\\addtocategory{#1}",
        ),
    ] {
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", source);
        let at = workspace.documents[&main]
            .text
            .position(source.rfind("alpha").unwrap() + 1);
        assert_eq!(
            latex_lsp::language::signature::help(&workspace, &main, at)
                .unwrap()
                .signatures[0]
                .label,
            expected
        );
    }
}

#[test]
fn category_assignments_do_not_claim_citation_warnings_or_spelling_fixes() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for cite in ["", "\\cite{alphaa}"] {
        let source =
            format!("\\addbibresource{{refs.bib}}\n\\addtocategory{{alphaa}}{{alphaa}}{cite}\n");
        std::fs::write(&main, &source).unwrap();
        std::fs::write(
            directory.path().join("refs.bib"),
            "@book{alpha,title={Alpha}}",
        )
        .unwrap();
        let diagnostics = latex_lsp::compilation::build_log::parse(
            &main,
            "Package biblatex Warning: Citation 'alphaa' on page 1 undefined on input line 2.\n",
        );
        let items = &diagnostics[&uri];
        assert_eq!(items.len(), 1);
        let text = Text::new(source.clone());
        if !cite.is_empty() {
            let start = source.rfind("alphaa").unwrap();
            assert_eq!(items[0].range, text.range(start, start + 6));
        } else {
            assert_eq!(items[0].range.start, Position::new(1, 0));
        }
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
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
        let fixes: Vec<_> = actions
            .iter()
            .filter_map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => {
                    action.edit.as_ref()?.changes.as_ref()?.get(&uri)
                }
                _ => None,
            })
            .flatten()
            .filter(|edit| edit.new_text == "alpha")
            .collect();
        if cite.is_empty() {
            assert!(fixes.is_empty(), "{actions:?}");
        } else {
            assert_eq!(fixes.len(), 1, "{actions:?}");
            assert_eq!(
                apply(&source, &[fixes[0].clone()]),
                source.replace("\\cite{alphaa}", "\\cite{alpha}")
            );
        }
    }
}

#[test]
fn category_key_renames_cover_shared_databases_and_reject_shared_source_ambiguity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("shared.tex"), "\\addtocategory{alpha}{alpha}").unwrap();
    for (project, database, assignment) in [
        ("a", "refs", "\\input{shared}"),
        ("b", "refs", "\\addtocategory{alpha}{alpha}"),
        ("c", "other", "\\addtocategory{alpha}{alpha}"),
    ] {
        std::fs::write(
            root.join(format!("{project}.tex")),
            format!("\\documentclass{{article}}\\addbibresource{{{database}.bib}}{assignment}"),
        )
        .unwrap();
    }
    for name in ["refs", "other"] {
        std::fs::write(
            root.join(format!("{name}.bib")),
            "@book{alpha,title={Alpha}}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let bib = Url::from_file_path(root.join("refs.bib")).unwrap();
    let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    for file in ["refs.bib", "shared.tex", "b.tex"] {
        assert_eq!(
            changes[&Url::from_file_path(root.join(file)).unwrap()].len(),
            1
        );
    }
    std::fs::write(
        root.join("c.tex"),
        "\\documentclass{article}\\addbibresource{other.bib}\\input{shared}",
    )
    .unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(features::rename(&workspace, &bib, Position::new(0, 8), "renamed").is_err());
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_category_key_renames_preserve_filtered_bibliographies_and_auxiliary_assignments() {
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source="\\documentclass{article}\n\\usepackage[backend=bibtex,style=numeric]{biblatex}\n\\addbibresource{refs.bib}\n\\DeclareBibliographyCategory{alpha}\n\\addtocategory{alpha}{alpha}\n\\begin{document}\n\\nocite{*}\\printbibliography[category=alpha]\n\\end{document}\n";
    let data="@book{alpha,author={Alpha, Alice},title={Selected reference},year={2020}}\n@book{beta,author={Beta, Bob},title={Other reference},year={2021}}\n";
    fs::write(root.join("main.tex"), source).unwrap();
    fs::write(root.join("refs.bib"), data).unwrap();
    let compile = || {
        for (program, args) in [
            (
                "pdflatex",
                vec!["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            ),
            ("bibtex", vec!["main"]),
            (
                "pdflatex",
                vec!["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            ),
            (
                "pdflatex",
                vec!["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            ),
        ] {
            let output = Command::new(program)
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{program}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        let log = fs::read_to_string(root.join("main.log")).unwrap();
        assert!(!log.contains("Empty bibliography"), "{log}");
        assert!(!log.contains("undefined"), "{log}");
        assert!(fs::metadata(root.join("main.pdf")).unwrap().len() > 1000);
    };
    compile();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let bib = Url::from_file_path(root.join("refs.bib")).unwrap();
    let at = workspace.documents[&bib]
        .text
        .position(data.find("alpha").unwrap() + 1);
    let rename = features::rename(&workspace, &bib, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    for (uri, edits) in rename {
        let path = uri.to_file_path().unwrap();
        fs::write(&path, apply(&fs::read_to_string(&path).unwrap(), &edits)).unwrap();
    }
    compile();
    let main = fs::read_to_string(root.join("main.tex")).unwrap();
    assert!(main.contains("\\DeclareBibliographyCategory{alpha}"));
    assert!(main.contains("\\addtocategory{alpha}{renamed}"));
    let aux = fs::read_to_string(root.join("main.aux")).unwrap();
    assert!(aux.contains("\\abx@aux@category{alpha}{renamed}"), "{aux}");
    assert!(!aux.contains("\\abx@aux@category{alpha}{alpha}"), "{aux}");
}
