use latex_lsp::{
    compilation::build,
    editing::{self, file_operations},
    language::{features, navigation, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;
use std::{path::Path, sync::atomic::AtomicBool};

const ENVIRONMENTS: [&str; 10] = [
    "Verbatim",
    "Verbatim*",
    "LVerbatim",
    "LVerbatim*",
    "BVerbatim",
    "BVerbatim*",
    "SaveVerbatim",
    "SaveVerbatim*",
    "VerbatimOut",
    "VerbatimOut*",
];

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn argument(environment: &str) -> &'static str {
    match environment.trim_end_matches('*') {
        "SaveVerbatim" => "{saved}",
        "VerbatimOut" => "{output.txt}",
        _ => "",
    }
}

fn signature_label(environment: &str) -> String {
    let required = match environment.trim_end_matches('*') {
        "SaveVerbatim" => "{name}",
        "VerbatimOut" => "{file}",
        _ => "",
    };
    format!("\\begin{{{environment}}}[options]{required}")
}

#[test]
fn header_signatures_follow_fancyverb_lookahead_and_token_boundaries() {
    let main = Url::parse("file:///fancyvrb/signatures.tex").unwrap();
    let mut workspace = Workspace::default();
    for environment in ENVIRONMENTS {
        let mut cases = vec![
            ("¦", 1),
            (" \t¦", 1),
            ("[frame=sin¦gle]", 1),
            ("[label={A]B},frame=sin¦gle]", 1),
            ("[label={\\%literal¦}]", 1),
            ("% continue\r\n [frame=sin¦gle]", 1),
            ("[label={A% ] ignored\r\nB¦}]", 1),
            ("[frame=¦", 1),
        ];
        if !argument(environment).is_empty() {
            cases.extend([
                ("{na¦me}", 2),
                ("[frame=single]¦", 2),
                ("[label=[literal]{na¦me}", 2),
                ("[frame=single]\r\n {😀中文¦}", 2),
                ("% continue\r\n {na¦me}", 2),
                ("{na% } ignored\r\nme¦}", 2),
                ("{na¦", 2),
                ("\\na¦me", 2),
            ]);
        }
        for (header, active) in cases {
            let marked = format!("% 😀\r\n\\begin{{{environment}}}{header}");
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            workspace.open(main.clone(), source.clone(), 7);
            let help = signature::help(&workspace, &main, Text::new(source).position(offset))
                .unwrap_or_else(|| panic!("missing signature: {marked}"));
            assert_eq!(help.signatures[0].label, signature_label(environment));
            assert_eq!(help.active_parameter, Some(active), "{marked}");
            assert_eq!(help.signatures[0].active_parameter, Some(active));
        }
    }
}

#[test]
fn header_comments_active_newlines_and_literal_bodies_have_no_signature() {
    let main = Url::parse("file:///fancyvrb/no-signatures.tex").unwrap();
    let mut workspace = Workspace::default();
    for environment in ENVIRONMENTS {
        for header in [
            "% comm¦ent\r\n[frame=single]".to_owned(),
            "[label={A% comm¦ent\r\nB}]".to_owned(),
            "[label={\\\\% comm¦ent\r\nB}]".to_owned(),
            "\r\n[frame=sin¦gle]".to_owned(),
            "\r\n{na¦me}".to_owned(),
            format!("[frame=single]{}¦", argument(environment)),
            format!("{} discarded¦\r\nbody", argument(environment)),
            format!(
                "{}\r\n\\frac{{a}}{{b¦}}\r\n\\end{{{environment}}}",
                argument(environment)
            ),
            format!(
                "{}\r\nbody\r\n\\end{{{environment}}}\\frac{{a}}{{b¦}}",
                argument(environment)
            ),
        ] {
            let marked = format!("\\begin{{{environment}}}{header}");
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            workspace.open(main.clone(), source.clone(), 7);
            assert!(
                signature::help(&workspace, &main, Text::new(source).position(offset)).is_none(),
                "{marked}"
            );
        }
    }
}

#[test]
fn project_header_overrides_refresh_after_unsaved_settings_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let settings = uri(root, "settings.sty");
    let unrelated = root.join("other-project");
    std::fs::create_dir(&unrelated).unwrap();
    std::fs::write(
        unrelated.join("main.tex"),
        r"\documentclass{article}\def\begin#1{}",
    )
    .unwrap();
    for environment in ENVIRONMENTS {
        let base = environment.trim_end_matches('*');
        let source = format!("\\documentclass{{article}}\\usepackage{{settings}}\r\n\\begin{{{environment}}}[frame=single]{}\r\nbody\r\n\\end{{{environment}}}\r\n", argument(environment));
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let at = Text::new(source.clone()).position(source.find("single").unwrap() + 2);
        for definition in [
            format!("\\renewenvironment{{{environment}}}[1]{{}}{{}}"),
            format!("\\NewDocumentEnvironment{{{environment}}}{{mm}}{{}}{{}}"),
            format!("\\RenewEnvironmentCopy{{{environment}}}{{other}}"),
            format!("\\csdef{{{environment}}}#1{{#1}}"),
            format!("\\csundef{{{environment}}}"),
            format!("\\RecustomVerbatimEnvironment{{{base}}}{{Verbatim}}{{}}"),
            format!("\\DefineVerbatimEnvironment{{{base}}}{{Verbatim}}{{}}"),
            "\\CustomVerbatimEnvironment{\\dynamic}{Verbatim}{}".into(),
            "\\def\\begin#1{}".into(),
            "\\csundef{\\dynamic}".into(),
        ] {
            std::fs::write(root.join("settings.sty"), &definition).unwrap();
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            workspace.add_root(unrelated.clone());
            workspace.open(main.clone(), source.clone(), 7);
            assert!(
                signature::help(&workspace, &main, at).is_none(),
                "{environment}: {definition}"
            );
            workspace.open(settings.clone(), String::new(), 8);
            let help = signature::help(&workspace, &main, at)
                .unwrap_or_else(|| panic!("did not refresh: {definition}"));
            assert_eq!(help.signatures[0].label, signature_label(environment));
        }
        // The package defines the base and starred control sequences
        // independently. Ordinary TeX redefinition changes only its target.
        let other = if environment.ends_with('*') {
            base.to_owned()
        } else {
            format!("{environment}*")
        };
        for definition in [
            format!("\\renewenvironment{{{other}}}[1]{{}}{{}}"),
            "\\RecustomVerbatimEnvironment{Unrelated}{Verbatim}{}".into(),
        ] {
            std::fs::write(root.join("settings.sty"), definition).unwrap();
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            assert_eq!(
                signature::help(&workspace, &main, at).unwrap().signatures[0].label,
                signature_label(environment)
            );
        }
    }
}

fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
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

fn versioned(edit: WorkspaceEdit, main: &Url) -> Vec<TextEdit> {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned document edits")
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *main);
    assert_eq!(documents[0].text_document.version, Some(7));
    documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!("unexpected annotated edit"),
        })
        .collect()
}

#[test]
fn literal_examples_do_not_define_symbols_import_files_or_participate_in_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), r"\label{chapterlabel}").unwrap();
    std::fs::write(root.join("example.tex"), r"\label{examplelabel}").unwrap();
    let main = uri(root, "main.tex");
    for environment in ENVIRONMENTS {
        let body = concat!(
            "\\label{kept}\\label{fake}\\newcommand{\\hidden}{Text}\\say\n",
            "\\newcounter{fakecounter}\\newenvironment{fakepanel}{}{}\n",
            "\\input{chapter}\\input{example}\\ref{kept}\n",
            "\\begin{unmatched}{ % literal unmatched brace and environment\n",
        );
        let before = "\\documentclass{article}\\newcommand{\\say}{Text}\\label{kept}\n";
        let after = "\\input{chapter}\\say\\ref{kept}\\ref{fake}\\ref{examplelabel}\n";
        let source = format!(
            "{before}\\begin{{{environment}}}{}\n{body}\\end{{{environment}}}\n{after}",
            argument(environment)
        );
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let mut workspace = Workspace {
            versioned_edits: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{environment}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1);
        assert_eq!(doc.syntax.definitions.len(), 1);
        assert!(doc.syntax.counters.is_empty() && doc.syntax.environments.is_empty());
        assert_eq!(
            workspace.dependencies(&main),
            vec![uri(root, "chapter.tex")]
        );
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{environment}: {links:?}");
        assert_eq!(links[0].target, Some(uri(root, "chapter.tex")));
        for absent in ["fake", "examplelabel"] {
            let at = doc.text.position(source.rfind(absent).unwrap());
            assert!(
                features::definitions(&workspace, &main, at).is_empty(),
                "{environment}/{absent}"
            );
        }
        for (old, new) in [("kept", "renamedlabel"), ("say", "renamedcommand")] {
            let at = doc.text.position(source.rfind(old).unwrap());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
            let edit = features::rename(&workspace, &main, at, new)
                .unwrap()
                .unwrap();
            let updated = apply(&source, versioned(edit, &main));
            assert_eq!(
                updated,
                format!(
                    "{}\\begin{{{environment}}}{}\n{body}\\end{{{environment}}}\n{}",
                    before.replace(old, new),
                    argument(environment),
                    after.replace(old, new)
                )
            );
        }
        let edit = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, "chapter.tex").into(),
                new_uri: uri(root, "renamed.tex").into(),
            }],
        )
        .unwrap()
        .unwrap();
        let updated = apply(&source, versioned(edit, &main));
        assert!(updated.contains(body), "{environment}: {updated}");
        assert!(updated.ends_with(&after.replace("chapter", "renamed")));
        let inside = doc.text.position(source.find("\\ref{kept}").unwrap() + 7);
        assert!(features::completion(&workspace, &main, inside).is_empty());
        assert!(features::definitions(&workspace, &main, inside).is_empty());
        assert!(signature::help(&workspace, &main, inside).is_none());
    }
}

