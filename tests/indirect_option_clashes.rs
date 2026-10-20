use latex_lsp::{build_log, text::Text};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Url};

fn call(form: &str, loader: &str) -> String {
    match form {
        "named" => format!("\\csuse{{{loader}}}"),
        "literal" => format!("\\csname {loader}\\endcsname"),
        "alias" => r"\loadpkg".into(),
        "named-alias" => r"\csuse{loadpkg}".into(),
        "literal-alias" => r"\csname loadpkg\endcsname".into(),
        _ => panic!(),
    }
}

fn report(line: usize, echo: &str, name: &str, numbered: bool) -> String {
    let header = if numbered {
        format!("./main.tex:{line}:")
    } else {
        "!".into()
    };
    format!("{header} LaTeX Error: Option clash for package {name}.\n\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \nl.{line} {echo}\n                                                  \nThe package {name} has already been loaded with options:\n  [draft]\nThere has now been an attempt to load it with options\n  [final]\nAdding the global options:\n")
}

fn parse(source: &str, log: &str) -> Vec<Diagnostic> {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    build_log::parse(&main, &format!("(./main.tex\n{log})\n"))[&Url::from_file_path(main).unwrap()]
        .clone()
}

#[test]
fn indirect_option_clashes_select_static_names_and_retain_reported_option_details() {
    for loader in ["usepackage", "RequirePackage"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for (argument, selected, name, version) in [
                ("{local}", "local", "local", false),
                ("{other,local}", "local", "local", false),
                (
                    "{% keep 😀\r\nother, local% keep\r\n}",
                    "local",
                    "local",
                    false,
                ),
                ("{sub/local}[2000/01/01]", "sub/local", "sub/local", true),
                ("{local.sty}", "local.sty", "local.sty", false),
                ("{my \n local}", "my \n local", "mylocal", false),
            ] {
                let source = format!(
                    "\\def\\loadpkg{{\\{loader}}}\n{}[final]{argument}\\relax\n",
                    call(form, loader)
                );
                let echo = source.lines().last().unwrap();
                let echo = if version {
                    echo.strip_suffix(r"\relax").unwrap()
                } else {
                    echo
                };
                for numbered in [false, true] {
                    let diagnostics = parse(
                        &source,
                        &report(source.lines().count(), echo, name, numbered),
                    );
                    assert_eq!(
                        diagnostics.len(),
                        1,
                        "{form}/{loader}/{argument}: {diagnostics:?}"
                    );
                    let diagnostic = &diagnostics[0];
                    assert_eq!(
                        diagnostic.code,
                        Some(NumberOrString::String("package-option-clash".into())),
                        "{form}/{loader}/{argument}: {diagnostic:?}"
                    );
                    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
                    let start = source.rfind(selected).unwrap();
                    assert_eq!(
                        diagnostic.range,
                        Text::new(source.clone()).range(start, start + selected.len())
                    );
                    assert_eq!(
                        diagnostic.data,
                        Some(
                            serde_json::json!({"package":name,"loaderCommand":loader,"loadedOptions":"draft","requestedOptions":"final"})
                        )
                    );
                    assert!(diagnostic
                        .message
                        .ends_with("Already loaded options: [draft]\nRequested options: [final]"));
                }
            }
        }
    }
}

#[test]
fn indirect_option_clashes_require_executed_unambiguous_static_loading_calls() {
    for source in [
        r"\newcommand{\later}{\csuse{usepackage}[final]{local}\relax}",
        r"\newcommand{\hold}[1]{}\hold\loadpkg[final]{local}\relax",
        r"\renewcommand{\usepackage}[1]{}\loadpkg[final]{local}\relax",
        r"\renewcommand{\csuse}[1]{}\csuse{usepackage}[final]{local}\relax",
        r"\csuse{usepackage}[final]{\filename}\relax",
        r"\csuse{usepackage}[final]{local,\filename}\relax",
        r"\csuse{usepackage}[final]{local,local}\relax",
        r"\csname \loader\endcsname[final]{local}\relax",
        r"\endinput\loadpkg[final]{local}\relax",
        r"\loadpkg[final]{local}\relax\loadpkg[final]{local}\relax",
        r"\loadpkg[final]{local}[broken",
        r"\csuse{RequirePackageWithOptions}{local}\relax",
        r"\def\usepackage#1{}\usepackage[final]{local}\relax",
        r"% \loadpkg[final]{local}\relax",
    ] {
        let full = format!("\\def\\loadpkg{{\\usepackage}}\n{source}\n");
        let echo = if source.ends_with(r"\relax\loadpkg[final]{local}\relax") {
            r"...\loadpkg[final]{local}\relax"
        } else {
            source
        };
        let diagnostics = parse(&full, &report(2, echo, "local", true));
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].code.is_none(),
            "{source}: {:?}",
            diagnostics[0]
        );
        assert!(
            diagnostics[0].data.is_none(),
            "{source}: {:?}",
            diagnostics[0]
        );
        assert_eq!(
            diagnostics[0].message,
            "LaTeX Error: Option clash for package local."
        );
    }
}

#[test]
fn indirect_option_clashes_can_report_the_following_line_without_losing_the_filename() {
    for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
        let source = format!(
            "\\def\\loadpkg{{\\usepackage}}\n{}[final]{{local}}\n\\begin{{document}}\n",
            call(form, "usepackage")
        );
        let diagnostic = parse(&source, &report(3, r"\begin", "local", true)).remove(0);
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("package-option-clash".into())),
            "{form}: {diagnostic:?}"
        );
        let start = source.find("local").unwrap();
        assert_eq!(diagnostic.range, Text::new(source).range(start, start + 5));
        assert_eq!(
            diagnostic.data.as_ref().unwrap()["requestedOptions"],
            "final"
        );
    }
}
