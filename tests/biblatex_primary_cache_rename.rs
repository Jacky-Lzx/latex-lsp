use latex_lsp::{features, workspace::Workspace};
use lsp_types::*;
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

#[test]
fn a_moved_ids_owner_cannot_take_a_later_database_primary_from_the_global_cache() {
    let (_dir, ws, main, default, _selected) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{newparent}\newrefsection\cite{old}",
        "@book{newparent,title={Selected parent}}",
    );
    let late = key(&ws, &main, r"\newrefsection\cite{", "old");
    assert_eq!(
        features::definitions(&ws, &main, late.range.start),
        vec![key(&ws, &default, "ids={", "old")]
    );
    let primary = key(&ws, &default, "@book{", "parent");
    assert!(features::rename(&ws, &default, primary.range.start, "newparent").is_err());
    assert!(
        features::rename(&ws, &default, primary.range.start, "fresh")
            .unwrap()
            .is_some()
    );
}
#[test]
fn moving_an_ordinary_primary_cannot_replace_a_foreign_ids_owner_cache() {
    let (_dir, ws, main, default, selected) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{member}\newrefsection\cite{old}",
        "@book{member,title={Selected parent}}",
    );
    let late = key(&ws, &main, r"\newrefsection\cite{", "old");
    assert_eq!(
        features::definitions(&ws, &main, late.range.start),
        vec![key(&ws, &default, "ids={", "old")]
    );
    let primary = key(&ws, &selected, "@book{", "member");
    assert!(features::rename(&ws, &selected, primary.range.start, "parent").is_err());
    assert!(
        features::rename(&ws, &selected, primary.range.start, "fresh")
            .unwrap()
            .is_some()
    );
}
#[test]
fn coupled_source_owners_recheck_the_real_primary_they_move() {
    let (_dir, ws, main, _default, _selected) = setup(
        r"\cite{old}\newrefsection[chosen]\defbibentryset{member}{leaf}\cite{member,other}\newrefsection\cite{old}",
        "@book{member,title={Selected parent},ids={other}} @book{leaf,title={Leaf}}",
    );
    let source = key(&ws, &main, r"\defbibentryset{", "member");
    assert!(features::rename(&ws, &main, source.range.start, "parent").is_err());
    assert!(features::rename(&ws, &main, source.range.start, "fresh")
        .unwrap()
        .is_some());
}
#[test]
fn disjoint_primary_names_without_ids_remain_editable() {
    let (_dir, mut ws, _main, default, selected) = setup(
        r"\cite{parent}\newrefsection[chosen]\cite{member}\newrefsection\cite{parent}",
        "@book{member,title={Selected parent}}",
    );
    ws.open(default, "@book{parent,title={Default parent}}".into(), 7);
    let member = key(&ws, &selected, "@book{", "member");
    assert!(
        features::rename(&ws, &selected, member.range.start, "parent")
            .unwrap()
            .is_some()
    );
}
#[test]
fn later_loaded_alias_owners_can_safely_share_an_earlier_plain_primary() {
    let (_dir, mut ws, _main, default, selected) = setup(
        r"\cite{foreign}\newrefsection[chosen]\cite{other}",
        "@book{member,title={Selected parent},ids={other}}",
    );
    ws.open(default, "@book{foreign,title={Default parent}}".into(), 7);
    let owner = key(&ws, &selected, "@book{", "member");
    assert!(
        features::rename(&ws, &selected, owner.range.start, "foreign")
            .unwrap()
            .is_some()
    );
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_primary_capture_changes_later_alias_data_without_biber_warnings() {
    for moved_owner in [false, true] {
        let (body, selected) = if moved_owner {
            (
                r"\cite{old}\newrefsection[chosen]\cite{newparent}\newrefsection\cite{old}\printbibliography",
                "@book{newparent,title={Selected parent}}",
            )
        } else {
            (
                r"\cite{old}\newrefsection[chosen]\cite{member}\newrefsection\cite{old}\printbibliography",
                "@book{member,title={Selected parent}}",
            )
        };
        let (dir, ws, main, _default, _selected) = setup(body, selected);
        let before = compile(dir.path());
        let last = before
            .split_once(r"\refsection{2}")
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        assert!(last.contains(r"\field{title}{Default parent}"), "{last}");
        if moved_owner {
            std::fs::write(
                dir.path().join("default.bib"),
                DEFAULT.replace("@book{parent,", "@book{newparent,"),
            )
            .unwrap();
        } else {
            std::fs::write(
                dir.path().join("section.bib"),
                selected.replace("@book{member,", "@book{parent,"),
            )
            .unwrap();
            std::fs::write(
                dir.path().join("main.tex"),
                ws.documents[&main]
                    .text
                    .source
                    .replace(r"\cite{member}", r"\cite{parent}"),
            )
            .unwrap();
        }
        for ext in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
        }
        let after = compile(dir.path());
        let last = after
            .split_once(r"\refsection{2}")
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        assert!(last.contains(r"\field{title}{Selected parent}"), "{last}");
        assert!(!last.contains("Default parent"), "{last}");
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_a_later_owner_can_reuse_an_earlier_plain_primary_without_data_changes() {
    let (dir, mut ws, _main, default, selected) = setup(
        r"\cite{foreign}\newrefsection[chosen]\cite{other}\printbibliography",
        "@book{member,title={Selected parent},ids={other}}",
    );
    let library = "@book{foreign,title={Default parent}}";
    std::fs::write(dir.path().join("default.bib"), library).unwrap();
    ws.open(default, library.into(), 7);
    let before = compile(dir.path());
    let owner = key(&ws, &selected, "@book{", "member");
    let edits = features::rename(&ws, &selected, owner.range.start, "foreign")
        .unwrap()
        .unwrap();
    let changes = edits.changes.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[&selected].len(), 1);
    for (uri, mut edits) in changes {
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
    let start = after.find(r"\refsection{1}").unwrap();
    let normalized = format!(
        "{}{}",
        &after[..start],
        after[start..].replace("{foreign}", "{member}")
    );
    assert_eq!(normalized, before);
}
