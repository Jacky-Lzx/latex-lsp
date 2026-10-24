use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
fn setup(body: &str, selected_text: &str) -> (tempfile::TempDir, Workspace, Url, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}\\addbibresource{{default.bib}}\\addsectionbib[label=chosen]{{section.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(dir.path().join("default.bib"), DEFAULT).unwrap();
    std::fs::write(dir.path().join("section.bib"), selected_text).unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let default = main.join("default.bib").unwrap();
    let bib = main.join("section.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, default, bib)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&doc.text.source[start..start + name.len()], name);
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
}
fn compile_case(dir: &std::path::Path, allow_missing: bool) -> String {
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
    assert!(allow_missing || !log.contains("undefined"), "{log}");
    let blg = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(
        (allow_missing || !blg.contains("WARN")) && !blg.contains("ERROR"),
        "{blg}"
    );
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}

fn compile(dir: &std::path::Path) -> String {
    compile_case(dir, false)
}
const DEFAULT: &str = "@book{parent,title={Default parent},ids={old}}";
const SELECTED: &str =
    "@book{parent,title={Selected parent},ids={other}} @book{member,title={Member}}";

#[test]
fn textual_bibliography_tests_keep_complete_section_references_and_edits() {
    for call in [
        r"\ifentryinbib{parent}{Present}{Missing}",
        r"\ifentryinbib{parent}{Literal parent}{No parent}",
        r"\ifentrycategory{parent}{group}{Yes}{No}",
        "\\ifentryinbib{parent}{% ignored \\newrefsection\nYes}{No}",
        r"\csuse{ifentryinbib}{parent}{}{}",
        r"\csname ifentrycategory\endcsname{parent}{group}{}{}",
    ] {
        let (_dir, ws, main, default, selected) = setup(
            &format!(r"\cite{{parent}}\newrefsection[chosen]\cite{{parent}}{call}"),
            SELECTED,
        );
        let declaration = key(&ws, &selected, "@book{", "parent");
        let references = features::references(&ws, &selected, declaration.range.start, true);
        assert_eq!(references.len(), 3, "{call}: {references:?}");
        assert!(references.iter().all(|reference| reference.uri != default));
        let edits = features::rename(&ws, &selected, declaration.range.start, "newparent")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[&main].len(), 2);
        assert_eq!(edits[&selected].len(), 1);
        let query = key(&ws, &main, &call[..call.find("parent").unwrap()], "parent");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![declaration]
        );
        let expected = vec![
            key(&ws, &main, r"\newrefsection[chosen]\cite{", "parent").range,
            query.range,
        ];
        let mut ranges = edits[&main]
            .iter()
            .map(|edit| edit.range)
            .collect::<Vec<_>>();
        ranges.sort_by_key(|range| range.start);
        assert_eq!(ranges, expected, "{call}");
    }
}
#[test]
fn textual_tests_do_not_preload_databases_or_allow_query_capture() {
    for call in [
        r"\ifentryinbib{parent}{Yes}{No}",
        r"\ifentrycategory{parent}{group}{}{}",
    ] {
        let (_dir, ws, main, default, _selected) = setup(
            &format!(r"{call}\newrefsection[chosen]\cite{{other}}\newrefsection\cite{{old}}"),
            SELECTED,
        );
        let late = key(&ws, &main, r"\newrefsection\cite{", "old");
        assert_eq!(
            features::definitions(&ws, &main, late.range.start),
            vec![key(&ws, &default, "ids={", "old")]
        );
        let (_dir, ws, _main, _default, selected) = setup(
            &format!(
                r"\newrefsection[chosen]\cite{{parent}}{}",
                call.replace("parent", "capture")
            ),
            SELECTED,
        );
        let declaration = key(&ws, &selected, "@book{", "parent");
        assert!(features::rename(&ws, &selected, declaration.range.start, "capture").is_err());
        assert!(
            features::rename(&ws, &selected, declaration.range.start, "fresh")
                .unwrap()
                .is_some()
        );
    }
}
#[test]
fn executable_or_incomplete_test_branches_keep_complete_graphs_protected() {
    for call in [
        r"\ifentryinbib{parent}{\newrefsection}{}",
        r"\ifentrycategory{parent}{group}{}{\cite{member}}",
        r"\ifentryinbib{parent}{#1}{}",
        r"\ifentryinbib{parent}{Text}{",
        r"\ifentryinbib{\unknown}{}{}",
        r"\ifentrycategory{parent}{\unknown}{}{}",
        r"\ifentryexists{parent}{}{}",
        r"\defbibentryset{parent}{member}\ifentryinbib{parent}{}{}",
    ] {
        let (_dir, ws, _main, _default, selected) = setup(
            &format!(r"\newrefsection[chosen]\cite{{parent}}{call}"),
            SELECTED,
        );
        let declaration = key(&ws, &selected, "@book{", "parent");
        assert!(
            features::references(&ws, &selected, declaration.range.start, true).is_empty(),
            "{call}"
        );
        assert!(
            features::rename(&ws, &selected, declaration.range.start, "fresh").is_err(),
            "{call}"
        );
    }
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_textual_tests_do_not_load_resources_and_preserve_complete_renames() {
    for call in [
        r"\ifentryinbib{parent}{Present}{Missing}",
        r"\ifentrycategory{parent}{group}{Yes}{No}",
    ] {
        let (dir, mut ws, main, _default, selected) = setup(
            &format!(
                r"\cite{{parent}}\printbibliography\newrefsection[chosen]\cite{{parent}}{call}\printbibliography"
            ),
            SELECTED,
        );
        let source = ws.documents[&main].text.source.replace(
            r"\begin{document}",
            r"\DeclareBibliographyCategory{group}\begin{document}",
        );
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        ws.open(main.clone(), source, 6);
        let before = compile(dir.path());
        let declaration = key(&ws, &selected, "@book{", "parent");
        let changes = features::rename(&ws, &selected, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes[&main].len(), 2);
        assert_eq!(changes[&selected].len(), 1);
        for (uri, edits) in changes {
            let doc = &ws.documents[&uri];
            let mut source = doc.text.source.clone();
            let mut spans = edits
                .into_iter()
                .map(|edit| {
                    (
                        doc.text.offset(edit.range.start).unwrap(),
                        doc.text.offset(edit.range.end).unwrap(),
                        edit.new_text,
                    )
                })
                .collect::<Vec<_>>();
            spans.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
            for (start, end, text) in spans {
                source.replace_range(start..end, &text);
            }
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        for extension in ["aux", "bcf", "bbl", "run.xml"] {
            let path = dir.path().join(format!("main.{extension}"));
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        let after = compile(dir.path());
        assert_eq!(after.replace("fresh", "parent"), before, "{call}");
    }
    let (dir, mut ws, main, _default, _selected) = setup(
        r"\ifentryinbib{parent}{Yes}{No}\ifentrycategory{parent}{group}{}{}\newrefsection[chosen]\cite{other}\newrefsection\cite{old}\printbibliography",
        SELECTED,
    );
    let source = ws.documents[&main].text.source.replace(
        r"\begin{document}",
        r"\DeclareBibliographyCategory{group}\begin{document}",
    );
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    ws.open(main.clone(), source.clone(), 6);
    let before = compile(dir.path());
    std::fs::write(
        dir.path().join("main.tex"),
        source.replace(
            r"\ifentryinbib{parent}{Yes}{No}\ifentrycategory{parent}{group}{}{}",
            "",
        ),
    )
    .unwrap();
    for extension in ["aux", "bcf", "bbl", "run.xml"] {
        let path = dir.path().join(format!("main.{extension}"));
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
    assert_eq!(compile(dir.path()), before);
}
#[test]
fn literal_examples_and_unsaved_predicates_preserve_project_meanings() {
    let (dir, mut ws, main, _default, selected) = setup(
        r"\newrefsection[chosen]\cite{parent}\ifentryinbib{parent}{存在}{缺失}\verb|\ifentryinbib{parent}{\newrefsection}{}|",
        SELECTED,
    );
    std::fs::write(dir.path().join("macros.tex"), "").unwrap();
    let source = ws.documents[&main]
        .text
        .source
        .replace(r"\begin{document}", r"\input{macros}\begin{document}");
    ws.open(main.clone(), source, 6);
    let provider = main.join("macros.tex").unwrap();
    assert!(ws.dependencies(&main).contains(&provider));
    let declaration = key(&ws, &selected, "@book{", "parent");
    assert_eq!(
        features::references(&ws, &selected, declaration.range.start, true).len(),
        3
    );
    ws.open(
        provider.clone(),
        r"\renewcommand\ifentryinbib[3]{}".into(),
        7,
    );
    assert!(features::references(&ws, &selected, declaration.range.start, true).is_empty());
    assert!(features::rename(&ws, &selected, declaration.range.start, "fresh").is_err());
    ws.open(provider, String::new(), 8);
    assert_eq!(
        features::references(&ws, &selected, declaration.range.start, true).len(),
        3
    );
}
