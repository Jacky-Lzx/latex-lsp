use latex_lsp::workspace::Workspace;

#[test]
fn index_limits_report_truncation_and_exclude_build_directories() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.tex", "b.tex"] {
        std::fs::write(dir.path().join(name), "text").unwrap();
    }
    std::fs::create_dir(dir.path().join("build")).unwrap();
    std::fs::write(dir.path().join("build/hidden.tex"), "text").unwrap();
    let mut ws = Workspace::default();
    ws.config.indexing.max_files = 1;
    ws.add_root(dir.path().into());
    assert_eq!(ws.documents.len(), 1);
    assert!(ws.index_truncated.contains("maxFiles"));
    let mut ws = Workspace::default();
    ws.add_root(dir.path().into());
    assert_eq!(ws.documents.len(), 2);
    assert!(ws.index_truncated.is_empty());
}

#[test]
fn total_bytes_file_size_and_directory_depth_have_independent_budgets() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.tex"), "12345678").unwrap();
    std::fs::write(dir.path().join("b.tex"), "12345678").unwrap();
    let mut ws = Workspace::default();
    ws.config.indexing.max_total_bytes = 10;
    ws.add_root(dir.path().into());
    assert_eq!(ws.documents.len(), 1);
    assert!(ws.index_truncated.contains("maxTotalBytes"));
    let mut ws = Workspace::default();
    ws.config.indexing.max_file_size = 4;
    ws.add_root(dir.path().into());
    assert!(ws.documents.is_empty());
    assert!(ws.index_truncated.contains("maxFileSize"));
    std::fs::create_dir_all(dir.path().join("one/two")).unwrap();
    std::fs::write(dir.path().join("one/two/c.tex"), "x").unwrap();
    let mut ws = Workspace::default();
    ws.config.indexing.max_depth = 1;
    ws.add_root(dir.path().into());
    assert_eq!(ws.documents.len(), 2);
    assert!(ws.index_truncated.contains("maxDepth"));
}
