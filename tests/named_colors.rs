use latex_lsp::{
    build, editing, features, semantic, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str, version: Option<i32>) -> Url {
    let uri = Url::parse(&format!("file:///named-colors/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), version));
    uri
}
fn setup(source: &str) -> (Workspace, Url) {
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    let uri = insert(&mut workspace, "main.tex", source, Some(7));
    (workspace, uri)
}
fn position(workspace: &Workspace, uri: &Url, marker: &str) -> Position {
    let doc = &workspace.documents[uri];
    doc.text.position(doc.text.source.find(marker).unwrap())
}
fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(|e| TextDocumentContentChangeEvent {
                    range: Some(e.range),
                    range_length: None,
                    text: e.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}
fn changes(workspace: &Workspace, edit: WorkspaceEdit) -> BTreeMap<Url, Vec<TextEdit>> {
    match edit.document_changes {
        Some(DocumentChanges::Edits(edits)) => edits
            .into_iter()
            .map(|e| {
                assert_eq!(
                    e.text_document.version,
                    workspace.documents[&e.text_document.uri].version
                );
                (
                    e.text_document.uri,
                    e.edits
                        .into_iter()
                        .map(|e| match e {
                            OneOf::Left(e) => e,
                            _ => panic!(),
                        })
                        .collect(),
                )
            })
            .collect(),
        _ => edit.changes.unwrap().into_iter().collect(),
    }
}

#[test]
fn cross_file_color_navigation_and_versioned_rename_keep_mixing_operators_and_namespaces() {
    let declarations = "\\definecolor[named]% type\r\n{accent}[prefix]{HTML}{123456}\r\n\\newcommand{\\accent}{Macro}\\label{accent}";
    let body = concat!("😀 \\textcolor{--accent!35!-red!50!accent}{accent}\\ref{accent}\\accent\r\n",
        "% \\color{accent}\r\n\\verb|\\color{accent}|\\detokenize{\\color{accent}}\\string\\color{accent}");
    let (mut workspace, uri) = setup(body);
    let definitions = insert(&mut workspace, "palette.sty", declarations, Some(3));
    let at = position(&workspace, &uri, "accent!35");
    let found = features::definitions(&workspace, &uri, at);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].uri, definitions);
    assert_eq!(features::references(&workspace, &uri, at, true).len(), 3);
    assert_eq!(features::references(&workspace, &uri, at, false).len(), 2);
    assert_eq!(editing::highlights(&workspace, &uri, at).len(), 2);
    assert!(features::hover(&workspace, &uri, at).is_some());
    assert!(
        matches!(features::prepare_rename(&workspace, &uri, at).unwrap(), Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) if placeholder == "accent")
    );
    for (from, at) in [
        (uri.clone(), at),
        (
            definitions.clone(),
            position(&workspace, &definitions, "accent}"),
        ),
    ] {
        let edits = changes(
            &workspace,
            features::rename(&workspace, &from, at, "theme-2.dark")
                .unwrap()
                .unwrap(),
        );
        assert_eq!(edits.len(), 2);
        assert_eq!(
            apply(declarations, &edits[&definitions]),
            declarations.replacen("{accent}", "{theme-2.dark}", 1)
        );
        assert_eq!(
            apply(body, &edits[&uri]),
            body.replacen(
                "--accent!35!-red!50!accent",
                "--theme-2.dark!35!-red!50!theme-2.dark",
                1
            )
        );
    }
    assert!(features::symbols(&workspace.documents[&definitions])
        .iter()
        .any(|s| s.name == "accent" && s.kind == SymbolKind::CONSTANT));
}

