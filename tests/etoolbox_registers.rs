use latex_lsp::{
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;

fn project(source: &str) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\\begin{document}\\input{content}Witness.\\end{document}\n").unwrap();
    std::fs::write(root.join("definitions.tex"), "\\newlength{\\measure}\\newdimen\\distance\\newcount\\quantity\\newcounter{exercise}\\newtheorem{claim}{Claim}\\newcommand{\\plain}{Plain}\n").unwrap();
    std::fs::write(root.join("content.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let content = Url::from_file_path(root.join("content.tex")).unwrap();
    (directory, workspace, content)
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
fn named_register_checks_navigate_and_rename_the_operand_without_editing_branch_text() {
    for (command, name, replacement) in [
        ("ifcslength", "measure", "renamed"),
        ("ifcsdimen", "distance", "renamed"),
        ("ifcscounter", "quantity", "renamed"),
        ("ifcscounter", "c@exercise", "c@renamed"),
        ("ifcscounter", "c@claim", "c@renamed"),
        ("ifcslength", "plain", "renamed"),
    ] {
        let source = format!("😀 \\{command} % name\r\n{{{name}}}{{{name}}}{{{name}}}\n% \\{command}{{{name}}}{{Y}}{{N}}\n\\verb|\\{command}{{{name}}}{{Y}}{{N}}|\n");
        let (_directory, workspace, content) = project(&source);
        let doc = &workspace.documents[&content];
        let at = doc.text.position(source.find(name).unwrap() + 2);
        let found = features::definitions(&workspace, &content, at);
        assert_eq!(found.len(), 1, "{command}/{name}: {found:?}");
        assert!(found[0].uri.path().ends_with("/definitions.tex"));
        assert_eq!(
            features::references(&workspace, &content, at, false).len(),
            1
        );
        assert!(features::prepare_rename(&workspace, &content, at)
            .unwrap()
            .is_some());
        let changes = features::rename(&workspace, &content, at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(
            apply(&source, &changes[&content]),
            source.replacen(&format!("{{{name}}}"), &format!("{{{replacement}}}"), 1)
        );
    }
}

#[test]
fn register_interfaces_offer_signatures_snippets_and_name_completion() {
    for (marked, parameter) in [
        (r"\ifcslength{mea¦sure}{Y}{N}", 0),
        (r"\ifcsdimen{dis¦tance}{Y}{N}", 0),
        (r"\ifcscounter{c@ex¦ercise}{Y}{N}", 0),
        (r"\ifcslength{measure}{Y¦}{N}", 1),
        (r"\ifcsdimen{distance}{Y}{N¦}", 2),
        (r"\ifdeflength{\measure}{Y¦}{N}", 1),
        (r"\ifdefdimen\distance{Y}{N¦}", 2),
        (r"\ifdefcounter\quantity{Y¦}{N}", 1),
        (r"\deflength{\measure}{2pt+3¦pt}", 1),
        (r"\deflength\measure{2pt+3¦pt}", 1),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (_directory, workspace, content) = project(&source);
        let at = workspace.documents[&content].text.position(offset);
        let help = signature::help(&workspace, &content, at).unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.active_parameter, Some(parameter), "{marked}");
        if parameter == 0 {
            let expected = if marked.contains("ifcslength") {
                "measure"
            } else if marked.contains("ifcsdimen") {
                "distance"
            } else {
                "c@exercise"
            };
            let items = features::completion(&workspace, &content, at);
            assert!(
                items.iter().any(|item| item.label == expected),
                "{marked}: {items:?}"
            );
        }
    }
    for command in [
        "ifcslength",
        "ifcsdimen",
        "ifcscounter",
        "ifdeflength",
        "ifdefdimen",
        "ifdefcounter",
        "deflength",
    ] {
        let prefix = format!("\\{}", &command[..command.len() - 1]);
        let (directory, mut workspace, content) = project(&prefix);
        workspace.snippets = true;
        let items = features::completion(
            &workspace,
            &content,
            workspace.documents[&content].text.position(prefix.len()),
        );
        let item = items
            .iter()
            .find(|item| item.label == command)
            .unwrap_or_else(|| panic!("{command}"));
        assert!(item.documentation.is_some());
        assert!(
            item.insert_text
                .as_ref()
                .is_some_and(|text| text.contains("${1:"))
                || matches!(&item.text_edit, Some(CompletionTextEdit::Edit(edit)) if edit.new_text.contains("${1:"))
        );
        let definitions = Url::from_file_path(directory.path().join("definitions.tex")).unwrap();
        let doc = &workspace.documents[&definitions];
        assert!(features::rename(
            &workspace,
            &definitions,
            doc.text
                .position(doc.text.source.find("plain").unwrap() + 2),
            command
        )
        .is_err());
    }
}

#[test]
fn register_check_names_exclude_dynamic_operands_branch_text_and_opaque_content() {
    for marked in [
        r"\ifcslength{\prefix mea¦sure}{Y}{N}",
        r"\ifcsdimen{dis¦tance\suffix}{Y}{N}",
        r"\ifcscounter{c@ex¦ercise\suffix}{Y}{N}",
        r"\ifcslength*{mea¦sure}{Y}{N}",
        r"\ifcsdimen*{dis¦tance}{Y}{N}",
        r"\ifcscounter*{c@ex¦ercise}{Y}{N}",
        r"\ifcslength{missing}{mea¦sure}{N}",
        r"\ifcsdimen{missing}{Y}{dis¦tance}",
        r"\verb|\ifcslength{mea¦sure}{Y}{N}|",
        r"% \ifcscounter{c@ex¦ercise}{Y}{N}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (_directory, workspace, content) = project(&source);
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
                .any(|item| matches!(item.label.as_str(), "measure" | "distance" | "c@exercise")),
            "{marked}"
        );
    }
}

#[test]
fn shared_register_checks_cannot_rename_independent_declarations_in_different_projects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a.tex", "b.tex"] {
        std::fs::write(root.join(name), "\\documentclass{article}\\newlength{\\measure}\\begin{document}\\input{shared}\\end{document}\n").unwrap();
    }
    let source = r"\ifcslength{measure}{Yes}{No}";
    std::fs::write(root.join("shared.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let shared = Url::from_file_path(root.join("shared.tex")).unwrap();
    assert!(features::rename(
        &workspace,
        &shared,
        workspace.documents[&shared]
            .text
            .position(source.find("measure").unwrap() + 2),
        "renamed"
    )
    .is_err());
    let a = Url::from_file_path(root.join("a.tex")).unwrap();
    let doc = &workspace.documents[&a];
    assert!(features::rename(
        &workspace,
        &a,
        doc.text
            .position(doc.text.source.find("measure").unwrap() + 2),
        "renamed"
    )
    .is_err());
}

#[test]
fn direct_type_checks_inspect_the_operand_without_executing_declarations_or_verbatim() {
    for command in ["ifdeflength", "ifdefdimen", "ifdefcounter"] {
        for marked_operand in [r"\sect¦", r"{\sect¦}"] {
            let marked = format!("\\{command}{marked_operand}");
            let source = marked.replace('¦', "");
            let (directory, mut workspace, content) = project(&source);
            workspace.snippets = true;
            let items = features::completion(
                &workspace,
                &content,
                workspace.documents[&content]
                    .text
                    .position(marked.find('¦').unwrap()),
            );
            let item = items.iter().find(|item| item.label == "section").unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.new_text, "section", "{marked}");
            drop(directory);
        }
        for operand in [r"\newcommand", r"{\newcommand}"] {
            let source =
                format!("\\{command}{operand}{{\\ghost}}{{No}}\\newcommand{{\\actual}}{{Real}}\n");
            let (_directory, workspace, content) = project(&source);
            let doc = &workspace.documents[&content];
            assert!(
                !doc.syntax
                    .definitions
                    .iter()
                    .any(|entry| entry.name == "ghost"),
                "{source}"
            );
            assert!(
                doc.syntax
                    .definitions
                    .iter()
                    .any(|entry| entry.name == "actual"),
                "{source}"
            );
        }
        let source = format!("\\{command}\\verb{{Y}}{{N}}\\plain\n");
        let (_directory, workspace, content) = project(&source);
        let doc = &workspace.documents[&content];
        let at = doc.text.position(source.find(r"\plain").unwrap() + 2);
        assert_eq!(
            features::definitions(&workspace, &content, at).len(),
            1,
            "{source}"
        );
        let source = format!(
            "\\renewcommand{{\\{command}}}[1]{{#1}}\\{command}\\newcommand{{\\actual}}{{Real}}\n"
        );
        let (_directory, workspace, content) = project(&source);
        assert!(
            workspace.documents[&content]
                .syntax
                .definitions
                .iter()
                .any(|entry| entry.name == "actual"),
            "{source}"
        );
    }
}

#[test]
fn direct_register_interface_completion_keeps_existing_grouped_and_bare_operands() {
    for (command, argument) in [
        ("ifdeflength", "measure"),
        ("ifdefdimen", "distance"),
        ("ifdefcounter", "quantity"),
        ("deflength", "measure"),
    ] {
        for operand in [format!("\\{argument}"), format!("{{\\{argument}}}")] {
            let marked = format!(
                "\\{}¦{}{operand}{{Y}}{{N}}",
                &command[..command.len() - 1],
                &command[command.len() - 1..]
            );
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let (_directory, mut workspace, content) = project(&source);
            workspace.snippets = true;
            let items = features::completion(
                &workspace,
                &content,
                workspace.documents[&content].text.position(offset),
            );
            let item = items.iter().find(|item| item.label == command).unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                source,
                "{marked}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_register_renames_preserve_type_checks_and_local_and_global_glue_assignments() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let source = concat!(
        "\\deflength\\measure{5pt plus 2pt minus 1pt}\n",
        "\\typeout{REGISTER-AUDIT:outer:\\the\\measure}\n",
        "{\\deflength{\\measure}{\\measure*2+1pt}\\typeout{REGISTER-AUDIT:local:\\the\\measure}}\n",
        "\\typeout{REGISTER-AUDIT:restored:\\the\\measure}\n",
        "{\\global\\deflength\\measure{\\measure+2pt}}\n",
        "\\typeout{REGISTER-AUDIT:global:\\the\\measure}\n",
        "\\ifcslength{measure}{\\typeout{REGISTER-AUDIT:length:yes}}{\\typeout{REGISTER-AUDIT:length:no}}\n",
        "\\ifcsdimen{distance}{\\typeout{REGISTER-AUDIT:dimen:yes}}{\\typeout{REGISTER-AUDIT:dimen:no}}\n",
        "\\ifcscounter{quantity}{\\typeout{REGISTER-AUDIT:count:yes}}{\\typeout{REGISTER-AUDIT:count:no}}\n",
        "\\ifcscounter{c@exercise}{\\typeout{REGISTER-AUDIT:counter:yes}}{\\typeout{REGISTER-AUDIT:counter:no}}\n",
        "\\ifcscounter{c@claim}{\\typeout{REGISTER-AUDIT:theorem:yes}}{\\typeout{REGISTER-AUDIT:theorem:no}}\n",
        "\\ifdeflength\\measure{\\typeout{REGISTER-AUDIT:direct-length:yes}}{\\typeout{REGISTER-AUDIT:direct-length:no}}\n",
        "\\ifdefdimen{\\distance}{\\typeout{REGISTER-AUDIT:direct-dimen:yes}}{\\typeout{REGISTER-AUDIT:direct-dimen:no}}\n",
        "\\ifdefcounter\\quantity{\\typeout{REGISTER-AUDIT:direct-count:yes}}{\\typeout{REGISTER-AUDIT:direct-count:no}}\n",
        "\\ifcslength{plain}{\\typeout{REGISTER-AUDIT:macro:wrong}}{\\typeout{REGISTER-AUDIT:macro:ok}}\n",
        "\\ifcsdimen{measure}{\\typeout{REGISTER-AUDIT:skip:wrong}}{\\typeout{REGISTER-AUDIT:skip:ok}}\n",
        "\\ifcscounter{exercise}{\\typeout{REGISTER-AUDIT:base:wrong}}{\\typeout{REGISTER-AUDIT:base:ok}}\n",
        "\\ifcslength{missing}{\\typeout{REGISTER-AUDIT:missing:wrong}}{\\typeout{REGISTER-AUDIT:missing:ok}}\n",
        "\\ifdeflength\\newcommand{\\ghost}{\\typeout{REGISTER-AUDIT:inspect-length:ok}}\n",
        "\\ifdefdimen{\\newcommand}{\\ghost}{\\typeout{REGISTER-AUDIT:inspect-dimen:ok}}\n",
        "\\ifdefcounter\\verb{\\ghost}{\\typeout{REGISTER-AUDIT:inspect-count:ok}}\n",
        "measure distance quantity exercise claim\\label{measure}\n",
        "% \\ifcslength{measure}{Y}{N} \\deflength\\measure{9pt}\n",
        "\\verb|\\ifcslength{measure}{Y}{N} \\deflength\\measure{9pt}|\n",
    );
    let expected = [
        "REGISTER-AUDIT:outer:5.0pt plus 2.0pt minus 1.0pt",
        "REGISTER-AUDIT:local:11.0pt plus 4.0pt minus 2.0pt",
        "REGISTER-AUDIT:restored:5.0pt plus 2.0pt minus 1.0pt",
        "REGISTER-AUDIT:global:7.0pt plus 2.0pt minus 1.0pt",
        "REGISTER-AUDIT:length:yes",
        "REGISTER-AUDIT:dimen:yes",
        "REGISTER-AUDIT:count:yes",
        "REGISTER-AUDIT:counter:yes",
        "REGISTER-AUDIT:theorem:yes",
        "REGISTER-AUDIT:direct-length:yes",
        "REGISTER-AUDIT:direct-dimen:yes",
        "REGISTER-AUDIT:direct-count:yes",
        "REGISTER-AUDIT:macro:ok",
        "REGISTER-AUDIT:skip:ok",
        "REGISTER-AUDIT:base:ok",
        "REGISTER-AUDIT:missing:ok",
        "REGISTER-AUDIT:inspect-length:ok",
        "REGISTER-AUDIT:inspect-dimen:ok",
        "REGISTER-AUDIT:inspect-count:ok",
    ];
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let (directory, _, content) = project(source);
        let root = directory.path();
        let wrapper = root.join("engine-wrapper");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = BuildConfig {
            executable: wrapper.to_string_lossy().into_owned(),
            arguments: Some(vec![
                "-interaction=nonstopmode".into(),
                "-no-shell-escape".into(),
                "-file-line-error".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            ..Default::default()
        };
        for rename in [
            None,
            Some("measure"),
            Some("distance"),
            Some("quantity"),
            Some("exercise"),
            Some("claim"),
            Some("plain"),
        ] {
            if let Some(name) = rename {
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                let definitions = Url::from_file_path(root.join("definitions.tex")).unwrap();
                let doc = &workspace.documents[&definitions];
                let at = doc.text.position(doc.text.source.find(name).unwrap() + 2);
                let changes =
                    features::rename(&workspace, &definitions, at, &format!("renamed{name}"))
                        .unwrap()
                        .unwrap()
                        .changes
                        .unwrap();
                assert_eq!(changes.len(), 2, "{engine}/{name}");
                for (uri, edits) in changes {
                    std::fs::write(
                        uri.to_file_path().unwrap(),
                        apply(&workspace.documents[&uri].text.source, &edits),
                    )
                    .unwrap();
                }
            }
            let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{rename:?}: {}",
                outcome.output
            );
            assert!(outcome.pdf.is_some());
            let observed: Vec<_> = outcome
                .output
                .lines()
                .filter(|line| line.starts_with("REGISTER-AUDIT:"))
                .collect();
            assert_eq!(observed, expected, "{engine}/{rename:?}");
        }
        let changed = std::fs::read_to_string(content.to_file_path().unwrap()).unwrap();
        for retained in [
            "measure distance quantity exercise claim\\label{measure}",
            "% \\ifcslength{measure}{Y}{N} \\deflength\\measure{9pt}",
            "\\verb|\\ifcslength{measure}{Y}{N} \\deflength\\measure{9pt}|",
            "\\ifcscounter{exercise}",
        ] {
            assert!(changed.contains(retained), "{engine}/{retained}: {changed}");
        }
    }
}