#[test]
fn raw_unicode_crlf_regions_fold_highlight_and_link_only_the_real_delimiters() {
    for environment in ENVIRONMENTS {
        let source = format!(
            "\\begin{{{environment}}}{}\r\n😀 \\label{{fake}} % literal\r\n\\begin{{fakepanel}}{{\r\n\\end{{{environment}}}\r\n\\label{{real}}\r\n",
            argument(environment)
        );
        let main = Url::parse("file:///fancy-environments/main.tex").unwrap();
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{environment}: {:?}",
            doc.syntax.diagnostics
        );
        let folds = navigation::folding_ranges(&doc, None);
        assert_eq!(folds.len(), 1, "{environment}: {folds:?}");
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 3));
        let mut position = Position::new(0, 0);
        let mut string_line = false;
        for token in semantic::tokens(&doc, None).unwrap().data {
            position = Position::new(
                position.line + token.delta_line,
                if token.delta_line == 0 {
                    position.character + token.delta_start
                } else {
                    token.delta_start
                },
            );
            if position.line == 1 {
                assert_eq!(position.character, 0);
                assert_eq!(
                    token.length,
                    "😀 \\label{fake} % literal".encode_utf16().count() as u32
                );
                assert_eq!(
                    semantic::options().legend.token_types[token.token_type as usize],
                    SemanticTokenType::STRING
                );
                string_line = true;
            }
        }
        assert!(string_line);
        for offset in [
            source.find(environment).unwrap(),
            source.rfind(environment).unwrap(),
        ] {
            let linked =
                editing::linked_editing_ranges(&doc, doc.text.position(offset + 1)).unwrap();
            assert_eq!(linked.ranges.len(), 2);
            for range in linked.ranges {
                let span =
                    doc.text.offset(range.start).unwrap()..doc.text.offset(range.end).unwrap();
                assert_eq!(&source[span], environment);
            }
        }
    }
}

#[test]
fn only_the_first_end_marker_on_a_fancyverb_line_can_close_the_environment() {
    let main = Url::parse("file:///fancy-environments/end-markers.tex").unwrap();
    for environment in ENVIRONMENTS {
        for newline in ["\n", "\r\n"] {
            let body = format!(
                "\\end{{other}}\\end{{{environment}}}\\label{{fakeafter}}{newline}\\label{{stillbody}}{newline}"
            );
            let source = format!(
                "\\begin{{{environment}}}{}{newline}{body}  \\end{{{environment}}}{newline}\\label{{real}}",
                argument(environment)
            );
            let doc = Document::new(&main, source.clone(), Some(7));
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{environment}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(
                doc.syntax.labels.len(),
                1,
                "{environment}: {:?}",
                doc.syntax.labels
            );
            assert_eq!(doc.syntax.labels[0].name, "real");
            for key in ["fakeafter", "stillbody"] {
                assert!(doc.syntax.is_ignored(source.find(key).unwrap()));
            }
            let folds = navigation::folding_ranges(&doc, None);
            assert_eq!(folds.len(), 1);
            assert_eq!((folds[0].start_line, folds[0].end_line), (0, 3));
            let at = doc.text.position(source.find(environment).unwrap());
            let linked = editing::linked_editing_ranges(&doc, at).unwrap();
            assert_eq!(linked.ranges.len(), 2);
            assert_eq!(linked.ranges[1].start.line, 3);
            let incomplete = source[..source.rfind("  \\end").unwrap()].to_owned();
            let doc = Document::new(&main, incomplete, Some(7));
            assert!(doc.syntax.labels.is_empty());
            assert_eq!(doc.syntax.diagnostics.len(), 1);
            assert_eq!(
                doc.syntax.fixes[0].edit.new_text,
                format!("\\end{{{environment}}}{newline}")
            );
        }
    }
    // Other packages retain their existing delimiter rules.
    let source = "\\begin{verbatim}\n\\end{other}\\end{verbatim}\\label{real}\n";
    let doc = Document::new(&main, source.into(), Some(7));
    assert!(doc.syntax.diagnostics.is_empty());
    assert_eq!(doc.syntax.labels[0].name, "real");
}

