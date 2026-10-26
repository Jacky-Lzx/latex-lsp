use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;

#[path = "support/completion.rs"]
mod completion;
fn project(extra: &str, body: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{refs.bib}}{extra}\begin{{document}}{body}\end{{document}}"
    );
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}
fn items(ws: &Workspace, uri: &Url, marker: &str) -> Vec<CompletionItem> {
    let doc = &ws.documents[uri];
    features::completion(
        ws,
        uri,
        doc.text
            .position(doc.text.source.find(marker).unwrap() + marker.len()),
    )
}
fn item(ws: &Workspace, uri: &Url, marker: &str, key: &str) -> CompletionItem {
    items(ws, uri, marker)
        .into_iter()
        .find(|item| item.label == key)
        .unwrap_or_else(|| panic!("missing {key} at {marker}"))
}
fn apply(source: &str, item: &CompletionItem) -> String {
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
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
fn xdata_candidates_require_xdata_declarations_including_ids_alias_owners() {
    let (_dir,ws,main)=project("",r"\cite{child}",&[("refs.bib","@book{ordinary,ids={bookalias},title={Ordinary book}}\n@XDATA{data,IDS={dataalias},title={Data preview},publisher={Inherited publisher}}\n@book{child,xdata={}}")] );
    let bib = main.join("refs.bib").unwrap();
    let candidates = items(&ws, &bib, "xdata={");
    let mut names: Vec<_> = candidates.iter().map(|item| item.label.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["data", "dataalias"]);
    let preview = "```bibtex\n@XDATA{data,IDS={dataalias},title={Data preview},publisher={Inherited publisher}}\n```";
    assert_eq!(
        completion::preview(&item(&ws, &bib, "xdata={", "data")),
        preview
    );
    assert_eq!(
        completion::preview(&item(&ws, &bib, "xdata={", "dataalias")),
        preview
    );
}
#[test]
fn selected_xdata_candidates_keep_section_identity_and_type_filters_together() {
    let (_dir,ws,main)=project(r"\addsectionbib[label=chosen]{section.bib}",r"\newrefsection[chosen]\cite{child}",&[("refs.bib","@book{parent,title={Default book}} @xdata{defaultdata,publisher={Default}}"),("section.bib","@xdata{parent,title={Selected data},publisher={Selected publisher}} @book{ordinary} @book{child,xdata={parBAD}}")]);
    let bib = main.join("section.bib").unwrap();
    let candidate = item(&ws, &bib, "xdata={par", "parent");
    assert!(completion::preview(&candidate).contains("title={Selected data}"));
    assert_eq!(
        apply(&ws.documents[&bib].text.source, &candidate),
        ws.documents[&bib].text.source.replace("parBAD", "parent")
    );
    let names: Vec<_> = items(&ws, &bib, "xdata={")
        .into_iter()
        .map(|item| item.label)
        .collect();
    assert_eq!(names, vec!["parent"]);
}
#[test]
fn source_set_projects_keep_real_xdata_entries_and_remove_book_or_set_overrides() {
    let (_dir,ws,main)=project(r"\addsectionbib[label=chosen]{section.bib}",r"\newrefsection[chosen]\defbibentryset{parent}{member}\defbibentryset{shadowed}{member}\cite{parent,shadowed,child}",&[("refs.bib","@book{default}"),("section.bib","@book{parent} @book{member} @xdata{shadowed,publisher={Hidden data}} @xdata{data,title={Real data},publisher={Real data}} @book{child,xdata={}}")]);
    let bib = main.join("section.bib").unwrap();
    let names: Vec<_> = items(&ws, &bib, "xdata={")
        .into_iter()
        .map(|item| item.label)
        .collect();
    assert_eq!(names, vec!["data"]);
    assert_eq!(
        completion::preview(&item(&ws, &bib, "xdata={", "data")),
        "```bibtex\n@xdata{data,title={Real data},publisher={Real data}}\n```"
    );
}
#[test]
fn unfinished_xdata_values_keep_certified_types_and_quote_list_editing() {
    let (_dir,mut ws,main)=project("",r"\cite{child}",&[("refs.bib","@book{ordinary} @xdata{data\"quoted,publisher={Value}} @book{child,xdata=\"ordinary, dataBAD,ordinary\"}")]);
    let bib = main.join("refs.bib").unwrap();
    let source = ws.documents[&bib].text.source.clone();
    let candidate = item(&ws, &bib, "xdata=\"ordinary, data", "data\"quoted");
    assert_eq!(
        apply(&source, &candidate),
        source.replace(
            "\"ordinary, dataBAD,ordinary\"",
            "{ordinary, data\"quoted,ordinary}"
        )
    );
    let source =
        "% 😀\r\n@XDATA{data,publisher={Value}}\r\n@book{ordinary}\r\n@book{child,xdata={da";
    ws.open(bib.clone(), source.into(), 7);
    let candidate = item(&ws, &bib, "xdata={da", "data");
    assert_eq!(
        apply(source, &candidate),
        source.replace("xdata={da", "xdata={data")
    );
}
#[test]
fn unsaved_provider_type_changes_refresh_candidates_without_changing_consumers() {
    let (_dir, mut ws, main) = project(
        r"\addbibresource{provider.bib}",
        r"\cite{child}",
        &[
            ("refs.bib", "@book{child,xdata={provider}}"),
            ("provider.bib", "@book{provider,title={Book}}"),
        ],
    );
    let bib = main.join("refs.bib").unwrap();
    let provider = main.join("provider.bib").unwrap();
    assert!(items(&ws, &bib, "xdata={pro").is_empty());
    ws.open(
        provider.clone(),
        "@XDATA{provider,title={Data preview},publisher={Data}}".into(),
        9,
    );
    assert_eq!(
        completion::preview(&item(&ws, &bib, "xdata={pro", "provider")),
        "```bibtex\n@XDATA{provider,title={Data preview},publisher={Data}}\n```"
    );
    assert_eq!(ws.documents[&main].version, Some(5));
    ws.open(provider, "@book{provider,title={Book again}}".into(), 10);
    assert!(items(&ws, &bib, "xdata={pro").is_empty());
    assert_eq!(ws.documents[&main].version, Some(5));
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn accepted_typed_xdata_completions_inherit_fields_in_real_biber() {
    let (dir,ws,main)=project(r"\addsectionbib[label=chosen]{section.bib}",r"\newrefsection[chosen]\cite{child}\printbibliography",&[("refs.bib","@book{default}"),("section.bib","@book{ordinary,title={Ordinary}}\n@xdata{data,ids={alias},publisher={Inherited publisher},location={Inherited city},year={2026}}\n@book{child,title={Child},author={Author, A},xdata={alBAD}}")]);
    let bib = main.join("section.bib").unwrap();
    let candidate = item(&ws, &bib, "xdata={al", "alias");
    let completed = apply(&ws.documents[&bib].text.source, &candidate);
    std::fs::write(dir.path().join("section.bib"), completed).unwrap();
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
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
    let child = bbl
        .split_once(r"\entry{child}")
        .unwrap()
        .1
        .split_once(r"\endentry")
        .unwrap()
        .0;
    assert!(
        child.contains("Inherited publisher")
            && child.contains("Inherited city")
            && child.contains(r"\field{year}{2026}"),
        "{child}"
    );
    let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(
        !blg.contains("not an XDATA entry")
            && !blg.contains("I didn't find")
            && !blg.contains("ERROR"),
        "{blg}"
    );
}

#[test]
fn ambiguous_xdata_names_do_not_hide_book_declarations_or_duplicate_aliases() {
    let (_dir,ws,main)=project(r"\addbibresource{provider.bib}",r"\cite{child}",&[("refs.bib","@book{child,xdata={}} @XDATA{data,ids={collision,ownedalias},title={First data}} @xdata{safe,title={Safe data}}"),("provider.bib","@book{data,title={Conflicting book}} @xdata{second,ids={collision},title={Second data}}")]);
    let bib = main.join("refs.bib").unwrap();
    let mut names: Vec<_> = items(&ws, &bib, "xdata={")
        .into_iter()
        .map(|item| item.label)
        .collect();
    names.sort();
    assert_eq!(names, vec!["safe", "second"]);
}
#[test]
fn ordinary_projects_also_exclude_statically_named_source_set_overrides_of_xdata() {
    for declaration in [
        r"\defbibentryset",
        r"\csuse{defbibentryset}",
        r"\csname defbibentryset\endcsname",
    ] {
        let (_dir,ws,main)=project("",&format!(r"{declaration}{{shadowed}}{{member}}\cite{{shadowed,child}}"),&[("refs.bib","@book{member} @xdata{shadowed,title={Overridden data}} @xdata{data,title={Available data}} @book{child,xdata={}}")]);
        let bib = main.join("refs.bib").unwrap();
        let names: Vec<_> = items(&ws, &bib, "xdata={")
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert_eq!(names, vec!["data"], "{declaration}");
    }
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn accepted_default_section_xdata_completion_avoids_a_named_source_set_override() {
    let (dir,ws,main)=project("",r"\csuse{defbibentryset}{shadowed}{member}\cite{shadowed,child}\printbibliography",&[("refs.bib","@book{member,title={Member},author={Author, A},year={2026}}\n@xdata{shadowed,title={Unavailable data},publisher={Wrong publisher}}\n@xdata{data,publisher={Correct publisher},year={2026}}\n@book{child,title={Child},author={Author, A},xdata={daBAD}}")]);
    let bib = main.join("refs.bib").unwrap();
    assert!(!items(&ws, &bib, "xdata={")
        .iter()
        .any(|item| item.label == "shadowed"));
    let candidate = item(&ws, &bib, "xdata={da", "data");
    std::fs::write(
        dir.path().join("refs.bib"),
        apply(&ws.documents[&bib].text.source, &candidate),
    )
    .unwrap();
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
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
    assert!(bbl.contains(r"\entry{shadowed}{set}"), "{bbl}");
    let child = bbl
        .split_once(r"\entry{child}")
        .unwrap()
        .1
        .split_once(r"\endentry")
        .unwrap()
        .0;
    assert!(
        child.contains("Correct publisher") && !child.contains("Wrong publisher"),
        "{child}"
    );
    let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(
        !blg.contains("ERROR")
            && !blg.contains("I didn't find")
            && !blg.contains("not an XDATA entry"),
        "{blg}"
    );
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
}

#[test]
fn shared_xdata_aliases_require_the_primary_owner_to_agree_in_every_section() {
    let (_dir, ws, main) = project(
        r"\addsectionbib[label=chosen]{section.bib}\addglobalbib{provider.bib}",
        r"\cite{child}\newrefsection[chosen]\cite{child}",
        &[
            ("refs.bib", "@book{data,title={Conflicting default book}}"),
            ("section.bib", "@book{selected}"),
            (
                "provider.bib",
                "@xdata{data,ids={dataalias}} @xdata{safe} @book{child,xdata={}}",
            ),
        ],
    );
    let bib = main.join("provider.bib").unwrap();
    let names: Vec<_> = items(&ws, &bib, "xdata={")
        .into_iter()
        .map(|item| item.label)
        .collect();
    assert_eq!(names, vec!["safe"]);
}
