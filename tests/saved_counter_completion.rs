use latex_lsp::{build, config::BuildConfig, features, text::Text, workspace::Workspace};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///saved-counter-completion/main.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.open(uri.clone(), source.into(), 7);
    (workspace, uri)
}

fn selected(marked: &str, name: &str) -> (String, CompletionItem) {
    let cursor = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let (workspace, uri) = setup(&source);
    let items = features::completion(
        &workspace,
        &uri,
        workspace.documents[&uri].text.position(cursor),
    );
    let item = items
        .into_iter()
        .find(|item| item.label == name)
        .unwrap_or_else(|| panic!("missing {name}: {marked}"));
    assert_eq!(item.kind, Some(CompletionItemKind::VARIABLE));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing completion edit")
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let result = Text::new(source)
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
        .source;
    (result, item)
}

#[test]
fn formatter_operands_complete_counter_names_inside_saved_name_data() {
    let definitions = "\\newcounter{exercise}\\newtheorem{claim}{Claim}\r\n";
    for formatter in [
        "arabic", "roman", "Roman", "alph", "Alph", "fnsymbol", "value",
    ] {
        for (before, after) in [
            ("\\UseVerb{", "}"),
            ("\\SaveVerb{", "}|Body|"),
            ("\\SaveGVerb{", "}|Body|"),
            ("\\UseMVerb{", "}"),
            ("\\pUseMVerb{", "}"),
            ("\\begin{SaveVerbatim}{", "}\r\nBody\r\n\\end{SaveVerbatim}"),
        ] {
            let marked = format!("{definitions}{before}\\{formatter}{{ex¦rcise}}{after}");
            let (result, _) = selected(&marked, "exercise");
            assert_eq!(result, marked.replace("ex¦rcise", "exercise"));
        }
    }
    for marked in [
        r"\UseVerb{\arabic{ex¦",
        r"\UseVerb{\number\value{ex¦rcise}}",
    ] {
        let (result, _) = selected(&format!("{definitions}{marked}"), "exercise");
        assert_eq!(
            result,
            format!(
                "{definitions}{}",
                marked
                    .replace("ex¦rcise", "exercise")
                    .replace("ex¦", "exercise")
            )
        );
    }
}

#[test]
fn choosing_a_counter_removes_semantic_whitespace_with_single_line_primary_edits() {
    let definitions = "\\newcounter{exercise}\r\n";
    for (marked, expected, additional) in [
        (
            r"\UseVerb{\arabic{ ex¦rcise }}",
            r"\UseVerb{\arabic{exercise}}",
            0,
        ),
        (
            "\\UseVerb{\\arabic{\r\n ex¦rcise \r\n}}",
            r"\UseVerb{\arabic{exercise}}",
            2,
        ),
        (
            "\\UseVerb{\\arabic{ ex¦rcise \r\n}}",
            r"\UseVerb{\arabic{exercise}}",
            1,
        ),
    ] {
        let (result, item) = selected(&format!("{definitions}{marked}"), "exercise");
        assert_eq!(result, format!("{definitions}{expected}"));
        assert_eq!(
            item.additional_text_edits.unwrap_or_default().len(),
            additional
        );
    }
    let marked = format!("{definitions}\\newtheorem{{claim}}{{Claim}}\\SaveVerb{{saved}}|Body|\\UseVerb{{\\arabic{{¦}}}}");
    let source = marked.replace('¦', "");
    let (workspace, uri) = setup(&source);
    let items = features::completion(
        &workspace,
        &uri,
        workspace.documents[&uri]
            .text
            .position(marked.find('¦').unwrap()),
    );
    for name in ["page", "exercise", "claim"] {
        assert!(items.iter().any(|item| item.label == name), "{name}");
    }
    assert!(!items.iter().any(|item| item.label == "saved"));
}