#[test]
fn closing_line_suffixes_are_discarded_without_losing_delimiters_or_the_next_source_line() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), r"\label{chapterlabel}").unwrap();
    std::fs::write(root.join("discarded.tex"), r"\label{discardedinputlabel}").unwrap();
    let main = uri(root, "main.tex");
    let tail = concat!(
        "\\label{kept}\\label{discarded}\\newcommand{\\hidden}{Text}\\say",
        "\\input{chapter}\\input{discarded}\\ref{kept}\\end{document}{",
        "% 😀 literal percent \\label{afterpercent}",
    );
    for environment in ENVIRONMENTS {
        for newline in ["\n", "\r\n"] {
            let before = format!("\\documentclass{{article}}\\newcommand{{\\say}}{{Text}}\\label{{kept}}\\begin{{document}}{newline}");
            let after = format!("\\input{{chapter}}\\say\\ref{{kept}}\\ref{{discarded}}\\ref{{discardedinputlabel}}{newline}\\end{{document}}{newline}");
            let source = format!(
                "{before}\\begin{{{environment}}}{}{newline}Literal.{newline}\\end{{{environment}}}{tail}{newline}{after}",
                argument(environment)
            );
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let mut workspace = Workspace {
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            let doc = &workspace.documents[&main];
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{environment}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1);
            assert_eq!(doc.syntax.definitions.len(), 1);
            assert_eq!(
                workspace.dependencies(&main),
                vec![uri(root, "chapter.tex")]
            );
            let links = navigation::document_links(&workspace, &main);
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].target, Some(uri(root, "chapter.tex")));
            for key in ["discarded", "discardedinputlabel"] {
                assert!(features::definitions(
                    &workspace,
                    &main,
                    doc.text.position(source.rfind(key).unwrap())
                )
                .is_empty());
            }
            for (old, new) in [("kept", "renamedlabel"), ("say", "renamedcommand")] {
                let at = doc.text.position(source.rfind(old).unwrap());
                assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
                let updated = apply(
                    &source,
                    versioned(
                        features::rename(&workspace, &main, at, new)
                            .unwrap()
                            .unwrap(),
                        &main,
                    ),
                );
                assert_eq!(updated, format!(
                    "{}\\begin{{{environment}}}{}{newline}Literal.{newline}\\end{{{environment}}}{tail}{newline}{}",
                    before.replace(old, new), argument(environment), after.replace(old, new)
                ));
            }
            let edit = file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: uri(root, "chapter.tex").into(),
                    new_uri: uri(root, "renamed.tex").into(),
                }],
            )
            .unwrap()
            .unwrap();
            let updated = apply(&source, versioned(edit, &main));
            assert!(updated.contains(tail));
            assert!(updated.ends_with(&after.replace("chapter", "renamed")));
            let ending_offset = source.find(&format!("\\end{{{environment}}}")).unwrap();
            let marker_end = ending_offset + format!("\\end{{{environment}}}").len();
            assert!(doc.syntax.ignored.iter().any(|span| span.end == marker_end));
            assert!(doc
                .syntax
                .is_ignored(source.find("literal percent").unwrap()));
            assert!(!doc
                .syntax
                .comments
                .iter()
                .any(|span| span.contains(&source.find("literal percent").unwrap())));
            let linked =
                editing::linked_editing_ranges(doc, doc.text.position(ending_offset + 6)).unwrap();
            assert_eq!(linked.ranges.len(), 2);
            assert_eq!(
                linked.ranges[1],
                doc.text.range(ending_offset + 5, marker_end - 1)
            );
            assert!(navigation::folding_ranges(doc, None)
                .iter()
                .any(|fold| fold.start_line == 1 && fold.end_line == 3));
            let mut at = Position::new(0, 0);
            let mut covered = false;
            for token in semantic::tokens(doc, None).unwrap().data {
                at = Position::new(
                    at.line + token.delta_line,
                    if token.delta_line == 0 {
                        at.character + token.delta_start
                    } else {
                        token.delta_start
                    },
                );
                if at.line == 3 {
                    assert_eq!(
                        semantic::options().legend.token_types[token.token_type as usize],
                        SemanticTokenType::STRING
                    );
                    let end = Position::new(at.line, at.character + token.length);
                    let value =
                        &source[doc.text.offset(at).unwrap()..doc.text.offset(end).unwrap()];
                    covered |= value.contains("literal percent");
                }
            }
            assert!(covered);
        }
        let at_eof = format!(
            "\\begin{{{environment}}}{}\nLiteral.\n\\end{{{environment}}}\\label{{discarded}}{{",
            argument(environment)
        );
        let doc = Document::new(&main, at_eof, Some(7));
        assert!(doc.syntax.labels.is_empty() && doc.syntax.diagnostics.is_empty());
    }
    let source = "\\begin{verbatim}\nLiteral.\n\\end{verbatim}\\label{retained}\n";
    let doc = Document::new(&main, source.into(), Some(7));
    assert_eq!(doc.syntax.labels[0].name, "retained");
}