#[test]
fn aliases_have_separate_identity_and_named_models_record_source_references() {
    for declaration in [r"\definecolor", r"\providecolor", r"\xdefinecolor"] {
        let source = format!("{declaration}{{accent}}{{rgb}}{{.2,.4,.6}}\\colorlet[named]{{alias}}[cmyk]{{accent!30!white}}\\providecolor{{second}}{{named}}{{alias}}\\textcolor[named]{{second}}{{Text}}");
        let (workspace, uri) = setup(&source);
        for (name, count) in [("accent", 2), ("alias", 2), ("second", 2)] {
            let at = position(&workspace, &uri, &format!("{{{name}}}"));
            // Move inside the group, past the opening delimiter.
            let at = Position::new(at.line, at.character + 1);
            assert_eq!(
                features::references(&workspace, &uri, at, true).len(),
                count,
                "{source}"
            );
            let edits = changes(
                &workspace,
                features::rename(&workspace, &uri, at, "renamed")
                    .unwrap()
                    .unwrap(),
            );
            let changed = apply(&source, &edits[&uri]);
            assert_eq!(changed.matches("renamed").count(), count);
            assert!(!changed.contains(name));
        }
    }
}

#[test]
fn independent_models_mask_blend_and_conversion_only_index_actual_named_operands() {
    let source = concat!(
        r"\definecolor{accent}{HTML}{123456}",
        r"\fcolorbox{accent}[rgb]{.2,.4,.6}{A}",
        r"\fcolorbox[rgb]{.2,.4,.6}[named]{accent!50!white}{B}",
        r"\fcolorbox[rgb]{.2,.4,.6}{.3,.5,.7}{C}",
        r"\maskcolors[rgb]{accent}\maskcolors{}",
        r"\blendcolors{!30!accent!50!white}\blendcolors*{!20!accent}\blendcolors{}",
        r"\extractcolorspec{accent}\out\extractcolorspecs{accent}\model\spec",
        r"\convertcolorspec{named}{accent}{rgb}\out",
        r"\color[rgb]{accent}\textcolor[HTML]{ABCDEF}{accent}",
        r"\rowcolor{accent}\columncolor{accent}\cellcolor{accent}",
        r"\arrayrulecolor{accent}\doublerulesepcolor{accent}"
    );
    let (workspace, uri) = setup(source);
    let doc = &workspace.documents[&uri];
    assert_eq!(
        doc.syntax
            .color_references
            .iter()
            .filter(|e| e.name == "accent")
            .count(),
        13
    );
    assert_eq!(doc.syntax.colors.len(), 1);
    let at = position(&workspace, &uri, "accent}");
    let edits = changes(
        &workspace,
        features::rename(&workspace, &uri, at, "newaccent")
            .unwrap()
            .unwrap(),
    );
    let changed = apply(source, &edits[&uri]);
    assert!(changed.contains(r"\color[rgb]{accent}\textcolor[HTML]{ABCDEF}{accent}"));
    assert!(changed.contains(r"\maskcolors[rgb]{newaccent}"));
    assert!(changed.contains(r"\blendcolors{!30!newaccent!50!white}"));
}

