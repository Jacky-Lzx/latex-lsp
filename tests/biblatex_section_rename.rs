use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}{body}\end{{document}}"
    );
    for (name, text) in [
        ("main.tex", source.as_str()),
        (
            "default.bib",
            "@book{parent,ids={legacy}}\n@book{child,crossref={parent},xref={legacy}}",
        ),
        (
            "section.bib",
            "@book{parent,ids={legacy}}\n@book{child,crossref={parent},xref={legacy}}",
        ),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}
fn at(ws: &Workspace, uri: &Url, marker: &str) -> Position {
    let doc = &ws.documents[uri];
    doc.text
        .position(doc.text.source.find(marker).unwrap() + marker.len())
}
fn edits(edit: WorkspaceEdit) -> std::collections::BTreeMap<Url, Vec<TextEdit>> {
    if let Some(changes) = edit.changes {
        return changes.into_iter().collect();
    }
    let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
        panic!("missing edits")
    };
    changes
        .into_iter()
        .map(|edit| {
            (
                edit.text_document.uri,
                edit.edits
                    .into_iter()
                    .map(|edit| match edit {
                        OneOf::Left(edit) => edit,
                        _ => panic!(),
                    })
                    .collect(),
            )
        })
        .collect()
}
fn apply(ws: &mut Workspace, edit: WorkspaceEdit) {
    for (uri, mut edits) in edits(edit) {
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let source = ws.documents[&uri]
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
        ws.open(uri, source, 20);
    }
}
#[test]
fn selected_primary_and_ids_renames_edit_only_their_complete_declaration_identity() {
    for (key, prefix, renamed) in [
        ("parent", "@book{", "selected"),
        ("legacy", "ids={", "selectedalias"),
    ] {
        let (_dir, mut ws, main) =
            setup(r"\cite{parent,legacy}\newrefsection[chosen]\cite{parent,legacy}");
        let bib = main.join("section.bib").unwrap();
        let default = main.join("default.bib").unwrap();
        let original = ws.documents[&default].text.source.clone();
        let pos = at(&ws, &bib, &format!("{prefix}{}", &key[..2]));
        let prepared = features::prepare_rename(&ws, &bib, pos).unwrap().unwrap();
        let PrepareRenameResponse::RangeWithPlaceholder { range, placeholder } = prepared else {
            panic!()
        };
        assert_eq!(placeholder, key);
        assert_eq!(
            &ws.documents[&bib].text.source[ws.documents[&bib].text.offset(range.start).unwrap()
                ..ws.documents[&bib].text.offset(range.end).unwrap()],
            key
        );
        let references = features::references(&ws, &bib, pos, true);
        assert_eq!(references.len(), 3);
        let edit = features::rename(&ws, &bib, pos, renamed).unwrap().unwrap();
        let changes = edits(edit.clone());
        assert_eq!(changes.values().map(Vec::len).sum::<usize>(), 3);
        assert!(!changes.contains_key(&default));
        apply(&mut ws, edit);
        assert_eq!(ws.documents[&default].text.source, original);
        let pos = at(&ws, &bib, &format!("{prefix}{}", &renamed[..2]));
        assert_eq!(features::references(&ws, &bib, pos, true).len(), 3);
        assert!(features::rename(&ws, &bib, pos, renamed)
            .unwrap()
            .unwrap()
            .changes
            .is_none());
    }
}
#[test]
fn source_and_relation_queries_prepare_exact_ranges_and_keep_versioned_edits() {
    let (_dir, mut ws, main) = setup(r"\cite{parent}\newrefsection[chosen]\cite{parent}");
    let bib = main.join("section.bib").unwrap();
    let source = ws.documents[&bib].text.source.clone();
    ws.open(bib.clone(), source, 7);
    ws.versioned_edits = true;
    let positions = [
        (&bib, at(&ws, &bib, "crossref={pa")),
        (
            &main,
            ws.documents[&main]
                .text
                .position(ws.documents[&main].text.source.rfind("parent").unwrap() + 2),
        ),
    ];
    for (uri, pos) in positions {
        assert!(features::prepare_rename(&ws, uri, pos).unwrap().is_some());
        let edit = features::rename(&ws, uri, pos, "updated").unwrap().unwrap();
        assert!(edit.changes.is_none());
        let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
            panic!()
        };
        assert_eq!(changes.len(), 2);
        for change in changes {
            assert_eq!(
                change.text_document.version,
                Some(if change.text_document.uri == main {
                    5
                } else {
                    7
                })
            );
        }
    }
}
#[test]
fn target_names_in_disjoint_resource_sections_are_allowed_but_coexisting_keys_and_aliases_are_not()
{
    let (_dir, mut ws, main) = setup(r"\cite{other}\newrefsection[chosen]\cite{parent}");
    let default = main.join("default.bib").unwrap();
    let bib = main.join("section.bib").unwrap();
    ws.open(default, "@book{other}".into(), 8);
    assert!(
        features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), "other")
            .unwrap()
            .is_some()
    );
    for target in ["legacy", "child"] {
        assert!(
            features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), target).is_err(),
            "{target}"
        );
    }
    for target in [
        "",
        "*",
        "two keys",
        "bad,comma",
        "bad%comment",
        "bad\\expansion",
    ] {
        assert!(
            features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), target).is_err(),
            "{target}"
        );
    }
}
#[test]
fn quoted_ids_and_repeated_relation_members_merge_into_valid_nonoverlapping_edits() {
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{legacy}");
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "% 😀\r\n@book{parent,ids=\"legacy,keep\"}\r\n@book{child,related=\"legacy, keep,legacy\"}"
            .into(),
        7,
    );
    let edit = features::rename(&ws, &bib, at(&ws, &bib, "ids=\"leg"), "new\"😀")
        .unwrap()
        .unwrap();
    assert_eq!(edits(edit.clone())[&bib].len(), 2);
    apply(&mut ws, edit);
    let source = &ws.documents[&bib].text.source;
    assert!(source.contains("ids={new\"😀,keep}"), "{source}");
    assert!(
        source.contains("related={new\"😀, keep,new\"😀}"),
        "{source}"
    );
    let reparsed = &ws.documents[&bib].syntax;
    assert!(
        reparsed.diagnostics.is_empty(),
        "{:?}",
        reparsed.diagnostics
    );
    assert_eq!(
        features::references(&ws, &bib, at(&ws, &bib, "ids={new"), true).len(),
        4
    );
}
#[test]
fn undefined_source_or_relation_targets_cannot_be_captured_by_a_rename() {
    for body in [
        r"\newrefsection[chosen]\cite{parent,missing}",
        r"\newrefsection[chosen]\newcommand\later{\cite{missing}}\cite{parent}",
    ] {
        let (_dir, ws, main) = setup(body);
        let bib = main.join("section.bib").unwrap();
        assert!(features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), "missing").is_err());
    }
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{parent}");
    let bib = main.join("section.bib").unwrap();
    let source = ws.documents[&bib].text.source.clone() + "\n@book{other,related={missing}}";
    ws.open(bib.clone(), source, 8);
    assert!(features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), "missing").is_err());
}