#[test]
fn header_markers_never_close_the_body_and_preserve_real_source_renames() {
    let main = Url::parse("file:///fancy-environments/header.tex").unwrap();
    for environment in ENVIRONMENTS {
        for newline in ["\n", "\r\n"] {
            let marker = format!("\\end{{{environment}}}");
            let label = format!("label={{\\detokenize{{{marker}}}}}");
            for options in [
                format!("[frame=single,{label}]"),
                format!("[frame=single,{newline}{label}{newline}]"),
                format!("% keep header{newline}[frame=single,{label}]"),
            ] {
                let header = format!("\\begin{{{environment}}}{options}{}", argument(environment));
                let body = "\\label{kept}\\label{fake}\\newcommand{\\hidden}{Text}\\input{fake}\\begin{fake}{";
                let before = format!("\\label{{kept}}{newline}");
                let after = format!("\\ref{{kept}}\\ref{{fake}}{newline}");
                let source =
                    format!("{before}{header}{newline}{body}{newline}{marker}{newline}{after}");
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(
                    doc.syntax.diagnostics.is_empty(),
                    "{environment}/{options}: {:?}",
                    doc.syntax.diagnostics
                );
                assert_eq!(doc.syntax.labels.len(), 1);
                assert!(doc.syntax.definitions.is_empty());
                assert!(workspace.dependencies(&main).is_empty());
                let at = doc.text.position(source.rfind("kept").unwrap());
                assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
                let updated = apply(
                    &source,
                    versioned(
                        features::rename(&workspace, &main, at, "renamed")
                            .unwrap()
                            .unwrap(),
                        &main,
                    ),
                );
                assert_eq!(
                    updated,
                    format!(
                        "{}{header}{newline}{body}{newline}{marker}{newline}{}",
                        before.replace("kept", "renamed"),
                        after.replace("kept", "renamed")
                    )
                );
                let real_end = source.rfind(&marker).unwrap();
                let expected = doc.text.range(real_end + 5, real_end + marker.len() - 1);
                for start in [source.find(environment).unwrap(), real_end + 5] {
                    let linked =
                        editing::linked_editing_ranges(doc, doc.text.position(start + 1)).unwrap();
                    assert_eq!(linked.ranges[1], expected);
                }
                let endline = doc.text.position(real_end).line;
                assert!(navigation::folding_ranges(doc, None)
                    .iter()
                    .any(|fold| fold.start_line == 1 && fold.end_line == endline));
                let incomplete = source[..real_end].to_owned();
                let doc = Document::new(&main, incomplete, Some(8));
                assert_eq!(doc.syntax.diagnostics.len(), 1);
                assert!(doc.syntax.fixes[0].edit.new_text.contains(&marker));
                assert_eq!(doc.syntax.labels.len(), 1);
            }
        }
    }
}

#[test]
fn required_header_names_and_begin_line_text_are_separate_from_raw_body_lines() {
    let main = Url::parse("file:///fancy-environments/header-arguments.tex").unwrap();
    for environment in ENVIRONMENTS {
        let marker = format!("\\end{{{environment}}}");
        let mut headers = vec![
            format!("{} {marker}", argument(environment)),
            format!("[frame=single]{} {marker}", argument(environment)),
        ];
        if environment.starts_with("SaveVerbatim") || environment.starts_with("VerbatimOut") {
            headers.extend([
                "x".into(),
                format!("[frame=single]\n{}", argument(environment)),
                format!("% keep\n{}", argument(environment)),
            ]);
        }
        if environment.starts_with("SaveVerbatim") {
            headers.push(format!("{{saved\\detokenize{{{marker}}}}}"));
        }
        for header in headers {
            let source = format!(
                "\\begin{{{environment}}}{header}\n\\label{{fake}}\n{marker}\n\\label{{real}}"
            );
            let doc = Document::new(&main, source, Some(7));
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{environment}/{header}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1);
            assert_eq!(doc.syntax.labels[0].name, "real");
        }
        // An un-commented newline stops option detection. The first body
        // line closes the environment even when it starts with a bracket.
        let source = format!(
            "\\begin{{{environment}}}{}\n[raw] {marker}\n\\label{{real}}",
            argument(environment)
        );
        let doc = Document::new(&main, source, Some(7));
        assert_eq!(doc.syntax.labels.len(), 1);
        assert_eq!(doc.syntax.labels[0].name, "real");
    }
}

