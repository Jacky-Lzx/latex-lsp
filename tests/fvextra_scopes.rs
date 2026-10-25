use latex_lsp::{features, navigation, text::Text, workspace::Workspace};
use lsp_types::*;

const PHASES: [&str; 4] = ["inside", "inner", "afterinner", "afterouter"];

type SettingsCase = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
);

fn cases() -> [SettingsCase; 3] {
    [
        (
            "",
            r"\fvset{extra=false}",
            r"\fvinlineset{extra=true}",
            &["inside", "afterinner"],
        ),
        (
            r"\fvset{extra=false}",
            r"\fvset{extra=true}",
            r"\fvinlineset{extra=false}",
            &["inner", "afterouter"],
        ),
        (
            r"\fvinlineset{extra=false}",
            r"\fvinlineset{extra=true}",
            r"\fvset{extra=false}",
            &["afterouter"],
        ),
    ]
}

fn source(initial: &str, outer: &str, inner: &str, caller: &str, literal: &str) -> String {
    let code = |phase| {
        format!("\\textbf{{\\{caller}|\\label{{{phase}}}\\ref{{kept}}\\input{{{phase}}}|}}\n")
    };
    // Bare and commented environment names use the same scope boundaries.
    let (begin, end) = if caller == "Verb" {
        ("\\begin% name\r\n{quote}", "\\end{quote}")
    } else {
        ("\\begin q", "\\end q")
    };
    let raw = if literal.is_empty() {
        String::new()
    } else {
        format!("\\begin{{{literal}}}\n\\end{{quote}}\\end q\n\\fvset{{extra=true}}\\fvinlineset{{extra=false}}\n\\end{{{literal}}}\n")
    };
    format!(
        "\\documentclass{{article}}\n\\usepackage{{fvextra,listings,verbatim}}\n\\newenvironment{{q}}{{}}{{}}\n\\let\\code\\Verb\n{initial}\n\\begin{{document}}\n\\label{{kept}}\n{begin}\n{outer}\n{{\\fvset{{extra=true}}}}\\begingroup\\fvset{{extra=true}}\\endgroup\n{}\\begin{{center}}\n{inner}\n{}\\end{{center}}\n{raw}{}{end}\n{}\\label{{real}}\\ref{{kept}}\n\\end{{document}}\n",
        code("inside"), code("inner"), code("afterinner"), code("afterouter")
    )
}

fn check(source: &str, root: &std::path::Path, expected: &[&str]) -> String {
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    std::fs::write(root.join("main.tex"), source).unwrap();
    for phase in PHASES {
        std::fs::write(root.join(format!("{phase}.tex")), "Chapter text").unwrap();
    }
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 7);
    let doc = &workspace.documents[&main];
    assert!(
        doc.syntax.diagnostics.is_empty(),
        "{source}: {:?}",
        doc.syntax.diagnostics
    );
    let labels: Vec<_> = doc
        .syntax
        .labels
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    let mut names = vec!["kept"];
    names.extend_from_slice(expected);
    names.push("real");
    assert_eq!(labels, names, "{source}");
    let mut dependencies = workspace.dependencies(&main);
    dependencies.sort();
    let mut inputs: Vec<_> = expected
        .iter()
        .map(|name| Url::from_file_path(root.join(format!("{name}.tex"))).unwrap())
        .collect();
    inputs.sort();
    assert_eq!(dependencies, inputs, "{source}");
    assert_eq!(
        navigation::document_links(&workspace, &main).len(),
        expected.len(),
        "{source}"
    );
    let at = doc.text.position(source.rfind("kept").unwrap());
    assert_eq!(features::definitions(&workspace, &main, at).len(), 1);
    let edit = features::rename(&workspace, &main, at, "changed")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.version, Some(7));
    assert_eq!(documents[0].edits.len(), 2 + expected.len(), "{source}");
    let mut changes: Vec<_> = documents[0]
        .edits
        .iter()
        .map(|edit| {
            let OneOf::Left(edit) = edit else { panic!() };
            TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }
        })
        .collect();
    changes.sort_by_key(|change| std::cmp::Reverse(change.range.unwrap().start));
    let updated = Text::new(source.into()).changed(changes).unwrap().source;
    assert_eq!(
        updated.matches(r"\ref{kept}").count(),
        PHASES.len() - expected.len()
    );
    updated
}

