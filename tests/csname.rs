use latex_lsp::{build, editing, features, signature, workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

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
    renamed_to(workspace, document, position, "greeting")
}

fn renamed_to(
    workspace: &Workspace,
    document: &Url,
    position: Position,
    name: &str,
) -> BTreeMap<Url, String> {
    features::rename(workspace, document, position, name)
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
fn names_used_only_as_strings_can_be_renamed_to_unicode_and_nonletters() {
    for declaration in [
        r"\csdef{hello}#1{Hello #1}",
        r"\csgdef{hello}#1{Hello #1}",
        r"\expandafter\def\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\newcommand\csname hello\endcsname[1]{Hello #1}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "\\documentclass{article}\\input{definitions}\n\\begin{document}😀 \\csuse{hello}{hello}\\csname hello\\endcsname{Literal}\\ifcsdef{hello}{hello}{No}\n% \\hello \\csuse{hello}\n\\verb|\\hello|\\end{document}";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", declaration)],
        );
        let main = uri(root, "main.tex");
        let definitions = uri(root, "definitions.tex");
        let position = workspace.documents[&main]
            .text
            .position(source.find("hello").unwrap());
        for name in ["发布-模式", "hello-two", "hello.2"] {
            let changed = renamed_to(&workspace, &main, position, name);
            assert_eq!(changed[&definitions], declaration.replace("hello", name));
            assert_eq!(
                changed[&main],
                format!("\\documentclass{{article}}\\input{{definitions}}\n\\begin{{document}}😀 \\csuse{{{name}}}{{hello}}\\csname {name}\\endcsname{{Literal}}\\ifcsdef{{{name}}}{{hello}}{{No}}\n% \\hello \\csuse{{hello}}\n\\verb|\\hello|\\end{{document}}")
            );
        }
        for name in ["hello world", "hello\\suffix", "hello#1", "hello,other", ""] {
            assert!(
                features::rename(&workspace, &main, position, name).is_err(),
                "{name}"
            );
        }
    }
}

