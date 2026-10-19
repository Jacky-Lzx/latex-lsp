use latex_lsp::{build, build_log, text::Text};
use std::path::Path;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Url};

const HELP: &str = "\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \n";
const SOURCE: &str = r"😀 \usepackage[draft]{local}\relax";

fn details(name: &str, loaded: &str, requested: &str) -> String {
    format!("The package {name} has already been loaded with options:\n  [{loaded}]\nThere has now been an attempt to load it with options\n  [{requested}]\nAdding the global options:\n")
}

fn log(name: &str, prefix: &str, help: &str, numbered: bool) -> String {
    let header = if numbered { "./main.tex:1:" } else { "!" };
    format!("(./main.tex\n{header} LaTeX Error: Option clash for package {name}.\n{HELP}l.1 {prefix}\n                                                  \n{help}\n)\n")
}

fn parse(root: &Path, source: &str, contents: &str) -> Vec<Diagnostic> {
    let main = root.join("main.tex");
    std::fs::write(&main, source).unwrap();
    build_log::parse(&main, contents)[&Url::from_file_path(&main).unwrap()].clone()
}

fn check_details(diagnostic: &Diagnostic, loaded: &str, requested: &str) {
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("package-option-clash".into()))
    );
    let data = diagnostic.data.as_ref().unwrap();
    assert_eq!(data["loadedOptions"], loaded, "{diagnostic:?}");
    assert_eq!(data["requestedOptions"], requested, "{diagnostic:?}");
    assert!(
        diagnostic
            .message
            .contains(&format!("Already loaded options: [{loaded}]")),
        "{diagnostic:?}"
    );
    assert!(
        diagnostic
            .message
            .contains(&format!("Requested options: [{requested}]")),
        "{diagnostic:?}"
    );
}

#[test]
fn option_clash_details_show_reported_strings_and_preserve_precise_ranges() {
    let directory = tempfile::tempdir().unwrap();
    for numbered in [false, true] {
        for (loaded, requested) in [
            ("", "draft"),
            ("key={a[b]c},  color=blue ", "key={b]c}, flag, 中文=值"),
            ("key=]", "key=[x]"),
            (
                "note={Warning: prior}",
                "note={Package local Warning: text}",
            ),
        ] {
            let entries = parse(
                directory.path(),
                SOURCE,
                &log(
                    "local",
                    SOURCE,
                    &details("local", loaded, requested),
                    numbered,
                ),
            );
            assert_eq!(entries.len(), 1);
            check_details(&entries[0], loaded, requested);
            assert!(entries[0]
                .message
                .starts_with("LaTeX Error: Option clash for package local.\n"));
            assert_eq!(entries[0].data.as_ref().unwrap()["package"], "local");
            assert_eq!(
                entries[0].data.as_ref().unwrap()["loaderCommand"],
                "usepackage"
            );
            let start = SOURCE.find("local").unwrap();
            assert_eq!(
                entries[0].range,
                Text::new(SOURCE.into()).range(start, start + 5)
            );
        }
    }
}

fn wrapped(value: &str, width: usize) -> String {
    let mut result = String::new();
    for line in value.lines() {
        let mut bytes = 0;
        for ch in line.chars() {
            if bytes + ch.len_utf8() > width {
                result.push('\n');
                bytes = 0;
            }
            result.push(ch);
            bytes += ch.len_utf8();
        }
        result.push('\n');
    }
    result
}

#[test]
fn wrapped_help_headers_option_values_and_delimiters_keep_original_characters() {
    let directory = tempfile::tempdir().unwrap();
    let name = format!("local{}", "x".repeat(90));
    let loaded = "shape={a[b]c},long-option=中文value, another=[one]".repeat(4);
    let requested = "new=value,brackets={[]},extra=数字".repeat(4);
    let source = format!("\\usepackage[draft]{{{name}}}\\relax");
    for width in [17, 72, 79] {
        let help = wrapped(&details(&name, &loaded, &requested), width);
        let entries = parse(directory.path(), &source, &log(&name, &source, &help, true));
        check_details(&entries[0], &loaded, &requested);
    }
}

#[test]
fn incomplete_mismatched_or_interrupted_help_does_not_invent_option_details() {
    let directory = tempfile::tempdir().unwrap();
    let valid = details("local", "old", "new");
    for help in [
        details("other", "old", "new"),
        valid.replace("[old]", "[old"),
        valid.replace("[new]", "[new"),
        valid.replace("Adding the global options:", ""),
        valid.replace(
            "There has now been an attempt",
            "Unexpected text\nThere has now been an attempt",
        ),
        valid.replace("  [new]", "! New error.\n  [new]"),
        valid.replace("  [new]", "(./unknown.tex\n  [new]"),
        valid.replace(
            "  [new]",
            "[latex-lsp: middle of compiler output omitted]\n  [new]",
        ),
        format!("Unrelated output\n{valid}"),
        details("local", &"x".repeat(8192), "new"),
        details("local", &"x".repeat(4097), "new"),
        wrapped(&valid, 1),
    ] {
        let entries = parse(directory.path(), SOURCE, &log("local", SOURCE, &help, true));
        let diagnostic = entries
            .iter()
            .find(|entry| entry.message.starts_with("LaTeX Error: Option clash"))
            .unwrap();
        assert_eq!(
            diagnostic.message, "LaTeX Error: Option clash for package local.",
            "{help}"
        );
        assert!(
            diagnostic
                .data
                .as_ref()
                .unwrap()
                .get("loadedOptions")
                .is_none(),
            "{help}"
        );
        assert!(
            diagnostic
                .data
                .as_ref()
                .unwrap()
                .get("requestedOptions")
                .is_none(),
            "{help}"
        );
    }
}

