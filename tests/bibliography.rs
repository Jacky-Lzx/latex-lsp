use lsp_types::*;
use texquill::{
    editing,
    language::features,
    project::workspace::{Document, Workspace},
    text::Text,
};

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///bibliography/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    uri
}

fn position(source: &str, needle: &str) -> Position {
    Text::new(source.into()).position(source.find(needle).unwrap() + needle.len())
}

fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
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

const NATBIB_VARIANTS: &[(&str, &str)] = &[
    (
        "citealt",
        " % before star\r\n* % after star\r\n[parent][p.~2]",
    ),
    ("citealp", "*[parent][p.~2]"),
    ("citenum", ""),
    ("citeyearpar", "[parent][p.~2]"),
    ("citefullauthor", "[parent][p.~2]"),
    ("Citet", "*[parent][p.~2]"),
    ("Citep", "*[parent][p.~2]"),
    ("Citealt", "*[parent][p.~2]"),
    ("Citealp", "*[parent][p.~2]"),
    ("Citeauthor", "*[parent][p.~2]"),
    ("citetalias", "[parent][p.~2]"),
    ("citepalias", "[parent][p.~2]"),
];

#[test]
fn citation_note_boundaries_keep_prose_out_of_key_edits() {
    for command in [
        "citep",
        "citep*",
        "Citealt*",
        "citetalias",
        "fullcite",
        "footfullcite",
    ] {
        let mut workspace = Workspace::default();
        let bibliography = "@book{parent,title={Literal parent}}\n";
        let bib = insert(&mut workspace, "refs.bib", bibliography);
        let valid = [
            "",
            "[parent]",
            "[see {nested [parent]}][p.~2]",
            " % keep\r\n[parent] % keep\r\n[p.~2]",
            "[see [parent]",
            "[see [parent][p.~2]",
            "[see {parent ] [ text}][p.~2]",
        ];
        let invalid = [
            "[see][p.~2][surplus]",
            "[see][p.~2][third][fourth]",
            " % keep\r\n[parent] % keep\r\n[p.~2] % keep\r\n[surplus]",
            "[see [parent]]",
            "[see [parent]][p.~2]",
            "[see][p.~[2]]",
        ];
        let make_source = |key: &str| {
            valid
                .iter()
                .map(|notes| format!("😀 \\{command}{notes}{{{key}}}\r\n"))
                .chain(
                    invalid
                        .iter()
                        .map(|notes| format!("😀 \\{command}{notes}{{parent}}\r\n")),
                )
                .collect::<String>()
        };
        let source = make_source("parent");
        let main = insert(&mut workspace, "main.tex", &source);
        for notes in valid {
            let at = position(&source, &format!("\\{command}{notes}{{par"));
            assert_eq!(
                features::definitions(&workspace, &main, at).len(),
                1,
                "{command}{notes}"
            );
            assert_eq!(
                features::references(&workspace, &main, at, false).len(),
                valid.len(),
                "{command}{notes}"
            );
            assert!(
                features::completion(&workspace, &main, at)
                    .iter()
                    .any(|item| item.label == "parent"),
                "{command}{notes}"
            );
        }
        for notes in invalid {
            let at = position(&source, &format!("\\{command}{notes}{{par"));
            assert!(
                features::definitions(&workspace, &main, at).is_empty(),
                "{command}{notes}"
            );
            assert!(features::hover(&workspace, &main, at).is_none());
            assert!(features::prepare_rename(&workspace, &main, at)
                .unwrap()
                .is_none());
            assert!(features::rename(&workspace, &main, at, "renamed")
                .unwrap()
                .is_none());
            assert!(texquill::language::signature::help(&workspace, &main, at).is_none());
            assert!(!features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == "parent"));
        }
        let at = position(&source, &format!("\\{command}{{par"));
        let changes = features::rename(&workspace, &main, at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            apply(&source, changes[&main].clone()),
            make_source("renamed")
        );
        assert_eq!(
            apply(bibliography, changes[&bib].clone()),
            bibliography.replacen("{parent,", "{renamed,", 1)
        );
        let incomplete = format!("\\{command}[see][p.~2][surplus]{{par");
        let main = insert(&mut workspace, "main.tex", &incomplete);
        assert!(!features::completion(
            &workspace,
            &main,
            Text::new(incomplete.clone()).position(incomplete.len())
        )
        .iter()
        .any(|item| item.label == "parent"));
    }
}

