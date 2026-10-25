use latex_lsp::{
    features, file_operations, navigation,
    workspace::{Document, Workspace},
};
use lsp_types::*;
use std::path::Path;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}
fn call(form: &str, loader: &str) -> String {
    match form {
        "direct" => format!("\\{loader}"),
        "named" => format!("\\csuse{{{loader}}}"),
        "literal" => format!("\\csname {loader}\\endcsname"),
        "alias" => r"\readpart".into(),
        "named-alias" => r"\csuse{readpart}".into(),
        "literal-alias" => r"\csname readpart\endcsname".into(),
        _ => panic!(),
    }
}
fn items(workspace: &mut Workspace, target: &Url, marked: &str) -> Vec<CompletionItem> {
    let offset = marked.find('|').unwrap();
    let source = marked.replace('|', "");
    workspace
        .documents
        .insert(target.clone(), Document::new(target, source, Some(7)));
    features::completion(
        workspace,
        target,
        workspace.documents[target].text.position(offset),
    )
}
fn complete(workspace: &mut Workspace, target: &Url, marked: &str, label: &str) -> String {
    let candidates = items(workspace, target, marked);
    let item = candidates
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: missing {label}: {candidates:?}"));
    let Some(CompletionTextEdit::Edit(main)) = item.text_edit.as_ref() else {
        panic!()
    };
    let offset = marked.find('|').unwrap();
    let text = &workspace.documents[target].text;
    assert_eq!(main.range.start.line, main.range.end.line);
    assert!(main.range.start <= text.position(offset) && text.position(offset) <= main.range.end);
    let typed = &text.source[text.offset(main.range.start).unwrap()..offset];
    assert!(
        item.filter_text.as_ref().unwrap().starts_with(typed),
        "{marked}: {item:?}"
    );
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(main.clone());
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for pair in edits.windows(2) {
        assert!(pair[1].range.end <= pair[0].range.start, "{item:?}");
    }
    text.changed(
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
fn file_items(items: &[CompletionItem]) -> Vec<&CompletionItem> {
    items
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                Some(CompletionItemKind::FILE | CompletionItemKind::FOLDER)
            )
        })
        .collect()
}

#[test]
fn static_source_invocations_complete_finished_and_unfinished_filename_groups() {
    for form in [
        "direct",
        "named",
        "literal",
        "alias",
        "named-alias",
        "literal-alias",
    ] {
        for loader in ["input", "include"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let mut workspace = project(root, &[("main.tex", ""), ("part.tex", "Part")]);
            let main = uri(root, "main.tex");
            let prefix = format!(
                "\\documentclass{{article}}\\def\\readpart{{\\{loader}}}\\begin{{document}}{}",
                call(form, loader)
            );
            for (tail, expected) in [
                ("{par|t}", "{part}"),
                ("{par|t.tex}", "{part.tex}"),
                ("{par|", "{part"),
                ("{|}", "{part}"),
                ("{ | }", "{ part }"),
                (
                    "% before 😀\r\n{% name\r\npar|t% keep\r\n}",
                    "% before 😀\r\n{% name\r\npart% keep\r\n}",
                ),
                ("{% name\r\npar|", "{% name\r\npart"),
            ] {
                assert_eq!(
                    complete(
                        &mut workspace,
                        &main,
                        &format!("{prefix}{tail}"),
                        "part.tex"
                    ),
                    format!("{prefix}{expected}"),
                    "{form} {loader} {tail}"
                );
            }
        }
    }
}

#[test]
fn bare_and_quoted_indirect_inputs_preserve_filename_boundaries_and_delimiter_spacing() {
    for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut workspace = project(
            root,
            &[
                ("main.tex", ""),
                ("part.tex", "Part"),
                ("my file.tex", "Part"),
                ("新 章节.tex", "Part"),
                ("my dir/chapter.tex", "Part"),
            ],
        );
        let main = uri(root, "main.tex");
        let prefix = format!(
            "\\documentclass{{article}}\\def\\readpart{{\\input}}\\begin{{document}}{} ",
            call(form, "input")
        );
        for (tail, label, expected) in [
            ("par|t next", "part.tex", "part next"),
            ("par|t.tex next", "part.tex", "part.tex next"),
            ("my| next", "my file.tex", "{my file}next"),
            ("my| \r\n next", "my file.tex", "{my file}% \r\n next"),
            ("my|", "my file.tex", "{my file}"),
            ("\"my f|ile\" next", "my file.tex", "\"my file\" next"),
            ("\"新 |旧\" \n next", "新 章节.tex", "{新 章节}% \n next"),
            ("my|/chapter next", "my dir/", "{my dir/chapter}next"),
        ] {
            assert_eq!(
                complete(&mut workspace, &main, &format!("{prefix}{tail}"), label),
                format!("{prefix}{expected}"),
                "{form} {tail}"
            );
        }
    }
}

