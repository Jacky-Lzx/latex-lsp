use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, text: &str, version: Option<i32>) -> Url {
    let uri = Url::parse(&format!("file:///rename-test/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, text.into(), version));
    uri
}

fn edits_result(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let changes = edits
        .into_iter()
        .map(|edit| TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        })
        .collect();
    Text::new(source.into()).changed(changes).unwrap().source
}

#[test]
fn command_copy_targets_and_source_parameters_remain_independent_symbols() {
    for name in ["NewCommandCopy", "RenewCommandCopy", "DeclareCommandCopy"] {
        for arguments in [
            r"{\alias}{\original}",
            r"\alias\original",
            "\\alias % keep\r\n{\\original}",
        ] {
            let mut workspace = Workspace::default();
            let definition = format!("\\newcommand{{\\original}}[1]{{#1}}\\{name}{arguments}");
            let definitions = insert(&mut workspace, "definitions.tex", &definition, Some(1));
            let source = "😀 \\alias{Copied}\\original{Direct}\\csname alias\\endcsname{Literal}\n% \\alias \\original\n\\verb|\\alias \\original|";
            let main = insert(&mut workspace, "main.tex", source, Some(2));
            for (old, new, count) in [
                ("alias", "copyrenamed", 3),
                ("original", "sourcerenamed", 3),
            ] {
                let doc = &workspace.documents[&main];
                let position = doc
                    .text
                    .position(source.find(&format!("\\{old}")).unwrap() + 2);
                assert_eq!(features::definitions(&workspace, &main, position).len(), 1);
                assert_eq!(
                    features::references(&workspace, &main, position, true).len(),
                    count
                );
                let changes = features::rename(&workspace, &main, position, new)
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes.len(), 2);
                let declarations = edits_result(&definition, &changes[&definitions]);
                assert_eq!(
                    declarations,
                    definition.replace(&format!("\\{old}"), &format!("\\{new}"))
                );
                let changed = edits_result(source, &changes[&main]);
                let (body, rest) = source.split_once('\n').unwrap();
                let expected = body
                    .replace(&format!("\\{old}"), &format!("\\{new}"))
                    .replace(&format!("csname {old}"), &format!("csname {new}"));
                assert_eq!(changed, format!("{expected}\n{rest}"));
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex with command copy support"]
fn real_command_copy_renames_preserve_copied_optional_arguments_after_source_redefinition() {
    for copy in ["NewCommandCopy", "DeclareCommandCopy"] {
        for declaration in [
            "\\DeclareRobustCommand{\\original}[2][blue]{\\typeout{COPY-AUDIT:#1:#2}#2}",
            "\\NewDocumentCommand{\\original}{O{blue}m}{\\typeout{COPY-AUDIT:#1:#2}#2}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let definitions = format!("{declaration}\n\\{copy}{{\\alias}}{{\\original}}% \\alias\n\\renewcommand{{\\original}}[1]{{Changed:#1}}\n");
            let content = "\\alias{Default}\\alias[red]{Explicit}\\csname alias\\endcsname{Literal}\n% \\alias\n\\verb|\\alias|\n";
            std::fs::write(&main, "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n").unwrap();
            std::fs::write(root.join("definitions.tex"), &definitions).unwrap();
            std::fs::write(root.join("content.tex"), content).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: "pdflatex".into(),
                ..Default::default()
            };
            let before = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(before.status, "success", "{copy}: {}", before.output);
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let uri = Url::from_file_path(root.join("content.tex")).unwrap();
            let position = workspace.documents[&uri].text.position(2);
            let changes = features::rename(&workspace, &uri, position, "renamedalias")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 2);
            for (uri, edits) in changes {
                std::fs::write(
                    uri.to_file_path().unwrap(),
                    edits_result(&workspace.documents[&uri].text.source, &edits),
                )
                .unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(root.join("definitions.tex")).unwrap(),
                definitions.replacen("\\alias", "\\renamedalias", 1)
            );
            assert!(std::fs::read_to_string(root.join("content.tex"))
                .unwrap()
                .ends_with("% \\alias\n\\verb|\\alias|\n"));
            let after = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(after.status, "success", "{copy}: {}", after.output);
            for outcome in [before, after] {
                let observed: Vec<_> = outcome
                    .output
                    .lines()
                    .filter(|line| line.starts_with("COPY-AUDIT:"))
                    .collect();
                assert_eq!(
                    observed,
                    [
                        "COPY-AUDIT:blue:Default",
                        "COPY-AUDIT:red:Explicit",
                        "COPY-AUDIT:blue:Literal"
                    ]
                );
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex with document command support"]
fn real_expandable_command_renames_preserve_edef_results_and_optional_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let definitions = "\\NewExpandableDocumentCommand % declaration\n{\\formatpair} % target\n{O{blue} +m}{#1:#2}% \\formatpair\n";
    let content = concat!(
        "\\edef\\observed{\\formatpair{Default}/\\formatpair[red]{Explicit}/\\csname formatpair\\endcsname{Dynamic}}\n",
        "\\typeout{EXPANDABLE-AUDIT:\\observed}\\observed\n",
        "% \\formatpair\n\\verb|\\formatpair|\n",
    );
    std::fs::write(&main, "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n").unwrap();
    std::fs::write(root.join("definitions.tex"), definitions).unwrap();
    std::fs::write(root.join("content.tex"), content).unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = latex_lsp::build::run(&main, &config, &Default::default());
    assert_eq!(before.status, "success", "{}", before.output);
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("content.tex")).unwrap();
    let doc = &workspace.documents[&uri];
    let position = doc.text.position(content.find("\\formatpair").unwrap() + 2);
    let changes = features::rename(&workspace, &uri, position, "renderpair")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    for (uri, edits) in changes {
        std::fs::write(
            uri.to_file_path().unwrap(),
            edits_result(&workspace.documents[&uri].text.source, &edits),
        )
        .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(root.join("definitions.tex")).unwrap(),
        definitions.replacen("\\formatpair", "\\renderpair", 1)
    );
    let changed = std::fs::read_to_string(root.join("content.tex")).unwrap();
    assert!(changed.ends_with("% \\formatpair\n\\verb|\\formatpair|\n"));
    let after = latex_lsp::build::run(&main, &config, &Default::default());
    assert_eq!(after.status, "success", "{}", after.output);
    let expected = "EXPANDABLE-AUDIT:blue:Default/red:Explicit/blue:Dynamic";
    assert!(
        before.output.lines().any(|line| line == expected),
        "{}",
        before.output
    );
    assert!(
        after.output.lines().any(|line| line == expected),
        "{}",
        after.output
    );
}

#[test]
#[ignore = "requires pdflatex and mathtools"]
fn real_paired_delimiter_renames_preserve_all_sizing_forms_and_formula_dimensions() {
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = concat!(
        "\\DeclarePairedDelimiter{\\absval}{\\lvert}{\\rvert}\n",
        "\\DeclarePairedDelimiterX\\innerprod[2]{\\langle}{\\rangle}{#1,#2}\n",
        "\\DeclarePairedDelimiterXPP{\\scalednorm}[1]{\\mathopen{}}{\\lVert}{\\rVert}{^2}{#1}\n",
    );
    let content = concat!(
        "\\setbox0=\\hbox{$\\absval{x}+\\absval*{\\frac12}+\\absval[\\Big]{x}$}\n",
        "\\typeout{DELIMITER-AUDIT-ABS:\\the\\wd0/\\the\\ht0/\\the\\dp0}\\box0\n",
        "\\setbox0=\\hbox{$\\innerprod{x}{y}+\\innerprod*{\\frac12}{y}+\\innerprod[\\Big]{x}{y}$}\n",
        "\\typeout{DELIMITER-AUDIT-INNER:\\the\\wd0/\\the\\ht0/\\the\\dp0}\\box0\n",
        "\\setbox0=\\hbox{$\\scalednorm{x}+\\scalednorm*{\\frac12}+\\scalednorm[\\Big]{x}$}\n",
        "\\typeout{DELIMITER-AUDIT-NORM:\\the\\wd0/\\the\\ht0/\\the\\dp0}\\box0\n",
        "% \\absval \\innerprod \\scalednorm\n",
        "\\verb|\\absval \\innerprod \\scalednorm|\n",
    );
    std::fs::write(root.join("main.tex"), "\\documentclass{article}\n\\usepackage{mathtools}\n\\input{definitions}\n\\begin{document}\n\\input{content}\n\\end{document}\n").unwrap();
    std::fs::write(root.join("definitions.tex"), definitions).unwrap();
    std::fs::write(root.join("content.tex"), content).unwrap();
    let main = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = latex_lsp::build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "success", "{}", before.output);
    for name in ["absval", "innerprod", "scalednorm"] {
        let mut workspace = Workspace::default();
        workspace.add_root(root.to_owned());
        let uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
        let doc = &workspace.documents[&uri];
        let position = doc
            .text
            .position(doc.text.source.find(&format!("\\{name}")).unwrap() + 2);
        let changes = features::rename(&workspace, &uri, position, &format!("{name}renamed"))
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        for (uri, edits) in changes {
            let changed = edits_result(&workspace.documents[&uri].text.source, &edits);
            std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
        }
    }
    let after = latex_lsp::build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
    let dimensions = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("DELIMITER-AUDIT-"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(dimensions(&before.output).len(), 3);
    assert_eq!(dimensions(&before.output), dimensions(&after.output));
    let changed = std::fs::read_to_string(root.join("content.tex")).unwrap();
    assert!(changed.contains("% \\absval \\innerprod \\scalednorm\n"));
    assert!(changed.contains(r"\verb|\absval \innerprod \scalednorm|"));
}

#[test]
fn pdfbookmark_renames_update_literal_levels_preserve_titles_comments_and_reference_labels() {
    for (option, old_level, new_name, new_level) in [
        ("", "0", "renamed.0", "0"),
        ("", "0", "chapter.final.2", "2"),
        ("[01]", "01", "new.+1", "+1"),
        ("[-1]", "-1", "part.0", "0"),
        ("[-3]", "-3", "lowest.0", "0"),
        ("[+1]", "+1", "next.+1", "+1"),
    ] {
        let definitions = format!("😀 \\pdfbookmark{option} % head\r\n{{Title key.0}}% title\r\n{{key}}\\hypertarget{{reserved.0}}{{Other}}\n");
        let old_name = format!("key.{old_level}");
        let source = format!("\\hyperlink{{{old_name}}}{{key.0}}\\hyperref{{}}{{}}{{{old_name}}}{{Text}}\\label{{{old_name}}}\\ref{{{old_name}}}\n");
        let mut workspace = Workspace::default();
        let declaration = insert(&mut workspace, "bookmarks.tex", &definitions, None);
        let main = insert(&mut workspace, "main.tex", &source, Some(2));
        for from_declaration in [false, true] {
            let (uri, position) = if from_declaration {
                let doc = &workspace.documents[&declaration];
                (
                    &declaration,
                    doc.text.position(doc.syntax.anchors[0].span.start + 1),
                )
            } else {
                (
                    &main,
                    Text::new(source.clone()).position(source.find(&old_name).unwrap() + 2),
                )
            };
            let prepared = features::prepare_rename(&workspace, uri, position)
                .unwrap()
                .unwrap();
            assert!(
                matches!(prepared, PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } if placeholder == old_name)
            );
            let changes = features::rename(&workspace, uri, position, new_name)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 2);
            let (base, _) = new_name.rsplit_once('.').unwrap();
            let new_option = if option.is_empty() && new_level == "0" {
                String::new()
            } else {
                format!("[{new_level}]")
            };
            assert_eq!(edits_result(&definitions, &changes[&declaration]), format!("😀 \\pdfbookmark{new_option} % head\r\n{{Title key.0}}% title\r\n{{{base}}}\\hypertarget{{reserved.0}}{{Other}}\n"));
            assert_eq!(edits_result(&source, &changes[&main]), format!("\\hyperlink{{{new_name}}}{{key.0}}\\hyperref{{}}{{}}{{{new_name}}}{{Text}}\\label{{{old_name}}}\\ref{{{old_name}}}\n"));
            for invalid in [
                "plain",
                ".1",
                "name.invalid",
                "name. 1",
                "name.2147483648",
                "name.-2147483648",
                "name.-2147483647",
                "name.-4",
                "reserved.0",
            ] {
                assert!(
                    features::rename(&workspace, uri, position, invalid).is_err(),
                    "{invalid}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_pdfbookmark_renames_keep_bookmarks_and_pdf_links_on_the_same_literal_destination() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let targets = root.join("targets.tex");
    let child = root.join("chapter.tex");
    std::fs::write(&main, "\\pdfobjcompresslevel=0\\pdfcompresslevel=0\\documentclass{article}\\usepackage{hyperref}\\begin{document}\\section{Title}\\input{targets}\\input{chapter}\\end{document}\n").unwrap();
    std::fs::write(
        &targets,
        "\\pdfbookmark % keep\n{Title base.0}% title\n{base}\\label{base.0}\n",
    )
    .unwrap();
    std::fs::write(
        &child,
        "\\hyperlink{base.0}{Direct}\\hyperref{}{}{base.0}{Compound}\\ref{base.0}\n",
    )
    .unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let initial = latex_lsp::build::run(&main, &config, &Default::default());
    assert_eq!(initial.status, "success", "{}", initial.output);
    let mut old_name = "base.0";
    for (from_definition, name) in [
        (false, "chapter.first.01"),
        (true, "part.final.-1"),
        (false, "plus.final.+1"),
    ] {
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let uri = Url::from_file_path(if from_definition { &targets } else { &child }).unwrap();
        let doc = &workspace.documents[&uri];
        let at = if from_definition {
            doc.syntax.anchors[0].span.start
        } else {
            doc.text.source.find(old_name).unwrap() + 2
        };
        let changes = features::rename(&workspace, &uri, doc.text.position(at), name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        for (uri, edits) in changes {
            std::fs::write(
                uri.to_file_path().unwrap(),
                edits_result(&workspace.documents[&uri].text.source, &edits),
            )
            .unwrap();
        }
        let (base, level) = name.rsplit_once('.').unwrap();
        assert_eq!(std::fs::read_to_string(&targets).unwrap(), format!("\\pdfbookmark[{level}] % keep\n{{Title base.0}}% title\n{{{base}}}\\label{{base.0}}\n"));
        assert_eq!(std::fs::read_to_string(&child).unwrap(), format!("\\hyperlink{{{name}}}{{Direct}}\\hyperref{{}}{{}}{{{name}}}{{Compound}}\\ref{{base.0}}\n"));
        let mut final_build = None;
        for _ in 0..2 {
            let outcome = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            final_build = Some(outcome);
        }
        let outcome = final_build.unwrap();
        assert!(outcome.diagnostics.is_empty(), "{:?}", outcome.diagnostics);
        let bytes = std::fs::read(config.artifact_path(&main, "pdf").unwrap()).unwrap();
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(
            pdf.contains(&format!("({name})")),
            "Missing PDF destination {name}"
        );
        let links: Vec<_> = pdf.split("/D").skip(1).collect();
        assert!(
            links
                .iter()
                .filter(|tail| tail.trim_start().starts_with(&format!("({name})")))
                .count()
                >= 2,
            "Missing PDF links to {name}"
        );
        assert!(!links
            .iter()
            .any(|tail| tail.trim_start().starts_with(&format!("({old_name})"))));
        let bookmarks =
            std::fs::read_to_string(config.artifact_path(&main, "out").unwrap()).unwrap();
        assert!(bookmarks.contains(&format!("{{{name}}}")), "{bookmarks}");
        old_name = name;
    }
}

#[test]
fn renames_labels_in_multiple_files_but_not_comments_or_verbatim() {
    let mut workspace = Workspace::default();
    let declaration = insert(&mut workspace, "chapter.tex", "\\label{sec:old}", None);
    let source =
        "😀 \\ref{sec:old} \\cref{other, sec:old}\n% \\ref{sec:old}\n\\verb|\\ref{sec:old}|";
    let main = insert(&mut workspace, "main.tex", source, Some(7));
    let edit = features::rename(&workspace, &main, Position::new(0, 10), "sec:中文")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(
        edits_result(
            &workspace.documents[&declaration].text.source,
            &changes[&declaration]
        ),
        r"\label{sec:中文}"
    );
    assert_eq!(
        edits_result(source, &changes[&main]),
        "😀 \\ref{sec:中文} \\cref{other, sec:中文}\n% \\ref{sec:old}\n\\verb|\\ref{sec:old}|"
    );
}

#[test]
fn label_rename_preserves_spaced_names_and_keeps_cref_list_separators() {
    let mut workspace = Workspace::default();
    let declarations = r"\label{sec:old}\label{ sec:old }";
    let declaration = insert(&mut workspace, "chapter.tex", declarations, None);
    let mut source = "😀 ".to_owned();
    for command in ["ref", "eqref", "pageref", "autoref", "vref", "nameref"] {
        source.push_str(&format!(
            "\\{command}{{sec:old}} \\{command}{{ sec:old }}\n"
        ));
    }
    source.push_str(r"\cref{other, sec:old }\Cref{sec:old,other}");
    let main = insert(&mut workspace, "main.tex", &source, Some(7));
    assert!(workspace.documents[&declaration]
        .syntax
        .diagnostics
        .is_empty());
    let text = &workspace.documents[&main].text;
    let position = text.position(source.find("sec:old").unwrap() + 2);
    assert_eq!(features::definitions(&workspace, &main, position).len(), 1);
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        9
    );
    let edit = features::rename(&workspace, &main, position, "sec:new")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(declarations, &edit[&declaration]),
        r"\label{sec:new}\label{ sec:old }"
    );
    let expected = source
        .replace("{sec:old}", "{sec:new}")
        .replace("other, sec:old }", "other, sec:new }")
        .replace("{sec:old,other}", "{sec:new,other}");
    assert_eq!(edits_result(&source, &edit[&main]), expected);
    for command in ["ref", "eqref", "pageref", "autoref", "vref", "nameref"] {
        let start = source.find(&format!("\\{command}{{ sec:old }}")).unwrap();
        let position = text.position(start + command.len() + 4);
        assert!(
            features::definitions(&workspace, &main, position).is_empty(),
            "{command}"
        );
        assert!(
            features::prepare_rename(&workspace, &main, position)
                .unwrap()
                .is_none(),
            "{command}"
        );
    }
}

#[test]
fn hyperref_links_rename_only_bracketed_labels_and_preserve_text_urls_and_spaced_keys() {
    let mut workspace = Workspace::default();
    let declaration = insert(&mut workspace, "chapter.tex", r"\label{sec:old}", None);
    let source = concat!(
        "😀 \\hyperref[sec:old]{sec:old and \\ref{sec:old}}\n",
        "\\hyperref % comment\n[sec:old]{text}\n",
        "\\hyperref[{sec:old}]{sec:old}\n",
        "\\hyperref[ sec:old ]{text}\n",
        "\\hyperref{sec:old}{sec:old}{sec:old}{sec:old}\n",
        "\\hyperref*[sec:old]{text}\n",
        "% \\hyperref[sec:old]{text}\n\\verb|\\hyperref[sec:old]{text}|\n",
    );
    let main = insert(&mut workspace, "main.tex", source, Some(4));
    let text = &workspace.documents[&main].text;
    let position = text.position(source.find("sec:old").unwrap() + 3);
    assert_eq!(
        features::definitions(&workspace, &main, position)[0].uri,
        declaration
    );
    assert!(features::hover(&workspace, &main, position).is_some());
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        5
    );
    let changes = features::rename(&workspace, &main, position, "sec:new")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(source, &changes[&main]),
        source
            .replacen("[sec:old]", "[sec:new]", 2)
            .replace("[{sec:old}]", "[{sec:new}]")
            .replace("\\ref{sec:old}", "\\ref{sec:new}")
    );
    for marker in ["{sec:old and", "[ sec:old ]", "{sec:old}{", "*[sec:old]"] {
        let at = source.find(marker).unwrap() + marker.find("sec:old").unwrap() + 3;
        let position = text.position(at);
        assert!(
            features::definitions(&workspace, &main, position).is_empty(),
            "{marker}"
        );
        assert!(
            features::prepare_rename(&workspace, &main, position)
                .unwrap()
                .is_none(),
            "{marker}"
        );
    }
    for name in ["sec:[new", "sec:new]"] {
        for (document, position) in [(&declaration, Position::new(0, 10)), (&main, position)] {
            let changes = features::rename(&workspace, document, position, name)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            let changed = edits_result(source, &changes[&main]);
            assert!(
                changed.contains(&format!(
                    "\\hyperref[{{{name}}}]{{sec:old and \\ref{{{name}}}}}"
                )),
                "{changed}"
            );
            assert!(
                changed.contains(&format!("\\hyperref[{{{name}}}]{{sec:old}}")),
                "{changed}"
            );
            let mut updated = Workspace::default();
            let label = insert(
                &mut updated,
                "chapter.tex",
                &edits_result(
                    &workspace.documents[&declaration].text.source,
                    &changes[&declaration],
                ),
                None,
            );
            let document = insert(&mut updated, "main.tex", &changed, None);
            let position = updated.documents[&document]
                .text
                .position(changed.find(name).unwrap() + 3);
            assert_eq!(
                features::definitions(&updated, &document, position)[0].uri,
                label
            );
            assert_eq!(
                features::references(&updated, &document, position, true).len(),
                5
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_hyperref_renames_wrap_delimiters_and_preserve_grouped_links_across_repeated_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let chapter = root.join("chapter.tex");
    std::fs::write(&main, "\\documentclass{article}\\usepackage{hyperref}\n\\begin{document}\\section{Title}\\label{sec:old}\\input{chapter}\\end{document}\n").unwrap();
    std::fs::write(
        &chapter,
        "\\hyperref[sec:old]{sec:old}\n\\hyperref[{sec:old}]{Grouped}\\ref*{sec:old}\n",
    )
    .unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let document = Url::from_file_path(&chapter).unwrap();
    let mut old_name = "sec:old";
    for name in ["sec:new]", "sec:[中文"] {
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let doc = &workspace.documents[&document];
        let position = doc
            .text
            .position(doc.text.source.find(old_name).unwrap() + 3);
        let changes = features::rename(&workspace, &document, position, name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        for (uri, edits) in changes {
            std::fs::write(
                uri.to_file_path().unwrap(),
                edits_result(&workspace.documents[&uri].text.source, &edits),
            )
            .unwrap();
        }
        let source = std::fs::read_to_string(&chapter).unwrap();
        assert!(
            source.contains(&format!("\\hyperref[{{{name}}}]{{sec:old}}")),
            "{source}"
        );
        assert!(
            source.contains(&format!("\\hyperref[{{{name}}}]{{Grouped}}")),
            "{source}"
        );
        for _ in 0..2 {
            let result = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(result.status, "success", "{}", result.output);
        }
        let result = latex_lsp::build::run(&main, &config, &Default::default());
        assert!(
            !result
                .diagnostics
                .values()
                .flatten()
                .any(|diagnostic| diagnostic.message.contains("undefined")),
            "{:?}",
            result.diagnostics
        );
        old_name = name;
    }
}

#[test]
fn hyperdef_labels_join_reference_navigation_and_rename_without_changing_destinations_or_text() {
    for grouped in [false, true] {
        let mut workspace = Workspace::default();
        let source = format!(
            "\\hyperdef[{}]{{sec:old}}{{sec:old}}{{sec:old}}",
            if grouped { "{sec:old}" } else { "sec:old" }
        );
        let declaration = insert(&mut workspace, "targets.tex", &source, None);
        let uses = r"\hyperref[sec:old]{sec:old}\ref{sec:old}\getrefnumber{sec:old}";
        let main = insert(&mut workspace, "main.tex", uses, Some(3));
        let declaration_position = workspace.documents[&declaration]
            .text
            .position(source.find("sec:old").unwrap() + 3);
        let position = workspace.documents[&main]
            .text
            .position(uses.find("sec:old").unwrap() + 3);
        let definitions = features::definitions(&workspace, &main, position);
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].uri, declaration);
        assert!(features::hover(&workspace, &main, position).is_some());
        assert!(features::completion(&workspace, &main, position)
            .iter()
            .any(|item| item.label == "sec:old"));
        assert_eq!(
            features::references(&workspace, &declaration, declaration_position, false).len(),
            3
        );
        for (document, position) in [(&declaration, declaration_position), (&main, position)] {
            let changes = features::rename(&workspace, document, position, "sec:new]")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(
                edits_result(&source, &changes[&declaration]),
                r"\hyperdef[{sec:new]}]{sec:old}{sec:old}{sec:old}"
            );
            assert_eq!(
                edits_result(uses, &changes[&main]),
                r"\hyperref[{sec:new]}]{sec:old}\ref{sec:new]}\getrefnumber{sec:new]}"
            );
        }
        let after_label = source.find(']').unwrap() + 1;
        for (index, (at, _)) in source[after_label..].match_indices("sec:old").enumerate() {
            let at = after_label + at + 3;
            let prepared = features::prepare_rename(
                &workspace,
                &declaration,
                workspace.documents[&declaration].text.position(at),
            )
            .unwrap();
            if index < 2 {
                assert!(
                    matches!(prepared, Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) if placeholder == "sec:old.sec:old")
                );
            } else {
                assert!(prepared.is_none());
            }
        }
    }
    for source in [
        r"\hyperdef{sec:old}{name}{Text}",
        r"\hyperdef*[sec:old]{category}{name}{Text}",
        r"\hyperdef[ sec:old ]{category}{name}{Text}",
        r"\hyperdef[\prefix sec:old]{category}{name}{Text}",
        r"% \hyperdef[sec:old]{category}{name}{Text}",
        r"\verb|\hyperdef[sec:old]{category}{name}{Text}|",
    ] {
        let mut workspace = Workspace::default();
        let uri = insert(&mut workspace, "main.tex", source, None);
        assert!(
            workspace.documents[&uri].syntax.labels.is_empty(),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_hyperdef_label_renames_keep_the_pdf_destination_and_resolve_custom_links() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let definitions = root.join("targets.tex");
    let child = root.join("chapter.tex");
    std::fs::write(&main, "\\documentclass{article}\\usepackage{hyperref}\\begin{document}\\input{targets}\\input{chapter}\\end{document}\n").unwrap();
    std::fs::write(&definitions, r"\hyperdef[sec:old]{section}{anchor}{Target}").unwrap();
    std::fs::write(
        &child,
        r"\hyperref[sec:old]{Link}\hyperref[{sec:old}]{Grouped}",
    )
    .unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let mut old_name = "sec:old";
    for (path, name) in [(&child, "sec:new]"), (&definitions, "sec:[中文")] {
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let document = Url::from_file_path(path).unwrap();
        let text = &workspace.documents[&document].text;
        let position = text.position(text.source.find(old_name).unwrap() + 3);
        let changes = features::rename(&workspace, &document, position, name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        for (uri, edits) in changes {
            std::fs::write(
                uri.to_file_path().unwrap(),
                edits_result(&workspace.documents[&uri].text.source, &edits),
            )
            .unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&definitions).unwrap(),
            format!("\\hyperdef[{{{name}}}]{{section}}{{anchor}}{{Target}}")
        );
        for _ in 0..2 {
            let result = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(result.status, "success", "{}", result.output);
        }
        let result = latex_lsp::build::run(&main, &config, &Default::default());
        assert!(
            !result
                .diagnostics
                .values()
                .flatten()
                .any(|diagnostic| diagnostic.message.contains("undefined")),
            "{:?}",
            result.diagnostics
        );
        let auxiliary =
            std::fs::read_to_string(config.artifact_path(&main, "aux").unwrap()).unwrap();
        assert!(
            auxiliary.contains(&format!(
                "\\newlabel{{{name}}}{{{{}}{{}}{{}}{{section.anchor}}{{}}}}"
            )),
            "{auxiliary}"
        );
        old_name = name;
    }
}

#[test]
fn capitalized_and_named_page_references_navigate_and_rename_both_star_forms() {
    let mut workspace = Workspace::default();
    let declaration = insert(
        &mut workspace,
        "chapter.tex",
        r"\label{sec:old}\label{ sec:old }",
        None,
    );
    let source = r"\Ref{sec:old}\Ref*{sec:old}\autopageref{sec:old}\autopageref * {sec:old}\Ref{ sec:old }\autopageref{ sec:old }";
    let main = insert(&mut workspace, "main.tex", source, Some(3));
    let text = &workspace.documents[&main].text;
    for (at, _) in source.match_indices("{sec:old}") {
        let position = text.position(at + 3);
        assert_eq!(
            features::definitions(&workspace, &main, position)[0].uri,
            declaration
        );
        assert_eq!(
            features::references(&workspace, &main, position, false).len(),
            4
        );
        let changes = features::rename(&workspace, &main, position, "sec:new")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            edits_result(source, &changes[&main]),
            source.replace("{sec:old}", "{sec:new}")
        );
        assert_eq!(
            edits_result(
                &workspace.documents[&declaration].text.source,
                &changes[&declaration]
            ),
            r"\label{sec:new}\label{ sec:old }"
        );
    }
    for source in [r"\Ref**{sec:old}", r"\autopageref * * {sec:old}"] {
        let uri = insert(&mut workspace, "invalid.tex", source, None);
        let position = workspace.documents[&uri]
            .text
            .position(source.find("sec:old").unwrap() + 3);
        assert!(features::definitions(&workspace, &uri, position).is_empty());
        assert!(features::prepare_rename(&workspace, &uri, position)
            .unwrap()
            .is_none());
    }
}

#[test]
fn pdf_destinations_rename_independently_of_equal_reference_labels() {
    let mut workspace = Workspace::default();
    let definitions =
        r"\hypertarget{shared}{shared}\label{shared}\label{other}\hypertarget{reserved}{Text}";
    let declaration = insert(&mut workspace, "targets.tex", definitions, None);
    let source = r"\hyperlink{shared}{shared}\hyperref{}{}{shared}{shared}\ref{shared}\hyperref[shared]{shared}";
    let main = insert(&mut workspace, "main.tex", source, Some(2));
    let text = &workspace.documents[&main].text;
    for marker in [r"\hyperlink{shared}", r"\hyperref{}{}{shared}"] {
        let at = source.find(marker).unwrap() + marker.find("shared").unwrap() + 2;
        let position = text.position(at);
        let found = features::definitions(&workspace, &main, position);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].uri, declaration);
        assert_eq!(found[0].range.start.character, 13);
        assert_eq!(
            features::references(&workspace, &main, position, false).len(),
            2
        );
        assert!(features::hover(&workspace, &main, position).is_some());
        let changes = features::rename(&workspace, &main, position, "other")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            edits_result(definitions, &changes[&declaration]),
            definitions.replacen("{shared}", "{other}", 1)
        );
        assert_eq!(
            edits_result(source, &changes[&main]),
            r"\hyperlink{other}{shared}\hyperref{}{}{other}{shared}\ref{shared}\hyperref[shared]{shared}"
        );
        assert!(features::rename(&workspace, &main, position, "reserved").is_err());
    }
    let position = text.position(source.find(r"\ref{shared}").unwrap() + 7);
    let changes = features::rename(&workspace, &main, position, "label-new")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(definitions, &changes[&declaration]),
        definitions.replace(r"\label{shared}", r"\label{label-new}")
    );
    assert_eq!(
        edits_result(source, &changes[&main]),
        r"\hyperlink{shared}{shared}\hyperref{}{}{shared}{shared}\ref{label-new}\hyperref[label-new]{shared}"
    );
}

#[test]
fn compound_pdf_destination_renames_preserve_separators_comments_labels_and_text() {
    let definitions = "\\hyperdef[label]{section}% definition separator\r\n{start}{section.start}";
    let source = "\\hyperlink{section.start}{section.start}\n\\hyperref{}{section} % link separator\n{start}{section.start}\n\\hyperref[label]{Text}";
    for name in ["section.updated", "chapter.final"] {
        let mut workspace = Workspace::default();
        let declaration = insert(&mut workspace, "targets.tex", definitions, None);
        let main = insert(&mut workspace, "main.tex", source, Some(2));
        for (document, marker) in [
            (&declaration, "{section}"),
            (&declaration, "{start}"),
            (&main, "{start}"),
        ] {
            let text = &workspace.documents[document].text;
            let position = text.position(text.source.find(marker).unwrap() + 2);
            let prepared = features::prepare_rename(&workspace, document, position)
                .unwrap()
                .unwrap();
            assert!(
                matches!(prepared, PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } if placeholder == "section.start")
            );
            assert_eq!(
                features::references(&workspace, document, position, false).len(),
                2
            );
            let changes = features::rename(&workspace, document, position, name)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            let (category, key) = if name == "section.updated" {
                ("section", "updated")
            } else {
                ("", "chapter.final")
            };
            assert_eq!(edits_result(definitions, &changes[&declaration]), format!("\\hyperdef[label]{{{category}}}% definition separator\r\n{{{key}}}{{section.start}}"));
            assert_eq!(edits_result(source, &changes[&main]), format!("\\hyperlink{{{name}}}{{section.start}}\n\\hyperref{{}}{{{category}}} % link separator\n{{{key}}}{{section.start}}\n\\hyperref[label]{{Text}}"));
        }
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_pdf_destination_renames_update_direct_and_compound_links_without_renaming_labels() {
    for compound in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let targets = root.join("targets.tex");
        let child = root.join("chapter.tex");
        std::fs::write(&main, "\\pdfobjcompresslevel=0\\pdfcompresslevel=0\\documentclass{article}\\usepackage{hyperref}\\begin{document}\\section{Title}\\input{targets}\\input{chapter}\\end{document}\n").unwrap();
        std::fs::write(
            &targets,
            if compound {
                "\\hyperdef[section.start]{section}% keep definition separator\n{start}{Target}"
            } else {
                "\\hypertarget{section.start}{Target}\\label{section.start}"
            },
        )
        .unwrap();
        std::fs::write(&child, "\\hyperlink{section.start}{Direct}\\hyperref{}{section}% keep link separator\n{start}{Internal}\\hyperref[section.start]{Label}").unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let mut old_name = "section.start";
        for (from_definition, name) in [(false, "section.next"), (true, "chapter.final")] {
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let document =
                Url::from_file_path(if from_definition { &targets } else { &child }).unwrap();
            let doc = &workspace.documents[&document];
            let offset = if from_definition {
                doc.syntax
                    .anchors
                    .iter()
                    .find(|entry| entry.name == old_name)
                    .unwrap()
                    .span
                    .start
            } else {
                doc.text.source.find(old_name).unwrap() + 3
            };
            let changes = features::rename(&workspace, &document, doc.text.position(offset), name)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 2);
            for (uri, edits) in changes {
                std::fs::write(
                    uri.to_file_path().unwrap(),
                    edits_result(&workspace.documents[&uri].text.source, &edits),
                )
                .unwrap();
            }
            let source = std::fs::read_to_string(&child).unwrap();
            assert!(
                source.contains(r"\hyperref[section.start]{Label}"),
                "{source}"
            );
            assert!(source.contains("% keep link separator\n"), "{source}");
            let mut last = None;
            for _ in 0..2 {
                let result = latex_lsp::build::run(&main, &config, &Default::default());
                assert_eq!(result.status, "success", "{}", result.output);
                last = Some(result);
            }
            let result = last.unwrap();
            assert!(
                !result.output.contains("undefined") && !result.output.contains("does not exist"),
                "{}",
                result.output
            );
            let bytes = std::fs::read(config.artifact_path(&main, "pdf").unwrap()).unwrap();
            let pdf = String::from_utf8_lossy(&bytes);
            assert!(
                pdf.split("/D")
                    .skip(1)
                    .filter(|tail| tail.trim_start().starts_with(&format!("({name})")))
                    .count()
                    >= 2,
                "Missing PDF link destination {name}"
            );
            let auxiliary =
                std::fs::read_to_string(config.artifact_path(&main, "aux").unwrap()).unwrap();
            assert!(
                auxiliary.contains(r"\newlabel{section.start}"),
                "{auxiliary}"
            );
            if compound {
                assert!(auxiliary.contains(name), "{auxiliary}");
            }
            old_name = name;
        }
    }
}

#[test]
fn pdf_destination_renames_respect_projects_and_reject_shared_references_with_different_definitions(
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for project in ["a", "b"] {
        std::fs::create_dir_all(root.join(project)).unwrap();
        std::fs::write(
            root.join(project).join("main.tex"),
            r"\documentclass{article}\input{targets}\hyperlink{shared}{Link}",
        )
        .unwrap();
        std::fs::write(
            root.join(project).join("targets.tex"),
            r"\hypertarget{shared}{Target}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let document = Url::from_file_path(root.join("a/main.tex")).unwrap();
    let text = &workspace.documents[&document].text;
    let position = text.position(text.source.find("shared").unwrap() + 2);
    let changes = features::rename(&workspace, &document, position, "changed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert!(changes
        .keys()
        .all(|uri| uri.to_file_path().unwrap().starts_with(root.join("a"))));
    std::fs::write(root.join("shared.tex"), r"\hyperlink{shared}{Link}").unwrap();
    for project in ["a", "b"] {
        std::fs::write(
            root.join(project).join("main.tex"),
            r"\documentclass{article}\hypertarget{shared}{Target}\input{../shared}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let text = &workspace.documents[&document].text;
    let position = text.position(text.source.find("shared").unwrap() + 2);
    assert!(features::rename(&workspace, &document, position, "changed").is_err());
}

#[test]
fn refcount_queries_navigate_complete_and_rename_only_the_label_argument() {
    for (command, tail) in [
        ("getrefnumber", ""),
        ("getpagerefnumber", ""),
        ("getrefbykeydefault", "{sec:old}{sec:old}"),
        ("IfRefUndefinedExpandable", "{sec:old}{sec:old}"),
        ("IfRefUndefinedBabel", "{sec:old}{sec:old}"),
        ("refused", ""),
    ] {
        let mut workspace = Workspace::default();
        let declaration = insert(&mut workspace, "chapter.tex", r"\label{sec:old}", None);
        let normal = format!("\\{command}{{sec:old}}{tail}");
        let spaced = format!("\\{command}{{ sec:old }}{tail}");
        let source = format!("😀 {normal}\n{spaced}");
        let main = insert(&mut workspace, "main.tex", &source, Some(7));
        let text = &workspace.documents[&main].text;
        let position = text.position(source.find("sec:old").unwrap() + 4);
        let locations = features::definitions(&workspace, &main, position);
        assert_eq!(locations.len(), 1, "{command}: {locations:?}");
        assert_eq!(locations[0].uri, declaration);
        assert!(features::hover(&workspace, &main, position).is_some());
        assert_eq!(
            features::references(&workspace, &main, position, false).len(),
            1,
            "{command}"
        );
        let candidates = features::completion(&workspace, &main, position);
        assert!(
            candidates.iter().any(|item| item.label == "sec:old"),
            "{command}: {candidates:?}"
        );
        let changes = features::rename(&workspace, &main, position, "sec:new")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            edits_result(&source, &changes[&main]),
            format!(
                "😀 {}\n{spaced}",
                normal.replacen("{sec:old}", "{sec:new}", 1)
            )
        );
        if !tail.is_empty() {
            let field =
                text.position(source.find(&normal).unwrap() + normal.len() - tail.len() + 3);
            assert!(
                features::definitions(&workspace, &main, field).is_empty(),
                "{command}"
            );
            assert!(
                features::completion(&workspace, &main, field).is_empty(),
                "{command}"
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex and refcount"]
fn real_label_rename_preserves_independent_spaced_keys_and_reference_values() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let source = concat!(
        "\\documentclass{article}\\usepackage{refcount}\n\\begin{document}\n",
        "\\refstepcounter{section}\\label{plain}\n",
        "\\refstepcounter{section}\\label{ plain }\n",
        "\\ref{plain}\\ref{ plain }\n",
        "\\typeout{WITNESS=\\getrefnumber{plain};\\getrefnumber{ plain }}\n",
        "\\typeout{PAGE=\\getpagerefnumber{plain};FIELD=\\getrefbykeydefault{plain}{page}{0}}\n",
        "\\IfRefUndefinedExpandable{plain}{\\typeout{EXPANDABLE=missing}}{\\typeout{EXPANDABLE=found}}\n",
        "\\IfRefUndefinedBabel{plain}{\\typeout{BABEL=missing}}{\\typeout{BABEL=found}}\n",
        "\\refused{plain}\n",
        "\\end{document}\n",
    );
    std::fs::write(&main, source).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let position = workspace.documents[&uri]
        .text
        .position(source.find("plain").unwrap());
    let changes = features::rename(&workspace, &uri, position, "changed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    let changed = edits_result(source, &changes[&uri]);
    assert_eq!(changed, source.replace("{plain}", "{changed}"));
    std::fs::write(&main, changed).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let first = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(first.status, "success", "{}", first.output);
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert!(outcome.output.contains("WITNESS=1;2"), "{}", outcome.output);
    for witness in ["PAGE=1;FIELD=1", "EXPANDABLE=found", "BABEL=found"] {
        assert!(outcome.output.contains(witness), "{}", outcome.output);
    }
    assert!(
        outcome
            .diagnostics
            .values()
            .flatten()
            .all(|diagnostic| !diagnostic.message.contains("undefined")),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn citation_rename_updates_bibtex_keys_and_only_matching_citations() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{old, title={old}}\n@article{other, title={Old}}",
        None,
    );
    let main = insert(
        &mut workspace,
        "main.tex",
        r"\cite[see]{old, other}\citep{old}",
        Some(1),
    );
    let edit = features::rename(&workspace, &bib, Position::new(0, 8), "new2026")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(&workspace.documents[&bib].text.source, &edit[&bib]),
        "@book{new2026, title={old}}\n@article{other, title={Old}}"
    );
    assert_eq!(
        edits_result(&workspace.documents[&main].text.source, &edit[&main]),
        r"\cite[see]{new2026, other}\citep{new2026}"
    );
}

#[test]
fn macro_rename_preserves_backslashes_and_carries_document_versions() {
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    let source = r"\newcommand{\foo}[1]{#1} \foo{中文}";
    let uri = insert(&mut workspace, "main.tex", source, Some(12));
    let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
        features::prepare_rename(&workspace, &uri, Position::new(0, 14)).unwrap()
    else {
        panic!("expected rename target");
    };
    assert_eq!(placeholder, "foo");
    assert_eq!(range.start.character, 13);
    let edit = features::rename(&workspace, &uri, Position::new(0, 14), "bar")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edit");
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.version, Some(12));
    let edits = documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect::<Vec<_>>();
    assert_eq!(edits.len(), 2);
    assert_eq!(
        edits_result(source, &edits),
        r"\newcommand{\bar}[1]{#1} \bar{中文}"
    );
}

#[test]
#[ignore = "requires pdflatex and a current LaTeX installation"]
fn renamed_custom_environments_still_compile() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.tex");
    let uri = Url::from_file_path(&path).unwrap();
    let source = "\\documentclass{article}\n\\NewDocumentEnvironment{custombox}{m}{\\begin{quote}#1}{\\end{quote}}\n\\begin{document}\n\\begin{custombox}{Hello}\nWorld\n\\end{custombox}\n\\end{document}\n";
    let mut workspace = Workspace::default();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let position = Text::new(source.into()).position(source.find("custombox").unwrap() + 2);
    let edits = features::rename(&workspace, &uri, position, "remarkbox")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&uri].len(), 3);
    std::fs::write(&path, edits_result(source, &edits[&uri])).unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = latex_lsp::build::run(&path, &config, &std::sync::atomic::AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
}

#[test]
fn custom_environment_rename_updates_declarations_begin_and_end_across_files() {
    let mut workspace = Workspace::default();
    let declaration = insert(
        &mut workspace,
        "environments.tex",
        r"\NewDocumentEnvironment{custom}{O{x}}{}{}",
        Some(2),
    );
    let source =
        "😀 \\begin{custom}\nText\n\\end{custom}\n% \\begin{custom}\n\\verb|\\end{custom}|";
    let main = insert(&mut workspace, "main.tex", source, Some(5));
    let position = Text::new(source.into()).position(source.find("custom").unwrap() + 2);
    let Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) =
        features::prepare_rename(&workspace, &main, position).unwrap()
    else {
        panic!();
    };
    assert_eq!(placeholder, "custom");
    assert_eq!(
        features::definitions(&workspace, &main, position)[0].uri,
        declaration
    );
    assert_eq!(
        features::references(&workspace, &main, position, false).len(),
        2
    );
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        3
    );
    assert!(features::hover(&workspace, &main, position).is_some());
    let symbols = features::symbols(&workspace.documents[&declaration]);
    assert_eq!(symbols[0].name, "custom");
    assert_eq!(symbols[0].kind, SymbolKind::CLASS);
    let edits = features::rename(&workspace, &main, position, "renamed*")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(
            &workspace.documents[&declaration].text.source,
            &edits[&declaration]
        ),
        r"\NewDocumentEnvironment{renamed*}{O{x}}{}{}"
    );
    assert_eq!(
        edits_result(source, &edits[&main]),
        "😀 \\begin{renamed*}\nText\n\\end{renamed*}\n% \\begin{custom}\n\\verb|\\end{custom}|"
    );
    for name in ["itemize", "section", "bad name"] {
        assert!(features::rename(&workspace, &main, position, name).is_err());
    }
    let macros = insert(
        &mut workspace,
        "macros.tex",
        r"\newcommand{\taken}{X}\newcommand{\endoccupied}{Y}",
        None,
    );
    assert!(features::rename(&workspace, &main, position, "taken").is_err());
    assert!(features::rename(&workspace, &main, position, "occupied").is_err());
    let macro_position = Text::new(workspace.documents[&macros].text.source.clone()).position(15);
    assert!(features::rename(&workspace, &macros, macro_position, "custom").is_err());
    insert(
        &mut workspace,
        "duplicate.tex",
        r"\newenvironment{custom}{}{}",
        None,
    );
    assert!(features::prepare_rename(&workspace, &main, position).is_err());
}

#[test]
fn environment_declaration_variants_are_indexed_and_complete() {
    for command in [
        "newenvironment",
        "renewenvironment",
        "provideenvironment",
        "NewDocumentEnvironment",
        "RenewDocumentEnvironment",
        "ProvideDocumentEnvironment",
        "DeclareDocumentEnvironment",
        "NewEnviron",
        "RenewEnviron",
        "newtheorem",
    ] {
        let mut workspace = Workspace::default();
        let declaration = insert(
            &mut workspace,
            "env.tex",
            &format!("\\{command}{{special}}{{}}{{}}"),
            Some(1),
        );
        let main = insert(&mut workspace, "main.tex", r"\begin{spe", Some(1));
        assert_eq!(
            workspace.documents[&declaration].syntax.environments.len(),
            1,
            "{command}"
        );
        assert!(
            features::completion(&workspace, &main, Position::new(0, 10))
                .iter()
                .any(|item| item.label == "special"),
            "{command}"
        );
        let edit = features::rename(
            &workspace,
            &declaration,
            Position::new(0, command.len() as u32 + 3),
            "different",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert!(edits_result(
            &workspace.documents[&declaration].text.source,
            &edit[&declaration]
        )
        .contains("{different}"));
    }
    let mut workspace = Workspace::default();
    let uri = insert(
        &mut workspace,
        "main.tex",
        r"\begin{itemize}\end{itemize}",
        Some(1),
    );
    assert!(
        features::prepare_rename(&workspace, &uri, Position::new(0, 9))
            .unwrap()
            .is_none()
    );
}

#[test]
fn modern_and_unbraced_macro_declarations_support_navigation_completion_and_rename() {
    for declaration in [
        r"\newcommand\hello[1]{Hello #1}",
        r"\renewcommand* \hello [1] {Hello #1}",
        "\\providecommand % declaration\n\\hello[1]{Hello #1}",
        r"\DeclareMathOperator*{\hello}{hello}",
        r"\DeclarePairedDelimiter{\hello}{\lvert}{\rvert}",
        r"\DeclarePairedDelimiter\hello{(}{)}",
        r"\DeclarePairedDelimiterX{\hello}[2]{\langle}{\rangle}{#1,#2}",
        r"\DeclarePairedDelimiterX\hello[1]{[}{]}{#1}",
        r"\DeclarePairedDelimiterXPP{\hello}[1]{\mathopen{}}{\lVert}{\rVert}{^2}{#1}",
        r"\DeclarePairedDelimiterXPP\hello[1]{}{(}{)}{}{#1}",
        r"\DeclareRobustCommand\hello[1]{#1}",
        r"\newrobustcmd{\hello}[1]{Hello #1}",
        r"\csdef{hello}#1{Hello #1}",
        r"\csgdef{ hello }#1{Hello #1}",
        r"\csedef{hello}#1{Hello #1}",
        r"\csxdef{hello}#1{Hello #1}",
        r"\renewrobustcmd* \hello [1] {Hello #1}",
        "\\providerobustcmd % declaration\n\\hello[1]{Hello #1}",
        r"\NewDocumentCommand{\hello}{m}{#1}",
        r"\RenewDocumentCommand\hello{m}{#1}",
        r"\ProvideDocumentCommand{\hello}{O{x}}{#1}",
        r"\DeclareDocumentCommand{\hello}{m}{#1}",
        r"\NewExpandableDocumentCommand{\hello}{O{blue}m}{#1:#2}",
        r"\RenewExpandableDocumentCommand\hello{m}{#1}",
        r"\ProvideExpandableDocumentCommand{\hello}{m}{#1}",
        r"\DeclareExpandableDocumentCommand\hello{m}{#1}",
        r"\def\hello#1{Hello #1}",
        r"\gdef\hello#1{Hello #1}",
        r"\edef\hello{Hello}",
        r"\xdef\hello{Hello}",
        r"\let\hello=\textbf",
        r"\NewCommandCopy{\hello}{\textbf}",
        r"\RenewCommandCopy\hello\textbf",
        r"\DeclareCommandCopy\hello{\textbf}",
        r"\long\def\hello#1{Hello #1}",
    ] {
        let mut workspace = Workspace::default();
        let macros = insert(&mut workspace, "macros.tex", declaration, Some(1));
        let main = insert(&mut workspace, "main.tex", "\\hello{World}\n\\hel", Some(3));
        let definitions = &workspace.documents[&macros].syntax.definitions;
        assert_eq!(definitions.len(), 1, "{declaration}");
        assert_eq!(definitions[0].name, "hello");
        assert!(
            features::diagnostics(&workspace.documents[&macros]).is_empty(),
            "{declaration}"
        );
        assert_eq!(
            features::definitions(&workspace, &main, Position::new(0, 3))[0].uri,
            macros
        );
        assert_eq!(
            features::references(&workspace, &main, Position::new(0, 3), false).len(),
            1,
            "{declaration}"
        );
        assert_eq!(
            features::references(&workspace, &main, Position::new(0, 3), true).len(),
            2,
            "{declaration}"
        );
        assert!(features::completion(&workspace, &main, Position::new(1, 4))
            .iter()
            .any(|item| item.label == "hello"));
        let declaration_position = workspace.documents[&macros]
            .text
            .position(definitions[0].span.start + 2);
        let edits = features::rename(&workspace, &macros, declaration_position, "greeting")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            edits_result(declaration, &edits[&macros]),
            declaration.replacen("hello", "greeting", 1),
            "{declaration}"
        );
        assert_eq!(
            edits_result(&workspace.documents[&main].text.source, &edits[&main]),
            "\\greeting{World}\n\\hel"
        );
    }
}

#[test]
fn rejects_conflicts_ambiguous_definitions_invalid_names_and_builtins() {
    let mut workspace = Workspace::default();
    let uri = insert(
        &mut workspace,
        "main.tex",
        r"\label{old}\label{taken}\ref{old}\newcommand{\foo}{x}\foo",
        Some(1),
    );
    assert!(features::rename(&workspace, &uri, Position::new(0, 8), "taken").is_err());
    for name in ["", "bad name", "a,b", "a}x", "\\new"] {
        assert!(features::rename(&workspace, &uri, Position::new(0, 8), name).is_err());
    }
    assert!(
        features::prepare_rename(&workspace, &uri, Position::new(0, 3))
            .unwrap()
            .is_none()
    );
    let macro_position = workspace.documents[&uri]
        .text
        .position(workspace.documents[&uri].text.source.rfind("foo").unwrap() + 1);
    assert!(features::rename(&workspace, &uri, macro_position, "section").is_err());
    insert(&mut workspace, "another.tex", r"\label{old}", None);
    assert!(features::prepare_rename(&workspace, &uri, Position::new(0, 8)).is_err());
}

#[test]
fn macros_and_environments_cannot_be_renamed_over_io_catcode_box_or_expression_primitives() {
    let mut workspace = Workspace::default();
    let source =
        r"\newcommand{\localmacro}{Value}\newenvironment{localenv}{}{}\csdef{namedmacro}{Value}";
    let uri = insert(&mut workspace, "main.tex", source, Some(1));
    for target in ["localmacro", "localenv", "namedmacro"] {
        let position = workspace.documents[&uri]
            .text
            .position(source.find(target).unwrap());
        for name in [
            "write",
            "read",
            "openin",
            "closein",
            "openout",
            "closeout",
            "immediate",
            "catcode",
            "lccode",
            "uccode",
            "mathcode",
            "delcode",
            "sfcode",
            "escapechar",
            "endlinechar",
            "newlinechar",
            "afterassignment",
            "aftergroup",
            "globaldefs",
            "shipout",
            "setbox",
            "lastbox",
            "everyjob",
            "everyeof",
            "output",
            "char",
            "mathchar",
            "hskip",
            "vskip",
            "kern",
            "mkern",
            "penalty",
            "unpenalty",
            "detokenize",
            "scantokens",
            "readline",
            "numexpr",
            "dimexpr",
            "glueexpr",
            "muexpr",
            "-",
            "/",
        ] {
            let error = features::rename(&workspace, &uri, position, name).unwrap_err();
            assert!(
                error.message.contains("already defined"),
                "{target} -> {name}: {error}"
            );
        }
    }
}

#[test]
fn independent_projects_rename_their_own_labels_citations_macros_and_environments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(format!("{name}/main.tex")),"\\documentclass{article}\n\\usepackage{local}\n\\begin{document}\n\\input{chapter}\n\\ref{sec:intro} \\cite{book} \\localmacro \\begin{panel}Text\\end{panel}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
        std::fs::write(
            root.join(format!("{name}/chapter.tex")),
            "\\label{sec:intro}\n",
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{name}/refs.bib")),
            "@book{book,title={Example}}\n",
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{name}/local.sty")),
            "\\newcommand{\\localmacro}{Hello}\n\\newenvironment{panel}{}{}\n",
        )
        .unwrap();
    }
    // Names in another paper must neither cause conflicts nor receive edits.
    std::fs::write(
        root.join("b/chapter.tex"),
        "\\label{sec:intro}\\label{sec:new}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("b/refs.bib"),
        "@book{book,title={B}}\n@book{newbook,title={Other}}\n",
    )
    .unwrap();
    std::fs::write(root.join("b/local.sty"),"\\newcommand{\\localmacro}{B}\\newcommand{\\renamedmacro}{B}\\newenvironment{panel}{}{}\\newenvironment{renamedpanel}{}{}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
    let source = &workspace.documents[&main].text;
    for (needle, offset, new_name) in [
        ("ref{sec:intro}", 5, "sec:new"),
        ("cite{book}", 6, "newbook"),
        ("\\localmacro", 2, "renamedmacro"),
        ("begin{panel}", 7, "renamedpanel"),
    ] {
        let position = source.position(source.source.find(needle).unwrap() + offset);
        assert!(features::prepare_rename(&workspace, &main, position)
            .unwrap()
            .is_some());
        let changes = features::rename(&workspace, &main, position, new_name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert!(changes.len() >= 2, "{needle}: {changes:?}");
        assert!(
            changes
                .keys()
                .all(|uri| uri.to_file_path().unwrap().starts_with(root.join("a"))),
            "{needle}: {changes:?}"
        );
        assert!(changes[&main]
            .iter()
            .all(|edit| edit.new_text.trim_start_matches('\\') == new_name));
    }
}

#[test]
fn shared_declarations_rename_every_dependent_project_and_check_all_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")),"\\documentclass{article}\n\\usepackage{common}\n\\begin{document}\n\\sharedmacro \\cite{sharedbook}\n\\bibliography{common}\n\\end{document}\n").unwrap();
    }
    std::fs::write(
        root.join("common.sty"),
        "\\newcommand{\\sharedmacro}{Hello}",
    )
    .unwrap();
    std::fs::write(root.join("common.bib"), "@book{sharedbook,title={Example}}").unwrap();
    std::fs::write(root.join("other.tex"),"\\documentclass{article}\\newcommand{\\sharedmacro}{Other}\\begin{document}\\sharedmacro\\cite{sharedbook}\\bibliography{other}\\end{document}").unwrap();
    std::fs::write(root.join("other.bib"), "@book{sharedbook,title={Other}}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let b = Url::from_file_path(root.join("b.tex")).unwrap();
    let shared = Url::from_file_path(root.join("common.sty")).unwrap();
    let position = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("\\sharedmacro")
            .unwrap()
            + 2,
    );
    let changes = features::rename(&workspace, &main, position, "renamedmacro")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        changes
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [main.clone(), b.clone(), shared.clone()].into()
    );
    let references = features::references(&workspace, &main, position, true);
    assert_eq!(
        references
            .iter()
            .map(|location| location.uri.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        [main.clone(), b.clone(), shared.clone()].into()
    );
    // Starting at the shared definition works even though its root is ambiguous.
    assert!(workspace.root_document(&shared).is_err());
    let definition = workspace.documents[&shared].text.position(
        workspace.documents[&shared]
            .text
            .source
            .find("\\sharedmacro")
            .unwrap()
            + 2,
    );
    assert_eq!(
        features::rename(&workspace, &shared, definition, "renamedmacro")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .len(),
        3
    );
    let citation = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("sharedbook")
            .unwrap()
            + 2,
    );
    let changes = features::rename(&workspace, &main, citation, "newbook")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        changes
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            main.clone(),
            b.clone(),
            Url::from_file_path(root.join("common.bib")).unwrap()
        ]
        .into()
    );
    let source = format!(
        "{}\n\\newcommand{{\\conflict}}{{Conflict}}",
        workspace.documents[&b].text.source
    );
    workspace
        .documents
        .insert(b.clone(), Document::new(&b, source, None));
    assert!(features::rename(&workspace, &main, position, "conflict").is_err());
    let source = format!(
        "{}\n\\newcommand{{\\sharedmacro}}{{Override}}",
        workspace.documents[&b].text.source
    );
    workspace
        .documents
        .insert(b.clone(), Document::new(&b, source, None));
    assert!(features::prepare_rename(&workspace, &main, position).is_err());
}

