use latex_lsp::{
    language::{features, navigation},
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;

#[test]
fn global_resources_join_project_dependencies_and_citation_key_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        "\\documentclass{article}\n\\addglobalbib[bibencoding=utf8]{refs.bib}\n\\cite{alpha}\n";
    let bibliography = "@book{alpha,title={Literal alpha}}\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("refs.bib"), bibliography).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let bib = Url::from_file_path(root.join("refs.bib")).unwrap();
    assert!(workspace.dependencies(&main).contains(&bib));
    assert_eq!(
        navigation::document_links(&workspace, &main)
            .iter()
            .filter(|link| link.target.as_ref() == Some(&bib))
            .count(),
        1
    );
    let at = Text::new(source.into()).position(source.find("alpha").unwrap() + 2);
    assert_eq!(features::definitions(&workspace, &main, at)[0].uri, bib);
    let edits = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&main].len(), 1);
    assert_eq!(edits[&bib].len(), 1);
}

fn uri(root: &std::path::Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn fixture(source: &str, name: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join(name),
        "@book{alpha,title={Literal alpha}}\n\\newcommand\\leaked{}\n",
    )
    .unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let bib = uri(root, name);
    (directory, workspace, main, bib)
}
fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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
fn form(name: &str, style: &str) -> String {
    match style {
        "direct" => format!("\\{name}"),
        "named" => format!("\\csuse{{{name}}}"),
        "literal" => format!("\\csname {name}\\endcsname"),
        "alias" => "\\load".into(),
        "named-alias" => "\\csuse{load}".into(),
        "literal-alias" => "\\csname load\\endcsname".into(),
        _ => panic!(),
    }
}
const FORMS: &[&str] = &[
    "direct",
    "named",
    "literal",
    "alias",
    "named-alias",
    "literal-alias",
];

#[test]
fn local_resources_and_indirect_forms_keep_data_roles_and_participate_in_key_and_file_renames() {
    for name in ["addbibresource", "addglobalbib"] {
        for style in FORMS {
            let source = format!("\\documentclass{{article}}\\def\\load{{\\{name}}}\n{}% comment before options\r\n[bibencoding={{utf8}},label={{\\emph{{refs, metadata}}}}]{{refs,part.bib}}\n\\cite{{alpha}}\\leaked\n", form(name, style));
            let (directory, workspace, main, bib) = fixture(&source, "refs,part.bib");
            assert!(
                workspace.dependencies(&main).contains(&bib),
                "{style}/{name}"
            );
            assert_eq!(
                workspace.documents[&bib].kind,
                latex_lsp::project::workspace::DocumentKind::Bibtex
            );
            let links = navigation::document_links(&workspace, &main);
            assert_eq!(links.len(), 1, "{style}/{name}: {links:?}");
            assert_eq!(links[0].target.as_ref(), Some(&bib));
            let text = Text::new(source.clone());
            assert_eq!(
                &source[text.offset(links[0].range.start).unwrap()
                    ..text.offset(links[0].range.end).unwrap()],
                "refs,part.bib"
            );
            let at = text.position(source.find("{alpha}").unwrap() + 2);
            assert_eq!(features::definitions(&workspace, &main, at)[0].uri, bib);
            assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
            let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(
                apply(&source, &changes[&main]),
                source.replace("{alpha}", "{renamed}")
            );
            assert!(features::definitions(
                &workspace,
                &main,
                text.position(source.rfind("\\leaked").unwrap() + 2)
            )
            .is_empty());
            let edit = latex_lsp::editing::file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: bib.to_string(),
                    new_uri: uri(directory.path(), "new%data.bib").to_string(),
                }],
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                apply(&source, &edit.changes.unwrap()[&main]),
                source.replace("refs,part.bib", "new%data.bib"),
                "{style}/{name}"
            );
        }
    }
}