#[test]
fn source_completion_keeps_directory_suffixes_unicode_commas_and_extension_rules() {
    for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
        for loader in ["input", "include"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let mut workspace = project(
                root,
                &[
                    ("main.tex", ""),
                    ("sections/中文.tex", "Part"),
                    ("one,two.tex", "Part"),
                    ("commands.def", "Part"),
                    ("photo.png", ""),
                ],
            );
            let main = uri(root, "main.tex");
            let prefix=format!("% 😀\r\n\\documentclass{{article}}\\def\\readpart{{\\{loader}}}\\begin{{document}}{}",call(form,loader));
            for (tail, label, expected) in [
                ("{se|ctions/中文}", "sections/", "{sections/中文}"),
                ("{sections/中|旧}", "sections/中文.tex", "{sections/中文}"),
                ("{one,|BAD}", "one,two.tex", "{one,two}"),
            ] {
                assert_eq!(
                    complete(&mut workspace, &main, &format!("{prefix}{tail}"), label),
                    format!("{prefix}{expected}")
                );
            }
            let candidates = items(&mut workspace, &main, &format!("{prefix}{{comm|}}"));
            assert_eq!(
                candidates.iter().any(|x| x.label == "commands.def"),
                loader == "input"
            );
            if loader == "input" {
                assert_eq!(
                    complete(
                        &mut workspace,
                        &main,
                        &format!("{prefix}{{comm|}}"),
                        "commands.def"
                    ),
                    format!("{prefix}{{commands.def}}")
                );
            }
            assert!(
                file_items(&items(&mut workspace, &main, &format!("{prefix}{{pho|}}"))).is_empty()
            );
        }
    }
}

#[test]
fn data_deferred_calls_redefinitions_and_endinput_do_not_offer_file_candidates() {
    for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut workspace = project(root, &[("main.tex", ""), ("part.tex", "Part")]);
        let main = uri(root, "main.tex");
        let invocation = format!("{}{{par|}}", call(form, "input"));
        for body in [
            format!("\\newcommand{{\\unused}}{{{invocation}}}"),
            format!("\\newenvironment{{unused}}{{{invocation}}}{{}}"),
            format!("\\newcommand{{\\take}}[1]{{\\detokenize{{#1}}}}\\take{{{invocation}}}"),
            format!("\\endinput\r\n{invocation}"),
            format!("\\def\\input#1{{Text}}{invocation}"),
            format!("\\def\\csuse#1{{Text}}{}{{par|}}", call("named", "input")),
        ] {
            let marked = format!("\\documentclass{{article}}\\def\\readpart{{\\input}}{body}");
            assert!(
                file_items(&items(&mut workspace, &main, &marked)).is_empty(),
                "{marked}"
            );
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root, &[("main.tex", ""), ("part.tex", "Part")]);
    let main = uri(root, "main.tex");
    for marked in [
        r"\csuse{\which}{par|}",
        r"\csname input\suffix\endcsname{par|}",
        r"\def\readpart{\input}\csuse{input}{par\macro|}",
        r"\def\readpart{\input}\readpart{pa% continuation
rt|}",
        r"\csuse{input}*{par|}",
        r"\csuse{include}[x]{par|}",
        r"\csuse{include} par|",
        r"\begin{verbatim}\csuse{input}{par|}\end{verbatim}",
    ] {
        assert!(
            file_items(&items(&mut workspace, &main, marked)).is_empty(),
            "{marked}"
        );
    }
}

#[test]
fn shared_sources_require_matching_loader_meaning_and_honor_explicit_root_selection() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = r"\documentclass{article}\def\readpart{\input}\input{shared}";
    let mut workspace = project(
        root,
        &[
            ("one.tex", first),
            ("two.tex", first),
            ("shared.tex", r"\readpart{part}"),
            ("part.tex", "Part"),
        ],
    );
    let shared = uri(root, "shared.tex");
    assert_eq!(
        complete(&mut workspace, &shared, r"\readpart{par|}", "part.tex"),
        r"\readpart{part}"
    );
    let two = uri(root, "two.tex");
    for definition in [
        r"\def\readpart{\include}",
        r"\newcommand{\readpart}[1]{Text}",
    ] {
        workspace.documents.insert(
            two.clone(),
            Document::new(
                &two,
                format!("\\documentclass{{article}}{definition}\\input{{shared}}"),
                Some(8),
            ),
        );
        assert!(file_items(&items(&mut workspace, &shared, r"\readpart{par|}")).is_empty());
        assert_eq!(
            complete(
                &mut workspace,
                &shared,
                "% !TeX root = one.tex\r\n\\readpart{par|}",
                "part.tex"
            ),
            "% !TeX root = one.tex\r\n\\readpart{part}"
        );
    }
}

