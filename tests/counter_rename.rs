use latex_lsp::{build, features, workspace::Workspace};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

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

fn renamed(workspace: &Workspace, document: &Url, position: Position) -> BTreeMap<Url, String> {
    features::rename(workspace, document, position, "problem")
        .unwrap()
        .unwrap()
        .changes
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
fn environment_and_counter_renames_preserve_distinct_names_with_semantic_whitespace() {
    for (source, name) in [
        (
            concat!(
                "\\newcounter{exercise}\n\\newcounter{ exercise }\n",
                "\\setcounter{exercise}{2}\\setcounter{ exercise }{7}\n",
                "\\value{exercise}\\value{ exercise }\\theexercise\n",
                "\\newcounter{inner}[ exercise ]\n",
                "\\newtheorem{lemma}[ exercise ]{Lemma}\n",
                "\\newtheorem{fact}{Fact}[ exercise ]\n",
                "\\newaliascnt{alias}{ exercise }\n",
                "\\counterwithin{inner}{ exercise }\n",
            ),
            "exercise",
        ),
        (
            concat!(
                "\\newenvironment{panel}{}{}\n\\newenvironment{ panel }{}{}\n",
                "\\begin{panel}Plain\\end{panel}\n",
                "\\begin{ panel }Spaced\\end{ panel }\n",
            ),
            "panel",
        ),
        (
            concat!(
                "\\newtheorem{claim}{Claim}\n\\newtheorem{ claim }{Spaced}\n",
                "\\begin{claim}Plain\\end{claim}\n",
                "\\begin{ claim }Spaced\\end{ claim }\n",
                "\\stepcounter{claim}\\stepcounter{ claim }\\theclaim\n",
            ),
            "claim",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(root, &[("main.tex", source)]);
        let main = uri(root, "main.tex");
        let text = &workspace.documents[&main].text;
        let position = text.position(source.find(name).unwrap() + 1);
        let locations = features::definitions(&workspace, &main, position);
        assert_eq!(locations.len(), 1, "{name}: {locations:?}");
        assert_eq!(
            renamed(&workspace, &main, position)[&main],
            source
                .replace(&format!("{{{name}}}"), "{problem}")
                .replace(&format!("\\the{name}"), "\\theproblem")
        );
        for (start, _) in source.match_indices(&format!(" {name} ")) {
            let position = text.position(start + 2);
            assert!(
                features::definitions(&workspace, &main, position).is_empty(),
                "{name}: {start}"
            );
            assert!(
                features::prepare_rename(&workspace, &main, position)
                    .unwrap()
                    .is_none(),
                "{name}: {start}"
            );
            assert!(
                features::rename(&workspace, &main, position, "problem")
                    .unwrap()
                    .is_none(),
                "{name}: {start}"
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_counter_and_environment_renames_keep_spaced_names_independent() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = concat!(
        "\\documentclass{article}\n",
        "\\newcounter{exercise}\n\\newcounter{ exercise }\n",
        "\\newenvironment{panel}{\\typeout{PLAIN-PANEL}}{}\n",
        "\\newenvironment{ panel }{\\typeout{SPACED-PANEL}}{}\n",
        "\\begin{document}\n",
        "\\setcounter{exercise}{2}\\setcounter{ exercise }{7}\n",
        "\\typeout{COUNTERS=\\arabic{exercise};\\arabic{ exercise }}\n",
        "\\begin{panel}Plain\\end{panel}\n",
        "\\begin{ panel }Spaced\\end{ panel }\n",
        "\\end{document}\n",
    )
    .to_owned();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    std::fs::write(root.join("main.tex"), &source).unwrap();
    let before = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
    assert_eq!(before.status, "success", "{}", before.output);
    for name in ["exercise", "panel"] {
        let workspace = project(root, &[("main.tex", &source)]);
        let main = uri(root, "main.tex");
        let position = workspace.documents[&main]
            .text
            .position(source.find(name).unwrap());
        let edit = features::rename(
            &workspace,
            &main,
            position,
            if name == "exercise" {
                "problem"
            } else {
                "card"
            },
        )
        .unwrap()
        .unwrap();
        let mut edits = edit.changes.unwrap()[&main].clone();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        source = workspace.documents[&main]
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
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        for witness in ["COUNTERS=2;7", "PLAIN-PANEL", "SPACED-PANEL"] {
            assert!(outcome.output.contains(witness), "{}", outcome.output);
        }
    }
}

#[test]
fn independent_counters_navigate_and_rename_relationships_operations_and_generated_macros() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newcounter{exercise}[section]\n\\newcounter{task}[exercise]\n\\newtheorem{lemma}[exercise]{Lemma}\n\\newtheorem{fact}{Fact}[exercise]\n\\newaliascnt{alias}{exercise}\n\\renewcommand{\\theexercise}{\\arabic{exercise}}\n\\providecommand{\\theHexercise}{\\arabic{exercise}}\n\\providecommand{\\exerciseautorefname}{Exercise}\n\\newcommand{\\exercise}{Independent macro}\n\\crefname{exercise}{exercise}{exercises}\n\\csname c@exercise\\endcsname\n";
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\setcounter{exercise}{2}\\addtocounter{exercise}{1}\\stepcounter{exercise}\\refstepcounter{exercise}\n\\value{exercise}\\Roman{exercise}\\theexercise\\theHexercise\n\\counterwithin{task}{exercise}\\counterwithout{task}{exercise}\\numberwithin{task}{exercise}\n\\exercise\n% \\value{exercise}\n\\verb|\\theexercise|\n\\end{document}\n";
    let workspace = project(
        root,
        &[("definitions.tex", definitions), ("main.tex", source)],
    );
    let definitions_uri = uri(root, "definitions.tex");
    let main = uri(root, "main.tex");
    let cases = [
        (definitions_uri.clone(), "\\newcounter{exercise}", 13),
        (definitions_uri.clone(), "\\csname c@exercise", 12),
        (main.clone(), "\\setcounter{exercise}", 14),
        (main.clone(), "\\theexercise", 2),
    ];
    for (document, marker, local) in cases {
        let text = &workspace.documents[&document].text;
        let position = text.position(text.source.find(marker).unwrap() + local);
        let locations = features::definitions(&workspace, &document, position);
        assert_eq!(locations.len(), 1, "{marker}: {locations:?}");
        assert_eq!(locations[0].uri, definitions_uri);
        assert!(features::hover(&workspace, &document, position).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &document, position).unwrap()
        else {
            panic!("missing prepared rename: {marker}")
        };
        assert_eq!(placeholder, "exercise");
        assert_eq!(
            &text.source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            "exercise"
        );
        let changed = renamed(&workspace, &document, position);
        assert_eq!(
            changed[&definitions_uri],
            definitions
                .replace("exercise", "problem")
                .replace(
                    "\\newcommand{\\problem}{Independent macro}",
                    "\\newcommand{\\exercise}{Independent macro}"
                )
                .replace(
                    "\\crefname{problem}{problem}{problems}",
                    "\\crefname{problem}{exercise}{exercises}"
                )
        );
        assert_eq!(
            changed[&main],
            source
                .replace("exercise", "problem")
                .replace("\\problem\n", "\\exercise\n")
                .replace("% \\value{problem}", "% \\value{exercise}")
                .replace("\\verb|\\theproblem|", "\\verb|\\theexercise|")
        );
    }
}

#[test]
fn counter_renames_reject_existing_counters_generated_commands_and_ambiguous_formatters() {
    for extra in [
        "\\newcounter{problem}",
        "\\newaliascnt{problem}{section}",
        "\\newcommand{\\theproblem}{X}",
        "\\newenvironment{theproblem}{}{}",
        "\\newtheorem{problem}{Problem}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\newcounter{{exercise}}\n{extra}\n");
        let workspace = project(root, &[("main.tex", &source)]);
        let main = uri(root, "main.tex");
        let position = workspace.documents[&main]
            .text
            .position(source.find("exercise").unwrap() + 2);
        assert!(
            features::rename(&workspace, &main, position, "problem").is_err(),
            "{extra}"
        );
        assert!(features::rename(&workspace, &main, position, "section").is_err());
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newcounter{exercise}\n\\theexercise\n";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("exercise").unwrap());
    assert!(features::rename(&workspace, &main, position, "problem-two").is_err());
    let source = "\\newcounter{exercise}\n\\newcounter{base}\n\\newtheorem{exercise}[base]{Exercise}\n\\value{exercise}\n";
    let workspace = project(root, &[("main.tex", source)]);
    let position = workspace.documents[&main]
        .text
        .position(source.find("exercise").unwrap());
    assert!(features::rename(&workspace, &main, position, "problem").is_err());
}

#[test]
fn generated_counter_macros_reserve_their_names_against_other_symbol_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        "\\newcounter{exercise}\n\\newcommand{\\oldmacro}{X}\n\\newenvironment{oldenv}{}{}\n";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    for old in ["oldmacro", "oldenv"] {
        let position = workspace.documents[&main]
            .text
            .position(source.find(old).unwrap() + 2);
        assert!(
            features::rename(&workspace, &main, position, "theexercise").is_err(),
            "{old}"
        );
        assert!(features::rename(&workspace, &main, position, "thesection").is_err());
    }
}

#[test]
fn counter_alias_renames_track_metadata_and_formatter_resets() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newcounter{base}\n\\newaliascnt{exercise}{base}\n\\newaliascnt{nested}{exercise}\n\\aliascntresetthe{exercise}\n\\renewcommand{\\theexercise}{\\arabic{exercise}}\n\\csname AC@cnt@exercise\\endcsname\n\\makeatletter\\AC@cnt@exercise\\makeatother\n";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    for marker in ["exercise}{base}", "exercise}\n\\renew", "AC@cnt@exercise"] {
        let position = text.position(source.find(marker).unwrap() + 2);
        assert_eq!(features::definitions(&workspace, &main, position).len(), 1);
        assert_eq!(
            renamed(&workspace, &main, position)[&main],
            source.replace("exercise", "problem")
        );
    }
    let conflict = format!("{source}\\newcommand{{\\AC@cnt@problem}}{{Conflicting metadata}}\n");
    let workspace = project(root, &[("main.tex", &conflict)]);
    let position = workspace.documents[&main]
        .text
        .position(conflict.find("exercise").unwrap());
    assert!(features::rename(&workspace, &main, position, "problem").is_err());
}

