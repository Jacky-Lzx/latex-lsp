use latex_lsp::{
    build, features, file_operations, navigation, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
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
fn conditional_inputs_join_the_correct_project_before_compilation_and_allow_symbol_rename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main_source = "\\documentclass{article}\n\\InputIfFileExists{definitions}{}{Fallback.}\n\\begin{document}\\ref{sec:old}\\end{document}\n";
    let definitions = "\\newcommand{\\custom}{Text}\\label{sec:old}\n";
    for (name, source) in [
        ("main.tex", main_source),
        ("definitions.tex", definitions),
        (
            "other.tex",
            "\\documentclass{article}\\begin{document}\\label{sec:old}\\ref{sec:new}\\end{document}\n",
        ),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let child = uri(root, "definitions.tex");
    assert_eq!(workspace.dependencies(&main), vec![child.clone()]);
    assert_eq!(workspace.root_document(&child).unwrap(), main);
    assert!(!workspace
        .project_files(&main)
        .contains(&uri(root, "other.tex")));
    let position = workspace.documents[&main]
        .text
        .position(main_source.find("sec:old").unwrap() + 3);
    let locations = features::definitions(&workspace, &main, position);
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].uri, child);
    let changes = features::rename(&workspace, &main, position, "sec:new")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(
        apply(main_source, &changes[&main]),
        main_source.replace("sec:old", "sec:new")
    );
    assert_eq!(
        apply(definitions, &changes[&child]),
        definitions.replace("sec:old", "sec:new")
    );
}

#[test]
fn conditional_file_renames_preserve_hooks_comments_and_utf16_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "😀 \\InputIfFileExists % keep\r\n{chapters/my file} % before\r\n{\\typeout{my file}}{\\typeout{missing my file}}\r\n";
    std::fs::create_dir(root.join("chapters")).unwrap();
    std::fs::create_dir(root.join("renamed")).unwrap();
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("chapters/my file.tex"), "Chapter.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let old = uri(root, "chapters/my file.tex");
    let links = navigation::document_links(&workspace, &main);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, Some(old.clone()));
    let start = source.find("chapters/my file").unwrap();
    assert_eq!(
        links[0].range,
        workspace.documents[&main].text.range(start, start + 16)
    );
    let changes = file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: old.to_string(),
            new_uri: uri(root, "renamed/新 file.tex").to_string(),
        }],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[&main].len(), 1);
    assert_eq!(
        apply(source, &changes[&main]),
        source.replace("chapters/my file", "renamed/新 file")
    );
}

#[test]
fn conditional_input_file_renames_check_the_entire_batch_for_tex_suffix_shadows() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\InputIfFileExists{chapter}{Before.}{Missing.}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("chapter.tex"), "Intended.").unwrap();
    std::fs::write(root.join("renamed.ltx.tex"), "Decoy.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let rename = FileRename {
        old_uri: uri(root, "chapter.tex").to_string(),
        new_uri: uri(root, "renamed.ltx").to_string(),
    };
    assert!(
        file_operations::will_rename(&workspace, std::slice::from_ref(&rename))
            .unwrap_err()
            .message
            .contains("different file")
    );
    let changes = file_operations::will_rename(
        &workspace,
        &[
            rename,
            FileRename {
                old_uri: uri(root, "renamed.ltx.tex").to_string(),
                new_uri: uri(root, "moved.tex").to_string(),
            },
        ],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(
        apply(source, &changes[&uri(root, "main.tex")]),
        r"\InputIfFileExists{renamed.ltx}{Before.}{Missing.}"
    );
}

fn complete(workspace: &mut Workspace, main: &Url, marked: &str, label: &str) -> String {
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    workspace.open(main.clone(), source.clone(), 1);
    let position = Text::new(source.clone()).position(offset);
    let items = features::completion(workspace, main, position);
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    assert!(edit.range.start <= position && position <= edit.range.end);
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    apply(&source, &edits)
}

