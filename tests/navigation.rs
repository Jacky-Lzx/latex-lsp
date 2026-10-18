use latex_lsp::{
    features, navigation,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::{FoldingRangeKind, Position, Url};

fn insert(workspace: &mut Workspace, uri: &Url, source: &str) {
    workspace
        .documents
        .insert(uri.clone(), Document::new(uri, source.into(), Some(1)));
}

#[test]
fn workspace_search_ranks_matches_and_supports_unicode_and_subsequences() {
    let mut workspace = Workspace::default();
    let main = Url::parse("file:///navigation/main.tex").unwrap();
    let bib = Url::parse("file:///navigation/refs.bib").unwrap();
    insert(
        &mut workspace,
        &main,
        "\\section{Analysis}\n\\section{中文章节}\n\\label{analysis:detail}",
    );
    insert(&mut workspace, &bib, "@book{alphaYear, title={Example}}");
    let matches = navigation::workspace_symbols(&workspace, "ANALYSIS");
    assert_eq!(
        matches.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Analysis", "analysis:detail"]
    );
    assert_eq!(
        navigation::workspace_symbols(&workspace, "中章")[0].name,
        "中文章节"
    );
    assert_eq!(
        navigation::workspace_symbols(&workspace, "ayr")[0]
            .location
            .uri,
        bib
    );
    assert!(navigation::workspace_symbols(&workspace, "absent").is_empty());
    let many = (0..300)
        .map(|n| format!("\\label{{key{n}}}\n"))
        .collect::<String>();
    insert(&mut workspace, &main, &many);
    assert_eq!(navigation::workspace_symbols(&workspace, "").len(), 256);
}

#[test]
fn folds_sections_nested_environments_groups_and_comments_without_parsing_raw_text() {
    let uri = Url::parse("file:///navigation/main.tex").unwrap();
    let source = "\\section{Main}\n% first\n  % second\n\\begin{itemize}\n\\item {\nbody\n}\n\\end{itemize}\n\\subsection{Child}\n\\begin{verbatim}\n{ % ignored\n\\begin{fake}\n\\end{verbatim}\n\\section{Next}\ntext\n";
    let doc = Document::new(&uri, source.into(), Some(1));
    let folds = navigation::folding_ranges(&doc, None);
    for (start, end) in [(0, 12), (1, 2), (3, 7), (4, 6), (8, 12), (9, 12), (13, 14)] {
        assert!(
            folds
                .iter()
                .any(|f| f.start_line == start && f.end_line == end),
            "missing {start}..{end}: {folds:?}"
        );
    }
    assert!(
        folds.iter().find(|f| f.start_line == 1).unwrap().kind == Some(FoldingRangeKind::Comment)
    );
    assert!(!folds
        .iter()
        .any(|f| f.start_line == 10 || f.start_line == 11));
    assert_eq!(navigation::folding_ranges(&doc, Some(2)).len(), 2);
    assert!(navigation::folding_ranges(&doc, Some(0)).is_empty());
    let doc = Document::new(&uri, "\\{\ntext\n\\}\n% {\n\\verb|{|\n".into(), None);
    assert!(navigation::folding_ranges(&doc, None).is_empty());
}

#[test]
fn links_and_definitions_select_individual_files_and_ignore_comments() {
    let directory = tempfile::tempdir().unwrap();
    for name in [
        "main.tex",
        "one.bib",
        "two.bib",
        "local.sty",
        "figure.png",
        "child.tex",
    ] {
        std::fs::write(directory.path().join(name), "").unwrap();
    }
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = "😀 \\bibliography{one, two}\n\\usepackage{missing,local}\n\\includegraphics[width=2cm]{figure}\n\\input{child}\n\\href{https://example.com/a\\_b?q=1\\&x=2}{Example}\n% \\input{child}\n\\verb|\\input{child}|\n";
    let mut workspace = Workspace::default();
    insert(&mut workspace, &uri, source);
    let text = Text::new(source.into());
    let links = navigation::document_links(&workspace, &uri);
    assert_eq!(links.len(), 6);
    let two = &links[1];
    assert_eq!(
        &source[text.offset(two.range.start).unwrap()..text.offset(two.range.end).unwrap()],
        "two"
    );
    assert!(two.target.as_ref().unwrap().path().ends_with("two.bib"));
    let definitions = features::definitions(&workspace, &uri, two.range.start);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, *two.target.as_ref().unwrap());
    assert!(links[2]
        .target
        .as_ref()
        .unwrap()
        .path()
        .ends_with("local.sty"));
    assert!(links[3]
        .target
        .as_ref()
        .unwrap()
        .path()
        .ends_with("figure.png"));
    assert_eq!(
        links[5].target.as_ref().unwrap().as_str(),
        "https://example.com/a_b?q=1&x=2"
    );
    assert!(features::definitions(&workspace, &uri, links[5].range.start).is_empty());
    assert!(
        navigation::document_links(&workspace, &Url::parse("file:///absent.tex").unwrap())
            .is_empty()
    );
    assert_eq!(links[0].range.start, Position::new(0, 17));
}

