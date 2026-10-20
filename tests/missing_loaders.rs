use latex_lsp::{build, build_log, editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

#[test]
fn missing_loader_diagnostics_follow_static_names_and_the_consumed_lookahead_token() {
    for (source, shown, line, file, command, expected) in [
        (
            "\\usepackage{my \n locla}\\begin{document}",
            r" locla}\begin",
            2,
            "mylocla.sty",
            "usepackage",
            "my \n locla",
        ),
        (
            r"\usepackage{locla}\begin{document}",
            r"\usepackage{locla}\begin",
            1,
            "locla.sty",
            "usepackage",
            "locla",
        ),
        (
            "\\usepackage{locla}\n\\begin{document}",
            r"\begin",
            2,
            "locla.sty",
            "usepackage",
            "locla",
        ),
        (
            r"\usepackage[draft]{graphicx, locla }\begin{document}",
            r"\usepackage[draft]{graphicx, locla }\begin",
            1,
            "locla.sty",
            "usepackage",
            "locla",
        ),
        (
            r"\RequirePackage{locla}[2020/01/01]\begin{document}",
            r"\RequirePackage{locla}[2020/01/01]",
            1,
            "locla.sty",
            "RequirePackage",
            "locla",
        ),
        (
            r"\RequirePackageWithOptions{graphicx, locla}[2020/01/01]",
            r"...kageWithOptions{graphicx, locla}[2020/01/01]",
            1,
            "locla.sty",
            "RequirePackageWithOptions",
            "locla",
        ),
        (
            "\\LoadClassWithOptions{locla}\n\\relax",
            r"\relax",
            2,
            "locla.cls",
            "LoadClassWithOptions",
            "locla",
        ),
        (
            r"\documentclass{locla}\begin{document}",
            r"\documentclass{locla}\begin",
            1,
            "locla.cls",
            "documentclass",
            "locla",
        ),
        (
            "\\LoadClass % keep\r\n{ locla }\r\n\\newcommand{\\hello}{}",
            r"\newcommand",
            3,
            "locla.cls",
            "LoadClass",
            "locla",
        ),
        (
            r"😀 long prefix \usepackage{locla}\begin{document}",
            r"...\usepackage{locla}\begin",
            1,
            "locla.sty",
            "usepackage",
            "locla",
        ),
        (
            r"\usepackage{locla.sty}\begin{document}",
            r"\usepackage{locla.sty}\begin",
            1,
            "locla.sty.sty",
            "usepackage",
            "locla.sty",
        ),
        (
            "\\usepackage[draft [literal]{% head, phantom, }\r\n local% first\r\n,% separator, }\r\n locla% missing\r\n}\r\n\\begin{document}",
            r"\begin", 6, "locla.sty", "usepackage", "locla",
        ),
        (
            "\\RequirePackageWithOptions{% head, phantom, }\n local% first\n,locla% missing\n}\n\\endinput",
            r"\endinput", 5, "locla.sty", "RequirePackageWithOptions", "locla",
        ),
        ("\\documentclass[draft [literal]{% head, phantom, }\r\n my class,nmae% missing\r\n}\r\n\\begin{document}", r"\begin", 4, "my class,nmae.cls", "documentclass", "my class,nmae"),
        ("\\LoadClassWithOptions{% head, phantom, }\n my class,nmae% missing\n}\n\\endinput", r"\endinput", 4, "my class,nmae.cls", "LoadClassWithOptions", "my class,nmae"),
    ] {
        for numbered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            std::fs::write(&main, source).unwrap();
            let stop = if numbered {
                format!("./main.tex:{line}: Emergency stop.")
            } else {
                "! Emergency stop.".into()
            };
            let log = format!("(./main.tex\n! LaTeX Error: File `{file}' not found.\n\nType X to quit or <RETURN> to proceed,\nor enter new name.\n\nEnter file name:\n{stop}\n<read *>\n\nl.{line} {shown}\n");
            let entries = build_log::parse(&main, &log);
            let uri = Url::from_file_path(&main).unwrap();
            let diagnostic = entries[&uri]
                .iter()
                .find(|entry| entry.message.contains("not found"))
                .unwrap();
            let code = if file.ends_with(".sty") {
                "missing-package"
            } else {
                "missing-class"
            };
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String(code.into())),
                "{log}\n{diagnostic:?}"
            );
            let text = Text::new(source.into());
            assert_eq!(
                &text.source[text.offset(diagnostic.range.start).unwrap()
                    ..text.offset(diagnostic.range.end).unwrap()],
                expected
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["file"], file);
            assert_eq!(diagnostic.data.as_ref().unwrap()["loaderCommand"], command);
        }
    }
}

