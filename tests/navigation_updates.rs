use latex_lsp::{
    language::features,
    project::workspace::{Document, Workspace},
};
use lsp_types::Url;

#[test]
fn navigation_and_rename_follow_unsaved_dependency_changes() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let chapter_a = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let chapter_b = Url::from_file_path(directory.path().join("b.tex")).unwrap();
    for uri in [&chapter_a, &chapter_b] {
        std::fs::write(
            uri.to_file_path().unwrap(),
            r"\section{Chapter}\label{shared}",
        )
        .unwrap();
    }
    let source = |chapter| {
        format!(
            r"\documentclass{{article}}\input{{{chapter}}}\begin{{document}}\ref{{shared}}\end{{document}}"
        )
    };
    std::fs::write(main.to_file_path().unwrap(), source("a")).unwrap();
    let mut workspace = Workspace::default();
    for (version, chapter, expected, excluded) in [
        (1, "a", &chapter_a, &chapter_b),
        (2, "b", &chapter_b, &chapter_a),
        (3, "a", &chapter_a, &chapter_b),
    ] {
        // Preserve the on-disk root: each query must use this unsaved version.
        let text = source(chapter);
        let position = Document::new(&main, text.clone(), Some(version))
            .text
            .position(text.rfind("shared").unwrap() + 2);
        workspace.open(main.clone(), text, version);
        let definitions = features::definitions(&workspace, &main, position);
        assert_eq!(definitions.len(), 1);
        assert_eq!(&definitions[0].uri, expected);
        let references = features::references(&workspace, &main, position, true);
        assert_eq!(references.len(), 2);
        assert!(references.iter().any(|location| &location.uri == expected));
        assert!(!references.iter().any(|location| &location.uri == excluded));
        assert!(features::hover(&workspace, &main, position).is_some());
        assert!(features::prepare_rename(&workspace, &main, position)
            .unwrap()
            .is_some());
        let edit = features::rename(&workspace, &main, position, "updated")
            .unwrap()
            .unwrap();
        let changes = edit.changes.unwrap();
        assert!(changes.contains_key(expected));
        assert!(changes.contains_key(&main));
        assert!(!changes.contains_key(excluded));
    }
}
