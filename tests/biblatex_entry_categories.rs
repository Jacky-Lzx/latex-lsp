use latex_lsp::{editing, features, semantic, signature, text::Text, workspace::Workspace};
use lsp_types::*;

fn setup(source: &str, bibliography: &str) -> (Workspace, Url, Url) {
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    let main = Url::parse("file:///entry-categories/main.tex").unwrap();
    let bib = Url::parse("file:///entry-categories/refs.bib").unwrap();
    ws.open(bib.clone(), bibliography.into(), 5);
    ws.open(main.clone(), source.into(), 9);
    (ws, main, bib)
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

#[test]
fn entry_category_queries_keep_key_category_and_branch_names_separate() {
    for key in ["alpha", "O\"Connor", "book(old)", "中文😀"] {
        let bibliography = format!("@book{{{key},title={{Literal {key}}}}}");
        let source = format!("% 😀\r\n\\DeclareBibliographyCategory{{{key}}}\r\n\\ifentrycategory{{% before\r\n{key}% after\r\n}}{{{key}}}{{Literal {key} \\cite{{{key}}}}}{{Literal {key}}}\\label{{{key}}}");
        let (ws, main, bib) = setup(&source, &bibliography);
        let doc = &ws.documents[&main];
        let start = source.find(&format!("\r\n{key}% after")).unwrap() + 2;
        let at = doc.text.position(start + 1);
        assert_eq!(features::definitions(&ws, &main, at)[0].uri, bib);
        assert!(features::hover(&ws, &main, at).is_some());
        assert_eq!(features::references(&ws, &main, at, true).len(), 3);
        assert_eq!(editing::highlights(&ws, &main, at).len(), 2);
        assert!(features::prepare_rename(&ws, &main, at).unwrap().is_some());
        let edit = features::rename(&ws, &main, at, "new\"key😀")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edit[&main].len(), 2);
        assert_eq!(
            apply(&source, edit[&main].clone()),
            source
                .replace(&format!("\r\n{key}% after"), "\r\nnew\"key😀% after")
                .replace(&format!("\\cite{{{key}}}"), "\\cite{new\"key😀}")
        );
        assert_eq!(
            apply(&bibliography, edit[&bib].clone()),
            format!("@book{{new\"key😀,title={{Literal {key}}}}}")
        );
        let mut selection = editing::selection_ranges(doc, &[at]).unwrap().remove(0);
        let call = source.find("\\ifentrycategory").unwrap()..source.find("\\label").unwrap();
        let mut ranges = vec![];
        loop {
            ranges.push(selection.range);
            let Some(parent) = selection.parent else {
                break;
            };
            selection = *parent;
        }
        assert!(ranges.contains(&doc.text.range(start, start + key.len())));
        assert!(ranges.contains(&doc.text.range(call.start, call.end)));
        let (mut line, mut column) = (0, 0);
        assert!(semantic::tokens(doc, None)
            .unwrap()
            .data
            .iter()
            .any(|token| {
                column = if token.delta_line == 0 {
                    column + token.delta_start
                } else {
                    token.delta_start
                };
                line += token.delta_line;
                token.token_type == 3
                    && Position::new(line, column) == doc.text.position(start)
                    && token.length == key.encode_utf16().count() as u32
            }));
    }
}