#[test]
fn environment_settings_restore_inline_precedence_symbols_dependencies_and_rename() {
    for (initial, outer, inner, expected) in cases() {
        for caller in ["Verb", "code"] {
            for literal in [
                "",
                "Verbatim",
                "lstlisting",
                "comment",
                "VerbatimWrite",
                "VerbatimBuffer",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let source = source(initial, outer, inner, caller, literal);
                check(&source, directory.path(), expected);
                if caller == "Verb" && literal.is_empty() {
                    check(
                        &source.replace("{center}", "{quote}"),
                        directory.path(),
                        expected,
                    );
                }
            }
        }
    }
}

#[test]
fn malformed_environment_ends_recover_settings_without_discarding_outer_scopes() {
    let main = Url::parse("file:///fvextra-scopes/main.tex").unwrap();
    for ending in [r"\end{unknown}", "}"] {
        let source = format!("\\usepackage{{fvextra}}\\begin{{quote}}\\fvset{{extra=false}}{ending}\\textbf{{\\Verb|\\label{{inside}}|}}\\end{{quote}}\\textbf{{\\Verb|\\label{{phantom}}|}}\\label{{real}}");
        let mut workspace = Workspace::default();
        workspace.open(main.clone(), source.clone(), 7);
        let labels: Vec<_> = workspace.documents[&main]
            .syntax
            .labels
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(labels, ["inside", "real"], "{source}");
        assert!(!workspace.documents[&main].syntax.diagnostics.is_empty());
    }
    // Closing a known outer environment also recovers past an unfinished inner one.
    let source = r"\usepackage{fvextra}\begin{quote}\fvset{extra=false}\begin{center}\fvset{extra=true}\fvinlineset{extra=false}\end{quote}\textbf{\Verb|\label{phantom}|}\label{real}";
    let mut workspace = Workspace::default();
    workspace.open(main.clone(), source.into(), 7);
    assert_eq!(workspace.documents[&main].syntax.labels[0].name, "real");
    assert_eq!(workspace.documents[&main].syntax.labels.len(), 1);
    assert!(!workspace.documents[&main].syntax.diagnostics.is_empty());
}

#[test]
#[ignore = "requires pdflatex, xelatex, fvextra, listings, and verbatim"]
fn real_environment_settings_match_labels_and_inputs_before_and_after_rename() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    for engine in ["pdflatex", "xelatex"] {
        for (initial, outer, inner, expected) in cases() {
            for caller in ["Verb", "code"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = source(initial, outer, inner, caller, "Verbatim");
                let source = if caller == "Verb" {
                    source.replace("{center}", "{quote}")
                } else {
                    source
                };
                let updated = check(&source, root, expected);
                for (content, kept, absent) in
                    [(&source, "kept", "changed"), (&updated, "changed", "kept")]
                {
                    std::fs::write(root.join("main.tex"), content).unwrap();
                    let outcome = build::run(
                        &root.join("main.tex"),
                        &BuildConfig {
                            executable: engine.into(),
                            ..Default::default()
                        },
                        &AtomicBool::new(false),
                    );
                    assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert!(aux.contains(&format!("newlabel{{{kept}}}")), "{aux}");
                    assert!(!aux.contains(&format!("newlabel{{{absent}}}")), "{aux}");
                    let fls =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.fls")).unwrap();
                    for phase in PHASES {
                        assert_eq!(
                            aux.contains(&format!("newlabel{{{phase}}}")),
                            expected.contains(&phase),
                            "{engine}/{phase}: {aux}"
                        );
                        assert_eq!(
                            fls.lines().any(|line| line
                                .strip_prefix("INPUT ")
                                .is_some_and(|path| std::path::Path::new(path)
                                    .ends_with(format!("{phase}.tex")))),
                            expected.contains(&phase),
                            "{engine}/{phase}: {fls}"
                        );
                    }
                }
            }
        }
    }
}