#[test]
fn saved_names_are_literal_but_formatting_options_remain_normal_source() {
    let main = Url::parse("file:///fancy-environments/saved-names.tex").unwrap();
    for command in ["UseVerbatim", "LUseVerbatim", "BUseVerbatim"] {
        let source = format!(
            "\\{command} *[label={{\\label{{live}}}}]{{saved\\detokenize{{\\end{{Verbatim}}}}\\label{{fake}}\\input{{fake}}}}\\label{{real}}"
        );
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{command}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(
            doc.syntax
                .labels
                .iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>(),
            ["live", "real"]
        );
        assert!(doc.syntax.is_ignored(source.find("\\end").unwrap()));
        assert!(!doc.syntax.is_ignored(source.find("\\label{live}").unwrap()));
        assert!(doc
            .syntax
            .commands
            .iter()
            .all(|command| command.name != "input"));
    }
}

#[test]
fn incomplete_raw_environments_offer_one_versioned_close_fix_without_parsing_the_body() {
    let main = Url::parse("file:///fancy-environments/incomplete.tex").unwrap();
    for environment in ENVIRONMENTS {
        let source = format!(
            "\\begin{{{environment}}}{}\r\n\\label{{fake}}{{\\begin{{fake}}\r\n",
            argument(environment)
        );
        let mut workspace = Workspace {
            versioned_edits: true,
            code_action_literals: true,
            ..Default::default()
        };
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        assert!(doc.syntax.labels.is_empty());
        assert_eq!(
            doc.syntax.diagnostics.len(),
            1,
            "{environment}: {:?}",
            doc.syntax.diagnostics
        );
        let diagnostic = doc.syntax.diagnostics[0].clone();
        assert_eq!(
            diagnostic.message,
            format!("Unclosed environment \u{60}{environment}\u{60}")
        );
        let actions = editing::code_actions(
            &workspace,
            &main,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        );
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = actions.into_iter().next().unwrap() else {
            panic!()
        };
        let updated = apply(&source, versioned(action.edit.unwrap(), &main));
        assert_eq!(updated, format!("{source}\\end{{{environment}}}\r\n"));
        let repaired = Document::new(&main, updated, Some(8));
        assert!(repaired.syntax.diagnostics.is_empty());
        assert!(repaired.syntax.labels.is_empty());
    }
}