#[test]
fn uncertain_missing_loader_sources_names_and_contexts_keep_line_diagnostics() {
    for (source, shown, file, frame) in [
        (
            r"\usepackage{locla,\dynamic}\begin{}",
            r"\usepackage{locla,\dynamic}\begin",
            "locla.sty",
            "",
        ),
        (r"\usepackage{locla}\begin{}", "...", "locla.sty", ""),
        (r"\usepackage{locla}\begin{}", "...n", "locla.sty", ""),
        (
            r"\usepackage{locla}[broken",
            r"\usepackage{locla}",
            "locla.sty",
            "",
        ),
        (
            r"\newcommand{\load}{\usepackage{locla}\begin{document}}\load\begin",
            r"...\begin",
            "locla.sty",
            "",
        ),
        (
            r"\usepackage{\dynamic}\begin{document}",
            r"\usepackage{\dynamic}\begin",
            "locla.sty",
            "",
        ),
        (
            r"% \usepackage{locla}\begin{document}",
            r"% \usepackage{locla}\begin",
            "locla.sty",
            "",
        ),
        (
            r"\verb|\usepackage{locla}\begin{document}|",
            r"\verb|\usepackage{locla}\begin",
            "locla.sty",
            "",
        ),
        (
            r"\usepackage{locla}\begin{document}",
            r"\different{locla}\begin",
            "locla.sty",
            "",
        ),
        (
            r"\usepackage{locla}\begin{document}",
            r"\usepackage{locla}\begin",
            "other.sty",
            "",
        ),
        (
            r"\usepackage{locla.sty}\begin{document}",
            r"\usepackage{locla.sty}\begin",
            "locla.sty",
            "",
        ),
        (
            r"\usepackage{locla}\begin{document}",
            r"\usepackage{locla}\begin",
            "locla.sty",
            "<argument> \\expanded\n",
        ),
        (
            r"\usepackage{locla}\begin{}\usepackage{locla}\begin{}",
            r"...\usepackage{locla}\begin",
            "locla.sty",
            "",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, source).unwrap();
        let log = format!("(./main.tex\n! LaTeX Error: File `{file}' not found.\n! Emergency stop.\n<read *>\n{frame}l.1 {shown}\n");
        let entries = build_log::parse(&main, &log);
        let uri = Url::from_file_path(&main).unwrap();
        let diagnostic = entries[&uri]
            .iter()
            .find(|entry| entry.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{source}: {diagnostic:?}");
        assert!(diagnostic.data.is_none(), "{source}: {diagnostic:?}");
    }
}

fn warning(workspace: &mut Workspace, uri: &Url, loader: &str, name: &str) -> Diagnostic {
    let source = &workspace.documents[uri].text;
    let start = source.source.find(name).unwrap();
    let extension = if matches!(
        loader,
        "usepackage" | "RequirePackage" | "RequirePackageWithOptions"
    ) {
        "sty"
    } else {
        "cls"
    };
    let file = format!(
        "{}.{extension}",
        if extension == "sty" {
            name.replace([' ', '\t', '\r', '\n'], "")
        } else {
            name.into()
        }
    );
    let diagnostic = Diagnostic {
        range: source.range(start, start + name.len()),
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String(
            if extension == "sty" {
                "missing-package"
            } else {
                "missing-class"
            }
            .into(),
        )),
        message: format!("LaTeX Error: File `{file}' not found."),
        data: Some(serde_json::json!({"file":file,"loaderCommand":loader})),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        uri.clone(),
        [(uri.clone(), vec![diagnostic.clone()])].into(),
    );
    diagnostic
}