#[test]
fn unicode_counter_names_keep_utf16_ranges_and_project_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\n\\newcounter{练习-一}\n\\begin{document}😀 \\value  {练习-一}\\csname the练习-一\\endcsname\\end{document}\n";
    let unrelated = "\\documentclass{article}\n\\newcounter{练习-一}\n\\begin{document}\\value{练习-一}\\end{document}\n";
    let workspace = project(root, &[("main.tex", source), ("other.tex", unrelated)]);
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    let position = text.position(source.find("value  {").unwrap() + "value  {".len());
    let references = features::references(&workspace, &main, position, false);
    assert_eq!(references.len(), 2);
    assert!(references.iter().all(|location| location.uri == main));
    let changed = renamed(&workspace, &main, position);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[&main], source.replace("练习-一", "problem"));
    assert!(features::symbols(&workspace.documents[&main])
        .iter()
        .any(|symbol| { symbol.kind == SymbolKind::VARIABLE && symbol.name == "练习-一" }));
}

#[test]
fn counter_shared_occurrences_cannot_change_a_different_projects_definition() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root, &[
        ("a.tex", "\\documentclass{article}\n\\newcounter{exercise}\n\\begin{document}\\input{shared}\\end{document}"),
        ("b.tex", "\\documentclass{article}\n\\newcounter{exercise}\n\\begin{document}\\input{shared}\\end{document}"),
        ("shared.tex", "\\setcounter{exercise}{2}\\theexercise"),
    ]);
    let main = uri(root, "a.tex");
    let text = &workspace.documents[&main].text;
    let position = text.position(text.source.find("exercise").unwrap() + 2);
    assert!(features::rename(&workspace, &main, position, "problem").is_err());
}

