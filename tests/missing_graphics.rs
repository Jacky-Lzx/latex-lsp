use latex_lsp::{build, build_log, editing, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{path::Path, sync::atomic::AtomicBool};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn parsed(
    marked: &str,
    message: &str,
    frames: &str,
    numbered: bool,
) -> (tempfile::TempDir, Url, Text, Vec<Diagnostic>) {
    let directory = tempfile::tempdir().unwrap();
    let offset = marked.find('¦').unwrap();
    let text = Text::new(marked.replace('¦', ""));
    let path = directory.path().join("main.tex");
    std::fs::write(&path, &text.source).unwrap();
    let uri = Url::from_file_path(&path).unwrap();
    let line = text.position(offset).line;
    let start = text.offset(Position::new(line, 0)).unwrap();
    let prefix = &text.source[start..offset];
    let number = line + 1;
    let header = if numbered {
        format!("./main.tex:{number}: {message}")
    } else {
        format!("! {message}")
    };
    let log = format!("(./main.tex\n{header}\n{frames}\nl.{number} {prefix}\n)\n");
    let diagnostics = build_log::parse(&path, &log)
        .remove(&uri)
        .unwrap_or_default();
    (directory, uri, text, diagnostics)
}

#[test]
fn missing_graphics_errors_select_static_paths_across_engine_messages_and_log_formats() {
    for (message, frames) in [
        ("LaTeX Error: File `imgae' not found.", "See the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ..."),
        ("Package pdftex.def Error: File `imgae.pdf' not found: using draft setting.", "See the pdftex.def package documentation for explanation.\nType  H <return>  for immediate help.\n ..."),
        ("Package luatex.def Error: File `imgae.pdf' not found: using draft setting.", "See the luatex.def package documentation for explanation.\nType  H <return>  for immediate help.\n ..."),
        ("Unable to load picture or PDF file 'imgae.pdf'.", "<to be read again>\n                   }"),
    ] {
        let name = if message.contains("imgae.pdf") { "imgae.pdf" } else { "imgae" };
        for numbered in [false, true] {
            for marked in [
                format!("😀 \\includegraphics*[width=2cm]{{ {name} }}¦ after\n"),
                format!("\\includegraphics % keep\r\n [width={{2cm}}] % keep\r\n {{ {name} }}¦\r\n"),
            ] {
                let (_directory, _uri, text, diagnostics) = parsed(&marked, message, frames, numbered);
                assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
                let item = &diagnostics[0];
                let start = text.source.find(name).unwrap();
                assert_eq!(item.range, text.range(start, start + name.len()), "{message}");
                assert_eq!(item.code, Some(NumberOrString::String("missing-image".into())));
                assert_eq!(item.data, Some(serde_json::json!({"file":name})));
                assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
            }
        }
    }
}

#[test]
fn quoted_graphics_errors_match_driver_names_and_select_raw_filename_spans() {
    for argument in [
        r#""plto image.png""#,
        r#""plto image".png"#,
        r#""plto" image.png"#,
    ] {
        for reported in ["plto image.png", argument] {
            for numbered in [false, true] {
                let marked = format!("😀 \\includegraphics*[width=2cm]{{ {argument} }}¦ after\n");
                let message = format!(
                    "Package pdftex.def Error: File `{reported}' not found: using draft setting."
                );
                let (_directory, _uri, text, diagnostics) = parsed(&marked, &message, "See the pdftex.def package documentation for explanation.\nType  H <return>  for immediate help.\n ...", numbered);
                assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
                let diagnostic = &diagnostics[0];
                let start = text.source.find(argument).unwrap();
                assert_eq!(
                    diagnostic.code,
                    Some(NumberOrString::String("missing-image".into())),
                    "{diagnostics:?}"
                );
                assert_eq!(diagnostic.range, text.range(start, start + argument.len()));
                let reported = reported
                    .strip_prefix('"')
                    .and_then(|name| name.strip_suffix('"'))
                    .unwrap_or(reported);
                assert_eq!(diagnostic.data, Some(serde_json::json!({"file": reported})));
            }
        }
    }
}

#[test]
fn missing_graphics_errors_reject_expansion_frames_dynamic_names_and_unmatched_contexts() {
    let message = "LaTeX Error: File `imgae' not found.";
    for (marked, frames) in [
        (
            r"\includegraphics{imgae}¦",
            "<argument> \\includegraphics{imgae}",
        ),
        (r"\includegraphics{imgae}\macro¦", ""),
        (r"\includegraphics{\imgae}¦", ""),
        (r"\includegraphics{other}¦", ""),
        (r"% \includegraphics{imgae}¦", ""),
        (r"\verb|\includegraphics{imgae}|¦", ""),
        (
            r"\includegraphics{imgae}¦",
            "See some unrelated documentation.",
        ),
        (
            r"\includegraphics{imgae}¦",
            "<recently read> \\includegraphics",
        ),
    ] {
        let (_directory, _uri, _text, diagnostics) = parsed(marked, message, frames, true);
        assert_eq!(diagnostics.len(), 1, "{marked}: {diagnostics:?}");
        assert_eq!(diagnostics[0].code, None, "{marked}: {frames}");
        assert_eq!(diagnostics[0].data, None, "{marked}: {frames}");
    }
}

fn actions(workspace: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> Vec<CodeActionOrCommand> {
    editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            ..Default::default()
        },
    )
}

