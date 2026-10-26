use lsp_types::*;
use texquill::{
    compilation::build_log,
    editing,
    language::{features, semantic},
    project::workspace::{Document, Workspace},
    text::Text,
};

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///commented-bibitems/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    uri
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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

#[test]
fn commented_bibitem_declarations_join_alias_and_citation_renames_without_editing_prose() {
    for declaration in [
        "\\bibitem{% before alpha, }\nalpha% tail alpha\n} Entry alpha.",
        "\\bibitem[Author alpha(2025)]{% before\r\nalpha% tail\r\n} Entry alpha.",
        "\\bibitem{alpha% tail alpha\n} Entry alpha.",
        "\\bibitem{% before\n% another\nalpha% tail\n% another\n} Entry alpha.",
    ] {
        let mut workspace = Workspace::default();
        let bib = insert(&mut workspace, "refs.tex", declaration);
        let alias_source = "\\defcitealias{alpha}{The alpha alias}";
        let aliases = insert(&mut workspace, "aliases.tex", alias_source);
        let source =
            "\\citetalias{alpha}\\citep[alpha]{alpha}\\label{alpha}\\verb|\\bibitem{alpha}|";
        let main = insert(&mut workspace, "main.tex", source);
        let name_start = declaration
            .find("\nalpha")
            .map(|at| at + 1)
            .unwrap_or_else(|| declaration.find("{alpha").unwrap() + 1);
        let expected_range = Text::new(declaration.into()).range(name_start, name_start + 5);
        let at = Text::new(source.into()).position(source.find("{alpha}").unwrap() + 3);
        let definitions = features::definitions(&workspace, &main, at);
        assert_eq!(definitions.len(), 1, "{declaration}");
        assert_eq!(definitions[0].uri, bib);
        assert_eq!(definitions[0].range, expected_range);
        let declared_at = Text::new(declaration.into()).position(name_start + 2);
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &bib, declared_at).unwrap()
        else {
            panic!("missing target: {declaration}")
        };
        assert_eq!(range, expected_range);
        assert_eq!(placeholder, "alpha");
        assert!(features::hover(&workspace, &main, at).is_some());
        assert_eq!(
            features::references(&workspace, &bib, declared_at, true).len(),
            4
        );
        assert_eq!(
            features::references(&workspace, &bib, declared_at, false).len(),
            3
        );
        let changes = features::rename(&workspace, &bib, declared_at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 3);
        let mut expected = declaration.to_string();
        expected.replace_range(name_start..name_start + 5, "renamed");
        assert_eq!(apply(declaration, &changes[&bib]), expected);
        assert_eq!(
            apply(alias_source, &changes[&aliases]),
            "\\defcitealias{renamed}{The alpha alias}"
        );
        assert_eq!(
            apply(source, &changes[&main]),
            source
                .replace("\\citetalias{alpha}", "\\citetalias{renamed}")
                .replace("\\citep[alpha]{alpha}", "\\citep[alpha]{renamed}")
        );
    }
}

#[test]
fn non_static_bibitems_do_not_advertise_keys_or_borrow_another_declarations_rename_target() {
    let mut workspace = Workspace::default();
    let bib = insert(&mut workspace, "refs.bib", "@book{alpha,title={Title}}");
    let main = insert(&mut workspace, "main.tex", "\\cite{}");
    for marked in [
        "{ al¦pha }",
        "{% before\n al¦pha}",
        "{al¦pha % after\n}",
        "{% before\nal¦pha\n}",
        "{al¦% middle\npha}",
        "{\\cho¦sen}",
        "{{al¦pha}}",
        "{al¦pha,beta}",
        "{¦}",
    ] {
        let offset = "\\bibitem".len() + marked.find('¦').unwrap();
        let source = format!("\\bibitem{} Entry alpha.", marked.replace('¦', ""));
        let invalid = insert(&mut workspace, "invalid.tex", &source);
        let at = Text::new(source.clone()).position(offset);
        assert!(
            features::definitions(&workspace, &invalid, at).is_empty(),
            "{source}"
        );
        assert!(
            features::prepare_rename(&workspace, &invalid, at)
                .unwrap()
                .is_none(),
            "{source}"
        );
        let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert!(!changes.contains_key(&invalid), "{source}: {changes:?}");
        let labels: Vec<_> = features::completion(&workspace, &main, Position::new(0, 6))
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert_eq!(labels, ["alpha"], "{source}");
    }
}