fn actions(workspace: &Workspace, uri: &Url, diagnostic: Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic],
            ..Default::default()
        },
    )
}

#[test]
fn loader_repairs_preserve_options_lists_versions_and_fix_redundant_suffixes() {
    for (source, loader, name, filename, replacement) in [
        ("\\documentclass[draft [literal]{% keep, phantom, }\r\n my class,nmae% typo\r\n}", "documentclass", "my class,nmae", "my class,name.cls", "my class,name"),
        ("\\LoadClassWithOptions{% keep, phantom\n my class,nmae% typo\n}", "LoadClassWithOptions", "my class,nmae", "my class,name.cls", "my class,name"),
        ("\\usepackage[draft [literal]{% keep, phantom, }\r\n graphicx% first\r\n, lcoal% typo\r\n}", "usepackage", "lcoal", "local.sty", "local"),
        ("\\RequirePackageWithOptions{% keep, phantom\n lcoal% typo\n}", "RequirePackageWithOptions", "lcoal", "local.sty", "local"),
        (
            r"\usepackage[draft]{graphicx, lcoal }[2024/01/01]",
            "usepackage",
            "lcoal",
            "local.sty",
            "local",
        ),
        (
            r"\RequirePackage{local.sty}",
            "RequirePackage",
            "local.sty",
            "local.sty",
            "local",
        ),
        (
            r"\RequirePackageWithOptions{graphicx, lcoal }[2024/01/01]",
            "RequirePackageWithOptions",
            "lcoal",
            "local.sty",
            "local",
        ),
        (
            r"\LoadClassWithOptions{lcoal}[2024/01/01]",
            "LoadClassWithOptions",
            "lcoal",
            "local.cls",
            "local",
        ),
        (
            r"\documentclass[12pt]{local.cls}",
            "documentclass",
            "local.cls",
            "local.cls",
            "local",
        ),
        (
            r"\LoadClass{lcoal}[2024/01/01]",
            "LoadClass",
            "lcoal",
            "local.cls",
            "local",
        ),
        (
            r"\usepackage{my locla}",
            "usepackage",
            "my locla",
            "mylocal.sty",
            "mylocal",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("main.tex"), source).unwrap();
        std::fs::write(root.join(filename), "Loaded source.").unwrap();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let diagnostic = warning(&mut workspace, &uri, loader, name);
        let result = actions(&workspace, &uri, diagnostic);
        assert_eq!(result.len(), 1, "{source}: {result:?}");
        let CodeActionOrCommand::CodeAction(action) = &result[0] else {
            panic!()
        };
        let edit = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
        let fixed = Text::new(source.into())
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        assert_eq!(fixed.source, source.replacen(name, replacement, 1));
    }
}

