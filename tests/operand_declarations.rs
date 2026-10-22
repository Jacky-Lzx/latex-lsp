use latex_lsp::{
    features,
    syntax::Syntax,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn count(parsed: &Syntax, kind: &str) -> usize {
    match kind {
        "command" => parsed
            .definitions
            .iter()
            .filter(|entry| entry.name != "saved")
            .count(),
        "environment" => parsed.environments.len(),
        "counter" => parsed.counters.len(),
        "conditional" => parsed.conditionals.len(),
        "toggle" => parsed.toggles.len(),
        "label" => parsed.labels.len(),
        "citation" => parsed.citations.len(),
        "anchor" => parsed.anchors.len(),
        "color" => parsed.colors.len(),
        "saved" => parsed.saved_text.len(),
        _ => unreachable!(),
    }
}

#[test]
fn copied_declaration_controls_do_not_declare_the_following_text() {
    for (name, tail, kind) in [
        ("newcommand", r"\ordinary{Body}", "command"),
        ("def", r"\ordinary{Body}", "command"),
        ("newenvironment", "{panel}{}{}", "environment"),
        ("newcounter", "{counter}", "counter"),
        ("newif", r"\ifdraft", "conditional"),
        ("newboolean", "{draft}", "conditional"),
        ("newtoggle", "{draft}", "toggle"),
        ("label", "{label}", "label"),
        ("bibitem", "{citation}", "citation"),
        ("hypertarget", "{anchor}{Body}", "anchor"),
        ("definecolor", "{shade}{rgb}{0,0,0}", "color"),
        ("csdef", "{ordinary}{Body}", "command"),
        ("newlength", r"{\ordinary}", "command"),
        ("SaveVerb", "{snippet}|Body|", "saved"),
    ] {
        for copy in [
            format!(r"\let\saved\{name}"),
            format!(r"\let\saved=\{name}"),
            format!(r"\NewCommandCopy{{\saved}}{{\{name}}}"),
            format!(r"\cslet{{saved}}\{name}"),
            format!(r"\cslet{{saved}}{{\{name}}}"),
            format!(r"\expandafter\let\csname saved\endcsname\{name}"),
            format!(r"\expandafter\NewCommandCopy\csname saved\endcsname{{\{name}}}"),
        ] {
            let source = format!("{copy}{tail}");
            let parsed = Syntax::latex(&Text::new(source.clone()));
            assert_eq!(count(&parsed, kind), 0, "{source}: {parsed:?}");
        }
    }
}

#[test]
fn declaration_targets_do_not_execute_declaration_or_verbatim_controls() {
    for (name, tail, kind) in [
        ("newcommand", r"\ordinary{Body}", "command"),
        ("newif", r"\ifdraft", "conditional"),
        ("newenvironment", "{panel}{}{}", "environment"),
        ("csdef", "{ordinary}{Body}", "command"),
        ("definecolor", "{shade}{rgb}{0,0,0}", "color"),
    ] {
        let source = format!(r"\futurelet\{name}{tail}");
        let parsed = Syntax::latex(&Text::new(source.clone()));
        assert_eq!(
            count(&parsed, kind),
            usize::from(kind == "command"),
            "{source}: {parsed:?}"
        );
        assert!(
            parsed
                .definitions
                .iter()
                .all(|entry| entry.detail == "Defined with \\futurelet"),
            "{source}: {parsed:?}"
        );
    }
    for source in [
        r"\let\saved\verb|\newcommand{\ordinary}{Body}|",
        r"\let\saved\begin{verbatim}\newcommand{\ordinary}{Body}",
        r"\let\saved{\newcommand{\ordinary}{Body}",
    ] {
        let parsed = Syntax::latex(&Text::new(source.into()));
        assert!(
            parsed
                .definitions
                .iter()
                .any(|entry| entry.name == "ordinary"),
            "{source}: {parsed:?}"
        );
    }
    let source = r"\let\saved\expandafter\newcommand\csname ordinary\endcsname{Body}";
    let parsed = Syntax::latex(&Text::new(source.into()));
    assert!(parsed
        .definitions
        .iter()
        .all(|entry| entry.name != "ordinary"));
    assert!(parsed
        .definitions
        .iter()
        .any(|entry| entry.name == "csname"));
}

#[test]
fn copied_literal_braces_do_not_create_syntax_diagnostics_or_fixes() {
    for target in [r"\saved", r"\csname saved\endcsname"] {
        let prefix = if target.starts_with(r"\csname") {
            r"\expandafter"
        } else {
            ""
        };
        for separator in ["", "=", " % target\n = % source\n "] {
            for brace in ['{', '}'] {
                let source = format!(
                    "{{{prefix}\\let{target}{separator}{brace}\\newcommand{{\\ordinary}}{{Body}}}}\\ordinary"
                );
                let parsed = Syntax::latex(&Text::new(source.clone()));
                assert!(parsed.diagnostics.is_empty(), "{source}: {parsed:?}");
                assert!(parsed.fixes.is_empty(), "{source}: {parsed:?}");
                assert!(
                    parsed
                        .definitions
                        .iter()
                        .any(|entry| entry.name == "ordinary"),
                    "{source}: {parsed:?}"
                );
            }
        }
    }
}

#[test]
fn literal_brace_operands_preserve_real_errors_and_replayed_tokens() {
    for (source, message) in [
        (r"\let\saved={ {", "Unclosed brace"),
        (r"\let\saved=} }", "Unexpected closing brace"),
        (r"\futurelet\saved\relax{", "Unclosed brace"),
        (r"\futurelet\saved\relax}", "Unexpected closing brace"),
        (r"\def\let{}\let\saved{", "Unclosed brace"),
        (r"\def\let{}\let\saved}", "Unexpected closing brace"),
        (r"\let\saved\let\other{", "Unclosed brace"),
    ] {
        let text = Text::new(source.into());
        let parsed = Syntax::latex(&text);
        assert_eq!(parsed.diagnostics.len(), 1, "{source}: {parsed:?}");
        let diagnostic = &parsed.diagnostics[0];
        assert_eq!(diagnostic.message, message, "{source}");
        assert_eq!(diagnostic.range, text.range(source.len() - 1, source.len()));
        assert_eq!(parsed.fixes.len(), 1, "{source}: {parsed:?}");
    }
}

#[test]
fn literal_brace_operands_in_macro_bodies_retain_lexical_pairing() {
    for declaration in [
        r"\def\wrapper",
        r"\csdef{wrapper}#1",
        r"\newcommand{\wrapper}",
        r"\expandafter\newcommand\csname wrapper\endcsname",
        r"\NewDocumentCommand{\wrapper}{}",
        r"\newenvironment{wrapper}{}",
    ] {
        for body in [r"{\let\saved{}}", r"{\let\saved}}"] {
            let source = format!("{declaration}{body}");
            let text = Text::new(source.clone());
            let parsed = Syntax::latex(&text);
            if body.ends_with("{}}") {
                assert!(parsed.diagnostics.is_empty(), "{source}: {parsed:?}");
            } else {
                assert_eq!(parsed.diagnostics.len(), 1, "{source}: {parsed:?}");
                assert_eq!(
                    parsed.diagnostics[0].range,
                    text.range(source.len() - 1, source.len())
                );
            }
        }
        let source = format!(r"{declaration}{{\let\saved{{}}");
        let text = Text::new(source.clone());
        let parsed = Syntax::latex(&text);
        assert_eq!(parsed.diagnostics.len(), 1, "{source}: {parsed:?}");
        assert_eq!(parsed.diagnostics[0].message, "Unclosed brace", "{source}");
    }
    for source in [
        r"\textbf{\let\saved{}}",
        r"\unknown{\let\saved{}}",
        r"\href{url}{\let\saved{}}",
        r"\section[\let\saved{}]{Title}",
    ] {
        assert!(
            Syntax::latex(&Text::new(source.into()))
                .diagnostics
                .is_empty(),
            "{source}"
        );
    }
}

#[test]
fn let_tokens_in_parameter_text_do_not_consume_the_definition_body_brace() {
    for declaration in ["def", "gdef", "edef", "xdef"] {
        let source = format!(r"\{declaration}\wrapper#1\let\saved={{Body}}");
        let parsed = Syntax::latex(&Text::new(source.clone()));
        assert!(parsed.diagnostics.is_empty(), "{source}: {parsed:?}");
        assert!(parsed.fixes.is_empty(), "{source}: {parsed:?}");
    }
}

#[test]
fn locally_rebound_declaration_keywords_do_not_hide_executed_definitions() {
    for prefix in [
        r"\def\let{}",
        r"\csdef{let}{}",
        r"\expandafter\def\csname let\endcsname{}",
    ] {
        let source = format!(r"{prefix}\def\saved{{}}\let\saved\gdef\ordinary{{Body}}\ordinary");
        let parsed = Syntax::latex(&Text::new(source.clone()));
        assert_eq!(
            parsed
                .definitions
                .iter()
                .filter(|entry| entry.name == "ordinary")
                .count(),
            1,
            "{source}: {parsed:?}"
        );
        let gdef = source.find("\\gdef").unwrap();
        assert!(!parsed.control_operands.contains(&gdef), "{source}");
    }
}

#[test]
fn copied_named_interfaces_do_not_supply_parameter_help_or_name_completion() {
    for name in ["ref", "setcounter", "csuse", "ifbool"] {
        let source = format!(r"\let\saved\{name}{{ordinary}}");
        let mut workspace = Workspace::default();
        let uri = Url::parse("file:///operand-declarations/main.tex").unwrap();
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(1)));
        let cursor = workspace.documents[&uri]
            .text
            .position(source.find("ordinary").unwrap() + 3);
        assert!(
            latex_lsp::signature::help(&workspace, &uri, cursor).is_none(),
            "{source}"
        );
        assert!(
            features::completion(&workspace, &uri, cursor).is_empty(),
            "{source}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, cursor)
                .unwrap()
                .is_none(),
            "{source}"
        );
    }
}