#[test]
fn disjoint_missing_keys_and_literal_examples_do_not_block_a_safe_rename() {
    let (_dir, ws, main) = setup(
        r"\cite{missing}\newrefsection[chosen]\verb|\cite{missing}|\detokenize{\cite{missing}}\cite{parent}",
    );
    let bib = main.join("section.bib").unwrap();
    assert!(
        features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), "missing")
            .unwrap()
            .is_some()
    );
}

#[test]
fn shared_occurrences_and_unknown_consumers_reject_prepare_and_rename_together() {
    let (dir, mut ws, main) = setup(r"\newrefsection[chosen]\input{chapter}");
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{parent}").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    let bib = main.join("section.bib").unwrap();
    let position = at(&ws, &bib, "@book{pa");
    assert!(features::prepare_rename(&ws, &bib, position)
        .unwrap()
        .is_some());
    let other = main.join("unsaved.tex").unwrap();
    for source in [
        r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\begin{document}\input{chapter}\end{document}",
        r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\ifunknown\newrefsection[section.bib]\fi\input{chapter}\end{document}",
    ] {
        ws.open(other.clone(), source.into(), 8);
        assert!(features::prepare_rename(&ws, &bib, position).is_err());
        assert!(features::rename(&ws, &bib, position, "updated").is_err());
    }
    ws.open(
        other,
        r"\documentclass{article}\begin{document}Independent\end{document}".into(),
        9,
    );
    assert!(features::rename(&ws, &bib, position, "updated")
        .unwrap()
        .is_some());
}