#[test]
fn conditional_input_completions_replace_the_filename_and_keep_both_code_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "main.tex",
        "chapter.tex",
        "config.def",
        "my file.tex",
        "part,one.tex",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (marked, label, expected) in [
        (
            r"\InputIfFileExists{cha¦pter}{Before.}{Missing.}",
            "chapter.tex",
            r"\InputIfFileExists{chapter}{Before.}{Missing.}",
        ),
        (
            "😀 \\InputIfFileExists % keep\r\n{co¦nfig}{Before.}{Missing.}",
            "config.def",
            "😀 \\InputIfFileExists % keep\r\n{config.def}{Before.}{Missing.}",
        ),
        (
            r"\InputIfFileExists{my ¦file}{Before.}{Missing.}",
            "my file.tex",
            r"\InputIfFileExists{my file}{Before.}{Missing.}",
        ),
        (
            r"\InputIfFileExists{part,¦one}{Before.}{Missing.}",
            "part,one.tex",
            r"\InputIfFileExists{part,one}{Before.}{Missing.}",
        ),
        (
            r"\InputIfFileExists{part¦,one}{Before.}{Missing.}",
            "part,one.tex",
            r"\InputIfFileExists{part,one}{Before.}{Missing.}",
        ),
        (
            r"\InputIfFileExists{part,¦",
            "part,one.tex",
            r"\InputIfFileExists{part,one",
        ),
        (r"\input{part¦,one}", "part,one.tex", r"\input{part,one}"),
    ] {
        assert_eq!(complete(&mut workspace, &main, marked, label), expected);
    }
    for marked in [
        r"\InputIfFileExists{chapter}{cha¦}{Missing.}",
        r"\InputIfFileExists{chapter}{Before.}{cha¦}",
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 2);
        assert!(
            features::completion(&workspace, &main, Text::new(source).position(at))
                .iter()
                .all(|item| item.kind != Some(CompletionItemKind::FILE))
        );
    }
}

#[test]
fn conditional_inputs_ignore_unexpanded_paths_and_unsupported_star_and_option_syntax() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = concat!(
        "% \\InputIfFileExists{child}{}{}\n",
        "\\verb|\\InputIfFileExists{child}{}{}|\n",
        "\\begin{verbatim}\\InputIfFileExists{child}{}{}\\end{verbatim}\n",
        "\\InputIfFileExists{\\dynamic}{}{}\n",
        "\\InputIfFileExists*{child}{}{}\n",
        "\\InputIfFileExists[options]{child}{}{}\n",
        "\\InputIfFileExists child {}{}\n",
        "\\InputIfFileExists{missing}{}{}\n",
        "\\IfFileExists{child}{}{}\n",
    );
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("child.tex"), "Child.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    assert!(workspace.dependencies(&main).is_empty());
    assert!(navigation::document_links(&workspace, &main).is_empty());
    for marked in [
        r"\InputIfFileExists*{ch¦ild}{}{}",
        r"\InputIfFileExists[options]{ch¦ild}{}{}",
        r"\InputIfFileExists{\dyn¦amic}{}{}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 1);
        assert!(
            features::completion(&workspace, &main, Text::new(source).position(offset))
                .iter()
                .all(|item| item.kind != Some(CompletionItemKind::FILE)),
            "{marked}"
        );
    }
}

