use latex_lsp::{features, signature, syntax::Syntax, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn workspace(source: &str) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    (directory, workspace, uri)
}

fn renamed(workspace: &Workspace, uri: &Url, offset: usize, name: &str) -> String {
    let edit = features::rename(
        workspace,
        uri,
        workspace.documents[uri].text.position(offset),
        name,
    )
    .unwrap()
    .unwrap();
    let mut edits = edit.changes.unwrap()[uri].clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[uri]
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
        .source
}

#[test]
fn literal_string_arguments_do_not_declare_symbols_load_files_or_reference_names() {
    let data = concat!(
        "\\newbool{ghost}\\newtoggle{ghost}\\newcounter{ghost}\\newcommand{\\ghost}{}",
        "\\label{ghost}\\ref{ghost}\\cite{ghost}\\input{ghost}",
        "\\begin{ghost}\\end{ghost}\\ifbool{ghost}{Y}{N}",
    );
    for name in ["ifstrequal", "ifstrempty", "ifblank", "notblank"] {
        let second = if name == "ifstrequal" {
            format!("{{{data}}}")
        } else {
            String::new()
        };
        let source = format!(
            "\\{name}{{{data}}}{second}{{\\label{{yes}}}}{{\\label{{no}}}}\\newbool{{real}}\n"
        );
        let parsed = Syntax::latex(&Text::new(source));
        assert!(
            parsed
                .commands
                .iter()
                .all(|command| !command.name.contains("ghost")
                    && command
                        .argument
                        .as_ref()
                        .is_none_or(|argument| argument.value != "ghost")),
            "{name}: {:?}",
            parsed.commands
        );
        assert_eq!(
            parsed
                .labels
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["yes", "no"]
        );
        assert!(parsed.definitions.is_empty());
        assert!(parsed.toggles.is_empty());
        assert!(parsed.counters.is_empty());
        assert_eq!(
            parsed
                .conditionals
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["real"]
        );
        assert!(
            parsed.diagnostics.is_empty(),
            "{name}: {:?}",
            parsed.diagnostics
        );
    }
}

#[test]
fn boolean_rename_preserves_test_tokens_and_updates_only_executable_branches() {
    let source = concat!(
        "\\newbool{draft}\\newtoggle{draft}\n",
        "\\ifstrequal{\\ifbool{draft}}{\\ifbool{release}}{\\booltrue{draft}}{\\boolfalse{draft}}\n",
        "\\ifstrempty{\\ifdraft}{\\booltrue{draft}}{\\boolfalse{draft}}\n",
        "\\ifblank{\\drafttrue}{\\booltrue{draft}}{\\boolfalse{draft}}\n",
        "\\notblank{\\newbool{draft}}{\\booltrue{draft}}{\\boolfalse{draft}}\n",
        "\\ifboolexpr{test{\\ifstrequal{\\ifbool{draft}}{X}}or bool{draft}}{Yes}{No}\n",
    );
    let (_directory, workspace, uri) = workspace(source);
    let position = Text::new(source.into()).position(source.find("draft").unwrap());
    assert_eq!(
        features::references(&workspace, &uri, position, true).len(),
        10
    );
    let changed = renamed(&workspace, &uri, source.find("draft").unwrap(), "release");
    let expected = source
        .replacen("\\newbool{draft}", "\\newbool{release}", 1)
        .replace("\\booltrue{draft}", "\\booltrue{release}")
        .replace("\\boolfalse{draft}", "\\boolfalse{release}")
        .replace("or bool{draft}", "or bool{release}");
    assert_eq!(changed, expected);
    let literal = Text::new(source.into()).position(source.find("\\ifbool{draft}").unwrap() + 9);
    assert!(features::definitions(&workspace, &uri, literal).is_empty());
    assert!(features::prepare_rename(&workspace, &uri, literal)
        .unwrap()
        .is_none());
    assert!(features::hover(&workspace, &uri, literal).is_none());
}

