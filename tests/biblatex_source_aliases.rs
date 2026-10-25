use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;
fn setup(body: &str, bib: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}\newrefsection[chosen]{body}\end{{document}}"
    );
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(dir.path().join("section.bib"), bib).unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let biburi = main.join("section.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, biburi)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert!(doc.text.source[start..].starts_with(name));
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
}
fn missing(ws: &Workspace, uri: &Url) -> Vec<String> {
    ws.diagnostics(uri)
        .into_iter()
        .filter(|d| d.code == Some(NumberOrString::String("undefined-citation".into())))
        .map(|d| d.data.unwrap()["key"].as_str().unwrap().into())
        .collect()
}
#[test]
fn disjoint_static_aliases_preserve_source_and_alias_navigation() {
    let (_dir, ws, main, bib) = setup(
        r"\defbibentryset{collected}{old}\cite{collected,old,parent,absent}",
        "@book{parent,title={Parent},ids={old}} @book{member} @book{child,crossref={old}}",
    );
    for (query, declaration) in [
        (
            key(&ws, &main, r"\cite{", "collected"),
            key(&ws, &main, r"\defbibentryset{", "collected"),
        ),
        (
            key(&ws, &main, r"\defbibentryset{collected}{", "old"),
            key(&ws, &bib, "ids={", "old"),
        ),
        (
            key(&ws, &bib, "crossref={", "old"),
            key(&ws, &bib, "ids={", "old"),
        ),
    ] {
        assert_eq!(
            features::definitions(&ws, &query.uri, query.range.start),
            vec![declaration]
        );
        assert!(features::hover(&ws, &query.uri, query.range.start).is_some());
    }
    assert_eq!(missing(&ws, &main), ["absent"]);
}
#[test]
fn disjoint_alias_and_source_renames_keep_their_independent_physical_edits() {
    let (_dir, ws, main, bib) = setup(
        r"\defbibentryset{collected}{old}\cite{collected,old}",
        "@book{parent,title={Parent},ids={old}} @book{child,crossref={old}}",
    );
    for (query, name, count) in [
        (
            key(&ws, &main, r"\defbibentryset{", "collected"),
            "newset",
            2,
        ),
        (key(&ws, &bib, "ids={", "old"), "newalias", 4),
    ] {
        assert!(features::prepare_rename(&ws, &query.uri, query.range.start)
            .unwrap()
            .is_some());
        let edits = features::rename(&ws, &query.uri, query.range.start, name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            edits.values().map(Vec::len).sum::<usize>(),
            count,
            "{edits:?}"
        );
    }
}
#[test]
fn disjoint_aliases_are_candidates_with_their_owner_metadata_and_types() {
    let (_dir,mut ws,main,bib)=setup(r"\defbibentryset{collected}{member}\cite{}", "@book{parent,title={Alias title},ids={old}} @book{member} @XDATA{data,publisher={Pub},ids={data-old}} @book{child,xdata={}}");
    let at = key(&ws, &main, r"\cite{", "").range.start;
    let items = features::completion(&ws, &main, at);
    for name in ["collected", "parent", "old", "member", "data-old"] {
        assert!(items.iter().any(|i| i.label == name), "{name}: {items:?}");
    }
    assert!(items
        .iter()
        .find(|i| i.label == "old")
        .unwrap()
        .detail
        .as_ref()
        .unwrap()
        .contains("Alias title"));
    let source = ws.documents[&main]
        .text
        .source
        .replace(r"\cite{}", r"\cite{collected}");
    ws.open(main.clone(), source, 6);
    let at = key(&ws, &bib, "xdata={", "").range.start;
    let items = features::completion(&ws, &bib, at);
    assert!(items.iter().any(|i| i.label == "data-old"), "{items:?}");
    assert!(!items
        .iter()
        .any(|i| matches!(i.label.as_str(), "old" | "parent" | "collected")));
    let source = ws.documents[&main]
        .text
        .source
        .replace(r"\cite{collected}", r"\cite{oldd}");
    ws.open(main.clone(), source, 6);
    assert_eq!(missing(&ws, &main), ["oldd"]);
}
#[test]
fn intersecting_alias_names_and_uncertain_ids_values_stay_protected() {
    for (set, bib) in [
        (
            "old",
            "@book{parent,ids={old}} @book{member} @book{child,crossref={old}}",
        ),
        ("collected", "@book{parent,ids=alias} @book{member}"),
        (
            "collected",
            "@book{parent,ids={old},ids={other}} @book{member}",
        ),
        (
            "collected",
            "@book{parent,ids={old}} @book{parent} @book{member}",
        ),
        (
            "collected",
            "@book{parent,ids={old}} @book{old} @book{member}",
        ),
    ] {
        let (_dir, ws, main, _) = setup(
            &format!(r"\defbibentryset{{{set}}}{{member}}\cite{{{set}}}"),
            bib,
        );
        let query = key(&ws, &main, r"\cite{", set);
        assert!(
            features::definitions(&ws, &main, query.range.start).is_empty(),
            "{set}: {bib}"
        );
        assert!(
            features::prepare_rename(&ws, &main, query.range.start).is_err(),
            "{set}: {bib}"
        );
    }
}
#[test]
fn unsaved_alias_owner_changes_keep_navigation_diagnostics_and_coupled_source_edits() {
    let (_dir, mut ws, main, bib) = setup(
        r"\defbibentryset{collected}{member}\cite{collected,absent}",
        "@book{parent,ids={old}} @book{member}",
    );
    let query = key(&ws, &main, r"\cite{", "collected");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start).len(),
        1
    );
    ws.open(
        bib.clone(),
        "@book{collected,ids={old}} @book{member}".into(),
        8,
    );
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![key(&ws, &main, r"\defbibentryset{", "collected")]
    );
    assert!(features::prepare_rename(&ws, &main, query.range.start)
        .unwrap()
        .is_some());
    assert_eq!(missing(&ws, &main), ["absent"]);
    ws.open(bib, "@book{parent,ids={old}} @book{member}".into(), 9);
    assert_eq!(
        features::definitions(&ws, &main, query.range.start).len(),
        1
    );
    assert_eq!(missing(&ws, &main), ["absent"]);
}
fn compile(dir: &std::path::Path) -> String {
    for (program, args) in [
        (
            "pdflatex",
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ],
        ),
        ("biber", vec!["main"]),
        (
            "pdflatex",
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ],
        ),
        (
            "pdflatex",
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ],
        ),
    ] {
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let log = std::fs::read_to_string(dir.join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
    let blg = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(!blg.contains("WARN") && !blg.contains("ERROR"), "{blg}");
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_disjoint_alias_members_and_renames_preserve_backend_primary_identities() {
    for field in ["crossref", "xref"] {
        for alias in [false, true] {
            let (dir,mut ws,main,bib)=setup(r"\defbibentryset{collected}{old}\cite{collected,old,child}\printbibliography",&format!("@book{{parent,title={{Raw parent}},ids={{old}}}} @book{{child,title={{Child}},{field}={{old}}}}"));
            let before = compile(dir.path());
            assert!(before.contains(r"\set{parent}"), "{before}");
            assert!(before.contains(r"\keyalias{old}{parent}"), "{before}");
            let (query, old, new) = if alias {
                (key(&ws, &bib, "ids={", "old"), "old", "newalias")
            } else {
                (
                    key(&ws, &main, r"\defbibentryset{", "collected"),
                    "collected",
                    "newset",
                )
            };
            let edits = features::rename(&ws, &query.uri, query.range.start, new)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            for (uri, mut edits) in edits {
                edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                let text = ws.documents[&uri]
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
                std::fs::write(uri.to_file_path().unwrap(), &text).unwrap();
                ws.open(uri, text, 6);
            }
            for extension in ["aux", "bcf", "bbl", "run.xml"] {
                let _ = std::fs::remove_file(dir.path().join(format!("main.{extension}")));
            }
            assert_eq!(compile(dir.path()), before.replace(old, new));
            assert!(missing(&ws, &main).is_empty());
        }
    }
}
#[test]
fn a_new_unsaved_consumer_with_an_intersecting_owner_blocks_shared_source_edits() {
    let (dir, mut ws, main, bib) = setup(
        r"\input{sets}\cite{collected,old}",
        "@book{parent,ids={old}} @book{member}",
    );
    let sets = main.join("sets.tex").unwrap();
    let declaration = r"\defbibentryset{collected}{old}";
    std::fs::write(dir.path().join("sets.tex"), declaration).unwrap();
    ws.open(sets.clone(), declaration.into(), 3);
    let query = key(&ws, &sets, r"\defbibentryset{", "collected");
    assert_eq!(
        features::definitions(&ws, &sets, query.range.start),
        vec![query.clone()]
    );
    assert!(features::rename(&ws, &sets, query.range.start, "newset")
        .unwrap()
        .is_some());
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\defbibentryset{parent}{member}\input{sets}\cite{collected}\end{document}".into(),8);
    assert!(features::definitions(&ws, &sets, query.range.start).is_empty());
    assert!(features::rename(&ws, &sets, query.range.start, "newset").is_err());
    ws.documents.remove(&other);
    assert_eq!(
        features::definitions(&ws, &sets, query.range.start),
        vec![query.clone()]
    );
    assert!(features::rename(&ws, &sets, query.range.start, "old").is_err());
    let alias = key(&ws, &bib, "ids={", "old");
    assert!(features::rename(&ws, &bib, alias.range.start, "collected").is_err());
}
#[test]
fn alias_renames_merge_quoted_lists_and_preserve_other_aliases_and_unicode() {
    let (_dir,mut ws,main,bib)=setup("% 😀\r\n\\csuse{defbibentryset}{collected}{old}\\cite{collected,old,legacy}","% 😀\r\n@book{parent,title={Title},IDS=\"old, legacy\"} @book{child,related=\"old,old,legacy\"}");
    let alias = key(&ws, &bib, "IDS=\"", "old");
    let edit = features::rename(&ws, &bib, alias.range.start, "O\"Connor")
        .unwrap()
        .unwrap();
    for (uri, mut edits) in edit.changes.unwrap() {
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let text = ws.documents[&uri]
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
        ws.open(uri, text, 9);
    }
    assert!(ws.documents[&bib]
        .text
        .source
        .contains("IDS={O\"Connor, legacy}"));
    assert!(ws.documents[&bib]
        .text
        .source
        .contains("related={O\"Connor,O\"Connor,legacy}"));
    let member = key(
        &ws,
        &main,
        r"\csuse{defbibentryset}{collected}{",
        "O\"Connor",
    );
    let alias = key(&ws, &bib, "IDS={", "O\"Connor");
    assert_eq!(
        features::definitions(&ws, &main, member.range.start),
        vec![alias]
    );
    assert!(missing(&ws, &main).is_empty());
}
#[test]
fn alias_spelling_repairs_share_the_certified_owner_and_member_candidate_context() {
    let (_dir, mut ws, main, _) = setup(
        r"\defbibentryset{collected}{oldd}\csuse{cite}{oldd}",
        "@book{parent,title={Parent},ids={old}}",
    );
    ws.code_action_literals = true;
    for diagnostic in ws
        .diagnostics(&main)
        .into_iter()
        .filter(|d| d.code == Some(NumberOrString::String("undefined-citation".into())))
    {
        let range = diagnostic.range;
        let actions = latex_lsp::editing::code_actions(
            &ws,
            &main,
            range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        );
        let repair = actions
            .into_iter()
            .find_map(|action| match action {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == "Replace with citation `old`" =>
                {
                    Some(action)
                }
                _ => None,
            })
            .expect("certified alias repair");
        let edit = repair.edit.unwrap().changes.unwrap();
        assert_eq!(
            edit[&main],
            vec![TextEdit {
                range,
                new_text: "old".into()
            }]
        );
    }
    assert_eq!(missing(&ws, &main), ["oldd", "oldd"]);
}
