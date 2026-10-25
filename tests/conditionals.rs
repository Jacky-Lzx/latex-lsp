use latex_lsp::{build, features, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

fn setup(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn renamed(workspace: &Workspace, document: &Url, position: Position) -> BTreeMap<Url, String> {
    features::rename(workspace, document, position, "release")
        .unwrap()
        .unwrap()
        .changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let changed = workspace.documents[&uri]
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
                .unwrap();
            (uri, changed.source)
        })
        .collect()
}

#[test]
fn conditionals_navigate_rename_and_complete_predicates_and_both_setters() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newif\\ifdraft\n\\draftfalse\n\\newcommand{\\draft}{Independent.}\n";
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n😀 \\drafttrue\\ifdraft Yes.\\else No.\\fi\n\\csname draftfalse\\endcsname\\csname ifdraft\\endcsname No.\\else Yes.\\fi\n\\draft\n% \\drafttrue\n\\verb|\\ifdraft|\n\\end{document}\n";
    let workspace = setup(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let declaration = Url::from_file_path(root.join("definitions.tex")).unwrap();
    for (uri, marker, local) in [
        (&declaration, "\\ifdraft", 2),
        (&declaration, "\\draftfalse", 8),
        (&main, "\\drafttrue", 7),
        (&main, "\\ifdraft", 2),
        (&main, "draftfalse\\endcsname", 6),
        (&main, "ifdraft\\endcsname", 1),
    ] {
        let text = &workspace.documents[uri].text;
        let position = text.position(text.source.find(marker).unwrap() + local);
        let targets = features::definitions(&workspace, uri, position);
        assert_eq!(targets.len(), 1, "{marker}: {targets:?}");
        assert_eq!(targets[0].uri, declaration);
        assert!(features::hover(&workspace, uri, position).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, uri, position).unwrap()
        else {
            panic!("{marker}")
        };
        assert_eq!(placeholder, "draft");
        assert_eq!(
            &text.source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            "draft"
        );
        let references = features::references(&workspace, uri, position, true);
        assert_eq!(references.len(), 6, "{marker}: {references:?}");
        assert_eq!(
            features::references(&workspace, uri, position, false).len(),
            5
        );
        let changed = renamed(&workspace, uri, position);
        assert_eq!(
            changed[&declaration],
            definitions
                .replace("\\ifdraft", "\\ifrelease")
                .replace("\\draftfalse", "\\releasefalse")
        );
        assert_eq!(
            changed[&main],
            source
                .replace("drafttrue", "releasetrue")
                .replace("ifdraft", "ifrelease")
                .replace("draftfalse", "releasefalse")
                .replace("% \\releasetrue", "% \\drafttrue")
                .replace("\\verb|\\ifrelease|", "\\verb|\\ifdraft|")
        );
    }
    for (marker, expected) in [("\\drafttrue", "drafttrue"), ("\\ifdraft", "ifdraft")] {
        let position = workspace.documents[&main]
            .text
            .position(source.find(marker).unwrap() + marker.len() - 1);
        let items = features::completion(&workspace, &main, position);
        assert!(items.iter().any(|item| item.label == expected), "{items:?}");
    }
    assert!(features::symbols(&workspace.documents[&declaration])
        .iter()
        .any(|symbol| symbol.name == "draft" && symbol.kind == SymbolKind::BOOLEAN));
}

#[test]
fn conditional_renames_reject_derived_command_collisions_builtins_and_invalid_control_words() {
    for extra in [
        r"\newif\ifrelease",
        r"\newcommand{\releasetrue}{}",
        r"\newlength{\releasefalse}",
        r"\newenvironment{ifrelease}{}{}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\newif\\ifdraft\n{extra}\n");
        let workspace = setup(root, &[("main.tex", &source)]);
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let position = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
        for name in ["release", "true", "false", "num", "release-two"] {
            assert!(
                features::rename(&workspace, &uri, position, name).is_err(),
                "{extra}: {name}"
            );
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newif\\ifdraft\n\\newcommand{\\oldmacro}{}\\newenvironment{oldenv}{}{}\\newcounter{oldcounter}\n";
    let workspace = setup(root, &[("main.tex", source)]);
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    for old in ["oldmacro", "oldenv"] {
        let position = Text::new(source.into()).position(source.find(old).unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, position, "drafttrue").is_err(),
            "{old}"
        );
    }
}