fn action_edit(action: &CodeActionOrCommand, uri: &Url) -> TextEdit {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!("{action:?}")
    };
    action.edit.as_ref().unwrap().changes.as_ref().unwrap()[uri][0].clone()
}

#[test]
fn image_fixes_use_declared_extension_settings_and_skip_unknown_searches() {
    for (setting, expected) in [
        (r"\DeclareGraphicsExtensions{.png,.pdf}", true),
        (r"\DeclareGraphicsExtensions{}", true),
        (r"\DeclareGraphicsExtensions{\chosen}", false),
        (r"\DeclareGraphicsRule{.png}{png}{.png}{}", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = uri(root, "main.tex");
        let source =
            format!("\\documentclass{{article}}\n{setting}\n\\includegraphics{{resutl}}\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        std::fs::write(root.join("result.png"), "image").unwrap();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        let start = source.find("resutl").unwrap();
        let diagnostic = Diagnostic {
            range: Text::new(source).range(start, start + "resutl".len()),
            source: Some("latex-build".into()),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("missing-image".into())),
            message: "LaTeX Error: File `resutl' not found.".into(),
            data: Some(serde_json::json!({"file":"resutl"})),
            ..Default::default()
        };
        workspace.build_diagnostics.insert(
            main.clone(),
            [(main.clone(), vec![diagnostic.clone()])].into(),
        );
        let fixes = actions(&workspace, &main, &diagnostic);
        assert_eq!(fixes.len(), usize::from(expected), "{setting}: {fixes:?}");
        if expected {
            assert_eq!(action_edit(&fixes[0], &main).new_text, "result.png");
        }
    }
}

#[test]
fn image_spelling_fixes_use_the_compilation_root_and_project_graphics_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in ["a/assets/plots", "b/assets/plots", "shared/assets/plots"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    for (name, source) in [
        (
            "a/main.tex",
            "\\documentclass{article}\\graphicspath{{assets/}}\\input{../shared/child}",
        ),
        (
            "b/main.tex",
            "\\documentclass{article}\\graphicspath{{assets/}}\\input{../shared/child}",
        ),
        (
            "shared/child.tex",
            "😀 \\includegraphics % keep\r\n [width=1cm]{ plots/resutl.pdf }\r\n",
        ),
        ("a/assets/plots/result.pdf", "pdf"),
        ("b/assets/plots/resut.pdf", "pdf"),
        ("shared/assets/plots/resutl.pdf", "Wrong working directory."),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let child = uri(root, "shared/child.tex");
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    let doc = &workspace.documents[&child];
    let start = doc.text.source.find("plots/resutl.pdf").unwrap();
    let diagnostic = Diagnostic {
        range: doc.text.range(start, start + "plots/resutl.pdf".len()),
        source: Some("latex-build".into()),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String("missing-image".into())),
        message:
            "Package pdftex.def Error: File `plots/resutl.pdf' not found: using draft setting."
                .into(),
        data: Some(serde_json::json!({"file":"plots/resutl.pdf"})),
        ..Default::default()
    };
    for (project, expected) in [
        ("a/main.tex", "plots/result.pdf"),
        ("b/main.tex", "plots/resut.pdf"),
    ] {
        workspace.build_diagnostics.clear();
        workspace.build_diagnostics.insert(
            uri(root, project),
            [(child.clone(), vec![diagnostic.clone()])].into(),
        );
        let fixes = actions(&workspace, &child, &diagnostic);
        assert_eq!(fixes.len(), 1, "{project}: {fixes:?}");
        let edit = action_edit(&fixes[0], &child);
        assert_eq!(edit.new_text, expected);
        let repaired = workspace.documents[&child]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap();
        assert_eq!(
            repaired.source,
            workspace.documents[&child]
                .text
                .source
                .replace("plots/resutl.pdf", expected)
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex and graphicx"]
fn missing_image_corrections_make_real_graphicx_builds_succeed() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("assets")).unwrap();
    let drawing = root.join("assets/result.tex");
    std::fs::write(
        &drawing,
        "\\documentclass{article}\\pagestyle{empty}\\begin{document}A figure.\\end{document}\n",
    )
    .unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let graphic = build::run(&drawing, &config, &AtomicBool::new(false));
    assert_eq!(graphic.status, "success", "{}", graphic.output);
    let pdf = graphic.pdf.unwrap();
    for name in ["result.pdf", "result 中文 图.pdf"] {
        std::fs::copy(&pdf, root.join("assets").join(name)).unwrap();
    }
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let compiler = root.join(engine);
        std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: compiler.to_string_lossy().into(),
            ..Default::default()
        };
        for (missing, replacement) in [
            ("resutl", "result.pdf"),
            ("resutl.pdf", "result.pdf"),
            ("resutl 中文 图.pdf", "result 中文 图.pdf"),
        ] {
            let source = format!("\\documentclass{{article}}\n\\usepackage{{graphicx}}\n\\graphicspath{{{{assets/}}}}\n\\begin{{document}}\n\\typeout{{😀}} \\includegraphics[width=1cm]{{{missing}}}\n\\end{{document}}\n");
            let main_path = root.join("main.tex");
            std::fs::write(&main_path, &source).unwrap();
            let outcome = build::run(&main_path, &config, &AtomicBool::new(false));
            assert_eq!(outcome.status, "error", "{engine}: {}", outcome.output);
            let main = uri(root, "main.tex");
            let diagnostic = outcome.diagnostics[&main]
                .iter()
                .find(|item| item.code == Some(NumberOrString::String("missing-image".into())))
                .unwrap_or_else(|| {
                    panic!(
                        "{engine}/{missing}: {:?}\n{}",
                        outcome.diagnostics, outcome.output
                    )
                })
                .clone();
            let text = Text::new(source.clone());
            let start = source.find(missing).unwrap();
            assert_eq!(diagnostic.range, text.range(start, start + missing.len()));
            let mut workspace = Workspace {
                code_action_literals: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace
                .build_diagnostics
                .insert(main.clone(), outcome.diagnostics);
            let fixes = actions(&workspace, &main, &diagnostic);
            assert_eq!(fixes.len(), 1, "{engine}/{missing}: {fixes:?}");
            let edit = action_edit(&fixes[0], &main);
            assert_eq!(edit.new_text, replacement);
            let repaired = text
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            std::fs::write(&main_path, repaired.source).unwrap();
            let outcome = build::run(&main_path, &config, &AtomicBool::new(false));
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(outcome
                .diagnostics
                .values()
                .flatten()
                .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)));
        }
    }
}