#[test]
#[ignore = "requires pdflatex"]
fn real_counter_renames_preserve_values_shared_theorems_and_nested_reset_relationships() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newcounter{exercise}[section]\n\\newcounter{task}[exercise]\n\\renewcommand{\\theexercise}{\\thesection.\\arabic{exercise}}\n\\renewcommand{\\thetask}{\\theexercise.\\arabic{task}}\n\\newtheorem{lemma}[exercise]{Lemma}\n";
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\section{First}\n\\setcounter{exercise}{2}\\refstepcounter{exercise}\\label{first}\\theexercise\n\\begin{lemma}\\label{shared}Shared.\\end{lemma}\n\\refstepcounter{task}\\label{nested}\\thetask\n\\end{document}\n";
    let workspace = project(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main_path = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let first = build::run(&main_path, &config, &AtomicBool::new(false));
    assert_eq!(first.status, "success", "{}", first.output);
    let aux_path = config.artifact_path(&main_path, "aux").unwrap();
    let before = std::fs::read_to_string(&aux_path).unwrap();
    let definitions_uri = uri(root, "definitions.tex");
    let position = workspace.documents[&definitions_uri]
        .text
        .position(definitions.find("exercise").unwrap() + 2);
    for (uri, source) in renamed(&workspace, &definitions_uri, position) {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let second = build::run(&main_path, &config, &AtomicBool::new(false));
    assert_eq!(second.status, "success", "{}", second.output);
    assert_eq!(second.pages, Some(1));
    let after = std::fs::read_to_string(aux_path).unwrap();
    // LaTeX records the renamed counter in hyperlink destinations, while
    // printed label values and page numbers must stay identical.
    assert_eq!(before.replace("{exercise.", "{problem."), after);
    for expected in [
        "\\newlabel{first}{{1.3}",
        "\\newlabel{shared}{{1.4}",
        "\\newlabel{nested}{{1.4.1}",
    ] {
        assert!(after.contains(expected), "{after}");
    }
}

#[test]
#[ignore = "requires pdflatex and aliascnt"]
fn real_alias_counter_renames_preserve_shared_registers_and_internal_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newcounter{base}\n\\newaliascnt{exercise}{base}\n\\newaliascnt{child}{exercise}\n\\aliascntresetthe{exercise}\n";
    let source = "\\documentclass{article}\n\\usepackage{aliascnt}\n\\input{definitions}\n\\begin{document}\n\\setcounter{base}{2}\\refstepcounter{exercise}\\label{first}\\theexercise\n\\refstepcounter{child}\\label{second}\\thechild\n\\typeout{ALIAS-ROOT:\\csname AC@cnt@exercise\\endcsname}\n\\end{document}\n";
    let workspace = project(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let first = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(first.status, "success", "{}", first.output);
    let aux = config.artifact_path(&main, "aux").unwrap();
    let before = std::fs::read_to_string(&aux).unwrap();
    let document = uri(root, "definitions.tex");
    let position = workspace.documents[&document]
        .text
        .position(definitions.find("exercise").unwrap());
    for (uri, changed) in renamed(&workspace, &document, position) {
        std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
    }
    let second = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(second.status, "success", "{}", second.output);
    assert!(first.output.contains("ALIAS-ROOT:base"), "{}", first.output);
    assert!(
        second.output.contains("ALIAS-ROOT:base"),
        "{}",
        second.output
    );
    let after = std::fs::read_to_string(aux).unwrap();
    assert_eq!(before.replace("{exercise.", "{problem."), after);
    for label in ["\\newlabel{first}{{3}", "\\newlabel{second}{{4}"] {
        assert!(after.contains(label), "{after}");
    }
}
