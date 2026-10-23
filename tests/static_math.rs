use latex_lsp::{config::Config, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn enabled() -> Config {
    Config::parse(
        serde_json::json!({"distribution":{"enabled":false},"diagnostics":{"mathDelimiters":true}}),
    )
    .unwrap()
}
fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:Math.tex").unwrap();
    let mut ws = Workspace {
        config: enabled(),
        ..Workspace::default()
    };
    ws.open(uri.clone(), source.into(), 1);
    (ws, uri)
}
fn math(ws: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    ws.diagnostics(uri).into_iter().filter(|item| {
        item.source.as_deref() == Some("latex-lsp") && matches!(item.code.as_ref(),
            Some(NumberOrString::String(code)) if ["unclosed-math", "nested-math", "mismatched-math", "unexpected-math-end"].contains(&code.as_str()))
    }).collect()
}
fn code(item: &Diagnostic) -> &str {
    match item.code.as_ref().unwrap() {
        NumberOrString::String(code) => code,
        _ => panic!(),
    }
}

#[test]
fn reports_missing_mismatched_nested_and_unexpected_delimiters_with_utf16_locations() {
    for (source, expected, marker) in [
        ("😀 $x", "unclosed-math", "$"),
        ("😀 $$x", "unclosed-math", "$$"),
        ("😀 \\(x", "unclosed-math", "\\("),
        ("😀 \\[x", "unclosed-math", "\\["),
        (r"\textbf{$x}", "unclosed-math", "$"),
        ("😀 \\(x\\]", "mismatched-math", "\\]"),
        ("😀 \\[x\\)", "mismatched-math", "\\)"),
        ("😀 \\) ", "unexpected-math-end", "\\)"),
        ("😀 \\] ", "unexpected-math-end", "\\]"),
        ("😀 \\(x\\(y\\)", "nested-math", "\\("),
    ] {
        let (ws, uri) = setup(source);
        let items = math(&ws, &uri);
        assert_eq!(items.len(), 1, "{source}: {items:?}");
        assert_eq!(code(&items[0]), expected);
        let at = if expected == "nested-math" {
            source.rfind(marker).unwrap()
        } else {
            source.find(marker).unwrap()
        };
        assert_eq!(
            items[0].range,
            Text::new(source.into()).range(at, at + marker.len())
        );
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::WARNING));
        if expected == "mismatched-math" || expected == "nested-math" {
            assert_eq!(
                items[0].related_information.as_ref().unwrap()[0]
                    .location
                    .uri,
                uri
            );
        }
    }
}

#[test]
fn accepts_equivalent_tex_delimiters_groups_text_boxes_and_implicit_math() {
    for source in [
        "$x$",
        "$$x$$",
        r"\(x\)",
        r"\[x\]",
        r"$x\)",
        r"\(x$",
        r"$$x\]",
        r"\[x$$",
        r"${x+y}$",
        r"$x\futurelet\thing\relax$",
        r"$x+\mbox{$y$}$",
        r"$$x+\mbox{$y$}$$",
        r"$$x+\hbox to 20pt{$y$}$$",
        r"\(x+\text{\(y\)}\)",
        r"\ensuremath{x+y}",
        r"\begin{equation}x+\text{$y$}\end{equation}",
        r"\begin{align}x &= y \\ z &= \text{$q$}\end{align}",
    ] {
        let (ws, uri) = setup(source);
        assert!(
            math(&ws, &uri).is_empty(),
            "{source}: {:?}",
            math(&ws, &uri)
        );
    }
}