#[test]
fn shared_conditional_occurrences_cannot_rename_another_projects_switch() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = setup(root, &[
        ("a.tex", "\\documentclass{article}\\newif\\ifdraft\\begin{document}\\input{shared}\\end{document}"),
        ("b.tex", "\\documentclass{article}\\newif\\ifdraft\\begin{document}\\input{shared}\\end{document}"),
        ("shared.tex", "\\drafttrue\\ifdraft Shared.\\fi"),
    ]);
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    let text = &workspace.documents[&uri].text;
    let position = text.position(text.source.find("draft").unwrap());
    assert!(features::rename(&workspace, &uri, position, "release").is_err());
}

#[test]
fn conditional_completions_follow_projects_and_skip_computed_or_ignored_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\n\\newif% declaration\n\\ifdraft\n% \\newif\\ifcomment\n\\verb|\\newif\\ifverbatim|\n\\newif\\other\n\\newif\\csname ifcomputed\\endcsname\n\\begin{document}\n\\drafttrue\\ifdraft Text.\\fi\n\\end{document}\n";
    let workspace = setup(
        root,
        &[
            ("a.tex", source),
            (
                "b.tex",
                "\\documentclass{article}\\newif\\ifdifferent\\begin{document}Text.\\end{document}",
            ),
        ],
    );
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    assert_eq!(workspace.documents[&uri].syntax.conditionals.len(), 1);
    let text = &workspace.documents[&uri].text;
    let position = text.position(source.find("\\drafttrue").unwrap() + 1);
    let labels = features::completion(&workspace, &uri, position)
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    for name in ["ifdraft", "drafttrue", "draftfalse"] {
        assert!(
            labels.iter().any(|label| label == name),
            "{name}: {labels:?}"
        );
    }
    for name in [
        "ifdifferent",
        "differenttrue",
        "ifcomment",
        "ifverbatim",
        "ifcomputed",
    ] {
        assert!(!labels.iter().any(|label| label == name), "{name}");
    }
    let changed = renamed(&workspace, &uri, position);
    assert_eq!(changed.len(), 1);
    assert!(changed[&uri].contains("\\newif% declaration\n\\ifrelease"));
}

#[test]
#[ignore = "requires pdflatex"]
fn real_conditional_renames_preserve_true_false_and_local_group_behavior() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newif\\ifdraft\n\\draftfalse\n";
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\ifdraft Wrong.\\else\\typeout{SWITCH:initial-false}\\fi\n\\drafttrue\\ifdraft\\typeout{SWITCH:true}\\fi\n{\\draftfalse\\ifdraft Wrong.\\else\\typeout{SWITCH:local-false}\\fi}\n\\ifdraft\\typeout{SWITCH:restored-true}\\fi\n\\csname draftfalse\\endcsname\\csname ifdraft\\endcsname Wrong.\\else\\typeout{SWITCH:csname-false}\\fi\nText.\\end{document}\n";
    let workspace = setup(
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
    let uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
    let position = Text::new(definitions.into()).position(definitions.find("draft").unwrap());
    for (uri, changed) in renamed(&workspace, &uri, position) {
        std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
    }
    let second = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(second.status, "success", "{}", second.output);
    assert_eq!(second.pages, Some(1));
    let states = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("SWITCH:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(states(&first.output), states(&second.output));
    assert_eq!(states(&second.output).len(), 5);
}

#[test]
fn ifthen_and_etoolbox_boolean_declarations_track_names_operations_and_generated_commands() {
    for declaration in ["newboolean", "provideboolean", "newbool", "providebool"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions =
            format!("\\{declaration}{{draft}}\n\\newcommand{{\\draft}}{{Independent.}}\n");
        let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\setboolean{draft}{true}\\boolean{draft}\\setbool{draft}{false}\\booltrue{draft}\\boolfalse{draft}\\ifbool{draft}{Yes}{No}\\notbool{draft}{No}{Yes}\n\\drafttrue\\ifdraft Yes.\\fi\\csname draftfalse\\endcsname\n\\draft\n% \\booltrue{draft}\n\\verb|\\boolean{draft}|\n\\end{document}\n";
        let workspace = setup(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = Url::from_file_path(root.join("main.tex")).unwrap();
        let definition = Url::from_file_path(root.join("definitions.tex")).unwrap();
        for (uri, marker, local) in [
            (&definition, "{draft}", 2),
            (&main, "\\setboolean{draft}", 14),
            (&main, "\\ifbool{draft}", 10),
            (&main, "\\drafttrue", 7),
        ] {
            let text = &workspace.documents[uri].text;
            let position = text.position(text.source.find(marker).unwrap() + local);
            assert_eq!(
                features::definitions(&workspace, uri, position).len(),
                1,
                "{declaration}: {marker}"
            );
            assert_eq!(
                features::references(&workspace, uri, position, true).len(),
                11
            );
            let changed = renamed(&workspace, uri, position);
            assert_eq!(
                changed[&definition],
                definitions.replacen("{draft}", "{release}", 1)
            );
            assert_eq!(
                changed[&main],
                source
                    .replace("draft", "release")
                    .replace("\\release\n", "\\draft\n")
                    .replace("% \\booltrue{release}", "% \\booltrue{draft}")
                    .replace("\\verb|\\boolean{release}|", "\\verb|\\boolean{draft}|")
            );
        }
    }
}

#[test]
fn boolean_argument_completions_preserve_names_suffixes_spaces_values_and_utf16_ranges() {
    for (marked, target, expected) in [
        (
            r"😀 \setboolean{dra¦ft}{true}",
            "draft",
            r"😀 \setboolean{draft}{true}",
        ),
        (r"\boolean{  dra¦ft  }", "draft", r"\boolean{  draft  }"),
        (
            r"\ifbool{dra¦}{Yes}{No}",
            "draft",
            r"\ifbool{draft}{Yes}{No}",
        ),
        (
            r"\notbool{dra¦ft}{No}{Yes}",
            "draft",
            r"\notbool{draft}{No}{Yes}",
        ),
        (r"\booltrue{dra¦", "draft", r"\booltrue{draft"),
        (r"\boolfalse{模¦式}", "模式", r"\boolfalse{模式}"),
        (
            r"\setboolean{draft}{tr¦ue}",
            "true",
            r"\setboolean{draft}{true}",
        ),
        (r"\setbool{draft}{fa¦}", "false", r"\setbool{draft}{false}"),
        (r"\ifbool{tr¦}{Yes}{No}", "true", r"\ifbool{true}{Yes}{No}"),
        (r"\boolean{fa¦}", "false", r"\boolean{false}"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(
            root,
            &[
                ("main.tex", &source),
                ("definitions.tex", "\\newboolean{draft}\\newbool{模式}"),
            ],
        );
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let text = &workspace.documents[&uri].text;
        let items = features::completion(&workspace, &uri, text.position(offset));
        let item = items
            .iter()
            .find(|item| item.label == target)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(
            text.changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone()
            }])
            .unwrap()
            .source,
            expected
        );
    }
}

