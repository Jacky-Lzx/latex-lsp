use latex_lsp::{editing, workspace::Workspace};
use tower_lsp::lsp_types::Url;

#[test]
fn first_open_without_a_workspace_resolves_the_editor_text_and_imported_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.tex");
    std::fs::write(
        &path,
        r"\documentclass{article}\begin{document}Saved text\end{document}",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("aliases.sty"),
        r"\def\load{\input}\def\start{\begin}\def\finish{\end}",
    )
    .unwrap();
    std::fs::write(directory.path().join("chapter.tex"), "Chapter").unwrap();
    let source = r"\documentclass{article}\usepackage{aliases}\load{chapter}\begin{document}\start{panel}Editor text\finish{panel}\end{document}";
    let uri = Url::from_file_path(&path).unwrap();
    let mut workspace = Workspace::default();
    workspace.open_with_language(uri.clone(), source.into(), 3, "latex");
    let doc = &workspace.documents[&uri];
    assert_eq!(doc.text.source, source);
    assert_eq!(doc.version, Some(3));
    let chapter = Url::from_file_path(directory.path().join("chapter.tex")).unwrap();
    assert!(workspace.project_documents(&uri).contains(&chapter));
    let names: Vec<_> = source.match_indices("panel").map(|(at, _)| at).collect();
    let pair =
        editing::linked_editing_ranges_in_workspace(&workspace, &uri, doc.text.position(names[0]))
            .unwrap();
    assert_eq!(
        pair.ranges,
        names
            .iter()
            .map(|at| doc.text.range(*at, at + 5))
            .collect::<Vec<_>>()
    );
}