#[test]
fn ignores_comments_verbatim_literal_data_and_token_operands() {
    for source in [
        "% $ \\(\n\\verb|$ \\]|\n\\begin{verbatim}\n$ \\(\n\\end{verbatim}",
        r"\detokenize{$\(}\string$\meaning\[",
        r"\let\dollar=$",
        r"\label{cash$}\ref{cash$}\cite{cash$}\includegraphics{cash$.pdf}",
        r"\csname cash$\endcsname",
        r"\def\!$#1{$}\renewcommand{\!}[1][$]{$}",
        r"\newcommand{\cash}{$",
        r"\def\cash$#1{$",
        r"\newenvironment{cash}{$}{$",
        r"\NewDocumentCommand{\cash}{O{$}m}{$",
        r"\def\foo$#1{#1}\newcommand{\bad}[1][$]{\(}\newenvironment{odd}{$}{\]}",
        r"\ExplSyntaxOn\cs_new:Npn \cash:w $#1 {#1}\tl_const:Nn \c_cash_tl {$}\ExplSyntaxOff",
        r"\newcommand{\foo}{\begin{equation}} $bad",
    ] {
        let (ws, uri) = setup(source);
        let items = math(&ws, &uri);
        let expected = if source.ends_with("$bad") { 1 } else { 0 };
        assert_eq!(items.len(), expected, "{source}: {items:?}");
    }
}

#[test]
fn defaults_off_and_configuration_or_edits_clear_results() {
    let (mut ws, uri) = setup("$x");
    assert_eq!(math(&ws, &uri).len(), 1);
    ws.open(uri.clone(), "$x$".into(), 2);
    assert!(math(&ws, &uri).is_empty());
    ws.open(uri.clone(), "$x".into(), 3);
    ws.config = Config::default();
    assert!(math(&ws, &uri).is_empty());
    ws.config = enabled();
    assert_eq!(math(&ws, &uri).len(), 1);
    let bib = Url::parse("untitled:Math.bib").unwrap();
    ws.open(bib.clone(), "@book{k,title={$x}}".into(), 1);
    assert!(math(&ws, &bib).is_empty());
}

#[test]
fn bounds_diagnostics_and_ignores_catcode_or_delimiter_redefinitions() {
    let (ws, uri) = setup(&r"\) ".repeat(1000));
    assert_eq!(math(&ws, &uri).len(), 128);
    for source in [
        r"\catcode`\$=12 $",
        r"\def\({literal} \(x",
        r"\let\]=\relax \]x",
    ] {
        let (ws, uri) = setup(source);
        assert!(math(&ws, &uri).is_empty(), "{source}");
    }
}

#[test]
fn input_boundaries_and_shared_files_do_not_invent_local_math_errors() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.tex");
    let child = dir.path().join("child.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\\begin{document}$x\\input{child}\\end{document}",
    )
    .unwrap();
    std::fs::write(&child, "y$ \\) \\(z\\]").unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    let mut ws = Workspace {
        config: enabled(),
        ..Workspace::default()
    };
    ws.add_root(dir.path().into());
    ws.open(uri.clone(), std::fs::read_to_string(main).unwrap(), 1);
    ws.load_dependencies(&uri, 0);
    assert!(math(&ws, &uri).is_empty(), "{:?}", math(&ws, &uri));
    let items = math(&ws, &child_uri);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(code(&items[0]), "mismatched-math");
    ws.open(
        uri.clone(),
        r"\def\readfile{\input}\(x\readfile{child}y\]".into(),
        2,
    );
    ws.load_dependencies(&uri, 0);
    assert!(math(&ws, &uri).is_empty(), "{:?}", math(&ws, &uri));
}

#[test]
fn captured_input_meanings_are_boundaries_without_asserting_dependencies() {
    for definition in [
        r"\let\readfile\input",
        r"\NewCommandCopy{\readfile}{\input}",
    ] {
        let (ws, uri) = setup(&format!("{definition}\\(x\\readfile{{child}}y\\]"));
        assert!(
            math(&ws, &uri).is_empty(),
            "{definition}: {:?}",
            math(&ws, &uri)
        );
    }
}

#[test]
fn literal_named_inputs_keep_name_data_opaque_and_reset_the_surrounding_mode() {
    for input in [
        r"\csname input\endcsname{child}",
        r"\csname readfile\endcsname{child}",
        r"\csuse{readfile}{child}",
    ] {
        let source = format!("\\def\\readfile{{\\input}}\\(x{input}y\\] \\(z\\]");
        let (ws, uri) = setup(&source);
        let items = math(&ws, &uri);
        assert_eq!(items.len(), 1, "{input}: {items:?}");
        assert_eq!(code(&items[0]), "mismatched-math");
        let close = source.rfind("\\]").unwrap();
        assert_eq!(
            items[0].range,
            Text::new(source.clone()).range(close, close + 2)
        );
    }
}