#[test]
fn excess_citation_notes_do_not_steal_another_calls_warning() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "😀 \\citep[see][p.~2][surplus]{paretn} \\citep[see [nested]]{paretn} \\citep[see [literal][p.~2]{paretn}\n";
    std::fs::write(&main, source).unwrap();
    let log = "(./main.tex\nPackage natbib Warning: Citation `paretn' on page 1 undefined on input line 1.\n)\n";
    let diagnostics = texquill::compilation::build_log::parse(&main, log);
    let items = &diagnostics[&uri];
    assert_eq!(items.len(), 1, "{items:?}");
    let start = source.rfind("{paretn}").unwrap() + 1;
    assert_eq!(
        items[0].range,
        Text::new(source.into()).range(start, start + "paretn".len())
    );
}

#[test]
#[ignore = "requires pdflatex and natbib"]
fn real_natbib_note_boundaries_preserve_prose_when_valid_keys_are_renamed() {
    for (notes, actual_key) in [
        ("[see][p.~2][surplus]", "["),
        ("[see [nested]]", "]"),
        ("[see [nested]][p.~2]", "]"),
        ("[see][p.~[2]]", "]"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let child = root.join("chapter.tex");
        let main_uri = Url::from_file_path(&main).unwrap();
        let child_uri = Url::from_file_path(&child).unwrap();
        let main_source = "\\documentclass{article}\\usepackage{natbib}\\begin{document}\n\\input{chapter}\n\\begin{thebibliography}{1}\n\\bibitem[Author(2026)]{parent} Entry parent.\n\\end{thebibliography}\\end{document}\n";
        let make_child =
            |key: &str| format!("\\citep{notes}{{parent}}\n\\citep[see [literal][p.~2]{{{key}}}\n");
        std::fs::write(&main, main_source).unwrap();
        std::fs::write(&child, make_child("parent")).unwrap();
        let config = texquill::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let compile = || {
            let outcome = texquill::compilation::build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            outcome
        };
        compile();
        let initial = compile();
        let aux = std::fs::read_to_string(initial.pdf.unwrap().with_extension("aux")).unwrap();
        assert!(
            aux.contains(&format!("\\citation{{{actual_key}}}"))
                && aux.contains("\\citation{parent}"),
            "{aux}"
        );
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        workspace.reload(&main_uri);
        workspace.reload(&child_uri);
        let at = position(&make_child("parent"), "\\citep[see [literal][p.~2]{par");
        let changes = features::rename(&workspace, &child_uri, at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        for (uri, edits) in changes {
            let path = uri.to_file_path().unwrap();
            let source = std::fs::read_to_string(&path).unwrap();
            std::fs::write(path, apply(&source, edits)).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&child).unwrap(),
            make_child("renamed")
        );
        assert_eq!(
            std::fs::read_to_string(&main).unwrap(),
            main_source.replace("{parent}", "{renamed}")
        );
        compile();
        let renamed = compile();
        let aux = std::fs::read_to_string(renamed.pdf.unwrap().with_extension("aux")).unwrap();
        assert!(
            aux.contains(&format!("\\citation{{{actual_key}}}"))
                && aux.contains("\\citation{renamed}")
                && !aux.contains("\\citation{parent}"),
            "{aux}"
        );
        assert!(
            renamed.output.contains(&format!("Citation `{actual_key}'"))
                && !renamed.output.contains("Citation `renamed'"),
            "{}",
            renamed.output
        );
    }
}

#[test]
fn full_citation_commands_share_keys_with_ordinary_citations_and_preserve_notes() {
    let mut workspace = Workspace::default();
    let bibliography = "@book{parent,title={Literal parent}}\n@book{other,title={Other}}\n";
    let bib = insert(&mut workspace, "refs.bib", bibliography);
    let make_source = |key: &str| {
        let mut source = format!("😀 \\cite{{{key}}}\r\n");
        for command in ["fullcite", "footfullcite"] {
            source.push_str(&format!(
                "\\{command} % keep\r\n[see \\emph{{parent}}][p.~2]{{other, {key}}}\r\n"
            ));
        }
        source.push_str(&format!("\\fullcite*{{{key}}}\\footfullcite*{{{key}}}\n% \\fullcite{{parent}}\n\\verb|\\footfullcite{{parent}}|\n"));
        source
    };
    let source = make_source("parent");
    let main = insert(&mut workspace, "main.tex", &source);
    let at = position(&source, "\\cite{par");
    assert_eq!(features::references(&workspace, &main, at, false).len(), 5);
    for command in ["fullcite", "footfullcite"] {
        let at = position(
            &source,
            &format!("\\{command} % keep\r\n[see \\emph{{parent}}][p.~2]{{other, par"),
        );
        let definitions = features::definitions(&workspace, &main, at);
        assert_eq!(definitions.len(), 1, "{command}: {definitions:?}");
        assert_eq!(definitions[0].uri, bib);
        assert!(features::hover(&workspace, &main, at).is_some());
        assert!(features::completion(&workspace, &main, at)
            .iter()
            .any(|item| item.label == "parent"));
        let starred = position(&source, &format!("\\{command}*{{par"));
        assert_eq!(features::definitions(&workspace, &main, starred).len(), 1);
    }
    let changes = features::rename(&workspace, &main, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        apply(&source, changes[&main].clone()),
        make_source("renamed")
    );
    assert_eq!(
        apply(bibliography, changes[&bib].clone()),
        bibliography.replacen("{parent,", "{renamed,", 1)
    );
}

#[test]
fn full_citation_warnings_select_keys_including_stars_and_preserve_notes() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source =
        "😀 \\fullcite[see paretn][p.~2]{paretn} \\footfullcite{paretn} \\fullcite*{paretn}\n";
    std::fs::write(&main, source).unwrap();
    let log = "(./main.tex\nPackage biblatex Warning: Citation 'paretn' on page 1 undefined on input line 1.\n)\n";
    let diagnostics = texquill::compilation::build_log::parse(&main, log);
    let items = &diagnostics[&uri];
    assert_eq!(items.len(), 3, "{items:?}");
    let text = Text::new(source.into());
    for (item, marker) in items
        .iter()
        .zip(["[p.~2]{", "\\footfullcite{", "\\fullcite*{"])
    {
        let start = source.find(marker).unwrap() + marker.len();
        assert_eq!(item.range, text.range(start, start + "paretn".len()));
    }
}

