use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{compilation::build, language::features, project::workspace::Workspace};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn renamed(workspace: &Workspace, root: &Path, old: &str, new: &str) -> BTreeMap<Url, String> {
    let declarations = uri(root, "definitions.tex");
    let document = &workspace.documents[&declarations];
    let source = &document.text;
    let offset = document
        .syntax
        .environments
        .iter()
        .find(|entry| entry.name == old)
        .unwrap()
        .span
        .start;
    let edit = features::rename(workspace, &declarations, source.position(offset), new)
        .unwrap()
        .unwrap();
    edit.changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let source = workspace.documents[&uri]
                .text
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
            (uri, source)
        })
        .collect()
}

#[test]
fn theorem_renames_update_counter_relationships_generated_commands_and_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newtheorem{claim}{Claim}[section]\n\\newtheorem{lemma}[claim]{Lemma}\n\\newtheorem{fact}{Fact}[claim]\n\\newcounter{inner}[claim]\n\\newaliascnt{alias}{claim}\n\\renewcommand{\\theclaim}{\\arabic{claim}}\n\\providecommand{\\theHclaim}{\\arabic{claim}}\n\\providecommand{\\claimautorefname}{Claim}\n\\crefname{claim}{claim}{claims}\n\\Crefname{claim}{Claim}{Claims}\n\\csname theclaim\\endcsname\n\\csname c@claim\\endcsname\n";
    let main = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\setcounter{claim}{2}\n\\addtocounter{claim}{1}\n\\stepcounter{claim}\n\\refstepcounter{claim}\n\\value{claim} \\Roman{claim} \\theclaim \\theHclaim\n\\counterwithin{inner}{claim}\n\\counterwithout{inner}{claim}\n\\numberwithin{claim}{section}\n\\begin{claim}Body.\\end{claim}\n\\claim Direct.\\endclaim\n% \\setcounter{claim}{9}\n\\verb|\\theclaim|\n\\end{document}\n";
    let workspace = project(
        root,
        &[("main.tex", main), ("definitions.tex", definitions)],
    );
    let changed = renamed(&workspace, root, "claim", "proposition");
    let definitions = &changed[&uri(root, "definitions.tex")];
    for expected in [
        "\\newtheorem{proposition}{Claim}[section]",
        "\\newtheorem{lemma}[proposition]{Lemma}",
        "\\newtheorem{fact}{Fact}[proposition]",
        "\\newcounter{inner}[proposition]",
        "\\newaliascnt{alias}{proposition}",
        "\\theproposition",
        "\\theHproposition",
        "\\propositionautorefname",
        "\\crefname{proposition}",
        "\\Crefname{proposition}",
        "\\csname theproposition\\endcsname",
        "\\csname c@proposition\\endcsname",
    ] {
        assert!(
            definitions.contains(expected),
            "missing {expected}: {definitions}"
        );
    }
    let main = &changed[&uri(root, "main.tex")];
    for expected in [
        "\\setcounter{proposition}{2}",
        "\\addtocounter{proposition}",
        "\\stepcounter{proposition}",
        "\\refstepcounter{proposition}",
        "\\value{proposition}",
        "\\Roman{proposition}",
        "\\theproposition",
        "\\theHproposition",
        "\\counterwithin{inner}{proposition}",
        "\\counterwithout{inner}{proposition}",
        "\\numberwithin{proposition}{section}",
        "\\begin{proposition}",
        "\\end{proposition}",
        "\\proposition Direct.\\endproposition",
    ] {
        assert!(main.contains(expected), "missing {expected}: {main}");
    }
    assert!(main.contains("% \\setcounter{claim}{9}"));
    assert!(main.contains("\\verb|\\theclaim|"));
    // Starting at a counter operation, a formatting macro, or a literal
    // csname must resolve to the same theorem declaration and edits.
    for (file, marker, local) in [
        ("main.tex", "\\setcounter{claim}", 13),
        ("main.tex", "\\theclaim", 2),
        ("definitions.tex", "\\csname c@claim\\endcsname", 11),
    ] {
        let document_uri = uri(root, file);
        let text = &workspace.documents[&document_uri].text;
        let position = text.position(text.source.find(marker).unwrap() + local);
        let definitions = features::definitions(&workspace, &document_uri, position);
        assert_eq!(definitions.len(), 1, "{marker}: {definitions:?}");
        assert_eq!(definitions[0].uri, uri(root, "definitions.tex"));
        let edit = features::rename(&workspace, &document_uri, position, "proposition")
            .unwrap()
            .unwrap();
        for (uri, mut edits) in edit.changes.unwrap() {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let changed_source = workspace.documents[&uri]
                .text
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
            assert_eq!(changed_source, changed[&uri], "{marker}");
        }
    }
}