#[test]
fn unchanged_parent_completion_refreshes_when_external_package_alias_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let defs = directory.path().join("defs.sty");
    std::fs::write(&defs, r"\ProvidesPackage{defs}\def\readpart{\input}").unwrap();
    let marked = r"\documentclass{article}\usepackage{../defs}\begin{document}\readpart{par|}";
    let mut workspace = project(
        &root,
        &[("main.tex", &marked.replace('|', "")), ("part.tex", "Part")],
    );
    let main = uri(&root, "main.tex");
    workspace.load_dependencies(&main, 0);
    let before = complete(&mut workspace, &main, marked, "part.tex");
    assert_eq!(before, marked.replace("par|", "part"));
    let version = workspace.documents[&main].version;
    std::fs::write(
        &defs,
        r"\ProvidesPackage{defs}\newcommand{\readpart}[1]{Text}",
    )
    .unwrap();
    workspace.refresh_project(&main);
    let source = &workspace.documents[&main].text;
    assert_eq!(workspace.documents[&main].version, version);
    assert!(file_items(&features::completion(
        &workspace,
        &main,
        source.position(marked.find('|').unwrap())
    ))
    .is_empty());
    std::fs::write(
        &defs,
        r"\ProvidesPackage{defs}\def\readpart{\nextpart}\def\nextpart{\input}",
    )
    .unwrap();
    workspace.refresh_project(&main);
    assert_eq!(complete(&mut workspace, &main, marked, "part.tex"), before);
}

#[test]
fn aliases_in_unexecuted_declarations_and_other_projects_cannot_supply_completions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(
        root,
        &[
            ("main.tex", ""),
            ("other.tex", r"\documentclass{article}\def\readpart{\input}"),
            ("part.tex", "Part"),
        ],
    );
    let main = uri(root, "main.tex");
    for declaration in [
        "",
        r"\newcommand{\unused}{\def\readpart{\input}}",
        r"\newenvironment{unused}{\def\readpart{\input}}{}",
        r"\newcommand{\take}[1]{\detokenize{#1}}\take{\csdef{readpart}{\input}}",
    ] {
        let marked =
            format!("\\documentclass{{article}}{declaration}\\begin{{document}}\\readpart{{par|}}");
        assert!(
            file_items(&items(&mut workspace, &main, &marked)).is_empty(),
            "{marked}"
        );
    }
}

#[test]
fn indirect_completion_searches_build_root_before_child_directory_and_keeps_shadow_rules() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(
        root,
        &[
            (
                "main.tex",
                r"\documentclass{article}\def\readpart{\input}\input{sections/child}",
            ),
            ("sections/child.tex", ""),
            ("local.tex", "Root"),
            ("sections/local.tex", "Near"),
            ("chapter.ltx", "Shadowed"),
            ("chapter.ltx.tex", "Selected"),
        ],
    );
    let child = uri(root, "sections/child.tex");
    let candidates = items(&mut workspace, &child, r"\readpart{lo|}");
    let locals: Vec<_> = candidates
        .iter()
        .filter(|item| item.label == "local.tex")
        .collect();
    assert_eq!(locals.len(), 1);
    assert_eq!(locals[0].detail.as_deref(), root.join("local.tex").to_str());
    let candidates = items(&mut workspace, &child, r"\readpart{chap|}");
    assert_eq!(file_items(&candidates).len(), 1, "{candidates:?}");
    assert_eq!(candidates[0].label, "chapter.ltx.tex");
}

#[test]
fn commented_literal_sources_stay_navigable_and_renamable_after_completion() {
    for loader in ["input", "include"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let marked = format!("\\documentclass{{article}}\\newenvironment{{x}}{{}}{{}}\\begin{{document}}\\begin{{x}}Before \\{loader}% before 😀\r\n{{% filename\r\npar|WRONG% keep\r\n}} Tail\\end{{document}}");
        let mut workspace = project(root, &[("main.tex", ""), ("part.tex", r"Part\end{x}")]);
        workspace.versioned_edits = true;
        let main = uri(root, "main.tex");
        let child = uri(root, "part.tex");
        let updated = complete(&mut workspace, &main, &marked, "part.tex");
        assert_eq!(updated, marked.replace("par|WRONG", "part"));
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, updated.clone(), Some(8)));
        assert!(workspace.project_documents(&main).contains(&child));
        assert!(workspace.diagnostics(&main).is_empty());
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, Some(child.clone()));
        let at = workspace.documents[&main]
            .text
            .position(updated.rfind("part").unwrap() + 1);
        assert_eq!(features::definitions(&workspace, &main, at)[0].uri, child);
        let edit = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: child.into(),
                new_uri: uri(root, "new part.tex").into(),
            }],
        )
        .unwrap()
        .unwrap();
        let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
            panic!()
        };
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].text_document.uri, main);
        assert_eq!(edits[0].text_document.version, Some(8));
        let changes = edits[0]
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
        assert_eq!(
            workspace.documents[&main]
                .text
                .changed(changes)
                .unwrap()
                .source,
            updated.replace("part% keep", "new part% keep")
        );
    }
}