#[test]
fn natbib_variants_and_single_key_aliases_navigate_complete_and_rename_without_changing_text() {
    let mut workspace = Workspace::default();
    let bibliography = "@book{parent, title={parent}}\n@book{other, title={Other}}\n";
    let bib = insert(&mut workspace, "refs.bib", bibliography);
    let make_source = |key: &str| {
        let mut source = format!("😀 \\defcitealias % keep\r\n{{{key}}}{{Literal parent}}\n");
        for (command, options) in NATBIB_VARIANTS {
            source.push_str(&format!("\\{command}{options}{{other, {key}}}\n"));
        }
        source.push_str("\\defcitealias{parent,other}{Not a list}\\defcitealias{ parent }{Distinct key}\\citetext{parent}\n\\defcitealias*{parent}{Unsupported star}\\citenum*{parent}\\citenum[note]{parent}\n% \\citealt{parent}\n\\verb|\\Citep{parent}|\n");
        source
    };
    let source = make_source("parent");
    let main = insert(&mut workspace, "main.tex", &source);
    let prepared_at = position(&source, "{par");
    assert_eq!(
        features::definitions(&workspace, &main, prepared_at)[0].uri,
        bib
    );
    assert!(features::hover(&workspace, &main, prepared_at).is_some());
    let references = features::references(&workspace, &main, prepared_at, false);
    assert_eq!(references.len(), NATBIB_VARIANTS.len() + 1);
    for (command, options) in NATBIB_VARIANTS {
        let at = position(&source, &format!("\\{command}{options}{{other, par"));
        assert_eq!(
            features::definitions(&workspace, &main, at)[0].uri,
            bib,
            "{command}"
        );
        let completions = features::completion(&workspace, &main, at);
        assert!(
            completions.iter().any(|item| item.label == "parent"),
            "{command}: {completions:?}"
        );
    }
    let changes = features::rename(&workspace, &main, prepared_at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        apply(&source, changes[&main].clone()),
        make_source("renamed")
    );
    assert_eq!(
        apply(bibliography, changes[&bib].clone()),
        bibliography.replacen("{parent,", "{renamed,", 1)
    );
    for marker in [
        "\\defcitealias{par",
        "\\defcitealias{ par",
        "\\citetext{par",
    ] {
        assert!(features::definitions(&workspace, &main, position(&source, marker)).is_empty());
    }
    let incomplete = "\\defcitealias % keep\r\n{parBAD}{Display parent}";
    let main = insert(&mut workspace, "main.tex", incomplete);
    let items = features::completion(&workspace, &main, position(incomplete, "{par"));
    let item = items.iter().find(|item| item.label == "parent").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(
        apply(incomplete, vec![edit.clone()]),
        "\\defcitealias % keep\r\n{parent}{Display parent}"
    );
}