#[test]
fn shared_and_unnumbered_theorems_preserve_independently_declared_counters() {
    for (declaration, use_text, expected, unchanged) in [
        (
            "\\newtheorem{lemma}[claim]{Lemma}",
            "\\lemma Body.\\endlemma \\thelemma \\value{lemma}",
            "\\observation Body.\\endobservation \\theobservation \\value{lemma}",
            "\\newtheorem{observation}[claim]{Lemma}",
        ),
        (
            "\\newtheorem*{lemma}{Lemma}",
            "\\lemma Body.\\endlemma \\thelemma \\value{lemma}",
            "\\observation Body.\\endobservation \\thelemma \\value{lemma}",
            "\\newtheorem*{observation}{Lemma}",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("{declaration}\n\\newcounter{{lemma}}\n");
        let main = format!("\\documentclass{{article}}\n\\input{{definitions}}\n\\begin{{document}}\n{use_text}\n\\end{{document}}\n");
        let workspace = project(
            root,
            &[("main.tex", &main), ("definitions.tex", &definitions)],
        );
        let changed = renamed(&workspace, root, "lemma", "observation");
        assert!(
            changed[&uri(root, "main.tex")].contains(expected),
            "{changed:?}"
        );
        assert!(changed[&uri(root, "definitions.tex")].contains(unchanged));
        assert!(changed[&uri(root, "definitions.tex")].contains("\\newcounter{lemma}"));
    }
}

#[test]
fn theorem_renames_reject_counter_and_generated_macro_collisions() {
    for extra in [
        "\\newcounter{proposition}",
        "\\newaliascnt{proposition}{equation}",
        "\\newcommand{\\theproposition}{X}",
        "\\newenvironment{theproposition}{}{}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("\\newtheorem{{claim}}{{Claim}}\n{extra}\n");
        let workspace = project(root, &[("definitions.tex", &definitions)]);
        assert!(
            features::rename(
                &workspace,
                &uri(root, "definitions.tex"),
                Position::new(0, 13),
                "proposition"
            )
            .is_err(),
            "{extra}"
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(
        root,
        &[(
            "definitions.tex",
            "\\newtheorem{claim}{Claim}\n\\theclaim\n",
        )],
    );
    for new in ["page", "claim-two", "中文"] {
        assert!(
            features::rename(
                &workspace,
                &uri(root, "definitions.tex"),
                Position::new(0, 13),
                new
            )
            .is_err(),
            "{new}"
        );
    }
}

#[test]
fn shared_counter_references_cannot_change_a_different_projects_counter() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in ["a", "b"] {
        std::fs::create_dir(root.join(folder)).unwrap();
    }
    let workspace = project(root, &[
        ("shared.tex", "\\setcounter{claim}{1}\n"),
        ("a/main.tex", "\\documentclass{article}\n\\newtheorem{claim}{Claim}\n\\begin{document}\n\\input{../shared}\n\\begin{claim}A.\\end{claim}\n\\end{document}\n"),
        ("b/main.tex", "\\documentclass{article}\n\\newcounter{claim}\n\\begin{document}\n\\input{../shared}\nB.\\end{document}\n"),
    ]);
    let position = Position::new(1, 13);
    let a = uri(root, "a/main.tex");
    assert!(features::prepare_rename(&workspace, &a, position).is_err());
    assert!(features::rename(&workspace, &a, position, "proposition").is_err());
}

#[test]
#[ignore = "requires latexmk and amsmath"]
fn numbered_theorem_renames_preserve_real_shared_and_nested_counter_numbering() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newtheorem{claim}{Claim}\n\\newtheorem{lemma}[claim]{Lemma}\n\\newtheorem{fact}{Fact}[claim]\n\\newcounter{inner}[claim]\n\\renewcommand{\\theclaim}{\\arabic{claim}}\n";
    let main = "\\documentclass{article}\n\\usepackage{amsmath}\n\\input{definitions}\n\\begin{document}\n\\setcounter{claim}{2}\n\\begin{claim}\\label{first}First.\\end{claim}\n\\begin{lemma}\\label{shared}Shared.\\end{lemma}\n\\begin{fact}\\label{nested}Nested.\\end{fact}\n\\numberwithin{inner}{claim}\n\\claim\\label{direct}Direct.\\endclaim\n\\theclaim\n\\end{document}\n";
    let workspace = project(
        root,
        &[("main.tex", main), ("definitions.tex", definitions)],
    );
    let before = build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(before.status, "success", "{}", before.output);
    let expected_aux = std::fs::read_to_string(root.join("build/texquill/main.aux")).unwrap();
    for (key, number) in [
        ("first", "3"),
        ("shared", "4"),
        ("nested", "4.1"),
        ("direct", "5"),
    ] {
        assert!(
            expected_aux.contains(&format!("\\newlabel{{{key}}}{{{{{number}}}")),
            "{expected_aux}"
        );
    }
    for (uri, source) in renamed(&workspace, root, "claim", "proposition") {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let after = build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(
        std::fs::read_to_string(root.join("build/texquill/main.aux")).unwrap(),
        expected_aux
    );
    assert!(!after
        .diagnostics
        .values()
        .flatten()
        .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR)));
}

