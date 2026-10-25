use latex_lsp::{
    compilation::{build, build_log},
    editing,
    language::navigation,
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;
use std::path::Path;

const PDF_FRAME: &str = "\\FV@Error ... {FancyVerb Error:\n\\space \\space #1\n}\n";
const XE_FRAME: &str = "\\FV@Error ...ncyVerb Error:^^J\\space \\space #1^^J}\n";
const CODE: &str = "missing-fancyverb-input";

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn parse(
    marked: &str,
    report: &str,
    frame: &str,
    numbered: bool,
) -> (tempfile::TempDir, Url, Text, Vec<Diagnostic>) {
    let directory = tempfile::tempdir().unwrap();
    let main = uri(directory.path(), "main.tex");
    let end = marked.find('¦').unwrap();
    let text = Text::new(marked.replace('¦', ""));
    std::fs::write(main.to_file_path().unwrap(), &text.source).unwrap();
    let line = text.position(end).line;
    let start = text.offset(Position::new(line, 0)).unwrap();
    let number = line + 1;
    let header = if numbered {
        format!("./main.tex:{number}: FancyVerb Error:")
    } else {
        "! FancyVerb Error:".into()
    };
    let log = format!("(./main.tex\n{header}\n  No verbatim file {report}\n.\n{frame}                                                  \nl.{number} {}\n)\n", &text.source[start..end]);
    let items = build_log::parse(&main.to_file_path().unwrap(), &log)
        .remove(&main)
        .unwrap_or_default();
    (directory, main, text, items)
}

fn missing(items: &[Diagnostic]) -> Option<&Diagnostic> {
    items
        .iter()
        .find(|item| item.code == Some(NumberOrString::String(CODE.into())))
}

fn actions(workspace: &Workspace, main: &Url, item: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        main,
        item.range,
        &CodeActionContext {
            diagnostics: vec![item.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            ..Default::default()
        },
    )
}

fn edit(action: &CodeActionOrCommand, main: &Url) -> TextEdit {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!()
    };
    let Some(DocumentChanges::Edits(documents)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!()
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *main);
    assert_eq!(documents[0].text_document.version, Some(7));
    assert_eq!(documents[0].edits.len(), 1);
    let OneOf::Left(edit) = &documents[0].edits[0] else {
        panic!()
    };
    edit.clone()
}

fn apply(text: Text, edit: TextEdit) -> Text {
    text.changed(vec![TextDocumentContentChangeEvent {
        range: Some(edit.range),
        range_length: None,
        text: edit.new_text,
    }])
    .unwrap()
}

fn workspace(root: &Path, main: &Url, source: &str, items: Vec<Diagnostic>) -> Workspace {
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 7);
    workspace
        .build_diagnostics
        .insert(main.clone(), [(main.clone(), items)].into());
    workspace
}

#[test]
fn direct_calls_have_exact_inner_utf16_ranges_with_both_engine_frames_and_filename_wrappers() {
    for name in [
        "smaple.rs",
        "README",
        "sample.unknown",
        ".hidden",
        "dir.with.dot/code.rs",
        "样例, one.rs",
        "😀 one.rs",
        "name.",
    ] {
        for value in [format!("\"{name}\""), format!("\\detokenize{{\"{name}\"}}")]
            .into_iter()
            .chain((!name.contains(' ')).then(|| name.to_owned()))
        {
            let report = if value.contains('"') {
                format!("\"{name}\"")
            } else {
                name.into()
            };
            let marked = format!("% 😀\r\n\\VerbatimInput *[frame=single,label={{A,B [ C]}}] % keep\r\n{{{value}}}¦\r\n");
            for numbered in [false, true] {
                for frame in [PDF_FRAME, XE_FRAME, ""] {
                    let (_, _, text, items) = parse(&marked, &report, frame, numbered);
                    let item = missing(&items).unwrap_or_else(|| panic!("{marked}: {items:?}"));
                    let start = text.source.find(name).unwrap();
                    assert_eq!(item.range, text.range(start, start + name.len()));
                    assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
                    assert_eq!(item.source.as_deref(), Some("latex-build"));
                    assert_eq!(item.data, Some(serde_json::json!({"file":name})));
                }
            }
        }
    }
}