fn fix_fixture(marked: &str, name: &str) -> (tempfile::TempDir, Workspace, Url, Diagnostic) {
    let message = format!("LaTeX Error: File `{name}' not found.");
    let (directory, main, text, diagnostics) = parsed(marked, &message, "", true);
    let diagnostic = diagnostics
        .into_iter()
        .find(|item| item.code == Some(NumberOrString::String("missing-image".into())))
        .unwrap();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.open(main.clone(), text.source, 7);
    workspace.build_diagnostics.insert(
        main.clone(),
        [(main.clone(), vec![diagnostic.clone()])].into(),
    );
    (directory, workspace, main, diagnostic)
}

#[test]
fn image_corrections_filter_candidates_bound_results_and_keep_actual_extensions() {
    let (directory, workspace, main, diagnostic) =
        fix_fixture(r"😀 \includegraphics{imgae}¦", "imgae");
    for name in [
        "image.pdf",
        "image.png",
        "image.jpg",
        "image.jpeg",
        "image.eps",
        "image.svg",
        "imga.pdf",
        ".image.pdf",
        "imgae.txt",
        "imgae#1.pdf",
    ] {
        std::fs::write(directory.path().join(name), "image").unwrap();
    }
    std::fs::create_dir(directory.path().join("imga.png")).unwrap();
    let fixes = actions(&workspace, &main, &diagnostic);
    assert_eq!(fixes.len(), 5, "{fixes:?}");
    for fix in &fixes {
        let edit = action_edit(fix, &main);
        assert_eq!(edit.range, diagnostic.range);
        assert!(
            edit.new_text.starts_with("image.") || edit.new_text == "imga.pdf",
            "{edit:?}"
        );
        assert!(directory.path().join(&edit.new_text).is_file());
    }
    let CodeActionOrCommand::CodeAction(first) = &fixes[0] else {
        panic!()
    };
    assert_eq!(first.kind, Some(CodeActionKind::QUICKFIX));
    assert_eq!(first.diagnostics, Some(vec![diagnostic]));
}