#[test]
fn urls_accept_percent_escapes_and_keep_their_contents_out_of_latex_analysis() {
    let uri = Url::parse("file:///navigation/urls.tex").unwrap();
    let source = r"\url{https://example.com/a%20b?q=some_value&x=2#section}\href{https://example.com/\label{hidden}}{\label{visible}Text}";
    let mut workspace = Workspace::default();
    insert(&mut workspace, &uri, source);
    let doc = &workspace.documents[&uri];
    assert!(features::diagnostics(doc).is_empty());
    assert_eq!(doc.syntax.labels.len(), 1);
    assert_eq!(doc.syntax.labels[0].name, "visible");
    let links = navigation::document_links(&workspace, &uri);
    assert_eq!(links.len(), 2);
    assert_eq!(
        links[0].target.as_ref().unwrap().as_str(),
        "https://example.com/a%20b?q=some_value&x=2#section"
    );
    assert!(features::completion(
        &workspace,
        &uri,
        doc.text.position(source.find("hidden").unwrap())
    )
    .is_empty());
}

#[test]
fn magic_root_links_select_the_real_directive_and_support_definition_jumps() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("main.tex"),
        "\\documentclass{article}",
    )
    .unwrap();
    let uri = Url::from_file_path(directory.path().join("child.tex")).unwrap();
    let source = "  % !tEx root = \"main.tex\"\r\n% !TeX root = missing.tex\r\nBody";
    let mut workspace = Workspace::default();
    insert(&mut workspace, &uri, source);
    let links = navigation::document_links(&workspace, &uri);
    assert_eq!(links.len(), 1);
    let text = &workspace.documents[&uri].text;
    assert_eq!(
        &source
            [text.offset(links[0].range.start).unwrap()..text.offset(links[0].range.end).unwrap()],
        "main.tex"
    );
    assert_eq!(
        features::definitions(&workspace, &uri, links[0].range.start)[0].uri,
        *links[0].target.as_ref().unwrap()
    );
}

#[test]
fn child_file_links_fall_back_to_the_project_root() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("sections")).unwrap();
    let source = "\\documentclass{article}\n\\input{sections/child}";
    std::fs::write(directory.path().join("main.tex"), source).unwrap();
    std::fs::write(
        directory.path().join("sections/child.tex"),
        "\\includegraphics{figure}",
    )
    .unwrap();
    std::fs::write(directory.path().join("figure.pdf"), "pdf").unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let child = Url::from_file_path(directory.path().join("sections/child.tex")).unwrap();
    let mut workspace = Workspace::default();
    insert(&mut workspace, &main, source);
    insert(&mut workspace, &child, "\\includegraphics{figure}");
    assert!(navigation::document_links(&workspace, &child)[0]
        .target
        .as_ref()
        .unwrap()
        .path()
        .ends_with("figure.pdf"));
}