#[test]
fn directory_settings_enable_checks_and_imported_redefinitions_disable_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("main.tex");
    let local = dir.path().join("local.sty");
    let uri = Url::from_file_path(&root).unwrap();
    let local_uri = Url::from_file_path(&local).unwrap();
    std::fs::write(
        dir.path().join(".latex-lsp.json"),
        r#"{"diagnostics":{"mathDelimiters":true}}"#,
    )
    .unwrap();
    std::fs::write(
        &root,
        "\\documentclass{article}\\usepackage{local}\\begin{document}$x\\end{document}",
    )
    .unwrap();
    std::fs::write(&local, "% No changes").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    ws.load_dependencies(&uri, 0);
    assert_eq!(math(&ws, &uri).len(), 1);
    ws.open(local_uri, r"\let\(=\relax".into(), 2);
    assert!(math(&ws, &uri).is_empty());
}

#[test]
fn a_file_consumed_by_multiple_projects_uses_every_input_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("child.tex");
    std::fs::write(&child, r"\(x\readfile{leaf}\] \(z\]").unwrap();
    std::fs::write(dir.path().join("leaf.tex"), "data").unwrap();
    for (name, definition) in [
        ("a", r"\def\readfile{\input}"),
        ("b", r"\newcommand{\readfile}[1]{#1}"),
    ] {
        std::fs::write(dir.path().join(format!("{name}.tex")), format!("\\documentclass{{article}}{definition}\\begin{{document}}\\input{{child}}\\end{{document}}")).unwrap();
    }
    let mut ws = Workspace {
        config: enabled(),
        ..Workspace::default()
    };
    ws.add_root(dir.path().into());
    for name in ["a", "b"] {
        ws.load_dependencies(
            &Url::from_file_path(dir.path().join(format!("{name}.tex"))).unwrap(),
            0,
        );
    }
    let uri = Url::from_file_path(child).unwrap();
    let items = math(&ws, &uri);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(code(&items[0]), "mismatched-math");
    assert_eq!(items[0].range.start.character, 24);
}

#[test]
#[ignore = "requires pdflatex and amsmath"]
fn real_tex_accepts_mixed_spellings_and_data_and_rejects_reported_errors() {
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace {
        config: enabled(),
        ..Workspace::default()
    };
    for (body, valid) in [
        (r"$x\)", true),
        (r"\(x$", true),
        (r"$$x\]", true),
        (r"\[x$$", true),
        (r"$x+\mbox{$y$}$", true),
        (r"$$x+\mbox{$y$}$$", true),
        (r"$$x+\hbox to 20pt{$y$}$$", true),
        (r"\(x+\text{$y$}\)", true),
        (r"\def\foo$#1{#1}\csname cash$\endcsname", true),
        (
            r"\ExplSyntaxOn\cs_new:Npn \cash:w $#1 {#1}\tl_const:Nn \c_cash_tl {$}\ExplSyntaxOff",
            true,
        ),
        (r"\(x\]", false),
        (r"\[x\)", false),
        (r"\)", false),
        (r"\(x\(y\)", false),
        (r"$x", false),
        (r"$$x", false),
    ] {
        let package = if body.contains("\\text{") {
            "\\usepackage{amsmath}"
        } else {
            ""
        };
        let source = format!("\\documentclass{{article}}\n{package}\n\\begin{{document}}\n{body}\n\\end{{document}}\n");
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        ws.open(uri.clone(), source, 1);
        assert_eq!(
            math(&ws, &uri).is_empty(),
            valid,
            "{body}: {:?}",
            math(&ws, &uri)
        );
        let output = std::process::Command::new("pdflatex")
            .current_dir(dir.path())
            .args(["-interaction=nonstopmode", "-halt-on-error", "main.tex"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            valid,
            "{body}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}