#[test]
fn string_data_is_opaque_for_completion_but_signatures_and_branches_remain_available() {
    for (marked, expected, active) in [
        (
            r"\ifstrequal{\ifbool{dra¦ft}}{B}{Y}{N}",
            r"\ifstrequal{string}{string}{then}{else}",
            0,
        ),
        (
            r"\ifstrequal{A}{\label{dra¦ft}}{Y}{N}",
            r"\ifstrequal{string}{string}{then}{else}",
            1,
        ),
        (
            r"\ifstrempty{\ifbool{dra¦ft}}{Y}{N}",
            r"\ifstrempty{string}{then}{else}",
            0,
        ),
        (
            r"\ifblank{\ifbool{dra¦ft}}{Y}{N}",
            r"\ifblank{string}{then}{else}",
            0,
        ),
        (
            r"\notblank{\ifbool{dra¦ft}}{Y}{N}",
            r"\notblank{string}{then}{else}",
            0,
        ),
    ] {
        let source = format!("\\newbool{{draft}}\n{}", marked.replace('¦', ""));
        let (_directory, workspace, uri) = workspace(&source);
        let offset = source.find('\n').unwrap() + 1 + marked.find('¦').unwrap();
        let position = Text::new(source).position(offset);
        assert!(
            features::completion(&workspace, &uri, position).is_empty(),
            "{marked}"
        );
        let help =
            signature::help(&workspace, &uri, position).unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.signatures[0].label, expected, "{marked}");
        assert_eq!(help.active_parameter, Some(active), "{marked}");
    }
    let marked = r"\ifstrequal{A}{B}{\ifbool{dra¦ft}{Y}{N}}{No}";
    let source = format!("\\newbool{{draft}}\n{}", marked.replace('¦', ""));
    let (_directory, workspace, uri) = workspace(&source);
    let position = Text::new(source.clone())
        .position(source.find('\n').unwrap() + 1 + marked.find('¦').unwrap());
    assert!(features::completion(&workspace, &uri, position)
        .iter()
        .any(|item| item.label == "draft"));
    assert_eq!(
        signature::help(&workspace, &uri, position)
            .unwrap()
            .signatures[0]
            .label,
        r"\ifbool{boolean}{then}{else}"
    );
}

#[test]
fn literal_token_boundaries_comments_unfinished_groups_and_local_redefinitions_are_respected() {
    for source in [
        r"\ifstrempty\label{\label{real}}{No}",
        r"\ifblank*{\label{real}}{No}",
        "\\ifstrequal% first\n{\\label{ghost}}% second\n{\\input{ghost}}{\\label{real}}{No}",
        "\\ifblank{\\verb|{|\\label{ghost}|}|}{\\label{real}}{No}",
    ] {
        let parsed = Syntax::latex(&Text::new(source.into()));
        assert_eq!(
            parsed
                .labels
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["real"],
            "{source}: {:?}",
            parsed.labels
        );
        assert!(
            parsed.diagnostics.is_empty(),
            "{source}: {:?}",
            parsed.diagnostics
        );
    }
    for source in [r"\ifblank{\label{ghost}", r"\ifstrequal{A}{\label{ghost}"] {
        let parsed = Syntax::latex(&Text::new(source.into()));
        assert!(parsed.labels.is_empty(), "{source}");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("Unclosed")),
            "{:?}",
            parsed.diagnostics
        );
    }
    for (source, expected) in [
        (r"\ifblank{Data% tail", "\n}"),
        (r"\ifstrequal{A}{Data% tail", "\n}"),
        (r"\ifblank{Data\% tail", "}"),
    ] {
        let parsed = Syntax::latex(&Text::new(source.into()));
        assert_eq!(
            parsed
                .fixes
                .iter()
                .find(|fix| fix.diagnostic.message == "Unclosed brace")
                .unwrap()
                .edit
                .new_text,
            expected
        );
    }
    for definition in [
        r"\renewcommand{\ifblank}[1]{#1}",
        r"\def\ifblank#1{#1}",
        r"\let\ifblank\relax",
    ] {
        let source = format!("{definition}\\ifblank{{\\label{{real}}}}");
        let parsed = Syntax::latex(&Text::new(source));
        assert!(
            parsed.labels.iter().any(|entry| entry.name == "real"),
            "{definition}: {:?}",
            parsed.labels
        );
    }
    let parsed = Syntax::latex(&Text::new(r"\let\probe\ifblank{\label{real}}".into()));
    assert_eq!(parsed.labels[0].name, "real");
}

