use latex_lsp::{features, workspace::Workspace};
use lsp_types::*;
fn setup(numeric: bool, body: &str, preamble: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}\\addbibresource{{data.bib}}{preamble}\\begin{{document}}{}{body}\\end{{document}}",if numeric{r"\newrefsection"}else{""});
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(
        dir.path().join("data.bib"),
        "@book{parent,title={Database parent}}",
    )
    .unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("data.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&doc.text.source[start..start + name.len()], name);
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
}
#[test]
fn handwritten_bibitems_do_not_define_biblatex_backend_citations() {
    for numeric in [false, true] {
        let body = r"\cite{parent,manual}\begin{thebibliography}{9}\bibitem{parent}Handwritten parent\bibitem{manual}Manual\end{thebibliography}";
        let (_dir, ws, main, bib) = setup(numeric, body, "");
        let source = ws.documents[&main].text.source.clone();
        let parent = ws.documents[&main]
            .text
            .position(source.find("parent,manual").unwrap());
        let declaration = Location::new(bib.clone(), ws.documents[&bib].text.range(6, 12));
        assert_eq!(
            features::definitions(&ws, &main, parent),
            vec![declaration.clone()],
            "numeric={numeric}"
        );
        let manual = ws.documents[&main]
            .text
            .position(source.find("parent,manual").unwrap() + 7);
        assert!(features::definitions(&ws, &main, manual).is_empty());
        let refs = features::references(&ws, &bib, declaration.range.start, true);
        assert_eq!(refs.len(), 2);
        let edits = features::rename(&ws, &bib, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits[&main].len(), 1);
        assert_eq!(edits[&bib].len(), 1);
        let warnings: Vec<_> = ws
            .diagnostics(&main)
            .into_iter()
            .filter(|diag| diag.code == Some(NumberOrString::String("undefined-citation".into())))
            .collect();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].data.as_ref().unwrap()["key"], "manual");
    }
}
#[test]
fn static_named_items_keep_backend_definitions_and_complete_primary_edits() {
    for item in [
        r"\bibitem[Display]{parent}",
        r"\csuse{bibitem}{parent}",
        r"\csname bibitem\endcsname{parent}",
    ] {
        let (_dir, ws, main, bib) = setup(
            false,
            &format!(
                r"\cite{{parent}}\begin{{thebibliography}}{{9}}{item}Manual\end{{thebibliography}}"
            ),
            "",
        );
        let query = key(&ws, &main, r"\cite{", "parent");
        let declaration = key(&ws, &bib, "@book{", "parent");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![declaration.clone()],
            "{item}"
        );
        let edits = features::rename(&ws, &bib, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits[&main].len(), 1, "{item}");
    }
}
#[test]
fn unproved_forwarded_backend_loaders_do_not_fall_back_to_handwritten_definitions() {
    for numeric in [false, true] {
        let (_dir, mut ws, main, bib) = setup(
            numeric,
            r"\cite{parent}\begin{thebibliography}{9}\bibitem{parent}Manual\end{thebibliography}",
            "",
        );
        let source = ws.documents[&main].text.source.replace(
            r"\usepackage{biblatex}\addbibresource{data.bib}",
            r"\newcommand\loadbackend{\usepackage}\loadbackend{biblatex}\newcommand\adddb{\addbibresource}\adddb{data.bib}",
        );
        ws.open(main.clone(), source, 6);
        let query = key(&ws, &main, r"\cite{", "parent");
        assert!(features::definitions(&ws, &main, query.range.start).is_empty());
        let declaration = key(&ws, &bib, "@book{", "parent");
        assert!(features::rename(&ws, &bib, declaration.range.start, "fresh").is_err());
    }
}
#[test]
fn overwritten_dynamic_and_aliased_items_keep_their_namespace_protected() {
    for (preamble, item) in [
        (r"\renewcommand\bibitem[1]{}", r"\bibitem{parent}"),
        ("", r"\bibitem{\unknown}"),
        (r"\newcommand\itemalias{\bibitem}", r"\itemalias{parent}"),
    ] {
        let (_dir, ws, main, bib) = setup(
            false,
            &format!(
                r"\cite{{parent}}\begin{{thebibliography}}{{9}}{item}Manual\end{{thebibliography}}\label{{target}}\ref{{target}}\ref{{missing}}"
            ),
            preamble,
        );
        let query = key(&ws, &main, r"\cite{", "parent");
        assert!(
            features::definitions(&ws, &main, query.range.start).is_empty(),
            "{preamble}: {item}"
        );
        let declaration = key(&ws, &bib, "@book{", "parent");
        assert!(features::rename(&ws, &bib, declaration.range.start, "fresh").is_err());
        let label = key(&ws, &main, r"\label{", "target");
        let reference = key(&ws, &main, r"\ref{", "target");
        assert_eq!(
            features::definitions(&ws, &main, reference.range.start),
            vec![label]
        );
        assert!(ws.diagnostics(&main).iter().any(|diagnostic| diagnostic
            .data
            .as_ref()
            .is_some_and(|data| data["key"] == "missing")));
    }
}
#[test]
fn traditional_items_and_literal_examples_keep_their_existing_identities() {
    let (dir, mut ws, main, _bib) = setup(
        false,
        r"\cite{parent}\begin{thebibliography}{9}\bibitem{parent}Manual\end{thebibliography}",
        "",
    );
    let source = ws.documents[&main]
        .text
        .source
        .replace(r"\usepackage{biblatex}\addbibresource{data.bib}", "");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    ws.open(main.clone(), source, 6);
    let query = key(&ws, &main, r"\cite{", "parent");
    let declaration = key(&ws, &main, r"\bibitem{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![declaration]
    );
    let edits = features::rename(&ws, &main, query.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&main].len(), 2);
    let (_dir, ws, main, bib) = setup(false, r"\cite{parent}\verb|\bibitem{parent}|", "");
    let query = key(&ws, &main, r"\cite{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![key(&ws, &bib, "@book{", "parent")]
    );
    let (_dir, ws, main, bib) = setup(
        false,
        r"\cite{parent}\take{\bibitem{parent}}",
        r"\newcommand\take[1]{}",
    );
    let query = key(&ws, &main, r"\cite{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![key(&ws, &bib, "@book{", "parent")]
    );
}
#[test]
fn backend_candidates_and_manual_queries_keep_separate_namespaces() {
    for numeric in [false, true] {
        let (_dir, ws, main, bib) = setup(
            numeric,
            r"\cite{parent,manual}\begin{thebibliography}{9}\bibitem{parent}Manual\bibitem{manual}Only handwritten\end{thebibliography}",
            "",
        );
        let query = key(&ws, &main, r"\cite{", "parent");
        let items = features::completion(&ws, &main, query.range.start);
        assert!(items.iter().any(|item| item.label == "parent"));
        assert!(!items.iter().any(|item| item.label == "manual"));
        let manual = key(&ws, &main, r"\bibitem{", "parent");
        assert!(features::definitions(&ws, &main, manual.range.start).is_empty());
        assert!(features::hover(&ws, &main, manual.range.start).is_none());
        assert!(features::references(&ws, &main, manual.range.start, true).is_empty());
        assert!(features::prepare_rename(&ws, &main, manual.range.start).is_err());
        assert!(features::rename(&ws, &main, manual.range.start, "fresh").is_err());
        let declaration = key(&ws, &bib, "@book{", "parent");
        assert!(features::hover(&ws, &bib, declaration.range.start).is_some());
    }
}
#[test]
fn unsaved_manual_providers_recompute_backend_identity_and_edit_certification() {
    for numeric in [false, true] {
        let (_dir, mut ws, main, bib) = setup(numeric, r"\cite{parent}\input{manual}", "");
        let provider = main.join("manual.tex").unwrap();
        std::fs::write(provider.to_file_path().unwrap(), "Saved provider").unwrap();
        ws.refresh_project(&main);
        let query = key(&ws, &main, r"\cite{", "parent");
        let declaration = key(&ws, &bib, "@book{", "parent");
        for (version, text) in [
            (
                8,
                r"\begin{thebibliography}{9}\bibitem{parent}Manual\end{thebibliography}",
            ),
            (
                10,
                r"\begin{thebibliography}{9}\csuse{bibitem}{parent}Manual\end{thebibliography}",
            ),
        ] {
            ws.open(provider.clone(), text.into(), version);
            assert!(ws.dependencies(&main).contains(&provider));
            assert_eq!(
                features::definitions(&ws, &main, query.range.start),
                vec![declaration.clone()]
            );
            let edits = features::rename(&ws, &bib, declaration.range.start, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(edits.len(), 2);
            assert!(!edits.contains_key(&provider));
            ws.open(
                provider.clone(),
                r"\renewcommand\bibitem[1]{}\bibitem{parent}".into(),
                version + 1,
            );
            assert!(
                features::definitions(&ws, &main, query.range.start).is_empty(),
                "numeric={numeric} version={version}: {:?}; deps={:?}",
                features::definitions(&ws, &main, query.range.start),
                ws.dependencies(&main)
            );
            assert!(features::rename(&ws, &bib, declaration.range.start, "fresh").is_err());
        }
    }
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
        let out = std::process::Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
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
fn native_backend_primary_renames_leave_the_handwritten_namespace_unchanged() {
    for numeric in [false, true] {
        let (dir, ws, main, bib) = setup(
            numeric,
            r"\cite{parent}\begin{thebibliography}{9}\bibitem{parent}Handwritten title\end{thebibliography}\printbibliography",
            "",
        );
        let before = compile(dir.path());
        assert!(before.contains(r"\field{title}{Database parent}"));
        let declaration = key(&ws, &bib, "@book{", "parent");
        let edits = features::rename(&ws, &bib, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits[&main].len(), 1);
        for (uri, mut edits) in edits {
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
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        for ext in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
        }
        let after = compile(dir.path());
        assert_eq!(before, after.replace("fresh", "parent"));
        let aux = std::fs::read_to_string(dir.path().join("main.aux")).unwrap();
        assert!(aux.contains(r"\bibcite{parent}{1}"));
        assert!(!aux.contains(r"\bibcite{fresh}"));
    }
}
