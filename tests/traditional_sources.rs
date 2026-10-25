use latex_lsp::{compilation::build_log, text::Text};
use lsp_types::{DiagnosticSeverity, NumberOrString, Url};

#[test]
fn warning_prose_can_mention_missing_files_without_becoming_a_source_entry() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\bad").unwrap();
    let diagnostics = build_log::parse(
        &main,
        "(./main.tex\nLaTeX Warning: See\n(see file.tex for details).\n! Undefined control sequence.\nl.1 \\bad\n)\n",
    );
    let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "LaTeX Warning: See (see file.tex for details)."));
    assert!(diagnostics.iter().any(|diagnostic| diagnostic.code
        == Some(NumberOrString::String("undefined-control-sequence".into()))));
}

#[test]
fn unavailable_traditional_sources_never_borrow_parent_error_contexts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("child.tex");
    std::fs::create_dir(root.join("folder.tex")).unwrap();
    let unknown_sources = [
        "./gone.tex".to_owned(),
        "./folder.tex".to_owned(),
        "\"./missing file.tex\"".to_owned(),
        "./missing(paren).sty".to_owned(),
        "\"./missing (quoted).def\"".to_owned(),
        "./missing.bbx".to_owned(),
        "./missing.cbx".to_owned(),
        "./missing.lbx".to_owned(),
        "./missing.dbx".to_owned(),
        "./missing.fd".to_owned(),
        "./missing.ldf".to_owned(),
        root.join("missing absolute.tex")
            .to_string_lossy()
            .into_owned(),
    ];
    for (source, message, context, code) in [
        (
            "😀 \\bad{x}",
            "Undefined control sequence.",
            "l.1 😀 \\bad",
            "undefined-control-sequence",
        ),
        (
            "😀 $\\bad{x}$",
            "Undefined control sequence.",
            "<recently read> \\bad\n \nl.1 😀 $\\bad",
            "undefined-control-sequence",
        ),
        (
            "😀 \\begin{bad}Text",
            "LaTeX Error: Environment bad undefined.",
            "l.1 😀 \\begin{bad}",
            "undefined-environment",
        ),
        (
            "😀 \\setcounter{bad}{1}",
            "LaTeX Error: No counter 'bad' defined.",
            "l.1 😀 \\setcounter{bad}{1}",
            "undefined-counter",
        ),
        (
            "😀 \\input{bad}",
            "I can't find file `bad.tex'.",
            "l.1 😀 \\input{bad}",
            "missing-input",
        ),
    ] {
        std::fs::write(&main, source).unwrap();
        std::fs::write(&child, source).unwrap();
        for (name, path) in [("./main.tex", &main), ("./child.tex", &child)] {
            let diagnostics =
                build_log::parse(&main, &format!("({name}\n! {message}\n{context}\n)\n"));
            let diagnostic = &diagnostics[&Url::from_file_path(path).unwrap()][0];
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String(code.into())),
                "{name}: {diagnostic:?}"
            );
        }
        for name in &unknown_sources {
            let diagnostics = build_log::parse(
                &main,
                &format!("(./main.tex\n({name}\n! {message}\n{context}\n)\n)\n"),
            );
            assert_eq!(diagnostics.len(), 1);
            let diagnostic = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
            assert_eq!(diagnostic.message, message);
            assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
            assert_eq!(diagnostic.code, None, "{name}: {diagnostic:?}");
            assert_eq!(
                diagnostic.data,
                Some(serde_json::json!({"unresolvedSource":true})),
                "{name}: {diagnostic:?}"
            );
            assert_eq!(
                diagnostic.range,
                Text::new(source.into()).range(0, source.len())
            );
        }
    }
}

#[test]
fn traditional_source_barriers_restore_parents_and_allow_nested_resolved_children() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    let source = "😀 \\bad{x} \\ref{target}";
    std::fs::write(&main, source).unwrap();
    std::fs::write(&child, source).unwrap();
    let log = concat!(
        "(./main.tex\n(./gone.tex\n(note)\n(./child.tex\n",
        "! Undefined control sequence.\nl.1 😀 \\bad\n)\n",
        "LaTeX Warning: Reference `target' undefined on input line 1.\n)\n",
        "! Undefined control sequence.\nl.1 😀 \\bad\n)\n",
    );
    let diagnostics = build_log::parse(&main, log);
    let root_errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    let child_errors = &diagnostics[&Url::from_file_path(&child).unwrap()];
    assert_eq!(root_errors.len(), 2);
    assert_eq!(child_errors.len(), 1);
    let warning = root_errors
        .iter()
        .find(|d| d.severity == Some(DiagnosticSeverity::WARNING))
        .unwrap();
    assert_eq!(
        warning.data,
        Some(serde_json::json!({"unresolvedSource":true}))
    );
    assert_eq!(
        warning.range,
        Text::new(source.into()).range(0, source.len())
    );
    for error in [
        root_errors
            .iter()
            .find(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .unwrap(),
        &child_errors[0],
    ] {
        assert_eq!(
            error.code,
            Some(NumberOrString::String("undefined-control-sequence".into()))
        );
        let text = Text::new(source.into());
        assert_eq!(
            &source[text.offset(error.range.start).unwrap()..text.offset(error.range.end).unwrap()],
            r"\bad"
        );
    }
}

