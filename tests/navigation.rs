use lsp_types::{FoldingRangeKind, Position, Url};
use texquill::{
    language::{features, navigation},
    project::workspace::{Document, Workspace},
    text::Text,
};

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
fn workspace_search_keeps_containers_and_stable_top_results_across_edits() {
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    let main = Url::parse("untitled:search.tex").unwrap();
    let source = format!(
        "\\section{{Parent}}\\subsection{{Child}}😀 \\label{{needle}}{}",
        (0..300)
            .rev()
            .map(|n| format!("\\label{{needle:{n:03}}}"))
            .collect::<String>()
    );
    workspace.open(main.clone(), source.clone(), 1);
    let found = navigation::workspace_symbols(&workspace, "needle");
    assert_eq!(found.len(), 256);
    assert_eq!(found[0].name, "needle");
    assert_eq!(found[255].name, "needle:254");
    assert!(found
        .iter()
        .all(|symbol| symbol.container_name.as_deref() == Some("Child")));
    for symbol in &found {
        let text = &workspace.documents[&main].text;
        let range = symbol.location.range;
        assert_eq!(
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            symbol.name
        );
    }
    assert_eq!(navigation::workspace_symbols(&workspace, "needle"), found);
    // Editing an unsaved source must release its old outline; clones used by
    // in-flight requests must retain the original source and positions.
    let snapshot = workspace.clone();
    workspace.open(
        main.clone(),
        "\r\n😀 \\section{Changed}\\label{fresh}".into(),
        2,
    );
    assert!(navigation::workspace_symbols(&workspace, "needle").is_empty());
    let fresh = navigation::workspace_symbols(&workspace, "fresh");
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].container_name.as_deref(), Some("Changed"));
    assert_eq!(fresh[0].location.range.start.line, 1);
    assert_eq!(navigation::workspace_symbols(&snapshot, "needle"), found);
    // Reopening the same URI with a different language also replaces outlines.
    workspace.open_with_language(
        main.clone(),
        "@book{entry,title={Text}}".into(),
        3,
        "bibtex",
    );
    assert!(navigation::workspace_symbols(&workspace, "fresh").is_empty());
    let entries = navigation::workspace_symbols(&workspace, "");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "entry");
    assert_eq!(entries[0].location.uri, main);
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
fn hyperref_external_links_include_static_destinations_and_keep_label_text_out_of_url_links() {
    let uri = Url::parse("file:///navigation/hyperref.tex").unwrap();
    let cases = [
        (r"\hyperref{https://example.com/a%20b}{}{section.1}{One}", "https://example.com/a%20b#section.1"),
        (r"\hyperref{https://example.com/}{section}{1}{Two}", "https://example.com/#section.1"),
        (r"\hyperref{https://example.com/#old}{section}{2}{Three}", "https://example.com/#old#section.2"),
        (r"\hyperref{https://example.com/}{}{}{Four}", "https://example.com/"),
        (r"\hyperref{mailto:a@example.com}{section}{1}{Five}", "mailto:a@example.com#section.1"),
        (r"\hyperref{ftp://example.com/a}{category}{label}{Six}", "ftp://example.com/a#category.label"),
        ("😀 \\hyperref % link\r\n{https://example.com/a\\_b?q=1\\&x=2} % target\r\n{section}{intro_one}{Text}", "https://example.com/a_b?q=1&x=2#section.intro_one"),
        (r"\hyperref{https://example.com/}{section}{}{Text}", "https://example.com/#section."),
    ];
    for (source, expected) in cases {
        let mut workspace = Workspace::default();
        insert(&mut workspace, &uri, source);
        let doc = &workspace.documents[&uri];
        assert!(features::diagnostics(doc).is_empty(), "{source}");
        let links = navigation::document_links(&workspace, &uri);
        assert_eq!(links.len(), 1, "{source}: {links:?}");
        assert_eq!(
            links[0].target.as_ref().unwrap().as_str(),
            expected,
            "{source}"
        );
        let start = doc.text.offset(links[0].range.start).unwrap();
        let end = doc.text.offset(links[0].range.end).unwrap();
        assert!(
            source[start..end].starts_with("https://")
                || source[start..end].starts_with("mailto:")
                || source[start..end].starts_with("ftp://")
        );
        assert!(!source[start..end].contains('{'));
    }
    for source in [
        r"\hyperref[sec:intro]{https://example.com/}",
        r"\hyperref[{sec:intro}]{https://example.com/}",
        r"\hyperref*{https://example.com/}{section}{1}{Text}",
        r"\hyperref{https://example.com/}{\category}{1}{Text}",
        r"\hyperref{https://example.com/}{section}{\name}{Text}",
        r"\hyperref{https://example.com/}{section}{intro\_one}{Text}",
        r"\hyperref{https://example.com/\suffix}{section}{1}{Text}",
        r"\hyperref{https://example.com/}{section}",
        r"\hyperref{https://example.com/}{section}{unclosed",
        r"\hyperref{}{section}{1}{Internal link}",
        r"\hyperref{javascript:alert(1)}{}{}{Text}",
        r"% \hyperref{https://example.com/}{section}{1}{Text}",
        r"\verb|\hyperref{https://example.com/}{section}{1}{Text}|",
    ] {
        let mut workspace = Workspace::default();
        insert(&mut workspace, &uri, source);
        assert!(
            navigation::document_links(&workspace, &uri).is_empty(),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_hyperref_document_links_match_the_external_uri_annotations_in_the_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = concat!(
        "\\pdfobjcompresslevel=0\\pdfcompresslevel=0\n\\documentclass{article}\\usepackage{hyperref}\\begin{document}\n",
        "\\hyperref{https://example.com/a%20b}{}{section.1}{One}\n",
        "\\hyperref{https://example.com/}{section}{1}{Two}\n",
        "\\hyperref{https://example.com/#old}{section}{2}{Three}\n",
        "\\hyperref{https://example.com/}{}{}{Four}\n",
        "\\hyperref{mailto:a@example.com}{section}{1}{Five}\n",
        "\\hyperref{ftp://example.com/a}{category}{label}{Six}\n",
        "\\hyperref{https://example.com/a\\_b?q=1\\&x=2}{section}{intro_one}{Seven}\n",
        "\\hyperref{https://example.com/}{section}{}{Eight}\n",
        "\\end{document}\n",
    );
    std::fs::write(&main, source).unwrap();
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let result = texquill::compilation::build::run(&main, &config, &Default::default());
    assert_eq!(result.status, "success", "{}", result.output);
    let data = std::fs::read(config.artifact_path(&main, "pdf").unwrap()).unwrap();
    let pdf = String::from_utf8_lossy(&data);
    let targets: Vec<_> = pdf
        .split("/URI")
        .skip(1)
        .filter_map(|tail| {
            tail.trim_start()
                .strip_prefix('(')?
                .split_once(')')
                .map(|(uri, _)| uri)
        })
        .collect();
    assert_eq!(targets.len(), 8);
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let links = navigation::document_links(&workspace, &Url::from_file_path(&main).unwrap());
    assert_eq!(
        links
            .iter()
            .map(|link| link.target.as_ref().unwrap().as_str())
            .collect::<Vec<_>>(),
        targets
    );
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
