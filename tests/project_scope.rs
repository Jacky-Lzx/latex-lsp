use latex_lsp::{
    features, signature,
    workspace::{Document, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project() -> (tempfile::TempDir, Workspace) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(format!("{name}/main.tex")),"\\documentclass{article}\n\\usepackage{macros}\n\\begin{document}\n\\input{chapter}\n\\ref{sharedkey} \\cite{book} \\pair{first}{second}\n\\begin{panel}{first}{second}\\end{panel}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
        std::fs::write(
            root.join(format!("{name}/chapter.tex")),
            format!("\\label{{sharedkey}}\\label{{{name}:only}}\n"),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{name}/refs.bib")),
            format!(
                "@book{{book,title={{Shared name}}}}\n@book{{{name}onlybook,title={{Local}}}}\n"
            ),
        )
        .unwrap();
        let count = if name == "a" { 2 } else { 1 };
        std::fs::write(root.join(format!("{name}/macros.sty")),format!("\\newcommand{{\\pair}}[{count}]{{#1}}\n\\newcommand{{\\{name}onlycommand}}{{Local}}\n\\newenvironment{{panel}}[{count}]{{#1}}{{}}\n\\newenvironment{{{name}onlyenvironment}}{{}}{{}}\n")).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    (directory, workspace)
}

#[test]
fn definitions_references_hover_and_signatures_respect_the_active_paper() {
    let (directory, workspace) = project();
    let root = directory.path();
    let main = uri(root, "a/main.tex");
    let text = &workspace.documents[&main].text;
    for (needle, offset, target_file) in [
        ("ref{sharedkey}", 5, "a/chapter.tex"),
        ("cite{book}", 6, "a/refs.bib"),
        ("\\pair{first}", 2, "a/macros.sty"),
        ("begin{panel}", 7, "a/macros.sty"),
    ] {
        let position = text.position(text.source.find(needle).unwrap() + offset);
        let definitions = features::definitions(&workspace, &main, position);
        assert_eq!(definitions.len(), 1, "{needle}: {definitions:?}");
        assert_eq!(definitions[0].uri, uri(root, target_file));
        let references = features::references(&workspace, &main, position, true);
        assert!(
            references.iter().all(|location| location
                .uri
                .to_file_path()
                .unwrap()
                .starts_with(root.join("a"))),
            "{references:?}"
        );
        let hover = features::hover(&workspace, &main, position).unwrap();
        let HoverContents::Markup(MarkupContent { value, .. }) = hover.contents else {
            panic!();
        };
        assert!(value.contains("/a/"), "{value}");
    }
    let position = text.position(text.source.find("second}").unwrap() + 3);
    let help = signature::help(&workspace, &main, position).unwrap();
    assert_eq!(help.signatures[0].label, r"\pair{#1}{#2}");
    assert_eq!(help.active_parameter, Some(1));
    let position = text.position(text.source.rfind("second}").unwrap() + 3);
    let help = signature::help(&workspace, &main, position).unwrap();
    assert_eq!(help.signatures[0].label, r"\begin{panel}{#1}{#2}");
    assert_eq!(help.active_parameter, Some(2));
}

#[test]
fn completions_and_user_snippets_exclude_symbols_from_independent_papers() {
    let (directory, mut workspace) = project();
    let root = directory.path();
    let main = uri(root, "a/main.tex");
    workspace.snippets = true;
    let prefix =
        "\\documentclass{article}\\usepackage{macros}\\input{chapter}\\bibliography{refs}\n";
    for (suffix, present, absent) in [
        (r"\ref{", "a:only", "b:only"),
        (r"\cite{", "aonlybook", "bonlybook"),
        (r"\begin{", "aonlyenvironment", "bonlyenvironment"),
        (r"\", "aonlycommand", "bonlycommand"),
    ] {
        let source = format!("{prefix}{suffix}");
        let position = latex_lsp::text::Text::new(source.clone()).position(source.len());
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, source, Some(2)));
        let items = features::completion(&workspace, &main, position);
        assert!(
            items.iter().any(|item| item.label == present),
            "{suffix}: {items:?}"
        );
        assert!(
            !items.iter().any(|item| item.label == absent),
            "{suffix}: {items:?}"
        );
    }
    let source = format!("{prefix}\\pa");
    let position = latex_lsp::text::Text::new(source.clone()).position(source.len());
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source, Some(3)));
    let items = features::completion(&workspace, &main, position);
    let pair = items.iter().find(|item| item.label == "pair").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &pair.text_edit else {
        panic!();
    };
    assert_eq!(edit.new_text, "pair{${1:arg1}}{${2:arg2}}");
    let bib = uri(root, "a/refs.bib");
    let source = "@book{book,title={Example}}\n@book{aonlybook,title={Local}}\n@incollection{child,crossref={}}";
    workspace
        .documents
        .insert(bib.clone(), Document::new(&bib, source.into(), Some(1)));
    let position = workspace.documents[&bib]
        .text
        .position(source.rfind("{}").unwrap() + 1);
    let items = features::completion(&workspace, &bib, position);
    assert!(items.iter().any(|item| item.label == "aonlybook"));
    assert!(!items.iter().any(|item| item.label == "bonlybook"));
}