#[test]
fn conditional_input_command_completion_and_signature_describe_the_pre_input_hook() {
    let main = Url::parse("file:///conditional-input/main.tex").unwrap();
    let mut workspace = Workspace {
        snippets: true,
        ..Default::default()
    };
    workspace.open(main.clone(), r"\InputIfFi".into(), 1);
    let items = features::completion(&workspace, &main, Position::new(0, 10));
    let item = items
        .iter()
        .find(|item| item.label == "InputIfFileExists")
        .unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(
        edit.new_text,
        "InputIfFileExists{${1:file}}{${2:before input}}{${3:if missing}}"
    );
    for (marked, active) in [
        (r"\InputIfFileExists{fi¦le}{Before.}{Missing.}", 0),
        (r"\InputIfFileExists{file}{Be¦fore.}{Missing.}", 1),
        (r"\InputIfFileExists{file}{Before.}{Mis¦sing.}", 2),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, source.clone(), Some(2)));
        let help = signature::help(&workspace, &main, Text::new(source).position(offset)).unwrap();
        assert_eq!(
            help.signatures[0].label,
            r"\InputIfFileExists{file}{before input}{if missing}"
        );
        assert_eq!(help.active_parameter, Some(active));
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_conditional_inputs_match_filename_selection_and_keep_hooks_after_file_rename() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for (name, content) in [
            ("plain", r"\typeout{CONDITIONAL-DECOY}"),
            ("plain.tex", r"\typeout{CONDITIONAL-READ:plain}Plain."),
            ("other.ltx", r"\typeout{CONDITIONAL-DECOY}"),
            ("other.ltx.tex", r"\typeout{CONDITIONAL-READ:other}Other."),
            ("config.sty", r"\typeout{CONDITIONAL-READ:config}Config."),
            ("config.sty.tex", r"\typeout{CONDITIONAL-DECOY}"),
            ("my file.tex", r"\typeout{CONDITIONAL-READ:spaced}Spaced."),
            ("part,one.tex", r"\typeout{CONDITIONAL-READ:comma}Comma."),
        ] {
            std::fs::write(root.join(name), content).unwrap();
        }
        let mut source = "\\documentclass{article}\\begin{document}\n".to_owned();
        for name in ["plain", "other.ltx", "config.sty", "my file", "part,one"] {
            source.push_str(&format!("\\InputIfFileExists{{{name}}}{{\\typeout{{CONDITIONAL-BEFORE:{name}}}}}{{\\typeout{{CONDITIONAL-WRONG-BRANCH}}}}\n"));
        }
        source.push_str(
            "\\InputIfFileExists{missing}{}{\\typeout{CONDITIONAL-MISSING}}\n\\end{document}\n",
        );
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: engine.into(),
            ..Default::default()
        };
        for renamed in [false, true] {
            let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            assert!(
                !outcome.output.contains("CONDITIONAL-DECOY"),
                "{}",
                outcome.output
            );
            assert!(!outcome.output.contains("CONDITIONAL-WRONG-BRANCH"));
            assert!(outcome.output.contains("CONDITIONAL-MISSING"));
            let inputs = outcome.inputs.unwrap();
            for name in [
                if renamed {
                    "renamed file.tex"
                } else {
                    "plain.tex"
                },
                "other.ltx.tex",
                "config.sty",
                "my file.tex",
                "part,one.tex",
            ] {
                assert!(inputs.contains(&uri(root, name)), "{name}: {inputs:?}");
            }
            assert!(
                outcome.output.find("CONDITIONAL-BEFORE:plain").unwrap()
                    < outcome.output.find("CONDITIONAL-READ:plain").unwrap()
            );
            if !renamed {
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                let main = uri(root, "main.tex");
                let links = navigation::document_links(&workspace, &main);
                assert_eq!(links.len(), 5, "{links:?}");
                for link in &links {
                    assert!(inputs.contains(link.target.as_ref().unwrap()));
                }
                let changes = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: uri(root, "plain.tex").to_string(),
                        new_uri: uri(root, "renamed file.tex").to_string(),
                    }],
                )
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
                assert_eq!(changes[&main].len(), 1);
                let updated = apply(&source, &changes[&main]);
                assert_eq!(updated, source.replacen("{plain}", "{renamed file}", 1));
                std::fs::write(root.join("main.tex"), updated).unwrap();
                std::fs::rename(root.join("plain.tex"), root.join("renamed file.tex")).unwrap();
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_conditional_child_errors_keep_the_loaded_source_and_missing_files_use_the_fallback() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("main.tex"), "\\documentclass{article}\\begin{document}\n\\InputIfFileExists{child}{}{\\typeout{CONDITIONAL-FALLBACK}Fallback.}\n\\end{document}\n").unwrap();
        let child = "Child.\n\\conditionalundefined\n";
        std::fs::write(root.join("child.tex"), child).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: engine.into(),
            ..Default::default()
        };
        let failed = build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(failed.status, "error");
        let diagnostic = failed.diagnostics[&uri(root, "child.tex")]
            .iter()
            .find(|item| item.message == "Undefined control sequence.")
            .unwrap();
        assert_eq!(
            diagnostic.range,
            Text::new(child.into()).range(child.find('\\').unwrap(), child.trim_end().len())
        );
        std::fs::remove_file(root.join("child.tex")).unwrap();
        let passed = build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(passed.status, "success", "{}", passed.output);
        assert!(passed.output.contains("CONDITIONAL-FALLBACK"));
        assert!(
            passed.diagnostics.values().all(Vec::is_empty),
            "{:?}",
            passed.diagnostics
        );
    }
}
