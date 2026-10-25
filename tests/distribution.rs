use latex_lsp::{
    config::DistributionConfig,
    language::{features, navigation, signature},
    project::{distribution::Distribution, workspace::Workspace},
    text::Text,
};
use lsp_types::*;

fn fixture() -> (tempfile::TempDir, Distribution) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("tex/latex/base")).unwrap();
    std::fs::create_dir_all(dir.path().join("bibtex/bst/base")).unwrap();
    for file in ["amsmath.sty", "article.cls"] {
        std::fs::write(dir.path().join("tex/latex/base").join(file), "").unwrap();
    }
    std::fs::write(dir.path().join("bibtex/bst/base/plain.bst"), "").unwrap();
    std::fs::write(dir.path().join("ls-R"), "./tex/latex/base:\namsmath.sty\narticle.cls\nstale.sty\nbad%name.sty\n./bibtex/bst/base:\nplain.bst\n./../outside:\nunsafe.sty\n").unwrap();
    let db = Distribution::from_roots(&[dir.path().into()]);
    (dir, db)
}

fn open(ws: &mut Workspace, uri: &Url, marked: &str) -> Position {
    let offset = marked.find('|').unwrap();
    let text = Text::new(marked.replace('|', ""));
    let position = text.position(offset);
    ws.open(uri.clone(), text.source, 1);
    position
}

#[test]
fn installed_loader_completions_links_and_local_precedence() {
    let (_texmf, db) = fixture();
    let project = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(project.path().join("main.tex")).unwrap();
    let mut ws = Workspace {
        distribution: db,
        ..Default::default()
    };
    for (marked, label, inserted) in [
        (r"\usepackage{am|}", "amsmath.sty", "amsmath"),
        (r"\documentclass{art|}", "article.cls", "article"),
        (r"\bibliographystyle{pl|}", "plain.bst", "plain"),
        (r"\bibliographystyle{plain.b|}", "plain.bst", "plain.bst"),
        ("😀\\usepackage{a\n  m|}", "amsmath.sty", "amsmath"),
    ] {
        let at = open(&mut ws, &uri, marked);
        let items = features::completion(&ws, &uri, at);
        let item = items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, inserted);
        assert!(item.detail.as_ref().unwrap().contains("TeX distribution"));
    }
    open(&mut ws, &uri, r"\usepackage{amsmath|}");
    let links = navigation::document_links(&ws, &uri);
    assert_eq!(links.len(), 1);
    assert!(links[0]
        .target
        .as_ref()
        .unwrap()
        .path()
        .ends_with("tex/latex/base/amsmath.sty"));
    assert!(
        format!("{:?}", features::hover(&ws, &uri, Position::new(0, 15)))
            .contains("texdoc.org/pkg/amsmath")
    );
    std::fs::write(project.path().join("amsmath.sty"), "local").unwrap();
    let at = open(&mut ws, &uri, r"\usepackage{am|}");
    let items = features::completion(&ws, &uri, at);
    assert_eq!(items.iter().filter(|i| i.label == "amsmath.sty").count(), 1);
    assert!(!items
        .iter()
        .find(|i| i.label == "amsmath.sty")
        .unwrap()
        .detail
        .as_ref()
        .unwrap()
        .contains("TeX distribution"));
    open(&mut ws, &uri, r"\usepackage{amsmath|}");
    assert_eq!(
        navigation::document_links(&ws, &uri)[0].target,
        Some(Url::from_file_path(project.path().join("amsmath.sty")).unwrap())
    );
    let at = open(&mut ws, &uri, r"\usepackage{|}");
    assert!(!features::completion(&ws, &uri, at)
        .iter()
        .any(|i| i.label == "stale.sty" || i.label == "bad%name.sty" || i.label == "unsafe.sty"));
}

#[test]
fn package_commands_have_scoped_signatures_hover_and_user_precedence() {
    let mut ws = Workspace::default();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::write(a.path().join("main.tex"), "\\documentclass{article}").unwrap();
    std::fs::write(b.path().join("main.tex"), "\\documentclass{article}").unwrap();
    let uri = Url::from_file_path(a.path().join("main.tex")).unwrap();
    let other = Url::from_file_path(b.path().join("main.tex")).unwrap();
    let at = open(
        &mut ws,
        &uri,
        "\\documentclass{article}\n\\usepackage{mathtools}\n\\df|",
    );
    let items = features::completion(&ws, &uri, at);
    assert!(items.iter().any(|i| i.label == "dfrac"));
    let at = open(&mut ws, &other, "\\documentclass{article}\n\\df|");
    assert!(!features::completion(&ws, &other, at)
        .iter()
        .any(|i| i.label == "dfrac"));
    let at = open(&mut ws, &uri, "\\usepackage{mathtools}\n\\dfrac{1}{|}");
    let help = signature::help(&ws, &uri, at).unwrap();
    assert_eq!(help.signatures[0].label, r"\dfrac{numerator}{denominator}");
    assert_eq!(help.active_parameter, Some(1));
    let hover = features::hover(&ws, &uri, Position::new(1, 3)).unwrap();
    assert!(format!("{hover:?}").contains("amsmath"));
    let at = open(
        &mut ws,
        &uri,
        "\\usepackage{booktabs}\n\\cmidrule(lr){1-|3}",
    );
    assert_eq!(
        signature::help(&ws, &uri, at).unwrap().active_parameter,
        Some(2)
    );
    let at = open(
        &mut ws,
        &uri,
        "\\usepackage{amsmath}\n\\newcommand{\\dfrac}[1]{#1}\n\\dfrac{|}",
    );
    assert_eq!(
        signature::help(&ws, &uri, at).unwrap().signatures[0]
            .parameters
            .as_ref()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn configured_roots_work_without_kpsewhich_and_can_be_disabled() {
    let (dir, _) = fixture();
    let mut config = DistributionConfig {
        executable: "/nonexistent/kpsewhich".into(),
        roots: vec![dir.path().into()],
        ..Default::default()
    };
    let mut ws = Workspace {
        distribution: Distribution::discover(&config),
        ..Default::default()
    };
    let uri = Url::parse("file:///missing/main.tex").unwrap();
    let at = open(&mut ws, &uri, r"\usepackage{am|}");
    assert!(features::completion(&ws, &uri, at)
        .iter()
        .any(|i| i.label == "amsmath.sty"));
    config.enabled = false;
    let db = Distribution::discover(&config);
    let mut ws = Workspace {
        distribution: db,
        ..Default::default()
    };
    let uri = Url::parse("file:///missing/main.tex").unwrap();
    let at = open(&mut ws, &uri, r"\usepackage{am|}");
    assert!(!features::completion(&ws, &uri, at)
        .iter()
        .any(|i| i.label == "amsmath.sty"));
}

#[test]
#[ignore = "requires a TeX distribution and kpsewhich"]
fn real_tex_distribution_database() {
    let db = Distribution::discover(&DistributionConfig::default());
    let mut ws = Workspace {
        distribution: db,
        ..Default::default()
    };
    let uri = Url::parse("file:///missing/main.tex").unwrap();
    let at = open(&mut ws, &uri, r"\usepackage{amsm|}");
    assert!(features::completion(&ws, &uri, at)
        .iter()
        .any(|i| i.label == "amsmath.sty"));
    open(&mut ws, &uri, r"\documentclass{article|}");
    assert_eq!(navigation::document_links(&ws, &uri).len(), 1);
}