#[test]
fn boolean_name_completion_skips_declarations_expressions_computed_names_and_other_projects() {
    for marked in [
        r"\newboolean{dra¦}",
        r"\providebool{dra¦}",
        r"\boolean{dra¦\suffix}",
        r"\boolean{dra¦% rest",
        r"\setboolean{draft}{tr¦\suffix}",
        r"\ifthenelse{dra¦}{Yes}{No}",
        r"\verb|\boolean{dra¦}|",
        r"\setboolean{tr¦}{true}",
        r"\boolfalse{fa¦}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(
            directory.path(),
            &[
                ("main.tex", &source),
                ("definitions.tex", "\\newboolean{draft}"),
            ],
        );
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        assert!(
            features::completion(
                &workspace,
                &uri,
                Text::new(source).position(marked.find('¦').unwrap())
            )
            .is_empty(),
            "{marked}"
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        "\\documentclass{article}\\newboolean{draft}\\begin{document}\\booltrue{}\\end{document}";
    let workspace = setup(
        root,
        &[
            ("a.tex", source),
            (
                "b.tex",
                "\\documentclass{article}\\newbool{other}\\begin{document}Text.\\end{document}",
            ),
        ],
    );
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    let items = features::completion(
        &workspace,
        &uri,
        Text::new(source.into()).position(source.find("\\booltrue{}").unwrap() + 10),
    );
    assert!(items.iter().any(|item| item.label == "draft"));
    assert!(!items.iter().any(|item| item.label == "other"));
}

#[test]
fn boolean_operations_provide_signatures_without_confusing_expressions_with_names() {
    for (marked, expected, active) in [
        (r"\newboolean{dra¦}", r"\newboolean{boolean}", 0),
        (r"\newbool{dra¦}", r"\newbool{boolean}", 0),
        (
            r"\setboolean{draft}{tr¦}",
            r"\setboolean{boolean}{value}",
            1,
        ),
        (r"\ifbool{draft}{Y¦}{N}", r"\ifbool{boolean}{then}{else}", 1),
        (
            r"\ifboolexpr{bool {draft¦}}{Y}{N}",
            r"\ifboolexpr{condition}{then}{else}",
            0,
        ),
        (
            r"\ifboolexpe{bool {draft}}{Y}{N¦}",
            r"\ifboolexpe{condition}{then}{else}",
            2,
        ),
        (
            r"\whileboolexpr{bool {draft}}{Body¦}",
            r"\whileboolexpr{condition}{body}",
            1,
        ),
        (
            r"\unlessboolexpr{not bool {draft¦}}{Body}",
            r"\unlessboolexpr{condition}{body}",
            0,
        ),
        (
            r"\notbool{draft}{Y}{N¦}",
            r"\notbool{boolean}{then}{else}",
            2,
        ),
        (
            r"\ifthenelse{\boolean{draft}}{Y¦}{N}",
            r"\ifthenelse{condition}{then}{else}",
            1,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(directory.path(), &[("main.tex", &source)]);
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let help = latex_lsp::signature::help(
            &workspace,
            &uri,
            Text::new(source).position(marked.find('¦').unwrap()),
        )
        .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.signatures[0].label, expected);
        assert_eq!(help.active_parameter, Some(active));
    }
}

#[test]
#[ignore = "requires pdflatex, ifthen and etoolbox"]
fn real_ifthen_and_etoolbox_boolean_renames_preserve_values_and_group_scoping() {
    for (package, declaration, operations) in [
        ("ifthen", "newboolean", "\\setboolean{draft}{true}\\ifthenelse{\\boolean{draft}}{\\typeout{BOOL:true}}{Wrong.}\n{\\setboolean{draft}{false}\\ifthenelse{\\boolean{draft}}{Wrong.}{\\typeout{BOOL:local-false}}}\n\\ifthenelse{\\boolean{draft}}{\\typeout{BOOL:restored}}{Wrong.}\n"),
        ("ifthen", "provideboolean", "\\setboolean{draft}{true}\\ifthenelse{\\boolean{draft}}{\\typeout{BOOL:true}}{Wrong.}\n{\\setboolean{draft}{false}\\ifthenelse{\\boolean{draft}}{Wrong.}{\\typeout{BOOL:local-false}}}\n\\ifthenelse{\\boolean{draft}}{\\typeout{BOOL:restored}}{Wrong.}\n"),
        ("etoolbox", "newbool", "\\booltrue{draft}\\ifbool{draft}{\\typeout{BOOL:true}}{Wrong.}\n{\\setbool{draft}{false}\\notbool{draft}{\\typeout{BOOL:local-false}}{Wrong.}}\n\\ifbool{draft}{\\typeout{BOOL:restored}}{Wrong.}\n"),
        ("etoolbox", "providebool", "\\booltrue{draft}\\ifbool{draft}{\\typeout{BOOL:true}}{Wrong.}\n{\\setbool{draft}{false}\\notbool{draft}{\\typeout{BOOL:local-false}}{Wrong.}}\n\\ifbool{draft}{\\typeout{BOOL:restored}}{Wrong.}\n"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("\\{declaration}{{draft}}\n");
        let source = format!("\\documentclass{{article}}\\usepackage{{{package}}}\\input{{definitions}}\n\\begin{{document}}\n{operations}Text.\\end{{document}}\n");
        let workspace = setup(root, &[("main.tex", &source), ("definitions.tex", &definitions)]);
        let main = root.join("main.tex");
        let config = latex_lsp::config::BuildConfig { executable: "pdflatex".into(), ..Default::default() };
        let before = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{}", before.output);
        let uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
        let position = Text::new(definitions.clone()).position(definitions.find("draft").unwrap());
        for (uri, changed) in renamed(&workspace, &uri, position) { std::fs::write(uri.to_file_path().unwrap(), changed).unwrap(); }
        let after = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{}", after.output);
        assert_eq!(after.pages, Some(1));
        let states = |output: &str| output.lines().filter(|line| line.starts_with("BOOL:")).map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(states(&before.output), states(&after.output));
        assert_eq!(states(&after.output).len(), 3);
    }
}

#[test]
fn boolean_expressions_navigate_and_rename_static_operands_without_touching_test_text_or_toggles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newbool{draft}\\newbool{other}\\newcommand{\\draft}{Independent.}\n";
    let source = "\\documentclass{article}\\input{definitions}\n\\begin{document}\n\\ifboolexpr{bool {draft} and not (bool {other})}{Yes}{No}\n\\ifboolexpe{notbool {draft}}{No}{Yes}\n\\whileboolexpr{bool{draft}}{\\boolfalse{draft}}\n\\unlessboolexpr{bool {draft}}{\\booltrue{draft}}\n\\ifboolexpr{test {\\ifstrequal{bool {draft}}{Text}} or togl {draft}}{body bool {draft}}{No}\n\\ifboolexpr{bool {other} % bool {draft}\n}{Yes}{No}\n\\draft\\end{document}\n";
    let workspace = setup(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let text = &workspace.documents[&uri].text;
    let position = text.position(source.find("bool {draft}").unwrap() + 8);
    assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
    assert_eq!(
        features::references(&workspace, &uri, position, true).len(),
        7
    );
    let changed = renamed(&workspace, &uri, position);
    let expected = source
        .replace("bool {draft}", "bool {release}")
        .replace("bool{draft}", "bool{release}")
        .replace("\\boolfalse{draft}", "\\boolfalse{release}")
        .replace("\\booltrue{draft}", "\\booltrue{release}")
        .replace("\\ifstrequal{bool {release}}", "\\ifstrequal{bool {draft}}")
        .replace("body bool {release}", "body bool {draft}")
        .replace("% bool {release}", "% bool {draft}");
    assert_eq!(changed[&uri], expected);
    assert!(changed[&uri].contains("togl {draft}"));
}

#[test]
fn boolean_expression_operand_completion_supports_nested_and_incomplete_expressions() {
    for marked in [
        r"\ifboolexpr{bool {dra¦ft}}{Yes}{No}",
        r"\ifboolexpe{notbool{dra¦ft}}{No}{Yes}",
        r"\ifboolexpr{not (bool { dra¦ft })}{No}{Yes}",
        r"\whileboolexpr{bool {dra¦",
        r"\unlessboolexpr{bool{dra¦ft}}{Body}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(
            directory.path(),
            &[
                ("main.tex", &source),
                ("definitions.tex", "\\newbool{draft}"),
            ],
        );
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let items = features::completion(
            &workspace,
            &uri,
            Text::new(source.clone()).position(marked.find('¦').unwrap()),
        );
        let item = items
            .iter()
            .find(|item| item.label == "draft")
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit")
        };
        let changed = Text::new(source.clone())
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap()
            .source;
        assert_eq!(
            changed,
            marked.replace("dra¦ft", "draft").replace("dra¦", "draft"),
            "{marked}"
        );
        assert_eq!(edit.new_text, "draft");
    }
    for marked in [
        r"\ifboolexpr{test {\ifstrequal{bool {dra¦ft}}{Text}}}{Yes}{No}",
        r"\ifboolexpr{togl {dra¦ft}}{Yes}{No}",
        r"\ifboolexpr{bool {dra¦\suffix}}{Yes}{No}",
        r"\ifboolexpr{bool {other}}{bool {dra¦ft}}{No}",
        r"\ifboolexpr{mybool {dra¦ft}}{Yes}{No}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(
            directory.path(),
            &[
                ("main.tex", &source),
                ("definitions.tex", "\\newbool{draft}"),
            ],
        );
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        assert!(
            features::completion(
                &workspace,
                &uri,
                Text::new(source).position(marked.find('¦').unwrap())
            )
            .is_empty(),
            "{marked}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn real_boolean_expression_renames_preserve_nested_tests_expandable_tests_and_finite_loops() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newbool{draft}\\newbool{other}\\newcounter{iterations}\n";
    let source = "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\n\\begin{document}\n\\booltrue{draft}\\ifboolexpr{(bool {draft} and not bool {other})}{\\typeout{EXPR:true}}{Wrong.}\n\\boolfalse{draft}\\ifboolexpe{notbool {draft}}{\\typeout{EXPR:false}}{Wrong.}\n\\whileboolexpr{not bool {draft}}{\\booltrue{draft}\\stepcounter{iterations}}\n\\unlessboolexpr{not bool {draft}}{\\boolfalse{draft}\\stepcounter{iterations}}\n\\typeout{EXPR:iterations:\\arabic{iterations}}Text.\\end{document}\n";
    let workspace = setup(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        timeout_seconds: 5,
        ..Default::default()
    };
    let before = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "success", "{}", before.output);
    let uri = Url::from_file_path(&main).unwrap();
    let position = Text::new(source.into()).position(source.find("bool {draft}").unwrap() + 8);
    for (uri, changed) in renamed(&workspace, &uri, position) {
        std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
    }
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
    for state in ["EXPR:true", "EXPR:false", "EXPR:iterations:2"] {
        assert!(before.output.contains(state));
        assert!(after.output.contains(state));
    }
}