#[test]
#[ignore = "requires latexmk and amsthm"]
fn shared_and_starred_theorem_renames_compile_without_changing_independent_counters() {
    for (old, new) in [("lemma", "observation"), ("remark", "commentary")] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = "\\newcounter{lemma}\n\\newcounter{remark}\n\\newtheorem{claim}{Claim}\n\\newtheorem{lemma}[claim]{Lemma}\n\\newtheorem*{remark}{Remark}\n";
        let main = "\\documentclass{article}\n\\usepackage{amsthm}\n\\input{definitions}\n\\begin{document}\n\\setcounter{claim}{2}\n\\setcounter{lemma}{7}\n\\setcounter{remark}{9}\n\\begin{lemma}\\label{shared}Shared.\\end{lemma}\n\\thelemma\n\\remark Unnumbered.\\endremark\n\\theremark\n\\typeout{INDEPENDENT LEMMA \\arabic{lemma}}\n\\typeout{INDEPENDENT REMARK \\arabic{remark}}\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", main), ("definitions.tex", definitions)],
        );
        let before = build::run(
            &root.join("main.tex"),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(before.status, "success", "{}", before.output);
        let auxiliary = root.join("build/texquill/main.aux");
        let labels = std::fs::read_to_string(&auxiliary).unwrap();
        for (uri, source) in renamed(&workspace, root, old, new) {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let after = build::run(
            &root.join("main.tex"),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(after.status, "success", "{}", after.output);
        assert_eq!(std::fs::read_to_string(&auxiliary).unwrap(), labels);
        assert!(
            after.output.contains("INDEPENDENT LEMMA 7"),
            "{}",
            after.output
        );
        assert!(
            after.output.contains("INDEPENDENT REMARK 9"),
            "{}",
            after.output
        );
    }
}