#[test]
fn natbib_alias_setters_do_not_receive_another_calls_missing_citation_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "😀 \\defcitealias{paretn}{Display paretn}\\Citealp{paretn}\\citetalias{paretn}\n";
    std::fs::write(&main, source).unwrap();
    let log = "(./main.tex\nPackage natbib Warning: Citation `paretn' on page 1 undefined on input line 1.\n)\n";
    let diagnostics = texquill::compilation::build_log::parse(&main, log);
    let items = &diagnostics[&uri];
    assert_eq!(items.len(), 2, "{items:?}");
    let text = Text::new(source.into());
    for (item, marker) in items.iter().zip(["\\Citealp{", "\\citetalias{"]) {
        let start = source.find(marker).unwrap() + marker.len();
        assert_eq!(item.range, text.range(start, start + "paretn".len()));
    }
}

#[test]
#[ignore = "requires pdflatex and natbib"]
fn real_natbib_citation_fixes_and_alias_renames_preserve_text_and_compile_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("chapter.tex");
    let main_uri = Url::from_file_path(&main).unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    let main_source = concat!(
        "\\documentclass{article}\\usepackage[authoryear]{natbib}\\begin{document}\n",
        "\\defcitealias{parent}{Display parent}\\defcitealias{paretn}{Unused typo alias}\\defcitealias{other}{Other alias}\n",
        "\\input{chapter}\n\\begin{thebibliography}{1}\n",
        "\\bibitem[Author(2026)]{parent} Entry parent.\n",
        "\\bibitem[Other(2025)]{other} Other entry.\n",
        "\\end{thebibliography}\\end{document}\n",
    );
    let make_child = |key: &str| {
        NATBIB_VARIANTS
            .iter()
            .map(|(command, options)| format!("\\{command}{options}{{other, {key}}}\\par\n"))
            .collect::<String>()
    };
    std::fs::write(&main, main_source).unwrap();
    std::fs::write(&child, make_child("paretn")).unwrap();
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let outcome = texquill::compilation::build::run(&main, &config, &Default::default());
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        outcome
    };
    compile();
    let outcome = compile();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace
        .project
        .build_diagnostics
        .insert(main_uri.clone(), outcome.diagnostics.clone());
    let warnings: Vec<_> = outcome.diagnostics[&child_uri]
        .iter()
        .filter(|item| item.message.contains("Citation `paretn'"))
        .collect();
    assert_eq!(
        warnings.len(),
        NATBIB_VARIANTS.len(),
        "{:?}",
        outcome.diagnostics
    );
    let mut edits = Vec::new();
    for warning in warnings {
        let actions = editing::code_actions(
            &workspace,
            &child_uri,
            warning.range,
            &CodeActionContext {
                diagnostics: vec![warning.clone()],
                ..Default::default()
            },
        );
        assert_eq!(actions.len(), 1, "{actions:?}");
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!()
        };
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&child_uri].len(), 1);
        assert_eq!(changes[&child_uri][0].range, warning.range);
        assert_eq!(changes[&child_uri][0].new_text, "parent");
        edits.extend_from_slice(&changes[&child_uri]);
    }
    let fixed = apply(&make_child("paretn"), edits);
    assert_eq!(fixed, make_child("parent"));
    std::fs::write(&child, &fixed).unwrap();
    compile();
    assert!(compile().diagnostics.values().all(Vec::is_empty));
    workspace.reload(&child_uri);
    let at = position(main_source, "\\defcitealias{par");
    let changes = features::rename(&workspace, &main_uri, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    for (uri, edits) in changes {
        let path = uri.to_file_path().unwrap();
        let source = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, apply(&source, edits)).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&child).unwrap(),
        make_child("renamed")
    );
    assert_eq!(
        std::fs::read_to_string(&main).unwrap(),
        main_source
            .replace("\\defcitealias{parent}", "\\defcitealias{renamed}")
            .replace(
                "\\bibitem[Author(2026)]{parent}",
                "\\bibitem[Author(2026)]{renamed}"
            )
    );
    compile();
    assert!(compile().diagnostics.values().all(Vec::is_empty));
}