#[test]
fn loader_repairs_revalidate_legacy_versions_metadata_and_file_existence() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\usepackage{locla}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("local.sty"), "Loaded source.").unwrap();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace {
        apply_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(uri.clone(), source.into(), 7);
    let diagnostic = warning(&mut workspace, &uri, "usepackage", "locla");
    let result = actions(&workspace, &uri, diagnostic.clone());
    let CodeActionOrCommand::Command(command) = &result[0] else {
        panic!()
    };
    let request = command.arguments.as_ref().unwrap()[0].clone();
    let resolve = |workspace: &Workspace| {
        editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
    };
    assert!(resolve(&workspace).is_some());
    workspace.documents.get_mut(&uri).unwrap().version = Some(8);
    assert!(resolve(&workspace).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(7);
    std::fs::remove_file(root.join("local.sty")).unwrap();
    assert!(resolve(&workspace).is_none());
    std::fs::write(root.join("local.sty"), "Loaded source.").unwrap();
    std::fs::write(root.join("locla.sty"), "Restored source.").unwrap();
    assert!(resolve(&workspace).is_none());
    std::fs::remove_file(root.join("locla.sty")).unwrap();
    let mut invalid = diagnostic;
    invalid.data.as_mut().unwrap()["loaderCommand"] = "documentclass".into();
    workspace
        .build_diagnostics
        .get_mut(&uri)
        .unwrap()
        .get_mut(&uri)
        .unwrap()[0] = invalid.clone();
    assert!(actions(&workspace, &uri, invalid).is_empty());
}

#[test]
fn package_names_discard_tex_spaces_while_class_names_preserve_internal_spaces() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{my local}\\usepackage{my\tlocal}\n\\RequirePackage{ }";
    std::fs::write(root.join("main.tex"), source).unwrap();
    for file in [
        "my local.cls",
        "mylocal.cls",
        "my local.sty",
        "mylocal.sty",
        ".sty",
    ] {
        std::fs::write(root.join(file), "Loaded source.").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let links = latex_lsp::navigation::document_links(&workspace, &uri);
    assert_eq!(links.len(), 2, "{links:?}");
    assert_eq!(
        links[0].target,
        Some(Url::from_file_path(root.join("my local.cls")).unwrap())
    );
    assert_eq!(
        links[1].target,
        Some(Url::from_file_path(root.join("mylocal.sty")).unwrap())
    );
    let rename = |old: &str, new: &str| FileRename {
        old_uri: Url::from_file_path(root.join(old)).unwrap().to_string(),
        new_uri: Url::from_file_path(root.join(new)).unwrap().to_string(),
    };
    assert!(latex_lsp::file_operations::will_rename(
        &workspace,
        &[rename("mylocal.cls", "decoy.cls")]
    )
    .unwrap()
    .is_none());
    assert!(latex_lsp::file_operations::will_rename(
        &workspace,
        &[rename("mylocal.sty", "new package.sty")]
    )
    .is_err());
    let class_edit = latex_lsp::file_operations::will_rename(
        &workspace,
        &[rename("my local.cls", "new class.cls")],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(class_edit[&uri].len(), 1);
    assert_eq!(class_edit[&uri][0].new_text, "new class");
    let edit = latex_lsp::file_operations::will_rename(
        &workspace,
        &[rename("mylocal.sty", "renamed.sty")],
    )
    .unwrap()
    .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes[&uri].len(), 1);
    assert_eq!(changes[&uri][0].new_text, "renamed");
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_missing_package_and_class_names_offer_versioned_repairs_that_compile() {
    for engine in ["pdflatex", "xelatex"] {
        for loader in [
            "usepackage",
            "RequirePackage",
            "RequirePackageWithOptions",
            "documentclass",
            "LoadClass",
            "LoadClassWithOptions",
            "spaced-package",
            "redundant-package",
            "redundant-class",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = match loader {
            "usepackage" => "\\documentclass{article}\n\\usepackage{localpckg}\n\\begin{document}Hello.\\end{document}\n",
            "RequirePackage" => "\\RequirePackage{localpckg}\n\\documentclass{article}\n\\begin{document}Hello.\\end{document}\n",
            "RequirePackageWithOptions" => "\\documentclass{article}\n\\usepackage{wrapperpkg}\n\\begin{document}Hello.\\end{document}\n",
            "documentclass" => "\\documentclass{localcalss}\n\\begin{document}Hello.\\end{document}\n",
            "spaced-package" => "\\documentclass{article}\n\\usepackage{my localpckg}\n\\begin{document}Hello.\\end{document}\n",
            "redundant-package" => "\\documentclass{article}\n\\usepackage{localpkg.sty}\n\\begin{document}Hello.\\end{document}\n",
            "redundant-class" => "\\documentclass{localclass.cls}\n\\begin{document}Hello.\\end{document}\n",
            _ => "\\documentclass{wrapper}\n\\begin{document}Hello.\\end{document}\n",
        };
            std::fs::write(root.join("main.tex"), source).unwrap();
            std::fs::write(root.join("localpkg.sty"), r"\typeout{CORRECT PACKAGE}").unwrap();
            std::fs::write(root.join("mylocalpkg.sty"), r"\typeout{CORRECT PACKAGE}").unwrap();
            std::fs::write(root.join("my localpkg.sty"), r"\typeout{SPACED DECOY}").unwrap();
            std::fs::write(
                root.join("localclass.cls"),
                r"\LoadClass{article}\typeout{CORRECT CLASS}",
            )
            .unwrap();
            std::fs::write(
                root.join("wrapper.cls"),
                if loader == "LoadClassWithOptions" {
                    "\\LoadClassWithOptions{localcalss}\n\\relax\n"
                } else {
                    "\\LoadClass{localcalss}\n\\relax\n"
                },
            )
            .unwrap();
            std::fs::write(
                root.join("wrapperpkg.sty"),
                "\\RequirePackageWithOptions{localpckg}\n\\relax\n",
            )
            .unwrap();
            let mut workspace = Workspace {
                code_action_literals: true,
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let uri = if matches!(loader, "LoadClass" | "LoadClassWithOptions") {
                Url::from_file_path(root.join("wrapper.cls")).unwrap()
            } else if loader == "RequirePackageWithOptions" {
                Url::from_file_path(root.join("wrapperpkg.sty")).unwrap()
            } else {
                main.clone()
            };
            let source = std::fs::read_to_string(uri.to_file_path().unwrap()).unwrap();
            workspace.open(uri.clone(), source.clone(), 7);
            let config = latex_lsp::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let before = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(before.status, "error", "{loader}: {}", before.output);
            let code = if matches!(
                loader,
                "usepackage"
                    | "RequirePackage"
                    | "RequirePackageWithOptions"
                    | "spaced-package"
                    | "redundant-package"
            ) {
                "missing-package"
            } else {
                "missing-class"
            };
            let diagnostic = before.diagnostics[&uri]
                .iter()
                .find(|entry| entry.code == Some(NumberOrString::String(code.into())))
                .unwrap()
                .clone();
            workspace.build_diagnostics.insert(main, before.diagnostics);
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic],
                    ..Default::default()
                },
            );
            assert_eq!(actions.len(), 1, "{loader}: {actions:?}");
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            let Some(DocumentChanges::Edits(changes)) =
                action.edit.as_ref().unwrap().document_changes.as_ref()
            else {
                panic!()
            };
            assert_eq!(changes[0].text_document.version, Some(7));
            let OneOf::Left(edit) = &changes[0].edits[0] else {
                panic!()
            };
            let fixed = Text::new(source)
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text.clone(),
                }])
                .unwrap();
            std::fs::write(uri.to_file_path().unwrap(), fixed.source).unwrap();
            let after = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(after.status, "success", "{loader}: {}", after.output);
            assert!(after.output.contains(if code == "missing-package" {
                "CORRECT PACKAGE"
            } else {
                "CORRECT CLASS"
            }));
            assert!(!after.output.contains("SPACED DECOY"));
        }
    }
}

#[test]
fn missing_loader_diagnostics_do_not_borrow_missing_sources_or_invalid_reported_lines() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, r"\usepackage{locla}\begin{document}").unwrap();
    for (file, line) in [("./missing.tex", 1), ("./main.tex", 0), ("./main.tex", 9)] {
        let log = format!("(./main.tex\n{file}:{line}: LaTeX Error: File `locla.sty' not found.\n{file}:{line}: Emergency stop.\n<read *>\nl.{line} \\usepackage{{locla}}\\begin\n");
        let entries = build_log::parse(&main, &log);
        let diagnostic = entries[&Url::from_file_path(&main).unwrap()]
            .iter()
            .find(|entry| entry.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{log}\n{diagnostic:?}");
    }
}