#[test]
fn label_rename_preserves_groups_after_copied_reference_controls() {
    let source = r"\label{real}\let\saved\ref{real}\ref{real}";
    let mut workspace = Workspace::default();
    let uri = Url::parse("file:///operand-declarations/main.tex").unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let position = workspace.documents[&uri]
        .text
        .position(source.find("real").unwrap());
    let edit = features::rename(&workspace, &uri, position, "renamed")
        .unwrap()
        .unwrap();
    let mut edits = edit.changes.unwrap().remove(&uri).unwrap();
    assert_eq!(edits.len(), 2);
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
    assert_eq!(
        changed.source,
        r"\label{renamed}\let\saved\ref{real}\ref{renamed}"
    );
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_copied_declaration_sources_preserve_ordinary_calls_and_symbol_renames() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let definitions = "\\newcommand{\\ordinary}[1]{\\typeout{OPERAND:#1}#1}\n\\newif\\ifdraft\\drafttrue\n\\newenvironment{panel}{}{}\n";
        let content = concat!(
            "\\let\\saved\\newcommand\\ordinary{Follower}\n",
            "\\let\\savedif\\newif\\ifdraft\\typeout{OPERAND:draft}\\fi\n",
            "\\let\\savedenv\\newenvironment{panel}{}{}\\begin{panel}\\ordinary{Panel}\\end{panel}\n",
            "{\\let\\opening={\\expandafter\\let\\csname closing\\endcsname=}\\typeout{OPERAND:braces}}\n",
            "\\newcommand{\\unusedbraces}{\\let\\saved{}}\n",
            "\\def\\unusedparameter#1\\let\\saved={Body}\n",
            "\\begingroup\\def\\let{}\\def\\unused{}\\let\\unused\\gdef\\restored{\\typeout{OPERAND:rebound}Rebound}\\endgroup\\restored\n",
            "% \\ordinary \\ifdraft \\begin{panel}\n",
            "\\verb|\\ordinary \\ifdraft \\begin{panel}|\n",
        );
        std::fs::write(&main, "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n").unwrap();
        std::fs::write(root.join("definitions.tex"), definitions).unwrap();
        std::fs::write(root.join("content.tex"), content).unwrap();
        assert!(Syntax::latex(&Text::new(content.into()))
            .diagnostics
            .is_empty());
        let compiler = root.join("compiler");
        std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: compiler.to_string_lossy().into(),
            arguments: Some(
                [
                    "-interaction=nonstopmode",
                    "-recorder",
                    "-output-directory=%o",
                    "%f",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ),
            ..Default::default()
        };
        let before = latex_lsp::build::run(&main, &config, &Default::default());
        assert_eq!(before.status, "success", "{engine}: {}", before.output);
        for (old, new, file, documents) in [
            ("ordinary", "renamedordinary", "definitions.tex", 2),
            ("draft", "renameddraft", "definitions.tex", 2),
            ("panel", "renamedpanel", "definitions.tex", 2),
            ("restored", "renamedrestored", "content.tex", 1),
        ] {
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let uri = Url::from_file_path(root.join(file)).unwrap();
            let text = &workspace.documents[&uri].text;
            let position = text.position(text.source.find(old).unwrap());
            let changes = features::rename(&workspace, &uri, position, new)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), documents, "{old}");
            for (uri, mut edits) in changes {
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
        }
        let changed = std::fs::read_to_string(root.join("content.tex")).unwrap();
        assert!(changed.contains(r"\newenvironment{panel}{}{}\begin{renamedpanel}"));
        assert_eq!(
            changed.split_once('%').unwrap().1,
            content.split_once('%').unwrap().1
        );
        let after = latex_lsp::build::run(&main, &config, &Default::default());
        assert_eq!(after.status, "success", "{engine}: {}", after.output);
        for outcome in [before, after] {
            let observed: Vec<_> = outcome
                .output
                .lines()
                .filter(|line| line.starts_with("OPERAND:"))
                .collect();
            assert_eq!(
                observed,
                [
                    "OPERAND:Follower",
                    "OPERAND:draft",
                    "OPERAND:Panel",
                    "OPERAND:braces",
                    "OPERAND:rebound"
                ]
            );
        }
    }
}
