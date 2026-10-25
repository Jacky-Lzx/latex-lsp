use latex_lsp::{
    compilation::{build, build_log},
    text::Text,
};
use lsp_types::{DiagnosticSeverity, NumberOrString, Url};

const HELP: &str = "\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \n";

#[test]
fn package_option_clashes_select_the_loading_name_instead_of_the_next_command() {
    for (source, shown, line, name, command, selected) in [
        (
            "\\usepackage[draft]{local}\n\\begin{document}",
            r"\begin",
            2,
            "local",
            "usepackage",
            "local",
        ),
        (
            r"\RequirePackage[draft]{other, local }[2020/01/01]",
            r"...ft]{other, local }[2020/01/01]",
            1,
            "local",
            "RequirePackage",
            "local",
        ),
        (
            "😀 \\usepackage[draft]{my \n local}\\begin{document}",
            r" local}\begin",
            2,
            "mylocal",
            "usepackage",
            "my \n local",
        ),
        (
            r"\usepackage[draft]{local.sty}\begin{document}",
            r"\usepackage[draft]{local.sty}\begin",
            1,
            "local.sty",
            "usepackage",
            "local.sty",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, source).unwrap();
        for header in ["!".to_owned(), format!("./main.tex:{line}:")] {
            let log = format!("(./main.tex\n{header} LaTeX Error: Option clash for package {name}.\n{HELP}l.{line} {shown}\n)\n");
            let entries = build_log::parse(&main, &log);
            let diagnostic = &entries[&Url::from_file_path(&main).unwrap()][0];
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("package-option-clash".into())),
                "{source}: {diagnostic:?}"
            );
            let text = Text::new(source.into());
            let start = source.find(selected).unwrap();
            assert_eq!(diagnostic.range, text.range(start, start + selected.len()));
            assert_eq!(
                diagnostic.data,
                Some(serde_json::json!({"package":name,"loaderCommand":command}))
            );
            assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        }
    }
}

#[test]
fn uncertain_option_clash_sources_names_and_contexts_retain_line_diagnostics() {
    for (source, name, context) in [
        (
            r"\usepackage[draft]{local,\dynamic}\begin{document}",
            "local",
            r"l.1 \usepackage[draft]{local,\dynamic}\begin",
        ),
        (
            r"\usepackage[draft]{local,local}\begin{document}",
            "local",
            r"l.1 \usepackage[draft]{local,local}\begin",
        ),
        (
            r"\usepackage[draft]{\name}\begin{document}",
            "local",
            r"l.1 \usepackage[draft]{\name}\begin",
        ),
        (
            r"\usepackage[draft]{other}\begin{document}",
            "local",
            r"l.1 \usepackage[draft]{other}\begin",
        ),
        (
            r"\usepackage[draft]{local}\begin{document}",
            "local",
            "l.1 ...",
        ),
        (
            r"\usepackage[draft]{local}\begin{document}",
            "local",
            "<argument> \\usepackage[draft]{local}\nl.1 \\usepackage[draft]{local}\\begin",
        ),
        (
            r"% \usepackage[draft]{local}\begin{document}",
            "local",
            r"l.1 % \usepackage[draft]{local}\begin",
        ),
        (
            r"\RequirePackageWithOptions{local}\begin{document}",
            "local",
            r"l.1 \RequirePackageWithOptions{local}\begin",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, source).unwrap();
        let log = format!(
            "(./main.tex\n! LaTeX Error: Option clash for package {name}.\n{HELP}{context}\n)\n"
        );
        let entries = build_log::parse(&main, &log);
        let diagnostic = &entries[&Url::from_file_path(&main).unwrap()][0];
        assert!(diagnostic.code.is_none(), "{source}: {diagnostic:?}");
        assert!(diagnostic.data.is_none(), "{source}: {diagnostic:?}");
    }
}

#[test]
fn option_clashes_do_not_borrow_unresolved_sources_or_invalid_reported_lines() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\usepackage[draft]{local}\\begin{document}\n").unwrap();
    for (file, line) in [("./missing.tex", 1), ("./main.tex", 0), ("./main.tex", 8)] {
        let log = format!("(./main.tex\n{file}:{line}: LaTeX Error: Option clash for package local.\n{HELP}l.{line} \\usepackage[draft]{{local}}\\begin\n)\n");
        let entries = build_log::parse(&main, &log);
        let diagnostic = &entries[&Url::from_file_path(&main).unwrap()][0];
        assert!(diagnostic.code.is_none(), "{log}: {diagnostic:?}");
    }
}

#[test]
fn repeated_option_clashes_on_one_line_keep_distinct_loading_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = r"😀 \usepackage[draft]{local}\usepackage[draft]{local}\relax";
    std::fs::write(&main, source).unwrap();
    let log = format!("(./main.tex\n./main.tex:1: LaTeX Error: Option clash for package local.\n{HELP}l.1 😀 \\usepackage[draft]{{local}}\\usepackage\n./main.tex:1: LaTeX Error: Option clash for package local.\n{HELP}l.1 ...{{local}}\\usepackage[draft]{{local}}\\relax\n)\n");
    let entries = build_log::parse(&main, &log);
    let diagnostics = &entries[&Url::from_file_path(&main).unwrap()];
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let text = Text::new(source.into());
    for (diagnostic, (start, _)) in diagnostics.iter().zip(source.match_indices("local")) {
        assert_eq!(diagnostic.range, text.range(start, start + 5));
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("package-option-clash".into()))
        );
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_option_clashes_point_to_the_conflicting_package_in_main_and_package_sources() {
    for engine in ["pdflatex", "xelatex"] {
        for package_source in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let loader = if package_source {
                "RequirePackage"
            } else {
                "usepackage"
            };
            let loads =
                format!("\\{loader}{{local}}\n\\{loader}[draft]{{other, local }}\n\\relax\n");
            std::fs::write(&main, if package_source {
                "\\documentclass{article}\n\\usepackage{wrapper}\n\\begin{document}Hello.\\end{document}\n".into()
            } else {
                format!("\\documentclass{{article}}\n{loads}\\begin{{document}}Hello.\\end{{document}}\n")
            }).unwrap();
            std::fs::write(root.join("wrapper.sty"), &loads).unwrap();
            for name in ["local.sty", "other.sty"] {
                std::fs::write(root.join(name), r"\DeclareOption*{}\ProcessOptions\relax").unwrap();
            }
            let outcome = build::run(
                &main,
                &latex_lsp::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                },
                &Default::default(),
            );
            assert_eq!(outcome.status, "error", "{}", outcome.output);
            let source = if package_source {
                root.join("wrapper.sty")
            } else {
                main.clone()
            };
            let uri = Url::from_file_path(&source).unwrap();
            let diagnostic = outcome.diagnostics[&uri]
                .iter()
                .find(|entry| {
                    entry.code == Some(NumberOrString::String("package-option-clash".into()))
                })
                .unwrap_or_else(|| {
                    panic!("{engine}: {}\n{:?}", outcome.output, outcome.diagnostics)
                });
            let text = Text::new(std::fs::read_to_string(&source).unwrap());
            let start = text.source.rfind("local").unwrap();
            assert_eq!(diagnostic.range, text.range(start, start + 5));
            assert_eq!(diagnostic.data.as_ref().unwrap()["loaderCommand"], loader);
            assert_eq!(diagnostic.data.as_ref().unwrap()["package"], "local");
        }
    }
}