#[test]
fn unsupported_calls_redefinitions_and_expansion_frames_keep_generic_errors() {
    for marked in [
        r"\VerbatimInput**{smaple.rs}¦",
        r"\VerbatimInput[a][b]{smaple.rs}¦",
        r"\VerbatimInput[commandchars=abc]{smaple.rs}¦",
        r"\VerbatimInput[codes=anything]{smaple.rs}¦",
        r"\VerbatimInput{\macro/smaple.rs}¦",
        r"\VerbatimInput{smaple one.rs}¦",
        r"\VerbatimInput{\detokenizeX{smaple.rs}}¦",
        r"\fvset{commandchars=abc}\VerbatimInput{smaple.rs}¦",
        r"\def\FV@@Input{}\VerbatimInput{smaple.rs}¦",
        r"\FV@AddToHook\FV@CatCodesHook{}\VerbatimInput{smaple.rs}¦",
        r"\setkeys{FV}{commandchars=abc}\VerbatimInput{smaple.rs}¦",
        r"\let\detokenize\relax\VerbatimInput{smaple.rs}¦",
        r"\VerbatimInput{smaple.rs}\later¦",
        r"% \VerbatimInput{smaple.rs}¦",
        r"\verb|\VerbatimInput{smaple.rs}|¦",
    ] {
        let (_, _, _, items) = parse(marked, "smaple.rs", PDF_FRAME, true);
        assert!(missing(&items).is_none(), "{marked}: {items:?}");
    }
    for frame in [
        "<argument> \\expanded\n",
        "\\FV@Error ... unknown implementation\n",
        "\\FV@Error ... {FancyVerb Error:\n\\space \\space #1\nwrong\n",
        "[latex-lsp: middle of compiler output omitted]\n",
    ] {
        let (_, _, _, items) = parse(r"\VerbatimInput{smaple.rs}¦", "smaple.rs", frame, true);
        assert!(missing(&items).is_none(), "{frame}: {items:?}");
    }
}

#[test]
fn long_dotted_reports_keep_child_context_and_match_a_shortened_source_echo() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = uri(root, "child.tex");
    std::fs::write(&main, r"\input{child}").unwrap();
    let name = format!("code/{}样例 one.rs", "long.part.".repeat(12));
    let source = format!("% 😀\n\\VerbatimInput *[frame=single]{{\\detokenize{{\"{name}\"}}}}\n");
    std::fs::write(child.to_file_path().unwrap(), &source).unwrap();
    let tail = source.trim_end();
    let tail = &tail[tail.len() - 38..];
    for width in [19, 35, 79, 121] {
        let body = format!("  No verbatim file \"{name}\"")
            .chars()
            .collect::<Vec<_>>()
            .chunks(width)
            .map(|part| part.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        let log = format!("(./main.tex\n(./child.tex\n./child.tex:2: FancyVerb Error:\n{body}\n.\n{XE_FRAME}\nl.2 ...{tail}\nPackage local Warning: Adjacent warning on input line 2.\n)\n)");
        let items = build_log::parse(&main, &log);
        let item = missing(&items[&child]).unwrap_or_else(|| panic!("{log}: {items:?}"));
        let text = Text::new(source.clone());
        let start = source.find(&name).unwrap();
        assert_eq!(item.range, text.range(start, start + name.len()));
        assert_eq!(item.data, Some(serde_json::json!({"file":name})));
        assert!(items[&child]
            .iter()
            .any(|item| item.message.contains("Adjacent warning")));
    }
}

#[test]
fn fixes_preserve_wrappers_options_crlf_and_versions_and_can_introduce_spaces_or_unicode() {
    for (wrong, right, value, expected) in [
        ("smaple.rs", "sample.rs", "smaple.rs", "sample.rs"),
        (
            "smaple.unknown",
            "sample.unknown",
            "smaple.unknown",
            "sample.unknown",
        ),
        (
            "CODEDOCUMETN",
            "CODEDOCUMENT",
            "CODEDOCUMETN",
            "CODEDOCUMENT",
        ),
        (".hiddne", ".hidden", ".hiddne", ".hidden"),
        ("my oen.rs", "my one.rs", "\"my oen.rs\"", "\"my one.rs\""),
        (
            "样例, oen.rs",
            "样例, one.rs",
            "\\detokenize{\"样例, oen.rs\"}",
            "\\detokenize{\"样例, one.rs\"}",
        ),
        (
            "sample.rs",
            "samplé.rs",
            "sample.rs",
            "\\detokenize{\"samplé.rs\"}",
        ),
        ("sample.rs", "sampl e.rs", "sample.rs", "\"sampl e.rs\""),
    ] {
        let marked = format!("% 😀\r\n\\VerbatimInput *[frame=single] % keep\r\n{{{value}}}¦\r\n");
        let report = if value.contains('"') {
            format!("\"{wrong}\"")
        } else {
            wrong.into()
        };
        let (directory, main, text, items) = parse(&marked, &report, PDF_FRAME, true);
        let item = missing(&items).unwrap().clone();
        std::fs::write(directory.path().join(right), r"\input{main}").unwrap();
        let mut workspace = workspace(directory.path(), &main, &text.source, items);
        let fixes = actions(&workspace, &main, &item);
        assert_eq!(fixes.len(), 1, "{wrong}: {fixes:?}");
        let updated = apply(text.clone(), edit(&fixes[0], &main));
        assert_eq!(updated.source, text.source.replace(value, expected));
        workspace.open(main.clone(), updated.source, 8);
        assert!(actions(&workspace, &main, &item).is_empty());
        assert_eq!(
            navigation::document_links(&workspace, &main)[0].target,
            Some(uri(directory.path(), right))
        );
    }
}