#[test]
fn string_literal_files_stay_out_of_project_dependencies_and_rename_results() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let definitions = Url::from_file_path(root.join("definitions.tex")).unwrap();
    let source = "\\documentclass{article}\\input{definitions}\n\\ifstrequal{\\input{ghost}\\greet\\ref{mark}\\iftoggle{draft}}{Literal}{\\greet\\ref{mark}\\iftoggle{draft}{Y}{N}}{No}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(
        definitions.to_file_path().unwrap(),
        "\\newcommand{\\greet}{Hello}\\label{mark}\\newtoggle{draft}\n",
    )
    .unwrap();
    std::fs::write(root.join("ghost.tex"), "\\newbool{phantom}\n").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert_eq!(workspace.dependencies(&main), vec![definitions.clone()]);
    for (marker, offset, replacement, literal) in [
        (
            r"\greet\ref{mark}\iftoggle{draft}{Y}",
            2,
            "salute",
            r"\input{ghost}\greet\ref{mark}\iftoggle{draft}",
        ),
        (
            r"\ref{mark}\iftoggle{draft}{Y}",
            6,
            "newmark",
            r"\input{ghost}\greet\ref{mark}\iftoggle{draft}",
        ),
        (
            r"\iftoggle{draft}{Y}",
            11,
            "release",
            r"\input{ghost}\greet\ref{mark}\iftoggle{draft}",
        ),
    ] {
        let position = Text::new(source.into()).position(source.find(marker).unwrap() + offset);
        let edit = features::rename(&workspace, &main, position, replacement)
            .unwrap()
            .unwrap();
        let changes = edit.changes.unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[&main].len(), 1);
        assert_eq!(changes[&definitions].len(), 1);
        let changed = Text::new(source.into())
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(changes[&main][0].range),
                range_length: None,
                text: changes[&main][0].new_text.clone(),
            }])
            .unwrap();
        assert!(changed.source.contains(literal), "{}", changed.source);
    }
    for name in ["ifstrequal", "ifstrempty", "ifblank", "notblank"] {
        let source = format!("\\{name}");
        workspace.open(main.clone(), source.clone(), 1);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source).position(name.len() + 1),
        );
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert!(item.documentation.as_ref().is_some_and(
            |doc| matches!(doc, Documentation::String(text) if text.contains("etoolbox"))
        ));
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_string_tests_keep_branches_tokens_and_cross_file_rename_results_in_three_engines() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let original = concat!(
        "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\n",
        "\\begin{document}\n",
        "\\ifstrequal{\\ifbool{draft}}{\\ifbool{release}}{Wrong.}{\\typeout{STRINGS:distinct}}\n",
        "\\ifstrempty{\\ifdraft}{Wrong.}{\\typeout{STRINGS:notempty}\\booltrue{draft}}\n",
        "\\ifblank{\\drafttrue}{Wrong.}{\\typeout{STRINGS:notblank}}\n",
        "\\ifblank{\\input{ghost}}{Wrong.}{\\typeout{STRINGS:blank-noexpand}}\n",
        "\\notblank{\\input{ghost}}{\\typeout{STRINGS:notblank-noexpand}}{Wrong.}\n",
        "\\notblank{\\newbool{draft}}{\\typeout{STRINGS:tokens}}{Wrong.}\n",
        "\\ifstrequal{\\iftoggle{draft}}{\\iftoggle{release}}{Wrong.}{\\typeout{STRINGS:toggledistinct}\\toggletrue{draft}}\n",
        "\\ifboolexpr{test{\\ifstrequal{\\ifbool{draft}}{\\ifbool{release}}} or bool{draft}}{\\typeout{STRINGS:expression}}{Wrong.}\n",
        "\\ifblank{   }{\\typeout{STRINGS:spaces}}{Wrong.}\n",
        "\\notblank{} {Wrong.}{\\typeout{STRINGS:empty}}\n",
        "\\ifstrempty{\\verb|{|\\input{ghost}|}|}{Wrong.}{\\typeout{STRINGS:raw}}\n",
        "\\ifbool{draft}{\\iftoggle{draft}{\\typeout{STRINGS:state}}{Wrong.}}{Wrong.}\n",
        "Text.\\end{document}\n",
    );
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let definitions_path = root.join("definitions.tex");
        std::fs::write(&main, original).unwrap();
        std::fs::write(&definitions_path, "\\newbool{draft}\\newtoggle{draft}\n").unwrap();
        let uri = Url::from_file_path(&main).unwrap();
        let definitions = Url::from_file_path(&definitions_path).unwrap();
        let wrapper = root.join("engine");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = BuildConfig {
            executable: wrapper.to_string_lossy().into_owned(),
            arguments: Some(vec![
                "-interaction=nonstopmode".into(),
                "-no-shell-escape".into(),
                "-file-line-error".into(),
                "-recorder".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            timeout_seconds: 20,
            ..Default::default()
        };
        let before = build::run(&main, &config, &Default::default());
        assert_eq!(before.status, "success", "{engine}: {}", before.output);
        let states = |output: &str| {
            output
                .lines()
                .filter(|line| line.starts_with("STRINGS:"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let expected = vec![
            "STRINGS:distinct",
            "STRINGS:notempty",
            "STRINGS:notblank",
            "STRINGS:blank-noexpand",
            "STRINGS:notblank-noexpand",
            "STRINGS:tokens",
            "STRINGS:toggledistinct",
            "STRINGS:expression",
            "STRINGS:spaces",
            "STRINGS:empty",
            "STRINGS:raw",
            "STRINGS:state",
        ];
        assert_eq!(states(&before.output), expected);
        for prefix in [r"\newbool{", r"\newtoggle{"] {
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let text = &workspace.documents[&definitions].text;
            let position = text.position(text.source.find(prefix).unwrap() + prefix.len());
            let edit = features::rename(&workspace, &definitions, position, "release")
                .unwrap()
                .unwrap();
            for (document, mut edits) in edit.changes.unwrap() {
                edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                let changed = workspace.documents[&document]
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
                std::fs::write(document.to_file_path().unwrap(), changed.source).unwrap();
            }
            let after = build::run(&main, &config, &Default::default());
            assert_eq!(
                after.status, "success",
                "{engine}/{prefix}: {}",
                after.output
            );
            assert_eq!(states(&after.output), expected);
            assert_eq!(after.pages, Some(1));
        }
        let changed = std::fs::read_to_string(&main).unwrap();
        for literal in [
            r"\ifstrequal{\ifbool{draft}}{\ifbool{release}}",
            r"\ifstrempty{\ifdraft}",
            r"\ifblank{\drafttrue}",
            r"\notblank{\newbool{draft}}",
            r"\ifstrequal{\iftoggle{draft}}{\iftoggle{release}}",
        ] {
            assert!(changed.contains(literal), "{engine}: {changed}");
        }
        let broken = changed.replace("or bool{release}", "or bool{relesae}");
        std::fs::write(&main, &broken).unwrap();
        let failed = build::run(&main, &config, &Default::default());
        assert_eq!(failed.status, "error", "{engine}: {}", failed.output);
        let error = failed.diagnostics[&uri]
            .iter()
            .find(|error| {
                error
                    .message
                    .starts_with("Package etoolbox Error: Invalid boolean expression.")
            })
            .unwrap()
            .clone();
        assert_eq!(
            error.code,
            Some(NumberOrString::String("undefined-boolean".into()))
        );
        let start = broken.find("relesae").unwrap();
        assert_eq!(
            error.range,
            Text::new(broken.clone()).range(start, start + 7)
        );
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        workspace
            .build_diagnostics
            .insert(uri.clone(), failed.diagnostics);
        workspace
            .build_diagnostic_sources
            .insert(uri.clone(), failed.diagnostic_sources);
        let actions = latex_lsp::editing::code_actions(
            &workspace,
            &uri,
            error.range,
            &CodeActionContext {
                diagnostics: vec![error],
                only: Some(vec![CodeActionKind::QUICKFIX]),
                trigger_kind: None,
            },
        );
        let edit = actions
            .into_iter()
            .find_map(|item| match item {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == "Replace with boolean `release`" =>
                {
                    action.edit
                }
                _ => None,
            })
            .unwrap();
        let edits = edit.changes.unwrap()[&uri].clone();
        assert_eq!(edits.len(), 1);
        let repaired = Text::new(broken)
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
        assert_eq!(repaired.source, changed);
        std::fs::write(&main, repaired.source).unwrap();
        let after = build::run(&main, &config, &Default::default());
        assert_eq!(after.status, "success", "{engine}: {}", after.output);
        assert_eq!(states(&after.output), expected);
        let workspace = {
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            workspace
        };
        assert_eq!(workspace.dependencies(&uri), vec![definitions]);
    }
}