#[test]
fn unresolved_traditional_pending_errors_do_not_inherit_explicit_stop_locations() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\input{missing}";
    std::fs::write(&main, source).unwrap();
    for stop in [
        "! Emergency stop.\nl.1 \\input{missing}",
        "./main.tex:1: Emergency stop.\nl.1 \\input{missing}",
        "",
    ] {
        let log = format!("(./main.tex\n(./gone.tex\n! I can't find file `missing.tex'.\n{stop}\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        let cause = errors
            .iter()
            .find(|d| d.message.contains("can't find"))
            .unwrap();
        assert_eq!(cause.code, None);
        assert_eq!(
            cause.data,
            Some(serde_json::json!({"unresolvedSource":true}))
        );
    }
}

#[test]
fn unknown_sources_cannot_reuse_parent_includes_or_extend_parent_box_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\include{missing}\nOther text.\n";
    std::fs::write(&main, source).unwrap();
    let log = concat!(
        "(./main.tex\n(./gone.tex\nNo file missing.tex.\n",
        "Overfull \\hbox (4.0pt too wide) in paragraph at lines 1--2\n)\n",
        "No file missing.tex.\n)\n",
    );
    let diagnostics = build_log::parse(&main, log);
    let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let box_warning = diagnostics
        .iter()
        .find(|d| d.message.starts_with("Overfull"))
        .unwrap();
    assert_eq!(box_warning.code, None);
    assert_eq!(
        box_warning.data,
        Some(serde_json::json!({"unresolvedSource":true}))
    );
    assert_eq!(box_warning.range.end.line, 0);
    let include = diagnostics
        .iter()
        .find(|d| d.message.starts_with("Included file"))
        .unwrap();
    assert_eq!(
        include.code,
        Some(NumberOrString::String("missing-include".into()))
    );
}

#[test]
fn unfinished_messages_stop_before_unavailable_source_entries() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\bad").unwrap();
    for message in ["LaTeX Warning: Unfinished warning", "! Unfinished error"] {
        let diagnostics = build_log::parse(&main, &format!("(./main.tex\n{message}\n(./gone.tex\n! Undefined control sequence.\nl.1 \\bad\n)\n)\n"));
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(diagnostics
            .iter()
            .any(|d| d.message == message.trim_start_matches("! ")));
        let unknown = diagnostics
            .iter()
            .find(|d| d.message == "Undefined control sequence.")
            .unwrap();
        assert_eq!(unknown.code, None);
        assert_eq!(
            unknown.data,
            Some(serde_json::json!({"unresolvedSource":true}))
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_deleted_child_sources_keep_traditional_errors_without_borrowing_the_root_line() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\\begin{document}\\input{child}\n\\bad\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(&child, "Text.\n\\bad\n").unwrap();
    let output = std::process::Command::new("pdflatex")
        .current_dir(directory.path())
        .args(["-interaction=nonstopmode", "-halt-on-error", "main.tex"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let log = std::fs::read_to_string(main.with_extension("log")).unwrap();
    let before = build_log::parse(&main, &log);
    assert!(
        before[&Url::from_file_path(&child).unwrap()]
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("undefined-control-sequence".into()))),
        "{before:?}"
    );
    std::fs::remove_file(&child).unwrap();
    let after = build_log::parse(&main, &log);
    let error = after[&Url::from_file_path(&main).unwrap()]
        .iter()
        .find(|d| d.message == "Undefined control sequence.")
        .unwrap();
    assert_eq!(error.code, None);
    assert_eq!(
        error.data,
        Some(serde_json::json!({"unresolvedSource":true}))
    );
    assert_eq!(error.range.start.line, 1);
    assert_eq!(error.range.start.character, 0);
    assert_eq!(error.range.end.character, 4);
}

#[test]
fn ordinary_parentheses_before_real_sources_do_not_hide_the_file_entry() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "\\bad").unwrap();
    std::fs::write(&child, "\\bad").unwrap();
    for prefix in ["(ordinary note) ", "(ordinary note ", "(note.texish) "] {
        let log = format!(
            "(./main.tex\n{prefix}(./child.tex\n! Undefined control sequence.\nl.1 \\bad\n)\n)\n"
        );
        let diagnostics = build_log::parse(&main, &log);
        let diagnostic = &diagnostics[&Url::from_file_path(&child).unwrap()][0];
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("undefined-control-sequence".into())),
            "{prefix}: {diagnostics:?}"
        );
    }
}

#[test]
fn wrapped_unavailable_explicit_paths_keep_their_barrier_until_the_file_closes() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\bad").unwrap();
    for name in [
        format!("./{}gone.tex", "missing-directory-".repeat(5)),
        format!("\"./missing {} (name).tex\"", "long name ".repeat(8)),
    ] {
        let log = format!("(./main.tex\n({}\n{}\n! Undefined control sequence.\nl.1 \\bad\n)\n! Undefined control sequence.\nl.1 \\bad\n)\n", &name[..70], &name[70..]);
        let diagnostics = build_log::parse(&main, &log);
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 2, "{name}: {diagnostics:?}");
        assert!(diagnostics
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("undefined-control-sequence".into()))));
        assert!(diagnostics
            .iter()
            .any(|d| d.code.is_none()
                && d.data == Some(serde_json::json!({"unresolvedSource":true}))));
    }
}
