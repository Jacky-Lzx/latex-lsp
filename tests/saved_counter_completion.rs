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
fn bare_formatter_operands_complete_as_one_grouped_counter_argument() {
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
            for operand in ["¦x", "x¦"] {
                let marked =
                    format!("\\newcounter{{xlong}}\r\n{before}\\{formatter} {operand}{after}");
                let (result, item) = selected(&marked, "xlong");
                assert_eq!(result, marked.replace(operand, "{xlong}"));
                let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                    panic!("missing edit")
                };
                assert_eq!(edit.new_text, "{xlong}");
                assert_eq!(edit.range.end.character - edit.range.start.character, 1);
                assert!(item.additional_text_edits.is_none());
                let (workspace, uri) = setup(&result);
                let position = workspace.documents[&uri]
                    .text
                    .position(result.rfind("xlong").unwrap() + 2);
                assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
                assert_eq!(
                    features::references(&workspace, &uri, position, true).len(),
                    2
                );
            }
        }
    }
}

#[test]
fn bare_completion_preserves_following_tokens_comments_and_unicode_counter_names() {
    for (marked, expected, name) in [
        (
            r"\UseVerb{\arabic x¦suffix}",
            r"\UseVerb{\arabic {xlong}suffix}",
            "xlong",
        ),
        (
            "\\UseVerb{\\arabic% operand\r\n x¦% retained\r\n}",
            "\\UseVerb{\\arabic% operand\r\n {xlong}% retained\r\n}",
            "xlong",
        ),
        (
            r"\UseVerb{\number\value x¦}",
            r"\UseVerb{\number\value {x-長}}",
            "x-長",
        ),
    ] {
        let definitions = format!("\\newcounter{{{name}}}\r\n");
        let (result, _) = selected(&format!("{definitions}{marked}"), name);
        assert_eq!(result, format!("{definitions}{expected}"));
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
        r"\UseVerb{\detokenize{\arabic e¦}}",
        r"\UseVerb{\string\arabic e¦}",
        "\\UseVerb{% \\arabic e¦\r\nexample}",
        r"\SaveVerb{saved}|\arabic e¦|",
        r"\UseVerb{\csuse{arabic}e¦}",
        r"\UseVerb{\csname arabic\endcsname e¦}",
        r"\UseVerb{\arabic 長¦}",
        r"\UseVerb{\arabic ex¦}",
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
#[ignore = "requires pdflatex and xelatex"]
fn real_bare_counter_completions_repair_failed_saved_name_compilations() {
    for engine in ["pdflatex", "xelatex"] {
        for call in [
            r"\SaveVerb{1}|MarkerUnique|\UseVerb{\arabic x¦}",
            r"\SaveVerb{\arabic x¦}|MarkerUnique|\UseVerb{1}",
            "\\begin{SaveVerbatim}{\\arabic x¦}\r\nMarkerUnique\r\n\\end{SaveVerbatim}\r\n\\UseVerbatim{1}",
            r"\SaveGVerb{1}|MarkerUnique|\UseMVerb{\arabic x¦}",
            "\\SaveVerb{1}|MarkerUnique|\\UseVerb{\\arabic% operand\r\n x¦% retained\r\n}",
            r"\SaveVerb{I}|MarkerUnique|\UseVerb{\Roman x¦}",
            r"\SaveVerb{1}|MarkerUnique|\UseVerb{\number\value x¦}",
            r"\SaveVerb{1suffix}|MarkerUnique|\UseVerb{\arabic x¦suffix}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let marked = format!("\\documentclass{{article}}\r\n\\usepackage{{fancyvrb}}\r\n\\newcounter{{xlong}}\\setcounter{{xlong}}{{1}}\r\n\\begin{{document}}\r\n{call}\r\n\\end{{document}}\r\n");
            let (result, _) = selected(&marked, "xlong");
            let config = BuildConfig { executable: engine.into(), ..Default::default() };
            std::fs::write(root.join("main.tex"), marked.replace('¦', "")).unwrap();
            let before = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
            assert_eq!(before.status, "error", "{engine}/{call}: {}", before.output);
            std::fs::write(root.join("main.tex"), result).unwrap();
            let after = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(root.join("build/latex-lsp/main.pdf").is_file());
        }
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