#[test]
fn entry_category_queries_complete_only_the_key_and_hint_all_four_roles() {
    for (tail, active) in [
        ("¦", 0),
        ("{al¦pha}{selected}{yes}{no}", 0),
        ("{alpha} ¦", 1),
        ("{alpha}{sel¦ected}{yes}{no}", 1),
        ("{alpha}{selected} ¦", 2),
        ("{alpha}{selected}{y¦es}{no}", 2),
        ("{alpha}{selected}{yes} ¦", 3),
        ("{alpha}{selected}{yes}{n¦o}", 3),
        ("{alpha}{selected}{yes}{n¦", 3),
    ] {
        let marked = format!("\\ifentrycategory{tail}");
        let (ws, main, _) = setup(&marked.replace('¦', ""), "@book{alpha,title={Alpha}}");
        let at = ws.documents[&main].text.position(marked.find('¦').unwrap());
        let help = signature::help(&ws, &main, at).unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(
            help.signatures[0].label,
            "\\ifentrycategory{key}{category}{true}{false}"
        );
        assert_eq!(help.active_parameter, Some(active), "{marked}");
    }
    for tail in [
        "{al¦BAD}{selected}{yes}{no}",
        "{% before\r\n al¦BAD % after\r\n}{selected}{yes}{no}",
        "{% before\r\nal¦",
    ] {
        let marked = format!("😀 \\ifentrycategory{tail}");
        let source = marked.replace('¦', "");
        let (ws, main, _) = setup(&source, "@book{alpha,title={Alpha}}");
        let at = ws.documents[&main].text.position(marked.find('¦').unwrap());
        let item = features::completion(&ws, &main, at)
            .into_iter()
            .find(|item| item.label == "alpha")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        assert_eq!(
            apply(&source, edits),
            marked
                .replace(" al¦BAD ", "alpha")
                .replace("al¦BAD", "alpha")
                .replace("al¦", "alpha")
        );
    }
    for tail in [
        "*{al¦pha}{selected}{yes}{no}",
        "[note]{al¦pha}{selected}{yes}{no}",
        "{al¦pha,beta}{selected}{yes}{no}",
        "{al¦pha beta}{selected}{yes}{no}",
        "{al¦pha\\suffix}{selected}{yes}{no}",
        "{al¦% joined\r\npha}{selected}{yes}{no}",
        "{beta}{al¦pha}{yes}{no}",
        "{beta}{selected}{al¦pha}{no}",
        "{beta}{selected}{yes}{al¦pha}",
        "{beta}{selected}{yes}{no}{al¦pha}",
    ] {
        let marked = format!("\\ifentrycategory{tail}");
        let (ws, main, _) = setup(
            &marked.replace('¦', ""),
            "@book{alpha,title={Alpha}} @book{beta,title={Beta}}",
        );
        let at = ws.documents[&main].text.position(marked.find('¦').unwrap());
        assert!(features::definitions(&ws, &main, at).is_empty(), "{marked}");
        assert!(
            features::prepare_rename(&ws, &main, at).unwrap().is_none(),
            "{marked}"
        );
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "alpha"),
            "{marked}"
        );
    }
}

#[test]
fn entry_category_query_keys_support_named_calls_aliases_and_user_redefinitions() {
    for call in [
        r"\ifentrycategory{alpha}{selected}{yes}{no}",
        r"\csuse{ifentrycategory}{alpha}{selected}{yes}{no}",
        r"\csname ifentrycategory\endcsname{alpha}{selected}{yes}{no}",
        r"\def\testentry{\ifentrycategory}\testentry{alpha}{selected}{yes}{no}",
    ] {
        let (ws, main, bib) = setup(call, "@book{alpha,title={Alpha}}");
        let at = ws.documents[&main]
            .text
            .position(call.find("alpha").unwrap() + 2);
        assert_eq!(features::definitions(&ws, &main, at)[0].uri, bib, "{call}");
        let changes = features::rename(&ws, &main, at, "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            apply(call, changes[&main].clone()),
            call.replace("{alpha}", "{updated}")
        );
    }
    let source = r"\newcommand\ifentrycategory[1]{}\ifentrycategory{alpha}";
    let (mut ws, main, _) = setup(source, "@book{alpha,title={Alpha}}");
    let at = ws.documents[&main]
        .text
        .position(source.find("alpha").unwrap() + 2);
    assert!(features::definitions(&ws, &main, at).is_empty());
    assert_eq!(
        signature::help(&ws, &main, at).unwrap().signatures[0].label,
        "\\ifentrycategory{#1}"
    );
    let source = r"\newcommand\ifentrycategory[1]{}\csuse{ifentrycategory}{alpha}";
    ws.open(main.clone(), source.into(), 10);
    let at = ws.documents[&main]
        .text
        .position(source.find("alpha").unwrap() + 2);
    assert!(features::definitions(&ws, &main, at).is_empty());
}