#[test]
fn bibliography_relations_navigate_highlight_and_rename_without_changing_literal_text() {
    let mut workspace = Workspace::default();
    let parents = "@book{parent, title={Parent}}\n@xdata{common, year={2026}}\n";
    let parent = insert(&mut workspace, "parents.bib", parents);
    let source = "@incollection{child,\n  crossref = {parent},\n  XREF = \"parent\",\n  xdata = {common, parent},\n  related = \"other, parent\",\n  entryset = {parent, child},\n  title = {parent, crossref={parent}},\n  note = \"crossref = {parent}\"\n}\n% crossref={parent}\n@comment{crossref={parent}}\n@string{crossref = \"parent\"}\n@preamble{\"crossref={parent}\"}\n";
    let child = insert(&mut workspace, "children.bib", source);
    let main = insert(&mut workspace, "main.tex", "\\cite{parent}\\nocite{*}");
    let cursor = position(source, "crossref = {par");
    let definitions = features::definitions(&workspace, &child, cursor);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, parent);
    let references = features::references(&workspace, &child, cursor, false);
    assert_eq!(references.len(), 6);
    assert!(features::hover(&workspace, &child, cursor).is_some());
    assert_eq!(editing::highlights(&workspace, &child, cursor).len(), 5);
    let edits = features::rename(&workspace, &child, cursor, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&child].len(), 5);
    assert_eq!(
        apply(parents, edits[&parent].clone()),
        parents.replacen("{parent,", "{renamed,", 1)
    );
    assert_eq!(
        apply(
            &workspace.documents[&main].text.source,
            edits[&main].clone()
        ),
        "\\cite{renamed}\\nocite{*}"
    );
    let updated = apply(source, edits[&child].clone());
    assert!(updated.contains("crossref = {renamed}"));
    assert!(updated.contains("xdata = {common, renamed}"));
    assert!(updated.contains("title = {parent, crossref={parent}}"));
    assert!(updated.contains("note = \"crossref = {parent}\""));
    assert!(updated.contains("@comment{crossref={parent}}"));
    assert!(features::rename(&workspace, &child, cursor, "*").is_err());
}

#[test]
fn multiline_citation_completion_preserves_notes_other_keys_and_crlf() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "refs.bib",
        "@book{parent,title={Parent}}\n@book{other,title={Other}}\n",
    );
    for (command, notes) in [
        ("cite", ""),
        ("citep*", "[see][p.~2]"),
        ("textcite", "[see]"),
        ("fullcite", "[see][p.~2]"),
        ("footfullcite", ""),
        ("nocite", ""),
    ] {
        for (arguments, expected) in [
            ("{other,\n par¦BAD,last}", "{other,\n parent,last}"),
            ("{\r\npar¦BAD,other}", "{\r\nparent,other}"),
            ("{other,\r\n par¦", "{other,\r\n parent"),
            ("{other,\r\n ¦}", "{other,\r\n parent}"),
        ] {
            let marked = format!("😀 \\{command}{notes}{arguments}");
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let uri = insert(&mut workspace, "main.tex", &source);
            let items =
                features::completion(&workspace, &uri, Text::new(source.clone()).position(offset));
            let item = items
                .iter()
                .find(|item| item.label == "parent")
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.range.start.line, edit.range.end.line);
            assert_eq!(
                apply(&source, vec![edit.clone()]),
                format!("😀 \\{command}{notes}{expected}")
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_multiline_citation_completion_and_bibitem_rename_compile_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let refs = directory.path().join("refs.tex");
    let main_uri = Url::from_file_path(&main).unwrap();
    let refs_uri = Url::from_file_path(&refs).unwrap();
    let source = "\\documentclass{article}\n\\begin{document}\nSee \\cite{alpha,\n betBAD}.\n\\input{refs}\n\\end{document}\n";
    let bibliography = "\\begin{thebibliography}{9}\n\\bibitem{alpha} First.\n\\bibitem{beta} Second.\n\\end{thebibliography}\n";
    std::fs::write(&main, source).unwrap();
    std::fs::write(&refs, bibliography).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(main_uri.clone(), source.into(), 7);
    workspace.open(refs_uri.clone(), bibliography.into(), 11);
    let items = features::completion(&workspace, &main_uri, position(source, "\n bet"));
    let item = items.iter().find(|item| item.label == "beta").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    let fixed = apply(source, vec![edit.clone()]);
    assert_eq!(fixed, source.replace("betBAD", "beta"));
    std::fs::write(&main, &fixed).unwrap();
    workspace.open(main_uri.clone(), fixed.clone(), 8);
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let result = texquill::compilation::build::run(&main, &config, &Default::default());
        assert_eq!(result.status, "success", "{}", result.output);
        assert!(result.pdf.as_ref().is_some_and(|path| path.is_file()));
        result
    };
    let compile_twice = || {
        compile();
        let result = compile();
        assert!(
            result.diagnostics.values().all(Vec::is_empty),
            "{:?}",
            result.diagnostics
        );
        result
    };
    compile_twice();
    let changes = features::rename(
        &workspace,
        &main_uri,
        position(&fixed, "\n beta"),
        "beta-renamed",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(changes.len(), 2);
    let renamed = apply(&fixed, changes[&main_uri].clone());
    let renamed_bibliography = apply(bibliography, changes[&refs_uri].clone());
    assert_eq!(renamed, fixed.replace("beta", "beta-renamed"));
    assert_eq!(
        renamed_bibliography,
        bibliography.replace("{beta}", "{beta-renamed}")
    );
    std::fs::write(&main, renamed).unwrap();
    std::fs::write(&refs, renamed_bibliography).unwrap();
    let result = compile_twice();
    let aux = std::fs::read_to_string(result.pdf.unwrap().with_extension("aux")).unwrap();
    assert!(aux.contains("\\bibcite{beta-renamed}"));
    assert!(!aux.contains("betBAD"));
}

#[test]
fn bibliography_completion_replaces_only_the_selected_key_and_supports_incomplete_values() {
    for (source, needle, expected) in [
        (
            "@article{child, crossref={pareBAD}}",
            "crossref={pare",
            "@article{child, crossref={parent}}",
        ),
        (
            "@article{child, related={other, pareBAD, last}}",
            "related={other, pare",
            "@article{child, related={other, parent, last}}",
        ),
        (
            "@article{child, xdata=\"other, pareBAD, last\"}",
            "xdata=\"other, pare",
            "@article{child, xdata=\"other, parent, last\"}",
        ),
        (
            "@article{child, crossref={pare",
            "crossref={pare",
            "@article{child, crossref={parent",
        ),
        (
            "@article{child, crossref=\"pare",
            "crossref=\"pare",
            "@article{child, crossref=\"parent",
        ),
        (
            "@article{child, crossref={",
            "crossref={",
            "@article{child, crossref={parent",
        ),
    ] {
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "parents.bib",
            if needle.starts_with("xdata=") {
                "@xdata{parent, title={Parent}}"
            } else {
                "@book{parent, title={Parent}}"
            },
        );
        let uri = insert(&mut workspace, "children.bib", source);
        let completions = features::completion(&workspace, &uri, position(source, needle));
        let item = completions
            .iter()
            .find(|item| item.label == "parent")
            .unwrap_or_else(|| panic!("missing completion: {source}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit");
        };
        assert_eq!(apply(source, vec![edit.clone()]), expected);
    }
}

