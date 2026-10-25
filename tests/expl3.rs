use latex_lsp::{
    features, semantic, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;
fn changed(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
        .source
}
#[test]
fn project_functions_and_variables_have_complete_names_across_package_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let package = r"\ProvidesExplPackage{demo}{2026-10-23}{1}{Demo}\tl_new:N \l_demo_text_tl \cs_new:Npn \demo_use:nn #1#2 { \tl_set:Nn \l_demo_text_tl {#1} \tl_use:N \l_demo_text_tl #2 }\cs_new_eq:NN \demo_alias:nn \demo_use:nn";
    std::fs::write(dir.path().join("demo.sty"), package).unwrap();
    let source="\\documentclass{article}\\usepackage{demo}\n\\ExplSyntaxOn\n😀 \\demo_use:nn {one} {two} \\tl_use:N \\l_demo_text_tl\n\\ExplSyntaxOff\n\\demo_use:nn";
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 8);
    let text = Text::new(source.into());
    let at = text.position(source.find("demo_use:nn").unwrap() + 6);
    let definitions = features::definitions(&ws, &uri, at);
    assert_eq!(definitions.len(), 1, "{definitions:?}");
    assert!(definitions[0].uri.path().ends_with("demo.sty"));
    assert_eq!(features::references(&ws, &uri, at, true).len(), 3);
    let changes = features::rename(&ws, &uri, at, "demo_render:nn")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        changed(source, changes[&uri].clone()),
        source.replacen("demo_use:nn", "demo_render:nn", 1)
    );
    assert_eq!(
        changed(package, changes[&definitions[0].uri].clone()),
        package.replace("demo_use:nn", "demo_render:nn")
    );
    assert!(features::rename(&ws, &uri, at, "demo_render:n").is_err());
    assert!(features::rename(&ws, &uri, at, "demo_alias:nn").is_err());
    let var = text.position(source.find("l_demo_text_tl").unwrap() + 8);
    assert_eq!(features::references(&ws, &uri, var, true).len(), 4);
    assert_eq!(features::definitions(&ws, &uri, var).len(), 1);
    assert!(features::rename(&ws, &uri, var, "g_demo_text_tl").is_err());
    assert!(features::rename(&ws, &uri, var, "l_demo_text_int").is_err());
    assert!(features::rename(&ws, &uri, var, "l_demo_content_tl")
        .unwrap()
        .is_some());
    assert!(format!("{:?}", features::hover(&ws, &uri, at)).contains("cs_new:Npn"));
    let hint = signature::help(&ws, &uri, text.position(source.find("two").unwrap() + 1)).unwrap();
    assert!(hint.signatures[0].label.contains("demo_use:nn"));
    assert_eq!(hint.active_parameter, Some(1));
    let items = features::completion(&ws, &uri, at);
    assert!(items.iter().any(|i| i.label == "demo_use:nn"));
    let Some(CompletionTextEdit::Edit(edit)) = &items
        .iter()
        .find(|i| i.label == "demo_use:nn")
        .unwrap()
        .text_edit
    else {
        panic!()
    };
    assert_eq!(
        edit.range,
        text.range(
            source.find("demo_use:nn").unwrap(),
            source.find("demo_use:nn").unwrap() + 11
        )
    );
}
#[test]
fn mode_groups_deferred_switches_comments_and_raw_text_are_respected() {
    let uri = Url::parse("file:///expl3/modes.tex").unwrap();
    let source="\\newcommand{\\switchlater}{\\ExplSyntaxOn}\n{\\ExplSyntaxOn \\tl_new:N \\l_demo_tl \\cs_new:Npn \\demo_func:n #1 {\\ExplSyntaxOff \\tl_use:N \\l_demo_tl #1} \\demo_func:n {yes} % \\demo_func:n\n\\verb|\\demo_func:n|}\n\\demo_func:n\n\\tl_use:N \\l_demo_tl";
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 1);
    let doc = &ws.documents[&uri];
    let at = doc.text.position(source.find("demo_func:n").unwrap() + 5);
    assert_eq!(features::references(&ws, &uri, at, true).len(), 2);
    let change = features::rename(&ws, &uri, at, "demo_next:n")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(change[&uri].len(), 2);
    let updated = changed(source, change[&uri].clone());
    assert!(updated.contains("% \\demo_func:n"));
    assert!(updated.contains("\\verb|\\demo_func:n|"));
    assert!(updated.ends_with("\\demo_func:n\n\\tl_use:N \\l_demo_tl"));
    let names: Vec<_> = features::symbols(doc).into_iter().map(|s| s.name).collect();
    assert!(names.contains(&"demo_func:n".into()));
    assert!(names.contains(&"l_demo_tl".into()));
}
#[test]
fn builtins_complete_hint_and_highlight_as_whole_control_sequences() {
    let uri = Url::parse("file:///expl3/semantic.tex").unwrap();
    let source="\\ExplSyntaxOn\n\\tl_new:N \\l_demo_tl\n😀 \\tl_set:Nn \\l_demo_tl {hello}\n\\tl_se\n\\ExplSyntaxOff";
    let mut ws = Workspace::default();
    ws.documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(2)));
    let doc = &ws.documents[&uri];
    let cursor = doc.text.position(source.find("tl_se\n").unwrap() + 5);
    assert!(features::completion(&ws, &uri, cursor)
        .iter()
        .any(|i| i.label == "tl_set:Nn"));
    let help = signature::help(
        &ws,
        &uri,
        doc.text.position(source.find("hello").unwrap() + 2),
    )
    .unwrap();
    assert!(help.signatures[0].label.contains("tl_set:Nn"));
    assert_eq!(help.active_parameter, Some(1));
    let mut position = Position::new(0, 0);
    let mut tokens = Vec::new();
    let legend = semantic::options().legend;
    for token in semantic::tokens(doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        let end = Position::new(position.line, position.character + token.length);
        tokens.push((
            doc.text.source[doc.text.offset(position).unwrap()..doc.text.offset(end).unwrap()]
                .to_string(),
            legend.token_types[token.token_type as usize]
                .as_str()
                .to_string(),
            token.token_modifiers_bitset,
        ));
    }
    assert!(
        tokens.contains(&("\\tl_set:Nn".into(), "macro".into(), 0)),
        "{tokens:?}"
    );
    assert!(
        tokens.contains(&("\\l_demo_tl".into(), "variable".into(), 1)),
        "{tokens:?}"
    );
    assert!(
        tokens.contains(&("\\l_demo_tl".into(), "variable".into(), 0)),
        "{tokens:?}"
    );
}
#[test]
fn constants_set_definitions_and_delimited_parameter_text_stay_conservative() {
    let uri = Url::parse("file:///expl3/parameters.tex").unwrap();
    let source = r"\ExplSyntaxOn \tl_const:Nn \c_demo_text_tl {fixed} \cs_set:Npn \demo_delimited:n #1, { \ExplSyntaxOff #1 } \demo_delimited:n text, \tl_use:N \c_demo_text_tl \cs_new:Nn \demo_simple:nn {#1#2} \demo_simple:nn {first}{second} \cs_new:cpn {dynamic:n} #1 {#1}";
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 1);
    let doc = &ws.documents[&uri];
    let at = doc
        .text
        .position(source.find("c_demo_text_tl").unwrap() + 5);
    assert_eq!(features::references(&ws, &uri, at, true).len(), 2);
    let cursor = doc.text.position(source.find("text,").unwrap() + 2);
    assert!(signature::help(&ws, &uri, cursor).is_none());
    let cursor = doc.text.position(source.find("second").unwrap() + 2);
    assert_eq!(
        signature::help(&ws, &uri, cursor).unwrap().active_parameter,
        Some(1)
    );
    assert!(!features::symbols(doc).iter().any(|s| s.name == "dynamic:n"));
}
#[test]
#[ignore = "requires pdflatex"]
fn real_expl3_renames_preserve_expansion_results() {
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let original="\\documentclass{article}\n\\ExplSyntaxOn\n\\tl_new:N \\l_demo_text_tl\n\\tl_set:Nn \\l_demo_text_tl {before}\n\\cs_new:Npn \\demo_identity:n #1 {#1}\n\\typeout{EXPLRESULT=\\demo_identity:n{\\tl_use:N \\l_demo_text_tl}}\n\\ExplSyntaxOff\n\\begin{document}Done\\end{document}";
    let mut ws = Workspace::default();
    ws.open(uri.clone(), original.into(), 1);
    let mut source = original.to_string();
    for (old, new) in [
        ("demo_identity:n", "demo_renamed:n"),
        ("l_demo_text_tl", "l_demo_content_tl"),
    ] {
        let at = Text::new(source.clone()).position(source.find(old).unwrap() + 3);
        let edits = features::rename(&ws, &uri, at, new)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        source = changed(&source, edits[&uri].clone());
        ws.open(uri.clone(), source.clone(), 2);
    }
    for (name, content) in [("original", original), ("renamed", source.as_str())] {
        std::fs::write(dir.path().join(format!("{name}.tex")), content).unwrap();
        let output = std::process::Command::new("pdflatex")
            .current_dir(dir.path())
            .args([
                "-interaction=nonstopmode",
                "-halt-on-error",
                &format!("{name}.tex"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("EXPLRESULT=before"));
    }
}