#[test]
fn image_fixes_reject_stale_names_metadata_ranges_and_newly_restored_files() {
    let (directory, mut workspace, main, diagnostic) =
        fix_fixture(r"\includegraphics{imgae.pdf}¦", "imgae.pdf");
    std::fs::write(directory.path().join("image.pdf"), "image").unwrap();
    assert_eq!(actions(&workspace, &main, &diagnostic).len(), 1);
    for alteration in ["name", "data", "range", "message"] {
        let mut changed = diagnostic.clone();
        match alteration {
            "name" => workspace.open(main.clone(), r"\includegraphics{other.pdf}".into(), 8),
            "data" => changed.data = Some(serde_json::json!({"file":"other.pdf"})),
            "range" => changed.range.start.character += 1,
            "message" => changed.message = "LaTeX Error: File `other.pdf' not found.".into(),
            _ => unreachable!(),
        }
        workspace
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), vec![changed.clone()])].into());
        assert!(
            actions(&workspace, &main, &changed).is_empty(),
            "{alteration}"
        );
        workspace.open(main.clone(), r"\includegraphics{imgae.pdf}".into(), 9);
    }
    workspace.build_diagnostics.insert(
        main.clone(),
        [(main.clone(), vec![diagnostic.clone()])].into(),
    );
    std::fs::write(directory.path().join("imgae.pdf"), "restored").unwrap();
    assert!(actions(&workspace, &main, &diagnostic).is_empty());
}

#[test]
fn legacy_image_corrections_revalidate_versions_candidates_and_current_source() {
    let (directory, mut workspace, main, diagnostic) =
        fix_fixture(r"\includegraphics{imgae.pdf}¦", "imgae.pdf");
    let candidate = directory.path().join("image.pdf");
    std::fs::write(&candidate, "image").unwrap();
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    workspace.versioned_edits = true;
    let fixes = actions(&workspace, &main, &diagnostic);
    assert_eq!(fixes.len(), 1);
    let CodeActionOrCommand::Command(command) = &fixes[0] else {
        panic!()
    };
    assert_eq!(command.command, "latex.applyQuickFix");
    let request = command.arguments.as_ref().unwrap()[0].clone();
    let resolve = |workspace: &Workspace| {
        editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
    };
    let edit = resolve(&workspace).unwrap();
    let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(edits[0].text_document.version, Some(7));
    let OneOf::Left(edit) = &edits[0].edits[0] else {
        panic!()
    };
    assert_eq!(edit.new_text, "image.pdf");
    std::fs::remove_file(&candidate).unwrap();
    assert!(resolve(&workspace).is_none());
    std::fs::write(&candidate, "image").unwrap();
    workspace.open(main.clone(), r"\includegraphics{imgae.pdf}".into(), 8);
    assert!(resolve(&workspace).is_none());
    workspace.open(main.clone(), r"\includegraphics{other.pdf}".into(), 7);
    assert!(resolve(&workspace).is_none());
}