#[test]
fn completion_does_not_open_literal_comments_dynamic_operands_or_indirect_calls() {
    for call in [
        r"\UseVerb{\detokenize{\arabic{ex¦rcise}}}",
        r"\UseVerb{\string\arabic{ex¦rcise}}",
        "\\UseVerb{% \\arabic{ex¦rcise}\r\nexample}",
        r"\SaveVerb{saved}|\arabic{ex¦rcise}|",
        "\\begin{SaveVerbatim}{saved}\r\n\\arabic{ex¦rcise}\r\n\\end{SaveVerbatim}",
        r"\UseVerb{\csuse{arabic}{ex¦rcise}}",
        r"\UseVerb{\csname arabic\endcsname{ex¦rcise}}",
        r"\UseVerb{\arabic{ex¦rcise\suffix}}",
        r"\UseVerb{\arabic{\prefix ex¦rcise}}",
    ] {
        let marked = format!("\\newcounter{{exercise}}\r\n{call}");
        let source = marked.replace('¦', "");
        let (workspace, uri) = setup(&source);
        let items = features::completion(
            &workspace,
            &uri,
            workspace.documents[&uri]
                .text
                .position(marked.find('¦').unwrap()),
        );
        assert!(items.is_empty(), "{call}: {items:?}");
    }
}

#[test]
fn saved_counter_completion_respects_project_boundaries_and_shared_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let marked = r"\documentclass{article}\usepackage{common}\newcounter{exercise}\begin{document}\UseVerb{\arabic{¦}}\end{document}";
    let source = marked.replace('¦', "");
    std::fs::write(root.join("a.tex"), &source).unwrap();
    std::fs::write(
        root.join("b.tex"),
        r"\documentclass{article}\newcounter{otherpaper}\begin{document}Other\end{document}",
    )
    .unwrap();
    std::fs::write(root.join("common.sty"), r"\newcounter{sharedcounter}").unwrap();
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let items = features::completion(
        &workspace,
        &uri,
        workspace.documents[&uri]
            .text
            .position(marked.find('¦').unwrap()),
    );
    for name in ["page", "exercise", "sharedcounter"] {
        assert!(items.iter().any(|item| item.label == name), "{name}");
    }
    assert!(!items.iter().any(|item| item.label == "otherpaper"));
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_counter_name_completions_repair_failed_saved_name_compilations() {
    for engine in ["pdflatex", "xelatex"] {
        for call in [
            r"\SaveVerb{1}|MarkerUnique|\UseVerb{\arabic{ex¦rcise}}",
            r"\SaveVerb{\arabic{ex¦rcise}}|MarkerUnique|\UseVerb{1}",
            "\\begin{SaveVerbatim}{\\arabic{ex¦rcise}}\r\nMarkerUnique\r\n\\end{SaveVerbatim}\r\n\\UseVerbatim{1}",
            r"\SaveGVerb{1}|MarkerUnique|\UseMVerb{\arabic{ex¦rcise}}",
            "\\SaveVerb{1}|MarkerUnique|\\UseVerb{\\arabic{\r\n ex¦rcise \r\n}}",
            r"\SaveVerb{I}|MarkerUnique|\UseVerb{\Roman{ex¦rcise}}",
            r"\SaveVerb{1}|MarkerUnique|\UseVerb{\number\value{ex¦rcise}}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let marked = format!("\\documentclass{{article}}\r\n\\usepackage{{fancyvrb}}\r\n\\newcounter{{exercise}}\\setcounter{{exercise}}{{1}}\r\n\\begin{{document}}\r\n{call}\r\n\\end{{document}}\r\n");
            let source = marked.replace('¦', "");
            let (result, _) = selected(&marked, "exercise");
            let config = BuildConfig { executable: engine.into(), ..Default::default() };
            std::fs::write(root.join("main.tex"), source).unwrap();
            let before = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
            assert_eq!(before.status, "error", "{engine}/{call}: {}", before.output);
            std::fs::write(root.join("main.tex"), result).unwrap();
            let after = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(root.join("build/latex-lsp/main.pdf").is_file());
        }
    }
}