#[test]
fn bibliography_string_expressions_escapes_comments_and_other_fields_are_not_key_references() {
    let mut workspace = Workspace::default();
    let source = "@article(child,\n title={crossref={parent}},\n crossref=prefix # {parent},\n xref=parent,\n xdata={parent\\macro},\n related={{parent}},\n note={xdata={parent}},\n % } crossref={parent}\n entryset={parent, other}\n)\n";
    let uri = insert(&mut workspace, "refs.bib", source);
    assert_eq!(
        workspace.documents[&uri]
            .syntax
            .citation_references
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["parent", "other"]
    );
    assert!(features::completion(&workspace, &uri, position(source, "xref=par")).is_empty());
    assert!(
        features::definitions(&workspace, &uri, position(source, "title={crossref={par"))
            .is_empty()
    );
    assert_eq!(workspace.documents[&uri].syntax.citations.len(), 1);
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn real_bibtex_crossrefs_still_compile_after_parent_entry_rename() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("refs.bib");
    let bib_uri = Url::from_file_path(&bib).unwrap();
    let source = "@inproceedings{child, author={A. Author}, title={Child work}, crossref={parent}}\n@proceedings{parent, title={Collected works}, booktitle={Collected works}, editor={E. Editor}, year={2026}}\n";
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{child}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
    std::fs::write(&bib, source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let edits = features::rename(
        &workspace,
        &bib_uri,
        position(source, "crossref={par"),
        "renamedParent",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    std::fs::write(&bib, apply(source, edits[&bib_uri].clone())).unwrap();
    let outcome = texquill::compilation::build::run(
        &main,
        &texquill::config::BuildConfig::default(),
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(
        !outcome
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("crossref")),
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        std::fs::read_to_string(directory.path().join("build/texquill/main.bbl"))
            .unwrap()
            .contains("Collected works")
    );
}