#[test]
fn environment_names_require_exact_case_spaces_and_one_star_and_completion_reserves_them() {
    let main = Url::parse("file:///fancy-environments/names.tex").unwrap();
    for name in [
        " Verbatim",
        "Verbatim ",
        "lVerbatim",
        "LVerbatim**",
        "BVerbatimX",
    ] {
        let source = format!("\\begin{{{name}}}\n\\label{{visible}}\n\\end{{{name}}}\n");
        let doc = Document::new(&main, source, Some(1));
        assert_eq!(doc.syntax.labels.len(), 1, "{name}");
    }
    let mut workspace = Workspace::default();
    for environment in ENVIRONMENTS {
        let source = format!("\\begin{{{environment}}}");
        workspace.open(main.clone(), source.clone(), 1);
        let at = workspace.documents[&main].text.position(source.len() - 1);
        assert!(
            features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == environment),
            "{environment}"
        );
    }
    let source = "\\newenvironment{panel}{}{}\\begin{panel}Text.\\end{panel}";
    workspace.open(main.clone(), source.into(), 2);
    let at = workspace.documents[&main]
        .text
        .position(source.find("panel").unwrap());
    for environment in ENVIRONMENTS {
        assert!(
            features::rename(&workspace, &main, at, environment).is_err(),
            "{environment}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_header_markers_keep_body_literal_after_versioned_label_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for environment in ENVIRONMENTS {
            let marker = format!("\\end{{{environment}}}");
            let label = format!("label={{\\detokenize{{{marker}}}}}");
            let use_saved = if environment.starts_with("SaveVerbatim") {
                "\\UseVerbatim{saved}\n"
            } else {
                ""
            };
            let mut cases: Vec<_> = [
                format!("[frame=single,{label}]"),
                format!("[frame=single,\n{label}\n]"),
                format!("% keep header\n[frame=single,{label}]"),
            ]
            .into_iter()
            .map(|options| {
                (
                    format!("{options}{}", argument(environment)),
                    use_saved.to_owned(),
                )
            })
            .collect();
            if environment.starts_with("SaveVerbatim") {
                let name = format!("saved\\detokenize{{{marker}}}");
                cases.push((format!("{{{name}}}"), format!("\\UseVerbatim{{{name}}}\n")));
            }
            if environment.starts_with("SaveVerbatim") || environment.starts_with("VerbatimOut") {
                let use_token = if environment.starts_with("SaveVerbatim") {
                    "\\UseVerbatim{x}\n"
                } else {
                    ""
                };
                cases.push(("x".into(), use_token.into()));
            }
            for (header, use_saved) in cases {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = uri(root, "main.tex");
                let fake = uri(root, "fake.tex");
                std::fs::write(root.join("fake.tex"), r"\label{fakeinput}").unwrap();
                let body = "\\label{kept}\\label{fake}\\newcommand{\\hidden}{Text}\\input{fake}";
                let source = format!(
                    "\\documentclass{{article}}\n\\usepackage{{fancyvrb}}\n\\begin{{document}}\n\\label{{kept}}\n\\begin{{{environment}}}{header}\n{body}\n{marker}\n{use_saved}Real.\\ref{{kept}}\n\\end{{document}}\n"
                );
                std::fs::write(root.join("main.tex"), &source).unwrap();
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(
                    doc.syntax.diagnostics.is_empty(),
                    "{environment}/{header}: {:?}",
                    doc.syntax.diagnostics
                );
                if let Some(offset) = source.find("frame=single") {
                    let help =
                        signature::help(&workspace, &main, doc.text.position(offset + 8)).unwrap();
                    assert_eq!(help.signatures[0].label, signature_label(environment));
                    assert_eq!(help.active_parameter, Some(1));
                }
                if !argument(environment).is_empty() {
                    if let Some(offset) = source.find(argument(environment)) {
                        let help =
                            signature::help(&workspace, &main, doc.text.position(offset + 2))
                                .unwrap();
                        assert_eq!(help.signatures[0].label, signature_label(environment));
                        assert_eq!(help.active_parameter, Some(2));
                    }
                }
                let at = doc.text.position(source.rfind("kept").unwrap());
                let updated = apply(
                    &source,
                    versioned(
                        features::rename(&workspace, &main, at, "renamedlabel")
                            .unwrap()
                            .unwrap(),
                        &main,
                    ),
                );
                assert!(updated.contains(&header) && updated.contains(body));
                std::fs::write(root.join("main.tex"), &updated).unwrap();
                let outcome = build::run(
                    &root.join("main.tex"),
                    &latex_lsp::config::BuildConfig {
                        executable: engine.into(),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                );
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{environment}/{header}: {}",
                    outcome.output
                );
                assert!(!outcome.inputs.unwrap().contains(&fake));
                let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                assert!(aux.contains("newlabel{renamedlabel}"), "{aux}");
                for key in ["kept", "fake", "fakeinput"] {
                    assert!(!aux.contains(&format!("newlabel{{{key}}}")), "{aux}");
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_discarded_closing_suffix_preserves_the_compiler_error_and_safe_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for environment in ENVIRONMENTS {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(root, "main.tex");
            let discarded = uri(root, "discarded.tex");
            std::fs::write(root.join("discarded.tex"), r"\label{discardedinputlabel}").unwrap();
            let tail = "\\label{kept}\\label{discarded}\\newcommand{\\hidden}{Oops}\\input{discarded}% literal";
            let use_saved = if environment.starts_with("SaveVerbatim") {
                "\\UseVerbatim{saved}\n"
            } else {
                ""
            };
            let source = format!(
                "\\documentclass{{article}}\n\\usepackage{{fancyvrb}}\n\\begin{{document}}\n\\label{{kept}}\n\\begin{{{environment}}}{}\nLiteral.\n\\end{{{environment}}}{tail}\n{use_saved}\\ref{{kept}}\\ref{{discarded}}\\ref{{discardedinputlabel}}\n\\end{{document}}\n",
                argument(environment)
            );
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            workspace.config.build.executable = engine.into();
            let at = workspace.documents[&main]
                .text
                .position(source.rfind("kept").unwrap());
            let changes = features::rename(&workspace, &main, at, "renamedlabel")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            let updated = apply(&source, changes[&main].clone());
            assert!(updated.contains(tail));
            for (contents, key) in [(&source, "kept"), (&updated, "renamedlabel")] {
                std::fs::write(root.join("main.tex"), contents).unwrap();
                workspace.open(main.clone(), contents.clone(), 7);
                let outcome = build::run(
                    &root.join("main.tex"),
                    &workspace.config.build,
                    &AtomicBool::new(false),
                );
                assert_eq!(
                    outcome.status, "error",
                    "{engine}/{environment}: {}",
                    outcome.output
                );
                assert_eq!(outcome.exit_code, Some(1));
                let diagnostic = outcome.diagnostics[&main]
                    .iter()
                    .find(|diagnostic| {
                        diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                            && diagnostic.message.contains("Extraneous input")
                    })
                    .unwrap_or_else(|| panic!("{engine}/{environment}: {:?}", outcome.diagnostics));
                assert_eq!(diagnostic.range.start.line, 6, "{diagnostic:?}");
                assert!(diagnostic.message.contains("line end"), "{diagnostic:?}");
                let inputs = outcome.inputs.unwrap();
                assert!(!inputs.contains(&discarded));
                workspace.recorded_inputs.insert(main.clone(), inputs);
                assert!(!workspace.project_documents(&main).contains(&discarded));
                let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                assert!(aux.contains(&format!("newlabel{{{key}}}")), "{aux}");
                assert!(
                    !aux.contains("newlabel{discarded}")
                        && !aux.contains("newlabel{discardedinputlabel}"),
                    "{aux}"
                );
                if key == "renamedlabel" {
                    assert!(!aux.contains("newlabel{kept}"), "{aux}");
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_fancyverb_environments_keep_example_labels_literal_before_and_after_renaming() {
    for engine in ["pdflatex", "xelatex"] {
        for environment in ENVIRONMENTS {
            for foreign_end in [false, true] {
                let prefix = if foreign_end {
                    format!("\\end{{other}}\\end{{{environment}}}\\label{{fakeafter}}\n")
                } else {
                    String::new()
                };
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = uri(root, "main.tex");
                let use_saved = if environment.starts_with("SaveVerbatim") {
                    "\\UseVerbatim{saved}\n"
                } else {
                    ""
                };
                let source = format!(
                "\\documentclass{{article}}\n\\usepackage{{fancyvrb}}\n\\begin{{document}}\n\\label{{kept}}\n\\begin{{{environment}}}{}\n{prefix}\\label{{kept}}\\label{{fake}}\\newcommand{{\\hidden}}{{Text}}\n\\end{{{environment}}}\n{use_saved}\\ref{{kept}}\n\\end{{document}}\n",
                argument(environment)
            );
                std::fs::write(root.join("main.tex"), &source).unwrap();
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                workspace.config.build.executable = engine.into();
                let position = workspace.documents[&main]
                    .text
                    .position(source.rfind("kept").unwrap());
                let changes = features::rename(&workspace, &main, position, "renamedlabel")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                let updated = apply(&source, changes[&main].clone());
                assert!(updated.contains("\\label{kept}\\label{fake}\\newcommand{\\hidden}{Text}"));
                for (contents, key) in [(&source, "kept"), (&updated, "renamedlabel")] {
                    std::fs::write(root.join("main.tex"), contents).unwrap();
                    workspace.open(main.clone(), contents.clone(), 1);
                    let outcome = build::run(
                        &root.join("main.tex"),
                        &workspace.config.build,
                        &AtomicBool::new(false),
                    );
                    assert_eq!(
                        outcome.status, "success",
                        "{engine}/{environment}: {}",
                        outcome.output
                    );
                    assert!(outcome
                        .diagnostics
                        .values()
                        .flatten()
                        .all(|diagnostic| diagnostic.severity != Some(DiagnosticSeverity::ERROR)));
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert!(aux.contains(&format!("newlabel{{{key}}}")), "{aux}");
                    assert!(!aux.contains("newlabel{fake}"), "{aux}");
                    assert!(!aux.contains("newlabel{fakeafter}"), "{aux}");
                    if key == "renamedlabel" {
                        assert!(!aux.contains("newlabel{kept}"), "{aux}");
                    }
                    if environment.starts_with("VerbatimOut") {
                        let output =
                            std::fs::read_to_string(root.join("build/latex-lsp/output.txt"))
                                .unwrap();
                        assert!(output.contains("\\label{kept}\\label{fake}"), "{output}");
                    }
                }
            }
        }
    }
}
