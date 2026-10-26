use lsp_types::*;
use texquill::{
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};

fn project(source: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n").unwrap();
    std::fs::write(root.join("definitions.tex"), "\\newcounter{exercise}\\newtheorem{claim}{Claim}\\newcounter{reserved}\n\\newcommand{\\exercise}{Independent}\\label{exercise}\n").unwrap();
    std::fs::write(root.join("content.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let content = Url::from_file_path(root.join("content.tex")).unwrap();
    let definitions = Url::from_file_path(root.join("definitions.tex")).unwrap();
    (directory, workspace, content, definitions)
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
        .source
}

#[test]
fn etoolbox_counter_assignments_and_tests_navigate_and_rename_across_files() {
    let source = concat!(
        "😀 \\defcounter % assignment\r\n{exercise}{2+3}\n",
        "\\ifltxcounter{exercise}{exercise}{exercise}\n",
        "\\defcounter{claim}{4}\\ifltxcounter{claim}{Claim}{Other}\n",
        "% \\defcounter{exercise}{9}\\ifltxcounter{claim}{A}{B}\n",
        "\\verb|\\defcounter{exercise}{9}\\ifltxcounter{claim}{A}{B}|\n",
    );
    let (_directory, workspace, content, definitions) = project(source);
    let doc = &workspace.documents[&content];
    for name in ["exercise", "claim"] {
        let at = doc
            .text
            .position(source.find(&format!("{{{name}}}")).unwrap() + 2);
        let found = features::definitions(&workspace, &content, at);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        assert_eq!(found[0].uri, definitions);
        assert_eq!(
            features::references(&workspace, &content, at, false).len(),
            2
        );
        let prepared = features::prepare_rename(&workspace, &content, at)
            .unwrap()
            .unwrap();
        assert!(
            matches!(prepared, PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } if placeholder == name)
        );
        let changed = features::rename(&workspace, &content, at, "problem")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changed.len(), 2);
        assert_eq!(
            apply(source, &changed[&content]),
            source.replacen(&format!("{{{name}}}"), "{problem}", 2)
        );
        let declaration = &workspace.documents[&definitions].text.source;
        assert_eq!(
            apply(declaration, &changed[&definitions]),
            declaration.replacen(&format!("{{{name}}}"), "{problem}", 1)
        );
        assert!(features::rename(&workspace, &content, at, "reserved").is_err());
    }
}

#[test]
fn etoolbox_counter_arguments_offer_name_completion_and_parameter_help() {
    for (marked, parameter) in [
        (r"\defcounter{ex¦ercise}{2+3}", 0),
        (r"\defcounter{exercise}{2+¦3}", 1),
        (r"\ifltxcounter{ex¦ercise}{Yes}{No}", 0),
        (r"\ifltxcounter{exercise}{Y¦es}{No}", 1),
        (r"\ifltxcounter{exercise}{Yes}{N¦o}", 2),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (_directory, workspace, content, _) = project(&source);
        let at = workspace.documents[&content].text.position(offset);
        let help = signature::help(&workspace, &content, at).unwrap();
        assert_eq!(help.active_parameter, Some(parameter), "{marked}");
        let items = features::completion(&workspace, &content, at);
        if parameter == 0 {
            let item = items
                .iter()
                .find(|item| item.label == "exercise")
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(apply(&source, std::slice::from_ref(edit)), source.clone());
        } else {
            assert!(
                !items.iter().any(|item| item.label == "exercise"),
                "{marked}"
            );
        }
    }
}