#[test]
fn resource_completion_keeps_full_names_suffixes_options_and_other_text() {
    for name in ["addbibresource", "addglobalbib"] {
        for style in FORMS {
            for tail in ["[label=metadata]{ref¦s,part.bib}", "{ref¦"] {
                let marked = format!(
                    "\\documentclass{{article}}\\def\\load{{\\{name}}}\n{}{tail}",
                    form(name, style)
                );
                let source = marked.replace('¦', "");
                let (_directory, workspace, main, _bib) = fixture(&source, "refs,part.bib");
                let at = Text::new(source.clone()).position(marked.find('¦').unwrap());
                let items = features::completion(&workspace, &main, at);
                let item = items
                    .iter()
                    .find(|item| item.label == "refs,part.bib")
                    .unwrap_or_else(|| panic!("{marked}: {items:?}"));
                let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                    panic!()
                };
                assert_eq!(
                    apply(&source, std::slice::from_ref(edit)),
                    marked
                        .replace("ref¦s,part.bib", "refs,part.bib")
                        .replace("ref¦", "refs,part.bib"),
                    "{marked}"
                );
            }
        }
        for filename in ["refs%part.bib", "refs#&$^~.bib", "refs 中文.bib"] {
            let source =
                format!("\\documentclass{{article}}\\{name}{{{filename}}}\\cite{{alpha}}\n");
            let (directory, workspace, main, bib) = fixture(&source, filename);
            assert!(workspace.dependencies(&main).contains(&bib));
            assert!(
                workspace.documents[&main].syntax.comments.is_empty(),
                "{source}"
            );
            assert!(
                workspace.documents[&main].syntax.diagnostics.is_empty(),
                "{source}: {:?}",
                workspace.documents[&main].syntax.diagnostics
            );
            let text = Text::new(source.clone());
            let at = text.position(source.find("{alpha}").unwrap() + 2);
            assert_eq!(features::definitions(&workspace, &main, at)[0].uri, bib);
            let at = text.position(source.find(filename).unwrap() + 3);
            let items = features::completion(&workspace, &main, at);
            let item = items
                .iter()
                .find(|item| item.label == filename)
                .unwrap_or_else(|| panic!("{source}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(apply(&source, std::slice::from_ref(edit)), source);
            let edits = latex_lsp::editing::file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: bib.to_string(),
                    new_uri: uri(directory.path(), "renamed%#&.bib").to_string(),
                }],
            )
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
            assert_eq!(
                apply(&source, &edits[&main]),
                source.replace(filename, "renamed%#&.bib")
            );
        }
    }
}

#[test]
fn unsupported_resources_do_not_borrow_local_files_or_treat_registration_as_loading() {
    for name in ["addbibresource", "addglobalbib"] {
        for tail in [
            "*{refs.bib}",
            "[label=x][extra]{refs.bib}",
            "[location=remote]{refs.bib}",
            "[type=unknown]{refs.bib}",
            "[datatype=ris]{refs.bib}",
            "[datatype=\\format]{refs.bib}",
            "[bibencoding=\\encoding]{refs.bib}",
            "[glob]{refs.bib}",
            "[glob=true]{refs.bib}",
            "[unknown=x]{refs.bib}",
            "{ refs.bib}",
            "{refs.bib }",
            "{refs\n.bib}",
            "{\\resource}",
            "{refs}",
        ] {
            let source = format!("\\documentclass{{article}}\\{name}{tail}");
            let (_directory, workspace, main, bib) = fixture(&source, "refs.bib");
            assert!(!workspace.dependencies(&main).contains(&bib), "{source}");
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{source}"
            );
        }
        for context in [
            format!("\\renewcommand\\{name}[1]{{}}\\{name}{{refs.bib}}"),
            format!("\\newcommand\\hold[1]{{}}\\hold\\{name}{{refs.bib}}"),
            format!("\\verb|\\{name}{{refs.bib}}|"),
            format!("% \\{name}{{refs.bib}}"),
        ] {
            let source = format!("\\documentclass{{article}}{context}");
            let (_directory, workspace, main, _bib) = fixture(&source, "refs.bib");
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{source}"
            );
        }
    }
    let (_directory, workspace, main, bib) = fixture(
        "\\documentclass{article}\\addsectionbib{refs.bib}",
        "refs.bib",
    );
    assert!(!workspace.dependencies(&main).contains(&bib));
}

#[test]
fn snippets_and_signatures_use_one_optional_options_group_and_the_full_filename() {
    for name in ["addbibresource", "addglobalbib"] {
        for following in ["", "{refs.bib}", "[label=refs]{refs.bib}"] {
            let source = format!("\\{name}{following}");
            let (_directory, mut workspace, main, _bib) = fixture(&source, "refs.bib");
            workspace.client.snippets = true;
            let items = features::completion(&workspace, &main, Position::new(0, 4));
            let item = items.iter().find(|item| item.label == name).unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                edit.new_text,
                if following.is_empty() {
                    format!("{name}{{${{1:references.bib}}}}")
                } else {
                    name.into()
                }
            );
        }
        let source = format!("\\{name}[label=refs]{{refs.bib}}");
        let (_directory, workspace, main, _bib) = fixture(&source, "refs.bib");
        let help = latex_lsp::language::signature::help(
            &workspace,
            &main,
            Text::new(source.clone()).position(source.find("refs.bib").unwrap() + 2),
        )
        .unwrap();
        assert_eq!(help.active_parameter, Some(1));
        assert_eq!(
            help.signatures[0].label,
            format!("\\{name}[options]{{file}}")
        );
    }
}