#[test]
fn fixes_reject_changed_files_tampered_metadata_shadowing_and_project_redefinitions() {
    for condition in [
        "metadata", "range", "message", "exists", "shadow", "settings",
    ] {
        let (directory, main, text, mut items) = parse(
            r"\input{settings}\VerbatimInput{smaple.rs}¦",
            "smaple.rs",
            PDF_FRAME,
            true,
        );
        std::fs::write(directory.path().join("sample.rs"), "Code.").unwrap();
        let item = items
            .iter_mut()
            .find(|item| item.code == Some(NumberOrString::String(CODE.into())))
            .unwrap();
        match condition {
            "metadata" => item.data = Some(serde_json::json!({"file":"different.rs"})),
            "range" => item.range.end.character -= 1,
            "message" => item.message = "FancyVerb Error:  No verbatim file other.rs.".into(),
            "exists" => std::fs::write(directory.path().join("smaple.rs"), "Restored.").unwrap(),
            "shadow" => std::fs::write(directory.path().join("sample.rs.tex"), "Shadow.").unwrap(),
            "settings" => {
                std::fs::write(directory.path().join("settings.tex"), r"\def\FV@@Input{}").unwrap()
            }
            _ => unreachable!(),
        }
        let item = item.clone();
        let workspace = workspace(directory.path(), &main, &text.source, items);
        assert!(actions(&workspace, &main, &item).is_empty(), "{condition}");
    }
}

#[test]
fn incomplete_error_blocks_unknown_sources_and_incorrect_echoes_do_not_guess_paths() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, r"\VerbatimInput{smaple.rs}").unwrap();
    for log in [
        format!("(./main.tex\n! FancyVerb Error:\n  No verbatim file smaple.rs\n{PDF_FRAME}l.1 \\VerbatimInput{{smaple.rs}}\n"),
        format!("(./main.tex\n! FancyVerb Error:\n  No verbatim file smaple.rs\nPackage local Warning: Adjacent warning.\n.\n{PDF_FRAME}l.1 \\VerbatimInput{{smaple.rs}}\n"),
        format!("(./unavailable.tex\n! FancyVerb Error:\n  No verbatim file smaple.rs\n.\n{PDF_FRAME}l.1 \\VerbatimInput{{smaple.rs}}\n"),
        format!("[latex-lsp: middle of compiler output omitted]\n! FancyVerb Error:\n  No verbatim file smaple.rs\n.\n{PDF_FRAME}l.1 \\VerbatimInput{{smaple.rs}}\n"),
        format!("(./main.tex\n./main.tex:2: FancyVerb Error:\n  No verbatim file smaple.rs\n.\n{PDF_FRAME}l.2 \\VerbatimInput{{smaple.rs}}\n"),
        format!("(./main.tex\n./main.tex:1: FancyVerb Error:\n  No verbatim file smaple.rs\n.\n{PDF_FRAME}l.1 \\VerbatimInput{{other.rs}}\n"),
    ] {
        let items = build_log::parse(&main, &log);
        assert!(items.values().all(|items| missing(items).is_none()), "{log}: {items:?}");
    }
}

