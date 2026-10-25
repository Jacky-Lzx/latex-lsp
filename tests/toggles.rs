use latex_lsp::{
    compilation::build, language::features, project::workspace::Workspace, text::Text,
};
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
fn toggle_rename_keeps_boolean_namespace_test_text_and_other_arguments_intact() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newtoggle{draft}\\newbool{draft}\\newtoggle{other}\n";
    let source = "\\documentclass{article}\\input{definitions}\n\\begin{document}\n\\settoggle{draft}{true}\\toggletrue{draft}\\togglefalse{draft}\\iftoggle{draft}{draft}{No}\\nottoggle{draft}{No}{Yes}\n\\ifboolexpr{togl {draft} and bool {draft} or not (togl {other})}{Yes}{No}\n\\ifboolexpe{nottogl{draft}}{No}{Yes}\n\\whileboolexpr{togl {draft}}{\\togglefalse{draft}}\n\\unlessboolexpr{togl {draft}}{\\toggletrue{draft}}\n\\ifboolexpr{test {\\ifstrequal{togl {draft}}{draft}}}{Yes}{No}\n\\csname etb@tgl@draft\\endcsname{Yes}{No}\\booltrue{draft}\\ifdraft Yes.\\fi\n% \\toggletrue{draft}\n\\verb|\\iftoggle{draft}{Yes}{No}|\n\\end{document}\n";
    let workspace = setup(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let declaration = Url::from_file_path(root.join("definitions.tex")).unwrap();
    for (uri, marker, local) in [
        (&declaration, "newtoggle{draft}", 11),
        (&main, "iftoggle{draft}", 10),
        (&main, "togl {draft}", 8),
        (&main, "nottogl{draft}", 9),
        (&main, "etb@tgl@draft", 2),
    ] {
        let text = &workspace.documents[uri].text;
        let position = text.position(text.source.find(marker).unwrap() + local);
        let targets = features::definitions(&workspace, uri, position);
        assert_eq!(targets.len(), 1, "{marker}: {targets:?}");
        assert_eq!(targets[0].uri, declaration);
        assert!(features::hover(&workspace, uri, position).is_some());
        assert!(
            matches!(features::prepare_rename(&workspace, uri, position).unwrap(), Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) if placeholder == "draft")
        );
        assert_eq!(
            features::references(&workspace, uri, position, true).len(),
            13
        );
        assert_eq!(
            features::references(&workspace, uri, position, false).len(),
            12
        );
        let changed = renamed(&workspace, uri, position);
        assert_eq!(
            changed[&declaration],
            definitions.replacen("newtoggle{draft}", "newtoggle{release}", 1)
        );
        for unchanged in [
            "bool {draft}",
            "booltrue{draft}",
            "ifdraft",
            "{draft}{No}",
            "test {\\ifstrequal{togl {draft}}{draft}}",
            "% \\toggletrue{draft}",
            "\\verb|\\iftoggle{draft}{Yes}{No}|",
        ] {
            assert!(
                changed[&main].contains(unchanged),
                "{unchanged}: {}",
                changed[&main]
            );
        }
        assert_eq!(changed[&main].matches("release").count(), 12);
    }
    assert!(features::symbols(&workspace.documents[&declaration])
        .iter()
        .any(|symbol| symbol.name == "draft"
            && symbol.kind == SymbolKind::BOOLEAN
            && symbol.detail.as_deref() == Some("Defined with \\newtoggle")));
}