#[test]
fn completion_replaces_only_the_name_under_the_cursor_and_offers_scoped_declarations() {
    for (marked, expected) in [
        (
            r"\textcolor{-ac¦cent!35!red}{Text}",
            r"\textcolor{-accent!35!red}{Text}",
        ),
        (
            r"\textcolor{red!35!-ac¦cent}{Text}",
            r"\textcolor{red!35!-accent}{Text}",
        ),
        (r"\color{ac¦", r"\color{accent"),
        (r"\colorlet{alias}{ac¦}", r"\colorlet{alias}{accent}"),
        (r"\maskcolors[rgb]{ac¦}", r"\maskcolors[rgb]{accent}"),
        (r"\blendcolors{!30!ac¦}", r"\blendcolors{!30!accent}"),
        (
            r"\blendcolors{}\textcolor{ac¦}{T}",
            r"\blendcolors{}\textcolor{accent}{T}",
        ),
        (
            "\\textcolor % note\r\n[named]% next\r\n{ac¦}{T}",
            "\\textcolor % note\r\n[named]% next\r\n{accent}{T}",
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (mut workspace, uri) = setup(&source);
        insert(
            &mut workspace,
            "palette.tex",
            r"\definecolor{accent}{HTML}{123456}",
            None,
        );
        let at = workspace.documents[&uri].text.position(offset);
        let items = features::completion(&workspace, &uri, at);
        let item = items
            .iter()
            .find(|item| item.label == "accent")
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        assert_eq!(item.kind, Some(CompletionItemKind::COLOR));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(apply(&source, std::slice::from_ref(edit)), expected);
    }
    for marked in [
        r"\textcolor{red!3¦5!accent}{Text}",
        r"\definecolor{ac¦}{rgb}{1,0,0}",
        r"\color{ ac¦}",
        r"\color[rgb]{ac¦}",
        r"\textcolor[[rgb]{ac¦}{T}",
        r"\textcolor[{]}]{ac¦}{T}",
        r"\blendcolors{!3¦0!accent}",
    ] {
        let offset = marked.find('¦').unwrap();
        let (mut workspace, uri) = setup(&marked.replace('¦', ""));
        insert(
            &mut workspace,
            "palette.tex",
            r"\definecolor{accent}{HTML}{123456}",
            None,
        );
        let items = features::completion(
            &workspace,
            &uri,
            workspace.documents[&uri].text.position(offset),
        );
        assert!(!items.iter().any(|i| i.label == "accent"), "{marked}");
    }
}

#[test]
fn color_rename_rejects_dynamic_names_customization_and_unparsed_consumers() {
    for extra in [
        r"\textcolor{\dynamic}{Text}",
        r"\color{ accent }",
        r"\color{accent!120!red}",
        r"\color{accent!!red}",
        r"\color{rgb:accent,1;red,2}",
        r"\color{accent>twheel,30}",
        r"\color{accent!50!}",
        r"\color*{accent}",
        r"\color[\model]{accent}",
        r"\color[]{accent}",
        r"\newcommand{\colored}[1]{\textcolor{#1}{Text}}",
        r"\renewcommand{\color}[1]{}",
        r"\csuse{color}{accent}",
        r"\csname textcolor\endcsname{accent}{Text}",
        r"\csdef{color}{custom}",
        r"\def\XC@definecolor{custom}",
        r"\csuse{XC@color}{accent}",
        r"\definecolors{alias=accent}",
        r"\definecolorset{rgb}{}{}{accent,1,0,0}",
        r"\definecolorseries{series}{rgb}{last}{accent}{red}",
        r"\hypersetup{linkcolor=accent}",
        r"\csuse{hypersetup}{linkcolor=accent}",
        r"\csname definecolors\endcsname{alias=accent}",
        r"\usepackage[\dynamic]{hyperref}",
        r"\usepackage[linkcolor=accent]{hyperref}",
        r"\PassOptionsToPackage{citecolor=accent}{hyperref}",
        r"\PassOptionsToClass{color=accent}{custom}",
        r"\documentclass[color=accent]{custom}",
        r"\tikz[draw=accent] \draw (0,0)--(1,1);",
        r"\begin{tikzpicture}[accent]\end{tikzpicture}",
        r"\tikzset{my style/.style={accent}}",
        r"\tcbset{colback=accent}",
        r"\colornameprefix{custom}",
        r"\maskcolors{\dynamic}",
        r"\blendcolors{!30!\dynamic}",
    ] {
        let source = format!("\\definecolor{{accent}}{{HTML}}{{123456}}\\color{{accent}}{extra}");
        let (workspace, uri) = setup(&source);
        let at = position(&workspace, &uri, "accent}");
        assert!(
            features::rename(&workspace, &uri, at, "renamed").is_err(),
            "{extra}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, at).is_err(),
            "{extra}"
        );
    }
}

#[test]
fn color_rename_rejects_ambiguous_definitions_capture_and_invalid_new_names() {
    for extra in [
        r"\definecolor{accent}{HTML}{FFFFFF}",
        r"\providecolor{accent}{HTML}{FFFFFF}",
        r"\colorlet{accent}{red}",
    ] {
        let (workspace, uri) = setup(&format!("\\definecolor{{accent}}{{HTML}}{{123456}}{extra}"));
        assert!(features::rename(
            &workspace,
            &uri,
            position(&workspace, &uri, "accent}"),
            "renamed"
        )
        .is_err());
    }
    let (workspace, uri) = setup(
        r"\definecolor{accent}{HTML}{123456}\definecolor{taken}{HTML}{FFFFFF}\color{external}",
    );
    let at = position(&workspace, &uri, "accent}");
    for name in [
        "red",
        "taken",
        "external",
        "",
        ".",
        "-new",
        "new!50!red",
        "new name",
        "颜色",
        "new%name",
        "new\\name",
    ] {
        assert!(
            features::rename(&workspace, &uri, at, name).is_err(),
            "{name}"
        );
    }
    assert_eq!(
        features::rename(&workspace, &uri, at, "accent")
            .unwrap()
            .unwrap(),
        WorkspaceEdit::default()
    );
    let (workspace, uri) = setup(r"\color{red}\hypersetup{linkcolor=red}");
    assert!(
        features::prepare_rename(&workspace, &uri, position(&workspace, &uri, "red}"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn semantic_color_tokens_preserve_utf16_ranges_and_declaration_flags() {
    let source = "😀 \\definecolor{accent}{HTML}{123456}\r\n\\textcolor{-accent!50!red}{accent}";
    let (workspace, uri) = setup(source);
    let doc = &workspace.documents[&uri];
    let mut at = Position::default();
    let tokens: Vec<_> = semantic::tokens(doc, None)
        .unwrap()
        .data
        .into_iter()
        .map(|t| {
            at = Position::new(
                at.line + t.delta_line,
                if t.delta_line == 0 {
                    at.character + t.delta_start
                } else {
                    t.delta_start
                },
            );
            let end = Position::new(at.line, at.character + t.length);
            (
                at,
                &source[doc.text.offset(at).unwrap()..doc.text.offset(end).unwrap()],
                t.token_type,
                t.token_modifiers_bitset,
            )
        })
        .collect();
    for (marker, flags) in [("accent}{HTML", 1), ("accent!50", 0), ("red}", 0)] {
        let expected = doc.text.position(source.find(marker).unwrap());
        assert!(tokens
            .iter()
            .any(|(at, _, kind, bits)| *at == expected && *kind == 1 && *bits == flags));
    }
    assert!(!tokens
        .iter()
        .any(|(at, _, kind, _)| *kind == 1
            && *at == doc.text.position(source.rfind("accent").unwrap())));
}

#[test]
fn shared_palette_renames_reach_all_consumers_and_keep_other_projects_separate() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let paper =
        r"\documentclass{article}\usepackage{common}\begin{document}\color{accent}\end{document}";
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")), paper).unwrap();
    }
    std::fs::write(
        root.join("common.sty"),
        r"\definecolor{accent}{HTML}{123456}",
    )
    .unwrap();
    std::fs::write(
        root.join("other.tex"),
        r"\documentclass{article}\definecolor{private}{HTML}{FFFFFF}\color{accent}",
    )
    .unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    let at = position(&workspace, &uri, "accent}");
    let edits = changes(
        &workspace,
        features::rename(&workspace, &uri, at, "renamed")
            .unwrap()
            .unwrap(),
    );
    assert_eq!(edits.len(), 3);
    assert!(edits.keys().all(|u| !u.path().ends_with("other.tex")));
    let marked = paper.replace("accent", "pri");
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, marked.clone(), None));
    let items = features::completion(
        &workspace,
        &uri,
        workspace.documents[&uri]
            .text
            .position(marked.find("pri}").unwrap() + 3),
    );
    assert!(!items.iter().any(|i| i.label == "private"));
    std::fs::write(root.join("shared.tex"), r"\color{accent}").unwrap();
    for name in ["a", "b"] {
        std::fs::write(
            root.join(format!("{name}.tex")),
            r"\documentclass{article}\definecolor{accent}{HTML}{123456}\input{shared}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(features::rename(
        &workspace,
        &uri,
        position(&workspace, &uri, "accent}"),
        "renamed"
    )
    .is_err());
}

#[test]
fn declaration_snippets_and_signatures_preserve_existing_parameters() {
    for name in ["providecolor", "xdefinecolor", "colorlet"] {
        let marked = if name == "colorlet" {
            format!("\\{name}[named]{{accent}}[cmyk]{{re¦d}}")
        } else {
            format!("\\{name}[named]{{accent}}[prefix]{{rgb}}{{.2,¦.4,.6}}")
        };
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (workspace, uri) = setup(&source);
        let help = signature::help(
            &workspace,
            &uri,
            workspace.documents[&uri].text.position(offset),
        )
        .unwrap();
        assert_eq!(
            help.active_parameter,
            Some(if name == "colorlet" { 3 } else { 4 })
        );
        let short = format!("\\{}", &name[..3]);
        let (mut workspace, uri) = setup(&short);
        workspace.snippets = true;
        let items = features::completion(
            &workspace,
            &uri,
            workspace.documents[&uri].text.position(short.len()),
        );
        assert!(items.iter().any(|i| i.label == name
            && matches!(&i.text_edit, Some(CompletionTextEdit::Edit(edit)) if edit.new_text.contains("${1:name}"))));
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn real_color_renames_preserve_expanded_rgb_specs_and_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for declaration in ["definecolor", "providecolor", "xdefinecolor"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let main = root.join("main.tex");
            let declarations = format!("\\{declaration}[named]{{accent}}[prefix]{{rgb}}{{.2,.4,.6}}\n\\colorlet{{alias}}[cmyk]{{accent!30!white}}\n\\providecolor{{second}}{{named}}{{accent}}\n");
            let body = concat!(
                r"\textcolor{-accent!35!-red!50!white}{MarkerUnique}",
                r"\fcolorbox{accent}[rgb]{.2,.4,.6}{FrameUnique}",
                r"\maskcolors[rgb]{accent}\textcolor{alias}{MaskUnique}\maskcolors{}",
                r"\blendcolors{!30!accent}\textcolor{alias}{BlendUnique}\blendcolors{}",
                r"\extractcolorspecs{accent}\model\spec\typeout{COLOR-AUDIT:\model:\spec}",
                r"\convertcolorspec{named}{accent}{rgb}\out\typeout{CONVERT-AUDIT:\out}",
                "\n% \\color{accent}\n\\verb|\\color{accent}|\n"
            );
            std::fs::write(&main, "\\documentclass{article}\\usepackage{xcolor}\\input{palette}\\begin{document}\\input{content}\\end{document}").unwrap();
            std::fs::write(root.join("palette.tex"), &declarations).unwrap();
            std::fs::write(root.join("content.tex"), body).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let before = build::run(&main, &config, &Default::default());
            assert_eq!(
                before.status, "success",
                "{engine}/{declaration}: {}",
                before.output
            );
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let uri = Url::from_file_path(root.join("content.tex")).unwrap();
            let edits = changes(
                &workspace,
                features::rename(
                    &workspace,
                    &uri,
                    position(&workspace, &uri, "accent!35"),
                    "theme-2.dark",
                )
                .unwrap()
                .unwrap(),
            );
            assert_eq!(edits.len(), 2);
            for (uri, edits) in edits {
                std::fs::write(
                    uri.to_file_path().unwrap(),
                    apply(&workspace.documents[&uri].text.source, &edits),
                )
                .unwrap();
            }
            let after = build::run(&main, &config, &Default::default());
            assert_eq!(
                after.status, "success",
                "{engine}/{declaration}: {}",
                after.output
            );
            let audit = |output: &str| {
                output
                    .lines()
                    .filter(|line| {
                        line.starts_with("COLOR-AUDIT:") || line.starts_with("CONVERT-AUDIT:")
                    })
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(audit(&before.output), audit(&after.output));
            assert_eq!(audit(&after.output).len(), 2);
            assert!(root.join("build/latex-lsp/main.pdf").is_file());
        }
    }
}