#[test]
fn oversized_or_ambiguous_context_does_not_consume_later_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let name = format!("{}.rs", "s".repeat(80));
    let source = format!(r"\VerbatimInput{{{name}}}\VerbatimInput{{{name}}}");
    std::fs::write(&main, &source).unwrap();
    let wrapped = name
        .chars()
        .map(|ch| ch.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    for body in [
        format!("  No verbatim file {name}"),
        format!("  No verbatim file \n{wrapped}"),
    ] {
        let log = format!("(./main.tex\n./main.tex:1: FancyVerb Error:\n{body}\n.\n{XE_FRAME}l.1 ...{{{name}}}\nPackage local Warning: Adjacent warning on input line 1.\n)\n");
        let items = build_log::parse(&main, &log);
        let items = &items[&uri(root, "main.tex")];
        assert!(missing(items).is_none(), "{body}: {items:?}");
        assert!(items
            .iter()
            .any(|item| item.message.contains("Adjacent warning")));
    }
}

#[test]
fn other_fancyverb_errors_keep_their_original_details_and_do_not_offer_file_fixes() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = r"\VerbatimInput[frame=unknown]{sample.rs}";
    std::fs::write(&main, source).unwrap();
    for body in [
        "Frame style `unknown' not defined.",
        "gobble parameter must be less than 10",
        "Empty verbatim environment",
    ] {
        for frame in [PDF_FRAME, XE_FRAME] {
            let log = format!("(./main.tex\n./main.tex:1: FancyVerb Error:\n  {body}\n.\n{frame}l.1 {source}\n)\n");
            let items = build_log::parse(&main, &log);
            let items = &items[&uri(directory.path(), "main.tex")];
            assert_eq!(items.len(), 1, "{items:?}");
            assert!(items[0].message.contains(body), "{items:?}");
            assert_eq!(items[0].severity, Some(DiagnosticSeverity::ERROR));
            assert!(items[0].code.is_none(), "{items:?}");
            let workspace = workspace(
                directory.path(),
                &uri(directory.path(), "main.tex"),
                source,
                items.clone(),
            );
            assert!(actions(&workspace, &uri(directory.path(), "main.tex"), &items[0]).is_empty());
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and fancyvrb"]
fn real_missing_fancyverb_fixes_read_the_intended_file_with_both_engines() {
    for engine in ["pdflatex", "xelatex"] {
        for (wrong, right, value) in [
            ("smaple.rs", "sample.rs", "smaple.rs"),
            ("smaple.unknown", "sample.unknown", "smaple.unknown"),
            ("CODEDOCUMETN", "CODEDOCUMENT", "CODEDOCUMETN"),
            ("my oen.rs", "my one.rs", "\"my oen.rs\""),
            (
                "样例, oen.rs",
                "样例, one.rs",
                "\\detokenize{\"样例, oen.rs\"}",
            ),
            ("sample.rs", "samplé.rs", "sample.rs"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(root, "main.tex");
            let source = format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{fancyvrb}}\\begin{{document}}\r\n\\VerbatimInput *[frame=single] % keep\r\n{{{value}}}\r\n\\end{{document}}\r\n");
            std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
            std::fs::write(root.join(right), "Original code.\n").unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let first = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
            assert_eq!(first.status, "error", "{engine}/{wrong}: {}", first.output);
            assert!(first.pdf.is_none());
            let item = missing(&first.diagnostics[&main])
                .unwrap_or_else(|| panic!("{engine}/{wrong}: {:?}", first.diagnostics))
                .clone();
            let start = source.find(wrong).unwrap();
            let text = Text::new(source.clone());
            assert_eq!(item.range, text.range(start, start + wrong.len()));
            let workspace = workspace(root, &main, &source, first.diagnostics[&main].clone());
            let fixes = actions(&workspace, &main, &item);
            assert_eq!(fixes.len(), 1, "{engine}/{wrong}: {fixes:?}");
            let updated = apply(text, edit(&fixes[0], &main));
            std::fs::write(main.to_file_path().unwrap(), updated.source).unwrap();
            let fixed = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
            assert_eq!(
                fixed.status, "success",
                "{engine}/{right}: {}",
                fixed.output
            );
            assert!(fixed.pdf.is_some());
            assert!(fixed
                .diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
            let recorder = std::fs::read_to_string(root.join("build/latex-lsp/main.fls")).unwrap();
            assert!(recorder
                .lines()
                .any(|line| line.strip_prefix("INPUT ").is_some_and(|path| root
                    .join(path.trim_matches('"'))
                    .canonicalize()
                    .ok()
                    .as_ref()
                    == Some(&root.join(right)))));
        }
    }
}