#[test]
fn shared_references_to_different_project_definitions_cannot_be_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("shared.tex"), "See \\ref{sec:intro}.\n").unwrap();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")),"\\documentclass{article}\n\\begin{document}\n\\label{sec:intro}\n\\input{shared}\n\\end{document}\n").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let position = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("sec:intro")
            .unwrap()
            + 2,
    );
    assert!(features::prepare_rename(&workspace, &main, position)
        .unwrap_err()
        .message
        .contains("shared occurrence"));
    assert!(features::rename(&workspace, &main, position, "sec:new").is_err());
    // A second consumer without any matching definition is equally unsafe.
    let b = Url::from_file_path(root.join("b.tex")).unwrap();
    let source = workspace.documents[&b]
        .text
        .source
        .replace("\\label{sec:intro}", "");
    workspace
        .documents
        .insert(b.clone(), Document::new(&b, source, None));
    assert!(features::rename(&workspace, &main, position, "sec:new").is_err());
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn both_projects_compile_after_renaming_a_shared_macro() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")),"\\documentclass{article}\n\\usepackage{common}\n\\begin{document}\\greet\\end{document}\n").unwrap();
    }
    std::fs::write(root.join("common.sty"), "\\newcommand{\\greet}{Hello}\n").unwrap();
    let other = "\\documentclass{article}\\newcommand{\\greet}{Other}\\begin{document}\\greet\\end{document}";
    std::fs::write(root.join("other.tex"), other).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let position = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("\\greet")
            .unwrap()
            + 2,
    );
    let changes = features::rename(&workspace, &main, position, "salute")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    for (uri, edits) in changes {
        let source = edits_result(&workspace.documents[&uri].text.source, &edits);
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(root.join("other.tex")).unwrap(),
        other
    );
    for name in ["a", "b"] {
        let outcome = latex_lsp::build::run(
            &root.join(format!("{name}.tex")),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(
            outcome.status, "success",
            "{}\n{}",
            outcome.message, outcome.output
        );
        assert!(outcome.pdf.unwrap().is_file());
    }
}