#[test]
fn bibitem_optional_labels_use_tex_boundaries_without_swallowing_the_real_key() {
    for marked in [
        "\\bibitem[Note [literal]{al¦pha} Entry alpha.",
        "\\bibitem[Note {literal ] [ text}]{al¦pha} Entry alpha.",
        "\\bibitem% before\n[Note [literal]% after\n{% key\nal¦pha% tail\n} Entry alpha.",
    ] {
        let mut workspace = Workspace::default();
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let bib = insert(&mut workspace, "refs.tex", &source);
        let references = "See \\cite{alpha}.";
        let main = insert(&mut workspace, "main.tex", references);
        let at = Text::new(references.into()).position(references.find("alpha").unwrap() + 2);
        let definitions = features::definitions(&workspace, &main, at);
        assert_eq!(definitions.len(), 1, "{marked}");
        assert_eq!(definitions[0].uri, bib);
        assert_eq!(
            definitions[0].range,
            Text::new(source.clone()).range(offset - 2, offset + 3)
        );
        let changes = features::rename(&workspace, &main, at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        let mut expected = source.clone();
        expected.replace_range(offset - 2..offset + 3, "renamed");
        assert_eq!(apply(&source, &changes[&bib]), expected);
        assert_eq!(apply(references, &changes[&main]), "See \\cite{renamed}.");
    }
}

#[test]
fn bibitem_extra_labels_stars_and_unfinished_labels_do_not_turn_prose_into_keys() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "refs.bib", "@book{alpha,title={Title}}");
    let main = insert(&mut workspace, "main.tex", "\\cite{}");
    for marked in [
        "\\bibitem[Note [nested]]{phan¦tom} Entry alpha.",
        "\\bibitem[one][two]{phan¦tom} Entry alpha.",
        "\\bibitem*{phan¦tom} Entry alpha.",
        "\\bibitem *{phan¦tom} Entry alpha.",
        "\\bibitem[unfinished {phan¦tom} Entry alpha.",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let invalid = insert(&mut workspace, "invalid.tex", &source);
        let at = Text::new(source).position(offset);
        assert!(
            features::definitions(&workspace, &invalid, at).is_empty(),
            "{marked}"
        );
        assert!(
            features::prepare_rename(&workspace, &invalid, at)
                .unwrap()
                .is_none(),
            "{marked}"
        );
        assert!(
            features::rename(&workspace, &invalid, at, "renamed")
                .unwrap()
                .is_none(),
            "{marked}"
        );
        let labels: Vec<_> = features::completion(&workspace, &main, Position::new(0, 6))
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert_eq!(labels, ["alpha"], "{marked}");
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_bibitem_note_boundaries_keep_prose_and_actual_auxiliary_keys_intact() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    let main_source = "\\documentclass{article}\n\\begin{document}\n\\input{child}\n\\begin{thebibliography}{9}\n\\bibitem[Note [literal]{alpha} Entry alpha.\n\\bibitem[Note [nested]]{unsafe} Body UNSAFE.\n\\bibitem[one][two]{excess} Body EXCESS.\n\\end{thebibliography}\n\\end{document}\n";
    let child_source = "See \\cite{alpha}.\n";
    let mut workspace = Workspace::default();
    for (path, source) in [(&main, main_source), (&child, child_source)] {
        std::fs::write(path, source).unwrap();
        workspace.open(Url::from_file_path(path).unwrap(), source.into(), 7);
    }
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let result = texquill::compilation::build::run(&main, &config, &Default::default());
        assert_eq!(result.status, "success", "{}", result.output);
        result
    };
    compile();
    assert!(compile().diagnostics.values().all(Vec::is_empty));
    let changes = features::rename(
        &workspace,
        &Url::from_file_path(&child).unwrap(),
        Position::new(0, 12),
        "renamed",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(changes.len(), 2);
    for (uri, edits) in changes {
        let source = &workspace.documents[&uri].text.source;
        std::fs::write(uri.to_file_path().unwrap(), apply(source, &edits)).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&main).unwrap(),
        main_source.replace("[Note [literal]{alpha}", "[Note [literal]{renamed}")
    );
    assert_eq!(
        std::fs::read_to_string(&child).unwrap(),
        child_source.replace("{alpha}", "{renamed}")
    );
    compile();
    let result = compile();
    assert!(result.diagnostics.values().all(Vec::is_empty));
    let aux = std::fs::read_to_string(result.pdf.unwrap().with_extension("aux")).unwrap();
    for key in ["renamed", "]", "["] {
        assert!(aux.contains(&format!("\\bibcite{{{key}}}")), "{aux}");
    }
    for key in ["alpha", "unsafe", "excess"] {
        assert!(!aux.contains(&format!("\\bibcite{{{key}}}")), "{aux}");
    }
}

#[test]
fn commented_bibitems_share_duplicate_checks_with_plain_declarations_and_skip_spaced_keys() {
    let source = "\\bibitem{% before\nalpha% tail\n} First.\n\\bibitem{% again\nalpha% tail\n} Second.\n\\bibitem{ alpha } Spaced.\n";
    let mut workspace = Workspace::default();
    let main = insert(&mut workspace, "main.tex", source);
    let diagnostics = features::diagnostics(&workspace.documents[&main]);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("Duplicate key `alpha`"));
    assert_eq!(
        diagnostics[0].range,
        Range::new(Position::new(4, 0), Position::new(4, 5))
    );
}