#[test]
fn entry_category_query_renames_observe_shared_bibliography_owners() {
    let dir = tempfile::tempdir().unwrap();
    for project in ["a", "b"] {
        std::fs::write(
            dir.path().join(format!("{project}.tex")),
            format!("\\documentclass{{article}}\\addbibresource{{{project}.bib}}\\input{{shared}}"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join(format!("{project}.bib")),
            "@book{alpha,title={Alpha}}",
        )
        .unwrap();
    }
    std::fs::write(
        dir.path().join("shared.tex"),
        r"\ifentrycategory{alpha}{selected}{}{}",
    )
    .unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let uri = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    assert!(features::rename(&ws, &uri, Position::new(0, 19), "updated").is_err());
}

#[test]
fn category_queries_do_not_create_missing_citation_warnings_or_claim_nested_ones() {
    let source = r"\ifentrycategory{missing}{selected}{Literal missing}{\cite{missing}}";
    let (ws, main, _) = setup(source, "");
    let missing: Vec<_> = ws
        .diagnostics(&main)
        .into_iter()
        .filter(|diagnostic| diagnostic.message.starts_with("Undefined citation"))
        .collect();
    assert_eq!(missing.len(), 1, "{missing:?}");
    let start = source.rfind("missing").unwrap();
    assert_eq!(
        missing[0].range,
        Text::new(source.into()).range(start, start + 7)
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.tex");
    let uri = Url::from_file_path(&path).unwrap();
    std::fs::write(&path, source).unwrap();
    let diagnostics = latex_lsp::build_log::parse(
        &path,
        "Package biblatex Warning: Citation 'missing' on page 1 undefined on input line 1.\n",
    );
    assert_eq!(
        diagnostics[&uri][0].range,
        Text::new(source.into()).range(start, start + 7)
    );
}

#[test]
#[ignore = "requires pdflatex, BibTeX and biblatex"]
fn real_entry_category_query_still_finds_the_renamed_bibliography_entry() {
    let dir = tempfile::tempdir().unwrap();
    let source = "\\documentclass{article}\n\\usepackage[backend=bibtex,style=numeric]{biblatex}\n\\addbibresource{refs.bib}\n\\DeclareBibliographyCategory{selected}\n\\addtocategory{selected}{alpha}\n\\begin{document}\n\\nocite{alpha}\n\\ifentrycategory{alpha}{selected}{\\typeout{RENAMED-ENTRY-CATEGORY-OK}}{\\errmessage{Entry category lost}}\n\\printbibliography[category=selected]\n\\end{document}";
    let bibliography =
        "@book{alpha,author={Doe, Jane},title={Category book},year={2026},publisher={Example}}";
    std::fs::write(dir.path().join("main.tex"), source).unwrap();
    std::fs::write(dir.path().join("refs.bib"), bibliography).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = Url::from_file_path(dir.path().join("refs.bib")).unwrap();
    let position = ws.documents[&main]
        .text
        .position(source.rfind("alpha").unwrap() + 2);
    let changes = features::rename(&ws, &main, position, "updated")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    let updated = apply(source, changes[&main].clone());
    assert!(!updated.contains("alpha"), "{updated}");
    std::fs::write(dir.path().join("main.tex"), updated).unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        apply(bibliography, changes[&bib].clone()),
    )
    .unwrap();
    for program in ["pdflatex", "bibtex", "pdflatex", "pdflatex"] {
        let mut command = std::process::Command::new(program);
        if program == "pdflatex" {
            command.args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ]);
        } else {
            command.arg("main");
        }
        let output = command.current_dir(dir.path()).output().unwrap();
        assert!(
            output.status.success(),
            "{program}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    assert!(log.contains("RENAMED-ENTRY-CATEGORY-OK"), "{log}");
    assert!(!log.contains("undefined on input line"), "{log}");
    assert!(!log.contains("Empty bibliography"), "{log}");
}
