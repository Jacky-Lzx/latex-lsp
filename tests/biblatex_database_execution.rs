use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;

const LIBRARY: &str = "@book{trigger,title={Trigger},execute={\\defbibentryset{collection}{member}}}\n@book{collection,title={Raw data}}\n@book{member,title={Member}}";

fn setup(body: &str, library: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"% 😀
\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{data.bib}}\begin{{document}}{body}\label{{safe}}\ref{{safe}}\end{{document}}"
    );
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(dir.path().join("data.bib"), library).unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("data.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}

fn point(ws: &Workspace, uri: &Url, prefix: &str) -> Position {
    let doc = &ws.documents[uri];
    doc.text
        .position(doc.text.source.find(prefix).unwrap() + prefix.len())
}

#[test]
fn entry_execution_cannot_certify_database_identity_or_partial_key_edits() {
    for body in [
        r"\cite{trigger}\cite{collection}\cite{injected}",
        r"\entrydata{trigger}{}\cite{collection}\cite{injected}",
        r"\newrefsection\cite{trigger}\cite{collection}\cite{injected}",
        r"\defbibentryset{other}{member}\cite{trigger}\cite{collection}\cite{injected}",
    ] {
        let (_dir, ws, main, bib) = setup(body, LIBRARY);
        let at = point(&ws, &main, r"\cite{col");
        assert!(features::rename(&ws, &main, at, "fresh").is_err(), "{body}");
        assert!(features::prepare_rename(&ws, &main, at).is_err());
        assert!(features::definitions(&ws, &main, at).is_empty());
        assert!(features::hover(&ws, &main, at).is_none());
        assert!(features::references(&ws, &main, at, true).is_empty());
        assert!(features::completion(&ws, &main, at).is_empty());
        let raw = point(&ws, &bib, "@book{col");
        assert!(features::rename(&ws, &bib, raw, "fresh").is_err());
        assert!(features::prepare_rename(&ws, &bib, raw).is_err());
        assert!(features::references(&ws, &bib, raw, true).is_empty());
        assert_eq!(features::definitions(&ws, &bib, raw).len(), 1);
        assert!(ws.diagnostics(&main).iter().all(|diag| {
            diag.code != Some(NumberOrString::String("undefined-citation".into()))
        }));
        let label = point(&ws, &main, r"\ref{sa");
        assert_eq!(features::definitions(&ws, &main, label).len(), 1);
        assert!(features::rename(&ws, &main, label, "freshlabel")
            .unwrap()
            .is_some());
    }
}

#[test]
fn execution_fields_are_case_insensitive_and_unexpanded_values_stay_opaque() {
    for field in [
        "execute={\\defbibentryset{collection}{member}}",
        "ExEcUtE=code",
        "execute={\\defbibentryset} # {collection}",
        "execute={}",
        "execute={Text},execute={\\unknown}",
        "execute=",
    ] {
        let library = format!("@book{{collection,title={{Collection}},{field}}}");
        let (_dir, ws, main, bib) = setup(r"\cite{collection}", &library);
        assert!(
            features::rename(&ws, &main, point(&ws, &main, r"\cite{col"), "fresh").is_err(),
            "{field}"
        );
        assert!(
            features::rename(&ws, &bib, point(&ws, &bib, "@book{col"), "fresh").is_err(),
            "{field}"
        );
    }
}

#[test]
fn unsaved_execution_changes_revoke_proofs_and_restoration_recovers_them() {
    let plain = LIBRARY.replace(",execute={\\defbibentryset{collection}{member}}", "");
    let (_dir, mut ws, main, bib) = setup(r"\cite{trigger}\cite{collection}", &plain);
    let at = point(&ws, &main, r"\cite{col");
    assert_eq!(features::definitions(&ws, &main, at).len(), 1);
    assert!(features::rename(&ws, &main, at, "fresh").unwrap().is_some());
    ws.open(bib.clone(), LIBRARY.into(), 8);
    assert!(features::definitions(&ws, &main, at).is_empty());
    assert!(features::rename(&ws, &main, at, "fresh").is_err());
    ws.open(bib.clone(), plain, 9);
    assert_eq!(features::definitions(&ws, &main, at).len(), 1);
    assert!(features::rename(&ws, &main, at, "fresh").unwrap().is_some());
}

#[test]
fn literal_examples_classic_bibtex_and_loose_databases_keep_their_symbol_model() {
    for library in [
        "@book{collection,title={Example execute={\\unknown}}}",
        "@book{collection,title={Collection}}\n@comment{execute={\\unknown}}",
    ] {
        let (_dir, ws, main, _bib) = setup(r"\cite{collection}", library);
        let at = point(&ws, &main, r"\cite{col");
        assert_eq!(features::definitions(&ws, &main, at).len(), 1);
        assert!(features::rename(&ws, &main, at, "fresh").unwrap().is_some());
    }
    let (_dir, mut ws, main, bib) = setup(r"\cite{collection}", LIBRARY);
    let classic = ws.documents[&main].text.source.replace(
        r"\usepackage{biblatex}\addbibresource{data.bib}",
        r"\bibliographystyle{plain}\bibliography{data}",
    );
    ws.open(main.clone(), classic, 6);
    let at = point(&ws, &main, r"\cite{col");
    assert_eq!(features::definitions(&ws, &main, at).len(), 1);
    assert!(features::rename(&ws, &main, at, "fresh").unwrap().is_some());
    let mut loose = Workspace::default();
    loose.documents.insert(
        bib.clone(),
        latex_lsp::workspace::Document::new(&bib, LIBRARY.into(), Some(2)),
    );
    assert!(
        features::rename(&loose, &bib, point(&loose, &bib, "@book{col"), "fresh")
            .unwrap()
            .is_some()
    );
}

#[test]
fn unrelated_opaque_projects_do_not_block_safe_keys_but_shared_databases_do() {
    let (dir, mut ws, main, bib) =
        setup(r"\cite{collection}", "@book{collection,title={Collection}}");
    let other = main.join("other.tex").unwrap();
    let library = main.join("other.bib").unwrap();
    std::fs::write(dir.path().join("other.bib"), LIBRARY).unwrap();
    let source = r"\documentclass{article}\usepackage{biblatex}\addbibresource{other.bib}\begin{document}\cite{collection}\end{document}";
    ws.open(other.clone(), source.into(), 8);
    assert!(ws.documents.contains_key(&library));
    let at = point(&ws, &main, r"\cite{col");
    assert_eq!(features::definitions(&ws, &main, at).len(), 1);
    assert!(features::rename(&ws, &main, at, "fresh").unwrap().is_some());
    ws.open(
        other,
        source.replace(
            r"\addbibresource{other.bib}",
            r"\addbibresource{other.bib}\addbibresource{data.bib}",
        ),
        9,
    );
    assert!(features::rename(&ws, &main, at, "fresh").is_err());
    assert!(features::rename(&ws, &bib, point(&ws, &bib, "@book{col"), "fresh").is_err());
}

#[test]
fn generated_preambles_without_resource_sections_also_need_execution_proofs() {
    let library = "@book{collection,title={Collection}}\n@book{member,title={Member}}\n@preamble{\"\\defbibentryset{collection}{member}\"}";
    let (_dir, ws, main, bib) = setup(r"\cite{collection}", library);
    let at = point(&ws, &main, r"\cite{col");
    assert!(features::definitions(&ws, &main, at).is_empty());
    assert!(features::rename(&ws, &main, at, "fresh").is_err());
    assert!(features::rename(&ws, &bib, point(&ws, &bib, "@book{col"), "fresh").is_err());
}

#[test]
fn named_and_forwarded_package_loaders_cannot_hide_database_execution() {
    for loader in [
        r"\usepackage{biblatex}",
        r"\RequirePackage[backend=biber]{biblatex}",
        r"\usepackage{etoolbox}\csuse{usepackage}{biblatex}",
        r"\csname usepackage\endcsname[backend=biber]{biblatex}",
        r"\def\loadbib{\usepackage}\loadbib{biblatex}",
    ] {
        let (_dir, mut ws, main, bib) = setup(r"\cite{trigger}\cite{collection}", LIBRARY);
        let source = ws.documents[&main].text.source.replace(
            r"\usepackage{biblatex}\addbibresource{data.bib}",
            &format!(r"{loader}\bibliography{{data}}"),
        );
        ws.open(main.clone(), source, 6);
        let at = point(&ws, &main, r"\cite{col");
        assert!(
            features::rename(&ws, &main, at, "fresh").is_err(),
            "{loader}"
        );
        assert!(features::definitions(&ws, &main, at).is_empty(), "{loader}");
        assert!(
            features::rename(&ws, &bib, point(&ws, &bib, "@book{col"), "fresh").is_err(),
            "{loader}"
        );
    }
}

fn compile_clean(dir: &std::path::Path) -> String {
    for extension in ["aux", "bcf", "bbl", "run.xml"] {
        let path = dir.join(format!("main.{extension}"));
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
    for program in [
        "pdflatex", "biber", "pdflatex", "biber", "pdflatex", "pdflatex",
    ] {
        let args = if program == "biber" {
            vec!["main"]
        } else {
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ]
        };
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
    assert!(
        !log.contains("Warning") && !log.contains("undefined"),
        "{log}"
    );
    let log = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(!log.contains("WARN") && !log.contains("ERROR"), "{log}");
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_entry_execution_generates_sets_and_incomplete_renames_change_the_bbl() {
    for (preamble, body, named) in [
        (
            "",
            r"\cite{trigger}\cite{collection}\printbibliography",
            false,
        ),
        (
            r"\AtEveryCitekey{\entrydata{trigger}{}}",
            r"\nocite{trigger}\cite{member}\cite{collection}\printbibliography",
            false,
        ),
        (
            "",
            r"\cite{trigger}\cite{collection}\printbibliography",
            true,
        ),
    ] {
        let (dir, mut ws, main, bib) = setup(body, LIBRARY);
        let mut source = ws.documents[&main].text.source.replace(
            r"\begin{document}",
            &format!(r"{preamble}\begin{{document}}"),
        );
        if named {
            source = source.replace(
                r"\usepackage{biblatex}\addbibresource{data.bib}",
                r"\csname usepackage\endcsname{biblatex}\bibliography{data}",
            );
        }
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        ws.open(main.clone(), source, 6);
        let before = compile_clean(dir.path());
        assert!(before.contains(r"\entry{collection}{set}"), "{before}");
        assert!(before.contains(r"\set{member}"), "{before}");
        assert!(features::rename(&ws, &main, point(&ws, &main, r"\cite{col"), "fresh").is_err());
        assert!(features::rename(&ws, &bib, point(&ws, &bib, "@book{col"), "fresh").is_err());
        assert_eq!(compile_clean(dir.path()), before);
        // Reproduce the old partial edit independently of the current server.
        let source = ws.documents[&main]
            .text
            .source
            .replace(r"\cite{collection}", r"\cite{fresh}");
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        std::fs::write(
            dir.path().join("data.bib"),
            LIBRARY.replace("@book{collection", "@book{fresh"),
        )
        .unwrap();
        let partial = compile_clean(dir.path());
        assert!(partial.contains(r"\entry{collection}{set}"), "{partial}");
        assert!(partial.contains(r"\entry{fresh}{book}"), "{partial}");
        assert_ne!(partial.replace("fresh", "collection"), before);
        // A complete manual edit proves the missing operand is consequential.
        std::fs::write(
            dir.path().join("data.bib"),
            LIBRARY.replace("collection", "fresh"),
        )
        .unwrap();
        assert_eq!(
            compile_clean(dir.path()).replace("fresh", "collection"),
            before
        );
    }
}