#[test]
fn option_details_require_the_verified_source_and_clashing_load_name() {
    let directory = tempfile::tempdir().unwrap();
    for source in [
        r"\usepackage{\name}\relax",
        r"\usepackage{local,\dynamic}\relax",
        r"% \usepackage{local}\relax",
    ] {
        let entries = parse(
            directory.path(),
            source,
            &log("local", source, &details("local", "old", "new"), true),
        );
        assert!(entries[0].code.is_none(), "{:?}", entries[0]);
        assert_eq!(
            entries[0].message,
            "LaTeX Error: Option clash for package local."
        );
        assert!(entries[0].data.is_none());
    }
    for header in ["./missing.tex:1:", "./main.tex:0:", "./main.tex:9:"] {
        let contents = log("local", SOURCE, &details("local", "old", "new"), true)
            .replace("./main.tex:1:", header);
        let entries = parse(directory.path(), SOURCE, &contents);
        assert!(entries[0].code.is_none());
        assert_eq!(
            entries[0].message,
            "LaTeX Error: Option clash for package local."
        );
        assert!(entries[0]
            .data
            .as_ref()
            .unwrap()
            .get("loadedOptions")
            .is_none());
    }
}

#[test]
fn adjacent_clashes_keep_their_own_details_and_cannot_borrow_a_later_help_block() {
    let directory = tempfile::tempdir().unwrap();
    let source = r"\usepackage[a]{local}\usepackage[b]{local}\relax";
    let first = log(
        "local",
        r"\usepackage[a]{local}\usepackage",
        &details("local", "original", "a"),
        true,
    );
    let second = log("local", source, &details("local", "original,a", "b"), true);
    for first_help in [true, false] {
        let first = if first_help {
            first.clone()
        } else {
            first.replace(&details("local", "original", "a"), "")
        };
        let entries = parse(directory.path(), source, &(first + &second));
        assert_eq!(entries.len(), 2, "{entries:?}");
        if first_help {
            check_details(&entries[0], "original", "a");
        } else {
            assert!(entries[0]
                .data
                .as_ref()
                .unwrap()
                .get("loadedOptions")
                .is_none());
        }
        check_details(&entries[1], "original,a", "b");
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_long_package_option_clashes_report_loaded_and_requested_values() {
    for engine in ["pdflatex", "xelatex"] {
        for package_source in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let name = format!("local{}", "x".repeat(72));
            let loaded = (0..8)
                .map(|i| format!("loaded{i}=value{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let requested = (0..8)
                .map(|i| format!("requested{i}=value{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let command = if package_source {
                "RequirePackage"
            } else {
                "usepackage"
            };
            let loads = format!(
                "\\{command}[{loaded}]{{{name}}}\n\\{command}[{requested}]{{{name}}}\n\\relax\n"
            );
            std::fs::write(
                root.join(format!("{name}.sty")),
                r"\DeclareOption*{}\ProcessOptions\relax",
            )
            .unwrap();
            std::fs::write(root.join("wrapper.sty"), &loads).unwrap();
            std::fs::write(root.join("main.tex"), if package_source {
                "\\documentclass{article}\n\\usepackage{wrapper}\n\\begin{document}Hello.\\end{document}\n".into()
            } else { format!("\\documentclass{{article}}\n{loads}\\begin{{document}}Hello.\\end{{document}}\n") }).unwrap();
            let outcome = build::run(
                &root.join("main.tex"),
                &latex_lsp::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                },
                &Default::default(),
            );
            assert_eq!(outcome.status, "error", "{}", outcome.output);
            let path = root.join(if package_source {
                "wrapper.sty"
            } else {
                "main.tex"
            });
            let entries = &outcome.diagnostics[&Url::from_file_path(&path).unwrap()];
            let diagnostic = entries
                .iter()
                .find(|entry| {
                    entry.code == Some(NumberOrString::String("package-option-clash".into()))
                })
                .unwrap();
            check_details(diagnostic, &loaded, &requested);
            assert_eq!(diagnostic.data.as_ref().unwrap()["package"], name);
            let text = Text::new(std::fs::read_to_string(path).unwrap());
            let start = text.source.rfind(&name).unwrap();
            assert_eq!(diagnostic.range, text.range(start, start + name.len()));
        }
    }
}