#[test]
fn unsupported_stars_dynamic_names_and_branch_text_are_not_counter_references() {
    for marked in [
        r"\defcounter*{ex¦ercise}{2+3}",
        r"\ifltxcounter*{ex¦ercise}{Yes}{No}",
        r"\setcounter*{ex¦ercise}{1}",
        r"\stepcounter*{ex¦ercise}",
        r"\value*{ex¦ercise}",
        r"\numberwithin*{ex¦ercise}{section}",
        r"\defcounter{\prefix ex¦ercise}{2+3}",
        r"\defcounter{section}{ex¦ercise}",
        r"\ifltxcounter{section}{ex¦ercise}{No}",
        r"\ifltxcounter{section}{Yes}{ex¦ercise}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (_directory, workspace, content, _) = project(&source);
        let at = workspace.documents[&content].text.position(offset);
        assert!(
            features::definitions(&workspace, &content, at).is_empty(),
            "{marked}"
        );
        assert!(
            features::prepare_rename(&workspace, &content, at)
                .unwrap()
                .is_none(),
            "{marked}"
        );
        assert!(
            !features::completion(&workspace, &content, at)
                .iter()
                .any(|item| item.label == "exercise"),
            "{marked}"
        );
    }
}

#[test]
fn etoolbox_counter_commands_offer_snippets_and_protect_their_command_names() {
    for (prefix, command) in [(r"\defcou", "defcounter"), (r"\ifltxcou", "ifltxcounter")] {
        let (_directory, mut workspace, content, definitions) = project(prefix);
        workspace.client.snippets = true;
        let at = workspace.documents[&content].text.position(prefix.len());
        let items = features::completion(&workspace, &content, at);
        let item = items.iter().find(|item| item.label == command).unwrap();
        assert!(item.documentation.is_some());
        assert!(
            item.insert_text
                .as_ref()
                .is_some_and(|text| text.contains("${1:counter}"))
                || matches!(&item.text_edit, Some(CompletionTextEdit::Edit(edit)) if edit.new_text.contains("${1:counter}"))
        );
        let declaration = &workspace.documents[&definitions].text;
        let position = declaration.position(declaration.source.find(r"\exercise").unwrap() + 2);
        assert!(features::rename(&workspace, &definitions, position, command).is_err());
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_etoolbox_counter_renames_preserve_local_global_and_existence_results() {
    use std::os::unix::fs::PermissionsExt;
    use texquill::{compilation::build, config::BuildConfig};

    let source = concat!(
        "% 😀\r\n\\defcounter % keep assignment\r\n{exercise}{2+3}\n",
        "\\typeout{COUNTER-AUDIT:outer:\\arabic{exercise}}\n",
        "{\\defcounter{exercise}{\\value{exercise}*2+1}\\typeout{COUNTER-AUDIT:local:\\arabic{exercise}}}\n",
        "\\typeout{COUNTER-AUDIT:restored:\\arabic{exercise}}\n",
        "{\\global\\defcounter{exercise}{\\value{exercise}+2}}\n",
        "\\typeout{COUNTER-AUDIT:global:\\arabic{exercise}}\n",
        "\\ifltxcounter{exercise}{\\typeout{COUNTER-AUDIT:exists:yes}}{\\typeout{COUNTER-AUDIT:exists:no}}\n",
        "\\ifltxcounter{missing}{\\typeout{COUNTER-AUDIT:missing:no}}{\\typeout{COUNTER-AUDIT:missing:yes}}\n",
        "\\defcounter{claim}{3+4}\\ifltxcounter{claim}{\\typeout{COUNTER-AUDIT:theorem:\\arabic{claim}}}{\\typeout{COUNTER-AUDIT:theorem:missing}}\n",
        "\\exercise\\label{exercise}\n",
        "% \\defcounter{exercise}{9}\\ifltxcounter{claim}{A}{B}\n",
        "\\verb|\\defcounter{exercise}{9}\\ifltxcounter{claim}{A}{B}|\n",
    );
    let expected = [
        "COUNTER-AUDIT:outer:5",
        "COUNTER-AUDIT:local:11",
        "COUNTER-AUDIT:restored:5",
        "COUNTER-AUDIT:global:7",
        "COUNTER-AUDIT:exists:yes",
        "COUNTER-AUDIT:missing:yes",
        "COUNTER-AUDIT:theorem:7",
    ];
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let (directory, _, content, _) = project(source);
        let root = directory.path();
        std::fs::write(root.join("main.tex"), "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n").unwrap();
        std::fs::write(root.join("definitions.tex"), "\\newcounter{exercise}\\newtheorem{claim}{Claim}\n\\newcommand{\\exercise}{Independent}\n").unwrap();
        let executable = root.join("engine-wrapper");
        std::fs::write(&executable, format!("#!/bin/sh\nexport TEXMFVAR=\"$1\"\nexport TEXMFCACHE=\"$1\"\nshift\nexec {engine} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = BuildConfig {
            executable: executable.to_string_lossy().into_owned(),
            arguments: Some(vec![
                root.join("tex-cache").to_string_lossy().into_owned(),
                "-interaction=nonstopmode".into(),
                "-no-shell-escape".into(),
                "-file-line-error".into(),
                "-recorder".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            ..Default::default()
        };
        let main = root.join("main.tex");
        for rename in [
            None,
            Some(("exercise", "task]")),
            Some(("claim", "proposition")),
        ] {
            if let Some((old, new)) = rename {
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                let doc = &workspace.documents[&content];
                let position = doc
                    .text
                    .position(doc.text.source.find(&format!("{{{old}}}")).unwrap() + 2);
                let changes = features::rename(&workspace, &content, position, new)
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes.len(), 2);
                for (uri, edits) in changes {
                    std::fs::write(
                        uri.to_file_path().unwrap(),
                        apply(&workspace.documents[&uri].text.source, &edits),
                    )
                    .unwrap();
                }
            }
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{rename:?}: {}",
                outcome.output
            );
            let observed: Vec<_> = outcome
                .output
                .lines()
                .filter(|line| line.starts_with("COUNTER-AUDIT:"))
                .collect();
            assert_eq!(observed, expected, "{engine}/{rename:?}");
            assert!(outcome.pdf.is_some());
        }
        let changed = std::fs::read_to_string(root.join("content.tex")).unwrap();
        assert!(changed.contains(r"\exercise\label{exercise}"));
        assert!(changed.contains("% \\defcounter{exercise}{9}\\ifltxcounter{claim}{A}{B}\n"));
        assert!(changed.contains(r"\verb|\defcounter{exercise}{9}\ifltxcounter{claim}{A}{B}|"));
    }
}

#[test]
fn explicit_completion_can_replace_a_spaced_counter_name_without_merging_its_identity() {
    let marked = r"\defcounter{ ex¦ercise }{2+3}";
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let (_directory, workspace, content, _) = project(&source);
    let at = workspace.documents[&content].text.position(offset);
    assert!(features::definitions(&workspace, &content, at).is_empty());
    let items = features::completion(&workspace, &content, at);
    let item = items.iter().find(|item| item.label == "exercise").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    assert_eq!(
        apply(&source, std::slice::from_ref(edit)),
        r"\defcounter{exercise}{2+3}"
    );
}

#[test]
fn semantic_tokens_mark_only_the_static_counter_operands() {
    use texquill::language::semantic;
    let source = r"\defcounter{exercise}{2+3}\ifltxcounter{exercise}{exercise}{exercise}\defcounter*{exercise}{1}\ifltxcounter*{exercise}{Y}{N}";
    let (_directory, workspace, content, _) = project(source);
    let doc = &workspace.documents[&content];
    let legend = semantic::options().legend;
    let mut position = Position::new(0, 0);
    let mut names = Vec::new();
    for token in semantic::tokens(doc, None).unwrap().data {
        position.line += token.delta_line;
        position.character = if token.delta_line == 0 {
            position.character + token.delta_start
        } else {
            token.delta_start
        };
        if legend.token_types[token.token_type as usize] == SemanticTokenType::VARIABLE {
            let start = doc.text.offset(position).unwrap();
            let end = doc
                .text
                .offset(Position::new(
                    position.line,
                    position.character + token.length,
                ))
                .unwrap();
            names.push(&source[start..end]);
        }
    }
    assert_eq!(names, ["exercise", "exercise"]);
}

#[test]
fn missing_counter_errors_locate_the_assignment_instead_of_a_same_line_existence_check() {
    use texquill::compilation::build_log;
    let source =
        "\\documentclass{article}\n\\ifltxcounter{equatoin}{Yes}{No}\\defcounter{equatoin}{2+3}\n";
    let (_directory, workspace, content, _) = project(source);
    let log = format!("(./content.tex\n{}:2: LaTeX Error: No counter 'equatoin' defined.\n\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...\n\nl.2 {}\n)\n", content.to_file_path().unwrap().display(), source.lines().nth(1).unwrap());
    let parsed = build_log::parse(&content.to_file_path().unwrap(), &log);
    let diagnostic = parsed[&content]
        .iter()
        .find(|item| item.severity == Some(DiagnosticSeverity::ERROR))
        .unwrap();
    let start = source.rfind("equatoin").unwrap();
    assert_eq!(
        diagnostic.range,
        workspace.documents[&content].text.range(start, start + 8)
    );
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("undefined-counter".into()))
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_missing_counter_assignments_offer_a_spelling_fix_and_compile_after_repair() {
    use std::os::unix::fs::PermissionsExt;
    use texquill::{compilation::build, config::BuildConfig, editing};

    let source = "\\ifltxcounter{equatoin}{Yes}{No}\\defcounter{equatoin}{2+3}\n";
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for numbered in [false, true] {
            let (directory, mut workspace, content, _) = project(source);
            let root = directory.path();
            let main = root.join("main.tex");
            std::fs::write(&main, "\\documentclass{article}\n\\usepackage{etoolbox}\n\\begin{document}\\input{content}Witness.\\end{document}\n").unwrap();
            let executable = root.join("engine-wrapper");
            std::fs::write(&executable, format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = BuildConfig {
                executable: executable.to_string_lossy().into_owned(),
                arguments: Some(vec![
                    "-interaction=nonstopmode".into(),
                    "-no-shell-escape".into(),
                    if numbered {
                        "-file-line-error"
                    } else {
                        "-no-file-line-error"
                    }
                    .into(),
                    "-output-directory=%o".into(),
                    "%f".into(),
                ]),
                ..Default::default()
            };
            let before = build::run(&main, &config, &Default::default());
            assert_eq!(
                before.status, "error",
                "{engine}/{numbered}: {}",
                before.output
            );
            let errors: Vec<_> = before.diagnostics[&content]
                .iter()
                .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
                .collect();
            assert_eq!(errors.len(), 1, "{engine}/{numbered}: {}", before.output);
            let diagnostic = errors[0].clone();
            let start = source.rfind("equatoin").unwrap();
            assert_eq!(
                diagnostic.range,
                workspace.documents[&content].text.range(start, start + 8)
            );
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("undefined-counter".into()))
            );
            workspace.client.code_action_literals = true;
            workspace
                .project
                .build_diagnostics
                .insert(Url::from_file_path(&main).unwrap(), before.diagnostics);
            let actions = editing::code_actions(
                &workspace,
                &content,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic],
                    only: Some(vec![CodeActionKind::QUICKFIX]),
                    trigger_kind: None,
                },
            );
            let edit = actions
                .into_iter()
                .find_map(|action| match action {
                    CodeActionOrCommand::CodeAction(action)
                        if action.title == "Replace with counter `equation`" =>
                    {
                        action.edit
                    }
                    _ => None,
                })
                .expect("missing counter spelling fix");
            let changes = edit.changes.unwrap();
            assert_eq!(changes.len(), 1);
            let repaired = apply(source, &changes[&content]);
            assert_eq!(
                repaired,
                "\\ifltxcounter{equatoin}{Yes}{No}\\defcounter{equation}{2+3}\n"
            );
            std::fs::write(content.to_file_path().unwrap(), repaired).unwrap();
            let after = build::run(&main, &config, &Default::default());
            assert_eq!(
                after.status, "success",
                "{engine}/{numbered}: {}",
                after.output
            );
            assert_eq!(after.pages, Some(1));
            assert!(after.pdf.is_some());
            assert!(!after
                .diagnostics
                .values()
                .flatten()
                .any(|diagnostic| diagnostic.code
                    == Some(NumberOrString::String("undefined-counter".into()))));
        }
    }
}