#[test]
fn commented_bibitem_semantic_tokens_mark_only_the_utf16_declaration_name() {
    let source = "😀 \\bibitem[作者]{% 键😀\r\n键😀% tail\r\n} Title 键😀.\r\n";
    let mut workspace = Workspace::default();
    let main = insert(&mut workspace, "main.tex", source);
    let doc = &workspace.documents[&main];
    let legend = semantic::options().legend;
    let declaration = 1
        << legend
            .token_modifiers
            .iter()
            .position(|item| item.as_str() == "declaration")
            .unwrap();
    let mut position = Position::default();
    let mut keys = Vec::new();
    for token in semantic::tokens(doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        if legend.token_types[token.token_type as usize].as_str() == "enumMember" {
            keys.push((position, token.length, token.token_modifiers_bitset));
        }
    }
    assert_eq!(keys, [(Position::new(1, 0), 3, declaration)]);
}

#[test]
fn commented_bibitems_supply_static_candidates_for_precise_missing_citation_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source =
        "\\bibitem{% before\nalpha% tail\n} Entry alpha.\n\\citep{% before\nalphaa% tail\n}\n";
    std::fs::write(&main, source).unwrap();
    let log = format!(
        "{}:6: Package natbib Warning: Citation `alphaa' on page 1 undefined on input line 6.\n",
        main.display()
    );
    let diagnostics = build_log::parse(&main, &log);
    let diagnostic = &diagnostics[&uri][0];
    assert_eq!(
        diagnostic.range,
        Range::new(Position::new(4, 0), Position::new(4, 6))
    );
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Workspace::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
    workspace
        .project
        .build_diagnostics
        .insert(uri.clone(), diagnostics.clone());
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: None,
            trigger_kind: None,
        },
    );
    let edit = actions
        .iter()
        .find_map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => action.edit.as_ref()?.changes.as_ref()?
                [&uri]
                .iter()
                .find(|edit| edit.new_text == "alpha"),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{actions:?}"));
    assert_eq!(edit.range, diagnostic.range);
    assert_eq!(
        apply(source, std::slice::from_ref(edit)),
        source.replace("\nalphaa%", "\nalpha%")
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn real_commented_bibitem_rename_updates_aliases_and_keeps_same_named_labels_independent() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let aliases = directory.path().join("aliases.tex");
    let child = directory.path().join("child.tex");
    let main_source = "\\documentclass{article}\n\\usepackage{natbib}\n\\input{aliases}\n\\begin{document}\n\\section{Intro}\\label{alpha}\n\\input{child}\n\\begin{thebibliography}{1}\n\\bibitem[Author(2025)]{% before alpha\nalpha% tail alpha\n} Author, Title alpha, 2025.\n\\end{thebibliography}\n\\end{document}\n";
    let alias_source = "\\defcitealias{alpha}{The ALIAS alpha}\n";
    let child_source = "See \\citetalias{alpha} and \\citep{alpha}, section \\ref{alpha}.\n";
    let mut workspace = Workspace::default();
    for (path, source) in [
        (&main, main_source),
        (&aliases, alias_source),
        (&child, child_source),
    ] {
        std::fs::write(path, source).unwrap();
        workspace.open(Url::from_file_path(path).unwrap(), source.into(), 7);
    }
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let result = texquill::compilation::build::run(&main, &config, &Default::default());
        assert_eq!(result.status, "success", "{}", result.output);
        result
    };
    compile();
    assert!(compile().diagnostics.values().all(Vec::is_empty));
    let main_uri = Url::from_file_path(&main).unwrap();
    let at = Text::new(main_source.into()).position(main_source.find("\nalpha%").unwrap() + 3);
    let changes = features::rename(&workspace, &main_uri, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    for (uri, edits) in changes {
        let source = &workspace.documents[&uri].text.source;
        std::fs::write(uri.to_file_path().unwrap(), apply(source, &edits)).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&main).unwrap(),
        main_source.replace("\nalpha%", "\nrenamed%")
    );
    assert_eq!(
        std::fs::read_to_string(&aliases).unwrap(),
        alias_source.replace("{alpha}", "{renamed}")
    );
    assert_eq!(
        std::fs::read_to_string(&child).unwrap(),
        child_source
            .replace("\\citetalias{alpha}", "\\citetalias{renamed}")
            .replace("\\citep{alpha}", "\\citep{renamed}")
    );
    compile();
    let result = compile();
    assert!(result.diagnostics.values().all(Vec::is_empty));
    let aux = std::fs::read_to_string(result.pdf.unwrap().with_extension("aux")).unwrap();
    assert!(aux.contains("\\bibcite{renamed}"));
    assert!(!aux.contains("\\bibcite{alpha}"));
    assert!(aux.contains("\\newlabel{alpha}"));
}