#[test]
fn shared_global_bibliography_renames_leave_independent_projects_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b", "other"] {
        std::fs::write(
            root.join(format!("{name}.tex")),
            format!(
                "\\documentclass{{article}}\\addglobalbib{{{}.bib}}\\cite{{alpha}}",
                if name == "other" { "other" } else { "shared" }
            ),
        )
        .unwrap();
    }
    for name in ["shared", "other"] {
        std::fs::write(
            root.join(format!("{name}.bib")),
            "@book{alpha,title={Alpha}}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let bib = uri(root, "shared.bib");
    let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    assert!(changes.contains_key(&uri(root, "a.tex")));
    assert!(changes.contains_key(&uri(root, "b.tex")));
    assert!(!changes.contains_key(&uri(root, "other.tex")));
    assert!(!changes.contains_key(&uri(root, "other.bib")));
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_global_resource_file_and_key_renames_compile_in_alternative_reference_sections() {
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source="\\documentclass{article}\n\\usepackage[backend=bibtex,style=numeric]{biblatex}\n\\addglobalbib[bibencoding=utf8]{global.bib}\n\\addsectionbib{unused.bib}\n\\begin{document}\n\\cite{alpha}\\printbibliography\n\\begin{refsection}[other.bib]\n\\cite{alpha,beta}\\printbibliography\n\\end{refsection}\n\\end{document}\n".to_string();
    let bibliography = "@book{alpha,author={Alpha, Alice},title={Literal alpha},year={2020}}\n";
    fs::write(root.join("main.tex"), &source).unwrap();
    fs::write(root.join("global.bib"), bibliography).unwrap();
    fs::write(
        root.join("other.bib"),
        "@book{beta,author={Beta, Bob},title={Other},year={2021}}\n",
    )
    .unwrap();
    fs::write(
        root.join("unused.bib"),
        "@book{alpha,title={Unused independent alpha}}\n",
    )
    .unwrap();
    let main = uri(root, "main.tex");
    let bib = uri(root, "global.bib");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(!workspace
        .dependencies(&main)
        .contains(&uri(root, "unused.bib")));
    let renamed = uri(root, "renamed.bib");
    let edits = latex_lsp::editing::file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: bib.to_string(),
            new_uri: renamed.to_string(),
        }],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    source = apply(&source, &edits[&main]);
    fs::rename(root.join("global.bib"), root.join("renamed.bib")).unwrap();
    fs::write(root.join("main.tex"), &source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let edits = features::rename(&workspace, &renamed, Position::new(0, 8), "renamed_key")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&main].len(), 2);
    assert!(!edits.contains_key(&uri(root, "unused.bib")));
    source = apply(&source, &edits[&main]);
    fs::write(root.join("main.tex"), &source).unwrap();
    let renamed_bib = apply(bibliography, &edits[&renamed]);
    assert!(renamed_bib.contains("Literal alpha"));
    fs::write(root.join("renamed.bib"), renamed_bib).unwrap();
    let run = |program: &str, args: &[&str]| {
        let output = Command::new(program)
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let tex = || {
        run(
            "pdflatex",
            &["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
        )
    };
    tex();
    let mut databases = 0;
    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|s| s == "aux")
            && fs::read_to_string(&path).unwrap().contains("\\bibdata{")
        {
            run("bibtex", &[path.file_stem().unwrap().to_str().unwrap()]);
            databases += 1;
        }
    }
    assert_eq!(databases, 2);
    tex();
    tex();
    let log = fs::read_to_string(root.join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
    assert!(fs::metadata(root.join("main.pdf")).unwrap().len() > 1000);
}

#[test]
#[ignore = "requires pdflatex, biblatex and logreq"]
fn real_biber_resource_requests_keep_exact_names_and_literal_percent_characters() {
    use std::{fs, process::Command};
    let source="\\documentclass{article}\n\\usepackage[backend=biber]{biblatex}\n\\addbibresource{refs}\n\\addglobalbib{literal%name.bib}\\label{after-resource}\n\\begin{document}Text.\\end{document}\n";
    let (directory, workspace, main, bib) = fixture(source, "literal%name.bib");
    let root = directory.path();
    fs::write(root.join("refs.bib"), "@book{ignored,title={Ignored}}\n").unwrap();
    assert!(workspace.dependencies(&main).contains(&bib));
    assert!(!workspace
        .dependencies(&main)
        .contains(&uri(root, "refs.bib")));
    let output = Command::new("pdflatex")
        .args(["-interaction=nonstopmode", "-halt-on-error", "main.tex"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let control = fs::read_to_string(root.join("main.bcf")).unwrap();
    assert!(control.contains(">refs</bcf:datasource>"));
    assert!(control.contains(">literal%name.bib</bcf:datasource>"));
    assert!(!control.contains(">refs.bib</bcf:datasource>"));
    let aux = fs::read_to_string(root.join("main.aux")).unwrap();
    assert!(aux.contains("\\newlabel{after-resource}"));
}