#[test]
fn direct_declarations_and_calls_restrict_new_names_in_every_consumer_project() {
    for (declaration, direct) in [
        (r"\newcommand{\hello}{Hello}", ""),
        (r"\csdef{hello}{Hello}", r"\hello"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(
            root,
            &[
                (
                    "a.tex",
                    r"\documentclass{article}\input{definitions}\begin{document}\csuse{hello}\end{document}",
                ),
                (
                    "b.tex",
                    &format!(
                        r"\documentclass{{article}}\input{{definitions}}\begin{{document}}{direct}\end{{document}}"
                    ),
                ),
                ("definitions.tex", declaration),
            ],
        );
        let main = uri(root, "a.tex");
        let text = &workspace.documents[&main].text;
        let position = text.position(text.source.find("hello").unwrap());
        for name in ["发布-模式", "hello-two", "hello.2"] {
            assert!(
                features::rename(&workspace, &main, position, name).is_err(),
                "{declaration}: {direct}: {name}"
            );
        }
        assert!(features::rename(&workspace, &main, position, "greeting")
            .unwrap()
            .is_some());
    }
}

#[test]
fn named_completion_includes_tex_io_and_expression_primitives_and_preserves_existing_arguments() {
    for (marked, name) in [
        (r"😀 \csuse{esc¦apechar}{Keep}", "escapechar"),
        (r"\ifcsequal{write}{op¦enin}{Yes}{No}", "openin"),
        (r"\csletcs{alias}{num¦expr}", "numexpr"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = project(directory.path(), &[("main.tex", &source)]);
        let main = uri(directory.path(), "main.tex");
        let text = &workspace.documents[&main].text;
        let items =
            features::completion(&workspace, &main, text.position(marked.find('¦').unwrap()));
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert_eq!(item.detail.as_deref(), Some("TeX primitive"));
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
            source
        );
    }
}

#[test]
fn named_macro_rename_cannot_overwrite_core_expansion_and_grouping_primitives() {
    let directory = tempfile::tempdir().unwrap();
    let source = r"\csdef{hello}{Hello}\csuse{hello}";
    let workspace = project(directory.path(), &[("main.tex", source)]);
    let main = uri(directory.path(), "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("hello").unwrap());
    for name in [
        "relax",
        "csname",
        "endcsname",
        "expandafter",
        "noexpand",
        "unexpanded",
        "expanded",
        "protected",
        "long",
        "outer",
        "global",
        "begingroup",
        "endgroup",
        "futurelet",
    ] {
        assert!(
            features::rename(&workspace, &main, position, name).is_err(),
            "{name}"
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn unicode_named_macro_renames_preserve_behavior_in_all_supported_engines() {
    use std::os::unix::fs::PermissionsExt;
    for executable in ["pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let declaration = r"\csdef{hello}#1{\typeout{VALUE:#1}#1}";
        let source = r"\documentclass{article}\usepackage{etoolbox}\input{definitions}
\begin{document}\csuse{hello}{Named}\csname hello\endcsname{Literal}\ifcsdef{hello}{\typeout{VALUE:defined}}{Wrong}\end{document}";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", declaration)],
        );
        let main = uri(root, "main.tex");
        let compiler = if executable == "lualatex" {
            let wrapper = root.join("lualatex");
            std::fs::write(&wrapper, "#!/bin/sh\nTEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"\n").unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
            wrapper.to_string_lossy().into_owned()
        } else {
            executable.into()
        };
        let config = latex_lsp::config::BuildConfig {
            executable: compiler,
            ..Default::default()
        };
        let before = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{executable}: {}", before.output);
        let position = workspace.documents[&main]
            .text
            .position(source.find("hello").unwrap());
        for (uri, source) in renamed_to(&workspace, &main, position, "发布-模式") {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let after = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{executable}: {}", after.output);
        assert_eq!(before.pages, after.pages);
        for output in [&before.output, &after.output] {
            for expected in ["VALUE:Named", "VALUE:Literal", "VALUE:defined"] {
                assert!(output.contains(expected), "{executable}: {output}");
            }
        }
    }
}

#[test]
fn literal_macro_names_support_navigation_prepared_rename_references_and_highlights() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newcommand{\\hello}[1]{Hello #1}\n";
    let source = "\\documentclass{article}\r\n\\input{definitions}\r\n\\begin{document}\r\n😀 \\csname hello\\endcsname{World}\r\n\\hello{Direct}\\csname  hello\\endcsname{Other}\r\n% \\csname hello\\endcsname\r\n\\verb|\\csname hello\\endcsname|\r\n\\end{document}\r\n";
    let workspace = project(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    for start in [
        source.find("hello").unwrap(),
        source.find("\\hello").unwrap() + 2,
        source.find("  hello").unwrap() + 2,
    ] {
        let position = text.position(start + 2);
        let locations = features::definitions(&workspace, &main, position);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].uri, uri(root, "definitions.tex"));
        assert!(features::hover(&workspace, &main, position).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &main, position).unwrap()
        else {
            panic!("missing prepared rename")
        };
        assert_eq!(placeholder, "hello");
        assert_eq!(
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            "hello"
        );
        assert_eq!(
            features::references(&workspace, &main, position, false).len(),
            3
        );
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            4
        );
        let highlights = editing::highlights(&workspace, &main, position);
        assert_eq!(highlights.len(), 3);
        assert!(highlights
            .iter()
            .all(|entry| entry.kind == Some(DocumentHighlightKind::READ)));
        let changed = renamed(&workspace, &main, position);
        assert_eq!(
            changed[&main],
            source
                .replace("😀 \\csname hello", "😀 \\csname greeting")
                .replace(
                    "\\hello{Direct}\\csname  hello",
                    "\\greeting{Direct}\\csname  greeting"
                )
        );
        assert_eq!(
            changed[&uri(root, "definitions.tex")],
            definitions.replace("hello", "greeting")
        );
    }
}

#[test]
fn computed_commented_and_unterminated_names_are_not_guessed_or_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newcommand{\\hello}{}\n\\csname hello world\\endcsname\n\\csname hello%comment\n\\endcsname\n\\csname \\prefix hello\\endcsname\n\\csname hello";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    let position = text.position(source.find("hello").unwrap());
    let changed = renamed(&workspace, &main, position);
    assert_eq!(changed[&main], source.replacen("\\hello", "\\greeting", 1));
    for (offset, _) in source.match_indices("hello").skip(1) {
        assert!(
            features::prepare_rename(&workspace, &main, text.position(offset + 2))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn shared_literal_calls_to_different_macro_definitions_are_ambiguous() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root, &[
        ("a.tex", "\\documentclass{article}\n\\newcommand{\\hello}{A}\n\\begin{document}\\input{shared}\\end{document}"),
        ("b.tex", "\\documentclass{article}\n\\newcommand{\\hello}{B}\n\\begin{document}\\input{shared}\\end{document}"),
        ("shared.tex", "\\csname hello\\endcsname"),
    ]);
    for file in ["a.tex", "shared.tex"] {
        let document = uri(root, file);
        let text = &workspace.documents[&document].text;
        let position = text.position(text.source.find("hello").unwrap() + 2);
        assert!(features::prepare_rename(&workspace, &document, position).is_err());
        assert!(features::rename(&workspace, &document, position, "greeting").is_err());
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn renamed_literal_macro_calls_keep_control_word_syntax_and_compile() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\csname hello\\endcsname{World}\n\\hello{Direct}\n\\end{document}\n";
    let workspace = project(
        root,
        &[
            ("main.tex", source),
            ("definitions.tex", "\\newcommand{\\hello}[1]{Hello #1}\n"),
        ],
    );
    let main = uri(root, "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("hello").unwrap() + 2);
    for (uri, source) in renamed(&workspace, &main, position) {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert_eq!(outcome.pages, Some(1));
}

#[test]
fn expanded_literal_definitions_index_the_resulting_name_and_preserve_declaration_roles() {
    for declaration in [
        r"\expandafter\def\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\gdef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\edef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\xdef\csname hello\endcsname#1{Hello #1}",
        "\\global\\expandafter % comment\n\\def % comment\n\\csname  hello\\endcsname#1{Hello #1}",
        r"\expandafter\newcommand\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\newrobustcmd\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\renewrobustcmd\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\providerobustcmd\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\NewDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\NewExpandableDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\RenewExpandableDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\ProvideExpandableDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\DeclareExpandableDocumentCommand\csname hello\endcsname{m}{Hello #1}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main_source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\hello{World} \\csname hello\\endcsname{Other}\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", main_source), ("definitions.tex", declaration)],
        );
        let definitions = uri(root, "definitions.tex");
        let main = uri(root, "main.tex");
        let document = &workspace.documents[&definitions];
        assert_eq!(document.syntax.definitions.len(), 1, "{declaration}");
        assert_eq!(document.syntax.definitions[0].name, "hello");
        assert_eq!(features::symbols(document)[0].name, "hello");
        let position = document
            .text
            .position(declaration.find("hello").unwrap() + 2);
        let references = features::references(&workspace, &definitions, position, false);
        assert_eq!(references.len(), 2, "{declaration}: {references:?}");
        assert!(references.iter().all(|entry| entry.uri == main));
        let highlights = editing::highlights(&workspace, &definitions, position);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
        let changed = renamed(&workspace, &definitions, position);
        assert_eq!(
            changed[&definitions],
            declaration.replace("hello", "greeting")
        );
        assert_eq!(changed[&main], main_source.replace("hello", "greeting"));
        let text = &workspace.documents[&main].text;
        let call = text.position(main_source.find("World").unwrap() + 2);
        let help = signature::help(&workspace, &main, call).unwrap();
        assert_eq!(help.signatures[0].label, r"\hello{#1}");
        assert_eq!(help.active_parameter, Some(0));
        let items = features::completion(
            &workspace,
            &main,
            text.position(main_source.find("\\hello").unwrap() + 3),
        );
        assert!(items
            .iter()
            .any(|item| item.label == "hello" && item.detail.as_deref() == Some(r"\hello{#1}")));
    }
}

#[test]
fn computed_expanded_definitions_do_not_define_the_csname_primitive() {
    for declaration in [
        r"\expandafter\def\csname hello\suffix\endcsname{Hello}",
        r"\expandafter\newcommand\csname hello\suffix\endcsname{Hello}",
        r"\expandafter\newrobustcmd\csname hello\suffix\endcsname{Hello}",
        "\\expandafter\\def\\csname hello% comment\n\\endcsname{Hello}",
        r"\expandafter\def\csname hello world\endcsname{Hello}",
        r"\expandafter\def\csname hello",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(root, &[("main.tex", declaration)]);
        let main = uri(root, "main.tex");
        let document = &workspace.documents[&main];
        assert!(document.syntax.definitions.is_empty(), "{declaration}");
        let position = document
            .text
            .position(declaration.find("csname").unwrap() + 2);
        assert!(features::prepare_rename(&workspace, &main, position)
            .unwrap()
            .is_none());
    }
}

#[test]
fn literal_aliases_and_nonletter_names_are_indexed_without_invalid_direct_completions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = r"\expandafter\let\csname hello\endcsname=\textbf
\expandafter\def\csname hello-two\endcsname{Other}";
    let workspace = project(
        root,
        &[("definitions.tex", definitions), ("main.tex", "\\hel")],
    );
    let main = uri(root, "main.tex");
    let definitions_uri = uri(root, "definitions.tex");
    assert_eq!(
        workspace.documents[&definitions_uri]
            .syntax
            .definitions
            .len(),
        2
    );
    let items = features::completion(&workspace, &main, Position::new(0, 4));
    assert!(items.iter().any(|item| item.label == "hello"));
    assert!(!items.iter().any(|item| item.label == "hello-two"));
    let text = &workspace.documents[&definitions_uri].text;
    let position = text.position(definitions.find("hello-two").unwrap() + 2);
    assert_eq!(
        renamed(&workspace, &definitions_uri, position)[&definitions_uri],
        definitions.replace("hello-two", "greeting")
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn renamed_expanded_literal_definitions_and_aliases_still_compile() {
    for declaration in [
        r"\expandafter\def\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\gdef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\edef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\xdef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\newcommand\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\NewDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\NewExpandableDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\ProvideExpandableDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\DeclareExpandableDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\let\csname hello\endcsname=\textbf",
        r"\expandafter\NewCommandCopy\csname hello\endcsname{\textbf}",
        r"\expandafter\DeclareCommandCopy\csname hello\endcsname\textbf",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\hello{World}\\csname hello\\endcsname{Other}\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", declaration)],
        );
        let definitions = uri(root, "definitions.tex");
        let position = workspace.documents[&definitions]
            .text
            .position(declaration.find("hello").unwrap() + 2);
        for (uri, source) in renamed(&workspace, &definitions, position) {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{declaration}: {}",
            outcome.output
        );
        assert_eq!(outcome.pages, Some(1));
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn robust_macro_renames_preserve_defaults_literal_calls_and_protected_expansion() {
    for declaration in [
        r"\newrobustcmd{\hello}",
        r"\newrobustcmd*\hello",
        r"\providerobustcmd\hello",
        r"\expandafter\newrobustcmd\csname hello\endcsname",
        r"\expandafter\providerobustcmd\csname hello\endcsname",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("{declaration}[2][blue]{{\\typeout{{ROBUST:#1:#2}}#2}}\n");
        let source = "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\n\\begin{document}\n\\hello{Default}\\hello[red]{Explicit}\\csname hello\\endcsname{Literal}\n\\edef\\protectedcall{\\hello{Protected}}\\typeout{ROBUST:before-call}\\protectedcall\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = root.join("main.tex");
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let before = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{declaration}: {}", before.output);
        let document = uri(root, "definitions.tex");
        let text = &workspace.documents[&document].text;
        let position = text.position(definitions.find("hello").unwrap() + 2);
        for (uri, changed) in renamed(&workspace, &document, position) {
            std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
        }
        let after = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{declaration}: {}", after.output);
        for output in [&before.output, &after.output] {
            for state in [
                "ROBUST:blue:Default",
                "ROBUST:red:Explicit",
                "ROBUST:blue:Literal",
                "ROBUST:blue:Protected",
            ] {
                assert!(output.contains(state), "{declaration}: {state}\n{output}");
            }
            assert!(
                output.find("ROBUST:before-call").unwrap()
                    < output.find("ROBUST:blue:Protected").unwrap()
            );
        }
    }
}

#[test]
fn named_command_apis_navigate_and_rename_only_control_sequence_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = r"\newrobustcmd{\hello}[1]{#1}\newcommand{\other}{}";
    let source = "\\documentclass{article}\\input{definitions}\n\\begin{document}\n\\csuse{hello}{hello}\\csmeaning{hello}\n\\ifcsdef{hello}{hello}{No}\\ifcsundef{hello}{No}{Yes}\\ifcsvoid{hello}{No}{Yes}\\ifcsmacro{hello}{Yes}{No}\\ifcsparam{hello}{Yes}{No}\\ifcsprotected{hello}{Yes}{No}\\ifcsltxprotect{hello}{Yes}{No}\n\\ifcsstring{hello}{hello}{Yes}{No}\\ifcsequal{hello}{hello}{Yes}{No}\\ifcsstrequal{other}{hello}{Yes}{No}\n\\csuse{hello\\suffix}{Dynamic}\\ifcsdef{other}{hello}{No}\n% \\csuse{hello}\n\\verb|\\csuse{hello}|\\end{document}\n";
    let workspace = project(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = uri(root, "main.tex");
    for marker in [
        "csuse{hello}",
        "ifcsstring{hello}",
        "ifcsequal{hello}",
        "ifcsstrequal{other}{hello}",
    ] {
        let text = &workspace.documents[&main].text;
        let offset = source.find(marker).unwrap() + marker.find("hello").unwrap() + 2;
        let position = text.position(offset);
        let targets = features::definitions(&workspace, &main, position);
        assert_eq!(targets.len(), 1, "{marker}: {targets:?}");
        assert_eq!(targets[0].uri, uri(root, "definitions.tex"));
        assert!(features::hover(&workspace, &main, position).is_some());
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            14
        );
        let changed = renamed(&workspace, &main, position);
        assert_eq!(changed[&main].matches("greeting").count(), 13);
        for preserved in [
            "{greeting}{hello}",
            "ifcsstring{greeting}{hello}",
            "csuse{hello\\suffix}",
            "ifcsdef{other}{hello}",
            "% \\csuse{hello}",
            "\\verb|\\csuse{hello}|",
        ] {
            assert!(
                changed[&main].contains(preserved),
                "{preserved}: {}",
                changed[&main]
            );
        }
    }
}

#[test]
fn named_command_completion_preserves_suffixes_and_skips_strings_dynamic_names_and_other_projects()
{
    for (marked, expected) in [
        (
            r"😀 \csuse{ he¦llo }{Existing}",
            r"😀 \csuse{hello}{Existing}",
        ),
        (
            r"\ifcsequal{other}{hel¦lo}{Yes}{No}",
            r"\ifcsequal{other}{hello}{Yes}{No}",
        ),
        (
            r"\ifcsstrequal{hel¦lo}{other}{Yes}{No}",
            r"\ifcsstrequal{hello}{other}{Yes}{No}",
        ),
        (r"\csmeaning{hel¦", r"\csmeaning{hello"),
        (r"\csappto{ hel¦lo }{hello}", r"\csappto{hello}{hello}"),
        (r"\csepreto{hel¦lo}{hello}", r"\csepreto{hello}{hello}"),
        (r"\csundef{hel¦", r"\csundef{hello"),
        (r"\csgundef{hel¦lo}", r"\csgundef{hello}"),
        (r"\csletcs{mirror}{hel¦lo}", r"\csletcs{mirror}{hello}"),
        (r"\letcs{\mirror}{hel¦lo}", r"\letcs{\mirror}{hello}"),
        (r"\letcs\mirror{hel¦", r"\letcs\mirror{hello"),
        (
            r"\expandafter\letcs\csname mirror\endcsname{hel¦lo}",
            r"\expandafter\letcs\csname mirror\endcsname{hello}",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = project(
            directory.path(),
            &[
                ("main.tex", &source),
                ("definitions.tex", r"\newcommand{\hello}[1]{#1}"),
            ],
        );
        let main = uri(directory.path(), "main.tex");
        let text = &workspace.documents[&main].text;
        let items =
            features::completion(&workspace, &main, text.position(marked.find('¦').unwrap()));
        let item = items.iter().find(|item| item.label == "hello").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
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
        r"\ifcsstring{hello}{hel¦lo}{Yes}{No}",
        r"\ifcsdef{other}{hel¦lo}{No}",
        r"\csuse{hel¦lo\suffix}",
        r"\verb|\csuse{hel¦lo}|",
        r"\ifcsequal{other}{hello}{hel¦lo}{No}",
        r"\csappto{hello}{hel¦lo}",
        r"\csepreto{hello}{hel¦lo}",
        r"\csundef{hel¦lo\suffix}",
        r"\csletcs{hel¦lo}{other}",
        r"\cslet{hel¦lo}{\other}",
        r"\letcs{\mirror\other}{hel¦lo}",
        r"\csletcs{mirror}{hel¦lo\suffix}",
        r"\expandafter\letcs\csname mirror\suffix\endcsname{hel¦lo}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let workspace = project(
            directory.path(),
            &[
                ("main.tex", &source),
                ("definitions.tex", r"\newcommand{\hello}{}"),
            ],
        );
        let main = uri(directory.path(), "main.tex");
        assert!(
            features::completion(
                &workspace,
                &main,
                workspace.documents[&main]
                    .text
                    .position(marked.find('¦').unwrap())
            )
            .is_empty(),
            "{marked}"
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let source =
        r"\documentclass{article}\newcommand{\hello}{}\begin{document}\csuse{hel}\end{document}";
    let workspace = project(
        directory.path(),
        &[
            ("a.tex", source),
            (
                "b.tex",
                r"\documentclass{article}\newcommand{\helpother}{}\begin{document}Other.\end{document}",
            ),
        ],
    );
    let main = uri(directory.path(), "a.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("csuse{hel}").unwrap() + 9);
    let items = features::completion(&workspace, &main, position);
    assert!(items.iter().any(|item| item.label == "hello"));
    assert!(!items.iter().any(|item| item.label == "helpother"));
}

#[test]
fn named_mutations_navigate_and_rename_without_treating_code_as_names_or_declarations() {
    for command in [
        "csappto", "cseappto", "csgappto", "csxappto", "cspreto", "csepreto", "csgpreto",
        "csxpreto", "csundef", "csgundef",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = r"\csdef{hello}{A}";
        let source = format!("\\documentclass{{article}}\\input{{definitions}}\n\\begin{{document}}😀 \\{command}{{hello}}{{hello \\csuse{{hello}}}}\n\\{command}{{hello\\suffix}}{{hello}}\n% \\{command}{{hello}}\n\\verb|\\{command}{{hello}}|\\end{{document}}");
        let workspace = project(
            root,
            &[("main.tex", &source), ("definitions.tex", definitions)],
        );
        let main = uri(root, "main.tex");
        let text = &workspace.documents[&main].text;
        let position = text.position(source.find("hello").unwrap());
        assert!(workspace.documents[&main].syntax.definitions.is_empty());
        let targets = features::definitions(&workspace, &main, position);
        assert_eq!(targets.len(), 1, "{command}");
        assert_eq!(targets[0].uri, uri(root, "definitions.tex"));
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            3,
            "{command}"
        );
        let changed = renamed_to(&workspace, &main, position, "发布-模式");
        assert_eq!(
            changed[&main],
            source.replacen("{hello}", "{发布-模式}", 1).replacen(
                "csuse{hello}",
                "csuse{发布-模式}",
                1
            )
        );
        assert_eq!(
            changed[&uri(root, "definitions.tex")],
            r"\csdef{发布-模式}{A}"
        );
    }
}

#[test]
fn mutations_in_shared_sources_must_resolve_to_the_same_definition_in_all_projects() {
    for command in ["csappto", "cspreto", "csundef", "csgundef"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let shared = format!(r"\{command}{{hello}}{{Code}}");
        let workspace = project(
            root,
            &[
                (
                    "a.tex",
                    r"\documentclass{article}\csdef{hello}{A}\begin{document}\input{shared}\end{document}",
                ),
                (
                    "b.tex",
                    r"\documentclass{article}\csdef{hello}{B}\begin{document}\input{shared}\end{document}",
                ),
                ("shared.tex", &shared),
            ],
        );
        for name in ["a.tex", "shared.tex"] {
            let document = uri(root, name);
            let text = &workspace.documents[&document].text;
            let position = text.position(text.source.find("hello").unwrap());
            assert!(
                features::rename(&workspace, &document, position, "greeting").is_err(),
                "{command}: {name}"
            );
        }
    }
}

#[test]
fn named_alias_declarations_keep_destination_and_source_symbol_identities_separate() {
    for alias in [
        r"\cslet{mirror}{\hello}",
        r"\cslet{mirror}\hello",
        r"\letcs{\mirror}{hello}",
        r"\letcs\mirror{hello}",
        r"\csletcs{mirror}{hello}",
        r"\expandafter\letcs\csname mirror\endcsname{hello}",
        "\\global\\expandafter % expanded target\n\\letcs % comment\n\\csname mirror\\endcsname{hello}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!(r"\csdef{{hello}}#1{{Hello #1}}{alias}");
        let source = r"\documentclass{article}\input{definitions}\begin{document}\mirror{Direct}\csuse{mirror}{Named}\csname mirror\endcsname{Literal}\csuse{hello}{Original}\end{document}";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = uri(root, "main.tex");
        let declaration = uri(root, "definitions.tex");
        let doc = &workspace.documents[&declaration];
        assert_eq!(
            doc.syntax.definitions.len(),
            2,
            "{alias}: {:?}",
            doc.syntax.definitions
        );
        let destination = doc.text.position(definitions.find("mirror").unwrap());
        let target = features::definitions(&workspace, &declaration, destination);
        assert_eq!(target.len(), 1, "{alias}");
        assert_eq!(target[0].uri, declaration);
        assert_eq!(
            features::references(&workspace, &declaration, destination, false).len(),
            3,
            "{alias}"
        );
        assert_eq!(
            features::references(&workspace, &declaration, destination, true).len(),
            4,
            "{alias}"
        );
        let changed = renamed(&workspace, &declaration, destination);
        assert_eq!(
            changed[&declaration],
            definitions.replace("mirror", "greeting")
        );
        assert_eq!(changed[&main], source.replace("mirror", "greeting"));
        let original = doc.text.position(definitions.rfind("hello").unwrap());
        assert_eq!(
            features::references(&workspace, &declaration, original, true).len(),
            3,
            "{alias}"
        );
        let changed = renamed(&workspace, &declaration, original);
        assert_eq!(
            changed[&declaration],
            definitions.replace("hello", "greeting")
        );
        assert_eq!(changed[&main], source.replace("hello", "greeting"));
        let unicode = features::rename(&workspace, &declaration, original, "发布-模式");
        assert_eq!(unicode.is_err(), alias.starts_with(r"\cslet{"), "{alias}");
        assert!(features::rename(&workspace, &declaration, original, "mirror").is_err());
    }
}

#[test]
fn dynamic_alias_destinations_do_not_turn_static_sources_into_declarations() {
    for alias in [
        r"\csletcs{mirror\suffix}{hello}",
        r"\cslet{mirror\suffix}\hello",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(
            root,
            &[
                ("main.tex", alias),
                ("definitions.tex", r"\csdef{hello}{A}"),
            ],
        );
        let main = uri(root, "main.tex");
        let doc = &workspace.documents[&main];
        assert!(
            doc.syntax.definitions.is_empty(),
            "{alias}: {:?}",
            doc.syntax.definitions
        );
        let position = doc.text.position(alias.find("hello").unwrap());
        assert_eq!(
            features::definitions(&workspace, &main, position).len(),
            1,
            "{alias}"
        );
        assert_eq!(
            features::references(&workspace, &main, position, false).len(),
            1,
            "{alias}"
        );
    }
}

#[test]
fn shared_aliases_can_be_renamed_while_their_project_specific_sources_remain_ambiguous() {
    for alias in [
        r"\cslet{mirror}\hello",
        r"\letcs\mirror{hello}",
        r"\csletcs{mirror}{hello}",
        r"\expandafter\letcs\csname mirror\endcsname{hello}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(
            root,
            &[
                (
                    "a.tex",
                    r"\documentclass{article}\csdef{hello}{A}\input{shared}\begin{document}\csuse{mirror}\end{document}",
                ),
                (
                    "b.tex",
                    r"\documentclass{article}\csdef{hello}{B}\input{shared}\begin{document}\csuse{mirror}\end{document}",
                ),
                ("shared.tex", alias),
            ],
        );
        let main = uri(root, "a.tex");
        let shared = uri(root, "shared.tex");
        let text = &workspace.documents[&main].text;
        assert!(
            features::rename(
                &workspace,
                &main,
                text.position(text.source.find("hello").unwrap()),
                "greeting"
            )
            .is_err(),
            "{alias}"
        );
        let position = workspace.documents[&shared]
            .text
            .position(alias.find("mirror").unwrap());
        let changed = renamed(&workspace, &shared, position);
        assert_eq!(changed.len(), 3, "{alias}");
        assert_eq!(changed[&shared], alias.replace("mirror", "greeting"));
        for name in ["a.tex", "b.tex"] {
            let document = uri(root, name);
            assert_eq!(
                changed[&document],
                workspace.documents[&document]
                    .text
                    .source
                    .replace("mirror", "greeting")
            );
        }
    }
}

#[test]
fn computed_or_unexpanded_alias_targets_do_not_claim_literal_names_or_source_arguments() {
    for alias in [
        r"\expandafter\letcs\csname mirror\suffix\endcsname{hello}",
        "\\expandafter\\letcs\\csname mirror% comment\n\\endcsname{hello}",
        r"\expandafter\letcs\csname mirror",
        r"\letcs\csname mirror\endcsname{hello}",
        r"\expandafter\relax\letcs\csname mirror\endcsname{hello}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(
            root,
            &[
                ("main.tex", alias),
                ("definitions.tex", r"\csdef{hello}{A}"),
            ],
        );
        let main = uri(root, "main.tex");
        let doc = &workspace.documents[&main];
        assert!(
            !doc.syntax
                .definitions
                .iter()
                .any(|entry| entry.name == "mirror" || entry.name == "hello"),
            "{alias}"
        );
        let declaration = uri(root, "definitions.tex");
        let position = workspace.documents[&declaration].text.position(7);
        assert!(
            features::references(&workspace, &declaration, position, false).is_empty(),
            "{alias}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn named_alias_renames_preserve_the_copied_meaning_when_the_original_is_redefined() {
    for alias in [
        r"\cslet{mirror}{\hello}",
        r"\cslet{mirror}\hello",
        r"\letcs{\mirror}{hello}",
        r"\letcs\mirror{hello}",
        r"\csletcs{mirror}{hello}",
        r"\expandafter\letcs\csname mirror\endcsname{hello}",
        "\\global\\expandafter % expanded target\n\\letcs % comment\n\\csname mirror\\endcsname{hello}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!(r"\csdef{{hello}}#1{{\typeout{{ORIGINAL:#1}}#1}}{alias}");
        let source = r"\documentclass{article}\usepackage{etoolbox}\input{definitions}
\begin{document}\csuse{mirror}{Before}\renewcommand{\hello}[1]{\typeout{CURRENT:#1}#1}\csuse{mirror}{Named}\csname mirror\endcsname{Literal}\hello{After}\end{document}";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = root.join("main.tex");
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let before = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{alias}: {}", before.output);
        let declaration = uri(root, "definitions.tex");
        let position = workspace.documents[&declaration]
            .text
            .position(definitions.find("mirror").unwrap());
        let name = if alias.starts_with(r"\letcs") {
            "greeting"
        } else {
            "发布-模式"
        };
        for (uri, source) in renamed_to(&workspace, &declaration, position, name) {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let after = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{alias}: {}", after.output);
        for output in [&before.output, &after.output] {
            for expected in [
                "ORIGINAL:Before",
                "ORIGINAL:Named",
                "ORIGINAL:Literal",
                "CURRENT:After",
            ] {
                assert!(output.contains(expected), "{alias}: {output}");
            }
            assert!(!output.contains("CURRENT:Named"), "{alias}: {output}");
        }
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn named_mutation_renames_preserve_scope_and_expansion_behavior() {
    for (command, local, global) in [
        ("csappto", "AC", "A"),
        ("cseappto", "AB", "A"),
        ("csgappto", "AC", "AD"),
        ("csxappto", "AB", "AB"),
        ("cspreto", "CA", "A"),
        ("csepreto", "BA", "A"),
        ("csgpreto", "CA", "DA"),
        ("csxpreto", "BA", "BA"),
        ("csundef", "missing", "A"),
        ("csgundef", "missing", "missing"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = r"\csdef{hello}{A}";
        let code = if command.ends_with("undef") {
            ""
        } else {
            r"{\suffix}"
        };
        let source = format!(
            r"\documentclass{{article}}\usepackage{{etoolbox}}\input{{definitions}}
\begin{{document}}Witness.\def\suffix{{B}}\begingroup\{command}{{hello}}{code}\def\suffix{{C}}
\ifcsundef{{hello}}{{\typeout{{LOCAL:missing}}}}{{\typeout{{LOCAL:\csuse{{hello}}}}}}
\endgroup\def\suffix{{D}}\ifcsundef{{hello}}{{\typeout{{GLOBAL:missing}}}}{{\typeout{{GLOBAL:\csuse{{hello}}}}}}\end{{document}}"
        );
        let workspace = project(
            root,
            &[("main.tex", &source), ("definitions.tex", definitions)],
        );
        let main = uri(root, "main.tex");
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let before = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{command}: {}", before.output);
        let position = workspace.documents[&main]
            .text
            .position(source.find("hello").unwrap());
        for (uri, source) in renamed_to(&workspace, &main, position, "发布-模式") {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let after = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{command}: {}", after.output);
        for output in [&before.output, &after.output] {
            assert!(
                output.contains(&format!("LOCAL:{local}")),
                "{command}: {output}"
            );
            assert!(
                output.contains(&format!("GLOBAL:{global}")),
                "{command}: {output}"
            );
        }
    }
}

#[test]
fn generated_symbols_keep_their_ownership_through_named_command_apis() {
    for (declaration, names) in [
        (r"\newenvironment{hello}{}{}", vec!["hello", "endhello"]),
        (r"\newcounter{hello}", vec!["thehello", "c@hello"]),
        (
            r"\newif\ifhello",
            vec!["ifhello", "hellotrue", "hellofalse"],
        ),
        (r"\newtoggle{hello}", vec!["etb@tgl@hello"]),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut source = String::new();
        for (index, name) in names.iter().enumerate() {
            source.push_str(&format!("\\ifcsdef{{{name}}}{{Yes}}{{No}}"));
            source.push_str(&format!(
                "\\csappto{{{name}}}{{Literal hello}}\\csundef{{{name}}}"
            ));
            source.push_str(&format!(
                "\\expandafter\\letcs\\csname cached-{index}\\endcsname{{{name}}}"
            ));
        }
        let workspace = project(
            directory.path(),
            &[("main.tex", &source), ("definitions.tex", declaration)],
        );
        let main = uri(directory.path(), "main.tex");
        for name in &names {
            let text = &workspace.documents[&main].text;
            let position = text.position(source.find(name).unwrap() + 1);
            assert_eq!(
                features::definitions(&workspace, &main, position).len(),
                1,
                "{name}"
            );
            let changed = renamed(&workspace, &main, position);
            assert_eq!(
                changed[&main],
                source
                    .replace("hello", "greeting")
                    .replace("Literal greeting", "Literal hello")
            );
            assert_eq!(
                changed[&uri(directory.path(), "definitions.tex")],
                declaration.replace("hello", "greeting")
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn named_calls_and_predicates_still_compile_after_cross_file_macro_rename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = r"\newrobustcmd{\hello}[2][blue]{\typeout{CALL:#1:#2}#2}";
    let source = "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\n\\begin{document}\n\\csuse{hello}{Default}\\csuse{hello}[red]{Explicit}\n\\ifcsdef{hello}{\\typeout{CALL:defined}}{Wrong.}\\ifcsundef{hello}{Wrong.}{\\typeout{CALL:not-undefined}}\n\\ifcsvoid{hello}{Wrong.}{\\typeout{CALL:not-void}}\\ifcsmacro{hello}{\\typeout{CALL:macro}}{Wrong.}\n\\ifcsparam{hello}{Wrong.}{\\typeout{CALL:optional-wrapper}}\\ifcsprotected{hello}{\\typeout{CALL:protected}}{Wrong.}\n\\ifcsltxprotect{hello}{Wrong.}{\\typeout{CALL:primitive-protection}}\\ifcsstring{hello}{hello}{Wrong.}{\\typeout{CALL:not-string}}\n\\ifcsequal{hello}{hello}{\\typeout{CALL:equal}}{Wrong.}\\ifcsstrequal{hello}{hello}{\\typeout{CALL:same-text}}{Wrong.}\n\\typeout{CALL:meaning: \\csmeaning{hello}}\\end{document}\n";
    let workspace = project(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "success", "{}", before.output);
    let document = uri(root, "main.tex");
    let position = workspace.documents[&document]
        .text
        .position(source.find("csuse{hello}").unwrap() + 8);
    for (uri, changed) in renamed(&workspace, &document, position) {
        std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
    }
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
    for state in [
        "blue:Default",
        "red:Explicit",
        "defined",
        "not-undefined",
        "not-void",
        "macro",
        "optional-wrapper",
        "protected",
        "primitive-protection",
        "not-string",
        "equal",
        "same-text",
    ] {
        let expected = format!("CALL:{state}");
        assert!(
            before.output.contains(&expected),
            "{expected}: {}",
            before.output
        );
        assert!(
            after.output.contains(&expected),
            "{expected}: {}",
            after.output
        );
    }
}

#[test]
fn static_named_declarations_share_navigation_references_and_rename_with_direct_calls() {
    for command in ["csdef", "csgdef", "csedef", "csxdef"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("\\{command}{{hello}}#1{{Hello #1}}\\csdef{{hello-two}}#1{{#1}}\\csdef{{模式}}#1{{#1}}\n");
        let source = "\\documentclass{article}\\input{definitions}\n\\begin{document}\n\\hello{Direct}\\csuse{hello}{Named}\\csname hello\\endcsname{Literal}\\ifcsdef{hello}{Yes}{No}\n\\csuse{hello-two}{Other}\\csuse{模式}{Unicode}\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = uri(root, "main.tex");
        let declaration = uri(root, "definitions.tex");
        assert_eq!(
            workspace.documents[&declaration].syntax.definitions.len(),
            3
        );
        for (document, marker) in [(&main, "hello"), (&declaration, "hello")] {
            let cursor = workspace.documents[&main]
                .text
                .position(source.find("模式").unwrap() + "模".len());
            let items = features::completion(&workspace, &main, cursor);
            assert!(items.iter().any(|item| item.label == "模式"));
            let declaration_cursor = workspace.documents[&declaration]
                .text
                .position(definitions.find("hello").unwrap() + 2);
            assert!(features::completion(&workspace, &declaration, declaration_cursor).is_empty());
            let text = &workspace.documents[document].text;
            let position = text.position(text.source.find(marker).unwrap() + 2);
            assert_eq!(
                features::definitions(&workspace, document, position).len(),
                1
            );
            assert_eq!(
                features::references(&workspace, document, position, true).len(),
                5
            );
            assert_eq!(
                features::references(&workspace, document, position, false).len(),
                4
            );
            let changed = renamed(&workspace, document, position);
            assert!(
                changed[&declaration].contains(&format!("\\{command}{{greeting}}#1{{Hello #1}}"))
            );
            assert!(changed[&main].contains("\\greeting{Direct}\\csuse{greeting}{Named}\\csname greeting\\endcsname{Literal}\\ifcsdef{greeting}"));
            assert!(changed[&main].contains("csuse{hello-two}"));
        }
        for name in ["hello-two", "模式"] {
            let text = &workspace.documents[&declaration].text;
            let position = text.position(definitions.find(name).unwrap());
            assert_eq!(
                features::references(&workspace, &declaration, position, true).len(),
                2
            );
            let changed = renamed(&workspace, &declaration, position);
            assert!(changed[&declaration].contains("csdef{greeting}"));
            assert!(changed[&main].contains("csuse{greeting}"));
        }
    }
}

#[test]
fn named_declarations_reject_computed_names_ambiguous_definitions_and_shared_project_conflicts() {
    for source in [
        r"\csdef{hello\suffix}#1{#1}",
        r"\csdef{hello world}{Value}",
        r"\csdef{hello",
        r"% \csdef{hello}{Value}",
        r"\verb|\csdef{hello}{Value}|",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let workspace = project(directory.path(), &[("main.tex", source)]);
        let main = uri(directory.path(), "main.tex");
        assert!(
            workspace.documents[&main].syntax.definitions.is_empty(),
            "{source}"
        );
        let position = workspace.documents[&main]
            .text
            .position(source.find("hello").unwrap() + 1);
        assert!(features::prepare_rename(&workspace, &main, position)
            .unwrap()
            .is_none());
    }
    let directory = tempfile::tempdir().unwrap();
    let workspace = project(
        directory.path(),
        &[
            (
                "a.tex",
                r"\documentclass{article}\csdef{hello}{A}\begin{document}\input{shared}\end{document}",
            ),
            (
                "b.tex",
                r"\documentclass{article}\csdef{hello}{B}\begin{document}\input{shared}\end{document}",
            ),
            ("shared.tex", r"\csuse{hello}"),
        ],
    );
    let main = uri(directory.path(), "a.tex");
    let text = &workspace.documents[&main].text;
    assert!(features::rename(
        &workspace,
        &main,
        text.position(text.source.find("hello").unwrap()),
        "greeting"
    )
    .is_err());
    let duplicate = r"\csdef{hello}{A}\csgdef{hello}{B}\csuse{hello}";
    let directory = tempfile::tempdir().unwrap();
    let workspace = project(directory.path(), &[("main.tex", duplicate)]);
    let main = uri(directory.path(), "main.tex");
    assert!(features::rename(
        &workspace,
        &main,
        workspace.documents[&main]
            .text
            .position(duplicate.find("hello").unwrap()),
        "greeting"
    )
    .is_err());
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn named_declaration_renames_preserve_local_global_and_expanded_definition_behavior() {
    for command in ["csdef", "csgdef", "csedef", "csxdef"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("\\{command}{{hello}}#1{{\\prefix#1}}\n");
        let source = "\\documentclass{article}\\usepackage{etoolbox}\n\\begin{document}\\def\\prefix{Alpha-}\\begingroup\\input{definitions}\n\\hello{Direct}\\csuse{hello}{Named}\\csname hello\\endcsname{Literal}\\typeout{NAMED:\\csuse{hello}{Value}}\n\\endgroup\\def\\prefix{Later-}\\ifcsdef{hello}{\\typeout{GLOBAL:\\csuse{hello}{Value}}}{\\typeout{NAMED:local}}\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = root.join("main.tex");
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let before = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{command}: {}", before.output);
        let declaration = uri(root, "definitions.tex");
        let position = workspace.documents[&declaration]
            .text
            .position(definitions.find("hello").unwrap());
        for (uri, changed) in renamed(&workspace, &declaration, position) {
            std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
        }
        let after = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{command}: {}", after.output);
        let expected = match command {
            "csgdef" => "GLOBAL:Later-Value",
            "csxdef" => "GLOBAL:Alpha-Value",
            _ => "NAMED:local",
        };
        for output in [&before.output, &after.output] {
            assert!(output.contains("NAMED:Alpha-Value"), "{command}: {output}");
            assert!(output.contains(expected), "{command}: {output}");
        }
    }
}