#[test]
fn new_names_are_checked_against_shared_consumers_and_ids_in_other_databases() {
    let (dir, mut ws, main) = setup(r"\newrefsection[chosen,extra.bib]\cite{parent}");
    let bib = main.join("section.bib").unwrap();
    let extra = main.join("extra.bib").unwrap();
    std::fs::write(dir.path().join("extra.bib"), "@book{other,ids={occupied}}").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    assert!(features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), "occupied").is_err());
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other,r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\cite{parent,newname}\end{document}".into(),8);
    assert!(features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), "newname").is_err());
    ws.open(extra, "@book{other}".into(), 9);
    assert!(
        features::rename(&ws, &bib, at(&ws, &bib, "@book{pa"), "occupied")
            .unwrap()
            .is_some()
    );
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_biber_accepts_selected_primary_and_ids_workspace_renames() {
    let (dir, mut ws, main) = setup(
        r"\cite{legacy}\nocite{parent,child,aliaschild,xrefchild,relatedchild}\printbibliography\newrefsection[chosen]\cite{legacy}\nocite{parent,child,aliaschild,xrefchild,relatedchild}\printbibliography",
    );
    for (name, title) in [
        ("default.bib", "Default parent"),
        ("section.bib", "Selected parent"),
    ] {
        let source = format!("@book{{parent,ids={{legacy}},title={{{title}}},author={{Parent, Pat}},year={{2001}}}}\n@inbook{{child,crossref={{parent}},title={{Child}}}}\n@inbook{{aliaschild,crossref={{legacy}},title={{Alias child}}}}\n@book{{xrefchild,xref={{parent}},title={{Xref child}},author={{Child, Chris}},year={{2002}}}}\n@book{{relatedchild,related={{parent}},title={{Related child}},author={{Child, Chris}},year={{2003}}}}");
        let bib = main.join(name).unwrap();
        std::fs::write(bib.to_file_path().unwrap(), &source).unwrap();
        ws.open(bib, source, 7);
    }
    let run = |program: &str, args: &[&str]| {
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let compile = || {
        for extension in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{extension}")));
        }
        let args = [
            "-no-shell-escape",
            "-interaction=nonstopmode",
            "-halt-on-error",
            "main.tex",
        ];
        run("pdflatex", &args);
        run("biber", &["main"]);
        run("pdflatex", &args);
        run("pdflatex", &args);
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        std::fs::read_to_string(dir.path().join("main.bbl")).unwrap()
    };
    compile();
    let selected = main.join("section.bib").unwrap();
    let default = main.join("default.bib").unwrap();
    let default_source = ws.documents[&default].text.source.clone();
    for (prefix, key, renamed, count) in [
        ("@book{", "parent", "selectedparent", 5),
        ("ids={", "legacy", "selectedlegacy", 3),
    ] {
        let position = at(&ws, &selected, &format!("{prefix}{}", &key[..2]));
        let refs = features::references(&ws, &selected, position, true);
        assert_eq!(refs.len(), count, "{refs:?}");
        assert!(refs
            .iter()
            .all(|location| location.uri == main || location.uri == selected));
        let edit = features::rename(&ws, &selected, position, renamed)
            .unwrap()
            .unwrap();
        apply(&mut ws, edit);
        for uri in [main.clone(), selected.clone()] {
            std::fs::write(uri.to_file_path().unwrap(), &ws.documents[&uri].text.source).unwrap();
        }
    }
    assert_eq!(ws.documents[&default].text.source, default_source);
    let bbl = compile();
    for (section, key, alias, title, other) in [
        (0, "parent", "legacy", "Default parent", "Selected parent"),
        (
            1,
            "selectedparent",
            "selectedlegacy",
            "Selected parent",
            "Default parent",
        ),
    ] {
        let body = bbl
            .split_once(&format!("\\refsection{{{section}}}"))
            .unwrap()
            .1
            .split_once("\\endrefsection")
            .unwrap()
            .0;
        assert!(body.contains(&format!("\\entry{{{key}}}")), "{body}");
        assert!(
            body.contains(&format!("\\keyalias{{{alias}}}{{{key}}}")),
            "{body}"
        );
        assert!(
            body.contains(&format!("\\field{{booktitle}}{{{title}}}")),
            "{body}"
        );
        assert!(!body.contains(other), "{body}");
    }
}

#[test]
fn configured_roots_do_not_allow_partial_edits_of_an_unloaded_database() {
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{parent}");
    let inactive = main.join("inactive.bib").unwrap();
    ws.open(
        inactive.clone(),
        "@book{orphan}\n@book{child,crossref={orphan}}".into(),
        8,
    );
    ws.config.build.root_file = Some("main.tex".into());
    let position = at(&ws, &inactive, "@book{or");
    assert!(features::prepare_rename(&ws, &inactive, position).is_err());
    assert!(features::rename(&ws, &inactive, position, "updated").is_err());
}