#[test]
fn graphics_error_messages_restore_wraps_at_driver_and_filename_periods() {
    for (message, frames) in [
        ("LaTeX Error: File `im.\ngae.pdf' not found.", ""),
        ("Package pdftex.\ndef Error: File `im.gae.pdf' not found: using draft setting.", "See the pdftex.def package documentation for explanation.\nType  H <return>  for immediate help.\n ..."),
        ("Package luatex.def Error: File `im.\ngae.pdf' not found: using draft setting.", "See the luatex.def package documentation for explanation.\nType  H <return>  for immediate help.\n ..."),
        ("Unable to load picture or PDF file 'im.\ngae.pdf'.", "<to be read again>\n                   }"),
    ] {
        let (_directory, _uri, text, diagnostics) = parsed(r"\includegraphics{im.gae.pdf}¦", message, frames, true);
        assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
        assert_eq!(diagnostics[0].code, Some(NumberOrString::String("missing-image".into())), "{message}: {diagnostics:?}");
        let start = text.source.find("im.gae.pdf").unwrap();
        assert_eq!(diagnostics[0].range, text.range(start, start + "im.gae.pdf".len()));
        assert_eq!(diagnostics[0].data, Some(serde_json::json!({"file":"im.gae.pdf"})));
    }
}

#[test]
fn image_spelling_candidates_follow_the_paths_active_at_the_error() {
    for (marked, expected) in [
        ("\\graphicspath{{first/}}\n\\includegraphics{plto.pdf}¦\n\\graphicspath{{second/}}\n\\includegraphics{plto.pdf}\n", "plot.pdf"),
        ("\\graphicspath{{first/}}\n\\includegraphics{plto.pdf}\n\\graphicspath{{second/}}\n\\includegraphics{plto.pdf}¦\n", "plum.pdf"),
    ] {
        let (directory, workspace, main, diagnostic) = fix_fixture(marked, "plto.pdf");
        let root = directory.path();
        for (path, file) in [("first", "plot.pdf"), ("second", "plum.pdf")] {
            std::fs::create_dir(root.join(path)).unwrap();
            std::fs::write(root.join(path).join(file), "image").unwrap();
        }
        let offered = actions(&workspace, &main, &diagnostic);
        assert_eq!(offered.len(), 1, "{expected}: {offered:?}; {diagnostic:?}");
        assert_eq!(action_edit(&offered[0], &main).new_text, expected);
    }
}

#[test]
fn image_alias_errors_offer_precise_spelling_fixes_and_reject_stale_calls() {
    for call in [
        r"\Image",
        r"\csuse{Image}",
        r"\csname Image\endcsname",
        r"\csuse{includegraphics}",
    ] {
        for numbered in [false, true] {
            let marked = format!("\\def\\Image{{\\includegraphics}}\n{call}*[width=1cm]{{imgae}}¦");
            let (directory, main, text, diagnostics) = parsed(
                &marked,
                "LaTeX Error: File `imgae' not found.",
                "",
                numbered,
            );
            assert_eq!(diagnostics.len(), 1);
            let diagnostic = &diagnostics[0];
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("missing-image".into())),
                "{call}: {diagnostics:?}"
            );
            let start = text.source.find("imgae").unwrap();
            assert_eq!(diagnostic.range, text.range(start, start + 5));
            std::fs::write(directory.path().join("image.png"), "image").unwrap();
            let mut workspace = Workspace {
                code_action_literals: true,
                ..Default::default()
            };
            workspace.add_root(directory.path().into());
            workspace
                .build_diagnostics
                .insert(main.clone(), [(main.clone(), diagnostics.clone())].into());
            let fixes = actions(&workspace, &main, diagnostic);
            assert_eq!(fixes.len(), 1, "{call}: {fixes:?}");
            assert_eq!(action_edit(&fixes[0], &main).new_text, "image.png");
            workspace.open(
                main.clone(),
                format!("{}\\def\\includegraphics#1{{other}}", text.source),
                8,
            );
            assert!(actions(&workspace, &main, diagnostic).is_empty());
        }
    }
}