#[test]
fn toggle_completion_uses_separate_names_and_preserves_utf16_suffixes_and_whitespace() {
    for (marked, target, expected) in [
        (
            r"😀 \settoggle{dra¦ft}{true}",
            "draft",
            r"😀 \settoggle{draft}{true}",
        ),
        (r"\toggletrue{ dra¦ft }", "draft", r"\toggletrue{ draft }"),
        (r"\togglefalse{模¦式}", "模式", r"\togglefalse{模式}"),
        (
            r"\iftoggle{dra¦}{Yes}{No}",
            "draft",
            r"\iftoggle{draft}{Yes}{No}",
        ),
        (
            r"\nottoggle{dra¦ft}{No}{Yes}",
            "draft",
            r"\nottoggle{draft}{No}{Yes}",
        ),
        (
            r"\ifboolexpr{not (togl {dra¦ft})}{Yes}{No}",
            "draft",
            r"\ifboolexpr{not (togl {draft})}{Yes}{No}",
        ),
        (
            r"\ifboolexpe{nottogl{dra¦ft}}{Yes}{No}",
            "draft",
            r"\ifboolexpe{nottogl{draft}}{Yes}{No}",
        ),
        (
            r"\whileboolexpr{togl {dra¦",
            "draft",
            r"\whileboolexpr{togl {draft",
        ),
        (
            r"\unlessboolexpr{togl {dra¦ft}}{Body}",
            "draft",
            r"\unlessboolexpr{togl {draft}}{Body}",
        ),
        (
            r"\settoggle{draft}{tr¦ue}",
            "true",
            r"\settoggle{draft}{true}",
        ),
        (
            r"\settoggle{draft}{fa¦}",
            "false",
            r"\settoggle{draft}{false}",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(
            directory.path(),
            &[
                ("main.tex", &source),
                (
                    "definitions.tex",
                    "\\newtoggle{draft}\\providetoggle{模式}\\newbool{different}",
                ),
            ],
        );
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let text = &workspace.documents[&uri].text;
        let items =
            features::completion(&workspace, &uri, text.position(marked.find('¦').unwrap()));
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
    for marked in [
        r"\iftoggle{diff¦}{Y}{N}",
        r"\iftoggle{tr¦}{Y}{N}",
        r"\booltrue{dra¦}",
        r"\ifboolexpr{bool {dra¦}}{Y}{N}",
        r"\newtoggle{dra¦}",
        r"\providetoggle{dra¦}",
        r"\toggletrue{dra¦\suffix}",
        r"\ifboolexpr{test {\ifstrequal{togl {dra¦}}{X}}}{Y}{N}",
        r"\ifboolexpr{mytogl {dra¦}}{Y}{N}",
        r"\iftoggle{draft}{dra¦}{No}",
        r"\verb|\iftoggle{dra¦}{Y}{N}|",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(
            directory.path(),
            &[
                ("main.tex", &source),
                ("definitions.tex", "\\newtoggle{draft}\\newbool{different}"),
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
fn toggle_rename_rejects_conflicts_and_shared_project_ambiguity() {
    for extra in [
        r"\providetoggle{release}",
        r"\newcommand{\etb@tgl@release}{}",
        r"\newlength{\etb@tgl@release}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("\\newtoggle{{draft}}{extra}");
        let workspace = setup(directory.path(), &[("main.tex", &source)]);
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        assert!(features::rename(
            &workspace,
            &uri,
            Text::new(source.clone()).position(source.find("draft").unwrap()),
            "release"
        )
        .is_err());
    }
    let directory = tempfile::tempdir().unwrap();
    let workspace = setup(
        directory.path(),
        &[
            (
                "a.tex",
                r"\documentclass{article}\newtoggle{draft}\begin{document}\input{shared}\end{document}",
            ),
            (
                "b.tex",
                r"\documentclass{article}\newtoggle{draft}\begin{document}\input{shared}\end{document}",
            ),
            ("shared.tex", r"\iftoggle{draft}{Y}{N}"),
        ],
    );
    let uri = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let text = &workspace.documents[&uri].text;
    assert!(features::rename(
        &workspace,
        &uri,
        text.position(text.source.find("draft").unwrap()),
        "release"
    )
    .is_err());
}

#[test]
fn toggle_indexing_respects_projects_ignored_text_dynamic_names_and_generated_control_words() {
    let directory = tempfile::tempdir().unwrap();
    let source = "\\documentclass{article}\n\\newtoggle{draft}\\newbool{release}\n% \\newtoggle{comment}\n\\verb|\\newtoggle{verbatim}|\n\\newtoggle{computed\\suffix}\n\\begin{document}\n\\iftoggle{}{Yes}{No}\\newcommand{\\oldmacro}{}\n\\etb@tgl@draft{Yes}{No}\n\\end{document}";
    let workspace = setup(
        directory.path(),
        &[
            ("a.tex", source),
            (
                "b.tex",
                r"\documentclass{article}\newtoggle{different}\begin{document}Other.\end{document}",
            ),
        ],
    );
    let uri = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let doc = &workspace.documents[&uri];
    assert_eq!(doc.syntax.toggles.len(), 1);
    let position = doc.text.position(source.find("iftoggle{}").unwrap() + 9);
    let items = features::completion(&workspace, &uri, position);
    assert_eq!(
        items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["draft"]
    );
    let position = doc.text.position(source.find("etb@tgl@draft").unwrap() + 2);
    assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
    assert!(features::rename(&workspace, &uri, position, "发布-模式").is_err());
    assert!(features::rename(&workspace, &uri, position, "release")
        .unwrap()
        .is_some());
    let position = doc.text.position(source.find("oldmacro").unwrap());
    assert!(features::rename(&workspace, &uri, position, "etb@tgl@draft").is_err());
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn real_toggle_renames_preserve_boolean_independence_and_finite_expression_loops() {
    for declaration in ["newtoggle", "providetoggle"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions =
            format!("\\{declaration}{{draft}}\\newbool{{draft}}\\newcounter{{iterations}}\n");
        let source = "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\n\\begin{document}\n\\settoggle{draft}{true}\\boolfalse{draft}\n\\ifboolexpr{togl {draft} and not bool {draft}}{\\typeout{TOGGLE:true}}{Wrong.}\n{\\togglefalse{draft}\\nottoggle{draft}{\\typeout{TOGGLE:local-false}}{Wrong.}}\n\\iftoggle{draft}{\\typeout{TOGGLE:restored}}{Wrong.}\n\\whileboolexpr{togl {draft}}{\\togglefalse{draft}\\stepcounter{iterations}}\n\\unlessboolexpr{togl {draft}}{\\toggletrue{draft}\\stepcounter{iterations}}\n\\ifboolexpe{nottogl{draft}}{Wrong.}{\\typeout{TOGGLE:expanded}}\n\\typeout{TOGGLE:iterations:\\arabic{iterations}}Text.\\end{document}\n";
        let workspace = setup(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
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
        let position = Text::new(source.into()).position(source.find("togl {draft}").unwrap() + 8);
        let edit = features::rename(&workspace, &uri, position, "发布-模式")
            .unwrap()
            .unwrap();
        for (uri, mut edits) in edit.changes.unwrap() {
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
            std::fs::write(uri.to_file_path().unwrap(), changed.source).unwrap();
        }
        let after = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{}", after.output);
        assert_eq!(after.pages, Some(1));
        for state in [
            "TOGGLE:true",
            "TOGGLE:local-false",
            "TOGGLE:restored",
            "TOGGLE:expanded",
            "TOGGLE:iterations:2",
        ] {
            assert!(before.output.contains(state), "{state}");
            assert!(after.output.contains(state), "{state}");
        }
    }
}
