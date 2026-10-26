use lsp_types::*;
use texquill::{
    compilation::build_log,
    editing,
    language::{features, semantic},
    project::workspace::{Document, Workspace},
    text::Text,
};

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///quoted-citations/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    uri
}

fn at(source: &str, key: &str) -> Position {
    Text::new(source.into()).position(source.find(key).unwrap())
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|e| e.range.start);
    assert!(
        edits
            .windows(2)
            .all(|pair| pair[0].range.end <= pair[1].range.start),
        "overlapping edits: {edits:?}"
    );
    Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .rev()
                .map(|e| TextDocumentContentChangeEvent {
                    range: Some(e.range),
                    range_length: None,
                    text: e.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}

fn changes(edit: WorkspaceEdit) -> std::collections::HashMap<Url, Vec<TextEdit>> {
    if let Some(DocumentChanges::Edits(changes)) = edit.document_changes {
        changes
            .into_iter()
            .map(|change| {
                assert_eq!(change.text_document.version, Some(7));
                (
                    change.text_document.uri,
                    change
                        .edits
                        .into_iter()
                        .map(|e| match e {
                            OneOf::Left(e) => e,
                            _ => panic!("unexpected annotated edit"),
                        })
                        .collect(),
                )
            })
            .collect()
    } else {
        edit.changes.unwrap()
    }
}

#[test]
fn quoted_keys_navigate_and_rename_across_all_citation_argument_styles() {
    for key in ["O\"Connor", "\"", "\"中文😀(old)\"", "book\"(old"] {
        for versioned in [false, true] {
            for declaration in [
                format!("@misc{{{key}}}"),
                format!("@book({key},title={{Metadata}})"),
                format!("\\bibitem[Display]{{% keep\r\n{key}% tail\r\n}} Body"),
            ] {
                let mut workspace = Workspace {
                    client: texquill::project::state::ClientCapabilities {
                        versioned_edits: versioned,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let bib = insert(
                    &mut workspace,
                    if declaration.starts_with('@') {
                        "refs.bib"
                    } else {
                        "refs.tex"
                    },
                    &declaration,
                );
                let calls = [
                    format!("\\cite[Note]{{other, % keep\r\n{key}% tail\r\n}}"),
                    format!("\\citet*{{{key}}}"),
                    format!("\\nocite{{{key}}}"),
                    format!("\\bibentry{{% keep\r\n{key}% tail\r\n}}"),
                    format!("\\defcitealias{{{key}}}{{Display}}"),
                    format!("\\citetalias{{{key}}}"),
                    format!("\\parencites(Global)[Note]{{{key},other}}[After]{{{key}}}"),
                    format!("\\volcite[Note]{{42}}{{{key}}}"),
                    format!("\\citefield{{{key}}}{{title}}"),
                    format!("\\addtocategory{{category}}{{{key}}}"),
                ]
                .join("\r\n");
                let source = format!("% 😀\r\n{calls}\r\nLiteral {key}\r\n% \\cite{{{key}}}\r\n\\verb|\\cite{{{key}}}|\r\n\\label{{{key}}}");
                let main = insert(&mut workspace, "main.tex", &source);
                let fields = format!("@misc{{child,title={{{key}}},crossref={{{key}}},xref={{{key}}},xdata={{other, {key}}},related={{{key},other}},entryset={{{key}}}}}");
                let child = insert(&mut workspace, "child.bib", &fields);
                let pos = at(&source, key);
                let defs = features::definitions(&workspace, &main, pos);
                assert_eq!(defs.len(), 1, "{source}");
                assert_eq!(defs[0].uri, bib);
                assert!(features::hover(&workspace, &main, pos).is_some());
                assert_eq!(
                    features::references(&workspace, &main, pos, true).len(),
                    17,
                    "{source}"
                );
                assert_eq!(editing::highlights(&workspace, &main, pos).len(), 11);
                assert!(features::prepare_rename(&workspace, &main, pos)
                    .unwrap()
                    .is_some());
                // Renaming from the database declaration must include the TeX uses.
                for (origin, position) in [(&main, pos), (&bib, at(&declaration, key))] {
                    let changes = changes(
                        features::rename(&workspace, origin, position, "renamed\"key")
                            .unwrap()
                            .unwrap(),
                    );
                    assert_eq!(changes.len(), 3);
                    assert_eq!(
                        apply(&source, &changes[&main]),
                        source.replacen(&calls, &calls.replace(key, "renamed\"key"), 1)
                    );
                    assert_eq!(
                        apply(&declaration, &changes[&bib]),
                        declaration.replacen(key, "renamed\"key", 1)
                    );
                    assert_eq!(
                        apply(&fields, &changes[&child]),
                        fields
                            .replacen(&format!("title={{{key}}}"), "title={PROTECTED}", 1)
                            .replace(key, "renamed\"key")
                            .replace("{PROTECTED}", &format!("{{{key}}}"))
                    );
                }
            }
        }
    }
}

#[test]
fn rename_into_quotes_converts_each_quoted_relationship_value_once() {
    for versioned in [false, true] {
        for tail in ["\"}", ""] {
            let mut workspace = Workspace {
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: versioned,
                    ..Default::default()
                },
                ..Default::default()
            };
            let source = format!("% 😀\r\n@book{{old,title={{old}}}}\r\n@misc{{child, crossref = \" old \", xref={{old}}, xdata=\"old, keep, old\", related = \"keep, old , old \" % note\r\n, entryset = \" old, old {tail}");
            let bib = insert(&mut workspace, "refs.bib", &source);
            let main_source = "\\cite{old}";
            let main = insert(&mut workspace, "main.tex", main_source);
            let edits = changes(
                features::rename(&workspace, &main, at(main_source, "old"), "新\"😀")
                    .unwrap()
                    .unwrap(),
            );
            let updated = apply(&source, &edits[&bib]);
            let expected = format!("% 😀\r\n@book{{新\"😀,title={{old}}}}\r\n@misc{{child, crossref = {{ 新\"😀 }}, xref={{新\"😀}}, xdata={{新\"😀, keep, 新\"😀}}, related = {{keep, 新\"😀 , 新\"😀 }} % note\r\n, entryset = {{ 新\"😀, 新\"😀 }}{}", if tail.is_empty() { "" } else { "}" });
            assert_eq!(updated, expected);
            let reparsed = Document::new(&bib, updated, Some(8));
            assert_eq!(
                reparsed
                    .syntax
                    .citation_references
                    .iter()
                    .filter(|e| e.name == "新\"😀")
                    .count(),
                8
            );
            assert_eq!(apply(main_source, &edits[&main]), "\\cite{新\"😀}");
        }
    }
}

#[test]
fn completion_preserves_notes_comments_lists_and_bibtex_delimiters() {
    for (marked, expected) in [
        (
            "\\cite[Note]{other, % keep\r\nO\"C¦BAD% tail\r\n}",
            "\\cite[Note]{other, % keep\r\nO\"Connor% tail\r\n}",
        ),
        (
            "\\bibentry{% keep\r\nO\"C¦BAD% tail\r\n}",
            "\\bibentry{% keep\r\nO\"Connor% tail\r\n}",
        ),
        (
            "\\defcitealias{O\"C¦BAD}{Display}",
            "\\defcitealias{O\"Connor}{Display}",
        ),
        (
            "\\parencites(Note)[Before]{other}[After]{O\"C¦BAD,other}",
            "\\parencites(Note)[Before]{other}[After]{O\"Connor,other}",
        ),
        ("\\volcite{42}{O\"C¦BAD}", "\\volcite{42}{O\"Connor}"),
        (
            "\\citefield{O\"C¦BAD}{title}",
            "\\citefield{O\"Connor}{title}",
        ),
        (
            "\\addtocategory{group}{O\"C¦BAD}",
            "\\addtocategory{group}{O\"Connor}",
        ),
        ("\\cite{O\"C¦", "\\cite{O\"Connor"),
        (
            "@misc{child,crossref={O\"C¦BAD}}",
            "@misc{child,crossref={O\"Connor}}",
        ),
        (
            "@misc{child,crossref=\" O¦BAD \" % keep\r\n}",
            "@misc{child,crossref={ O\"Connor } % keep\r\n}",
        ),
        (
            "@misc(child,related=\"keep, O¦BAD ,last\")",
            "@misc(child,related={keep, O\"Connor ,last})",
        ),
        (
            "@misc{child,related=\"keep,\r\n O¦BAD ,\r\n last\"}",
            "@misc{child,related={keep,\r\n O\"Connor ,\r\n last}}",
        ),
        (
            "@misc{child,crossref=\"\r\n O¦BAD\r\n \"}",
            "@misc{child,crossref={\r\n O\"Connor\r\n }}",
        ),
        (
            "@misc{child,entryset=\"keep,\r\n O¦BAD ",
            "@misc{child,entryset={keep,\r\n O\"Connor }",
        ),
        (
            "@misc{child,entryset=\"keep, O¦BAD ",
            "@misc{child,entryset={keep, O\"Connor }",
        ),
    ] {
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "refs.bib",
            "@misc{O\"Connor,title={Metadata}}",
        );
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let uri = insert(
            &mut workspace,
            if marked.starts_with('@') {
                "child.bib"
            } else {
                "main.tex"
            },
            &source,
        );
        let item =
            features::completion(&workspace, &uri, Text::new(source.clone()).position(offset))
                .into_iter()
                .find(|item| item.label == "O\"Connor")
                .unwrap_or_else(|| panic!("missing completion: {marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        assert_eq!(edit.range.start.line, edit.range.end.line, "{marked}");
        let position = Text::new(source.clone()).position(offset);
        assert!(
            edit.range.start <= position && position <= edit.range.end,
            "{marked}"
        );
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        assert_eq!(apply(&source, &edits), expected, "{marked}");
        if marked.starts_with('@') {
            let doc = Document::new(&uri, expected.into(), Some(8));
            assert!(doc
                .syntax
                .citation_references
                .iter()
                .any(|e| e.name == "O\"Connor"));
        }
    }
}

#[test]
fn semantic_tokens_cover_whole_quoted_citation_keys() {
    for (name, source, expected) in [
        ("main.tex", "% 😀\r\n\\bibitem{O\"Connor} Text \\cite{O\"Connor,other}\\bibentry{O\"Connor}\\defcitealias{O\"Connor}{Display}\\parencites{O\"Connor}{other}", vec!["O\"Connor", "O\"Connor", "other", "O\"Connor", "O\"Connor", "O\"Connor", "other"]),
        ("refs.bib", "@book{O\"Connor,title={O\"Connor}}\r\n@misc{child,crossref={O\"Connor},related={other,O\"Connor}}", vec!["O\"Connor", "child", "O\"Connor", "other", "O\"Connor"]),
    ] {
        let mut workspace = Workspace::default();
        let uri = insert(&mut workspace, name, source);
        let doc = &workspace.documents[&uri];
        let legend = semantic::options().legend;
        let mut pos = Position::default();
        let mut keys = Vec::new();
        for token in semantic::tokens(doc, None).unwrap().data {
            pos = Position::new(pos.line + token.delta_line, if token.delta_line == 0 { pos.character + token.delta_start } else { token.delta_start });
            if legend.token_types[token.token_type as usize].as_str() == "enumMember" {
                let end = Position::new(pos.line, pos.character + token.length);
                keys.push(&source[doc.text.offset(pos).unwrap()..doc.text.offset(end).unwrap()]);
            }
        }
        assert_eq!(keys, expected);
    }
}

#[test]
fn quoted_keys_keep_dynamic_inputs_and_label_namespaces_protected() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "refs.bib", "@misc{O\"Connor}");
    for source in [
        "\\cite{\\chosen\"Connor}",
        "\\cite{O\"Con% splice\nnor}",
        "\\cite{{O\"Connor}}",
        "\\cite{O\" Connor}",
        "\\ref{O\"Connor}",
        "\\label{O\"Connor}",
        "% \\cite{O\"Connor}",
        "\\verb|\\cite{O\"Connor}|",
        "@misc{child,crossref=\"O\"Connor\"}",
        "@misc{child,crossref={O\"Connor} # suffix}",
        "@misc{child,title={O\"Connor}}",
    ] {
        let uri = insert(
            &mut workspace,
            if source.starts_with('@') {
                "child.bib"
            } else {
                "main.tex"
            },
            source,
        );
        let pos = Text::new(source.into()).position(source.find('"').unwrap());
        assert!(
            features::definitions(&workspace, &uri, pos).is_empty(),
            "{source}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, pos)
                .unwrap()
                .is_none(),
            "{source}"
        );
    }
}

#[test]
fn quoted_undefined_citations_have_exact_warning_ranges_and_spelling_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for call in [
        "\\cite{other, % keep\r\nO\"Cononr% tail\r\n}",
        "\\bibentry{O\"Cononr}",
        "\\parencites{other}{O\"Cononr}",
        "\\volcite{42}{O\"Cononr}",
    ] {
        let source = format!("\\bibitem{{O\"Connor}} Body\r\n% 😀\r\n{call}\r\n");
        std::fs::write(&main, &source).unwrap();
        let line = Text::new(source.clone())
            .position(source.find("O\"Cononr").unwrap())
            .line
            + 1;
        let diagnostics = build_log::parse(&main, &format!("(./main.tex\nLaTeX Warning: Citation `O\"Cononr' on page 1 undefined on input line {line}.\n)\n"));
        let diagnostic = diagnostics[&uri][0].clone();
        let start = source.find("O\"Cononr").unwrap();
        assert_eq!(
            diagnostic.range,
            Text::new(source.clone()).range(start, start + "O\"Cononr".len())
        );
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.open(uri.clone(), source.clone(), 7);
        workspace
            .project
            .build_diagnostics
            .insert(uri.clone(), diagnostics);
        let context = CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: None,
            trigger_kind: None,
        };
        let actions = editing::code_actions(&workspace, &uri, diagnostic.range, &context);
        let edits = actions
            .into_iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(a) => a.edit,
                _ => None,
            })
            .flat_map(|e| {
                e.changes
                    .unwrap_or_default()
                    .remove(&uri)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        assert_eq!(edits.len(), 1, "{call}");
        assert_eq!(
            apply(&source, &edits),
            source.replace("O\"Cononr", "O\"Connor")
        );
        workspace.open(uri.clone(), source.replace("O\"Cononr", "Changed"), 8);
        assert!(editing::code_actions(&workspace, &uri, diagnostic.range, &context).is_empty());
    }
}

#[test]
fn quoted_citation_renames_keep_independent_projects_and_shared_uses_safe() {
    for shared in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("shared.tex"), "\\cite{O\"Connor}").unwrap();
        for name in ["a", "b"] {
            std::fs::create_dir(root.join(name)).unwrap();
            let included = if shared { "../shared" } else { "child" };
            std::fs::write(
                root.join(format!("{name}/main.tex")),
                format!("\\documentclass{{article}}\\bibliography{{refs}}\\input{{{included}}}"),
            )
            .unwrap();
            std::fs::write(root.join(format!("{name}/child.tex")), "\\cite{O\"Connor}").unwrap();
            std::fs::write(root.join(format!("{name}/refs.bib")), "@misc{O\"Connor}").unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let other = Url::from_file_path(root.join("b/main.tex")).unwrap();
        workspace.refresh_project(&other);
        let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
        workspace.refresh_project(&main);
        let bib = Url::from_file_path(root.join("a/refs.bib")).unwrap();
        let result = features::rename(&workspace, &bib, Position::new(0, 6), "New\"Key");
        if shared {
            assert!(result.is_err());
        } else {
            let changes = changes(result.unwrap().unwrap());
            assert_eq!(changes.len(), 2);
            assert!(changes.keys().all(|uri| uri.path().contains("/a/")));
        }
    }
}

#[test]
#[ignore = "requires pdflatex and BibTeX"]
fn real_quoted_key_renames_preserve_bibliography_and_crossref_inheritance() {
    use std::{fs, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = "\\documentclass{article}\n\\begin{document}\n\\cite{child,old}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n";
    let bib = "@incollection{child,author={Writer, A.},title={Child},crossref=\"old\",pages={1--2}}\n@book{old,editor={Editor, B.},title={Inherited title},booktitle={Inherited title},publisher={Press},year=2026}\n";
    fs::write(root.join("main.tex"), main).unwrap();
    fs::write(root.join("refs.bib"), bib).unwrap();
    let compile = || {
        for (program, args) in [
            (
                "pdflatex",
                vec!["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            ),
            ("bibtex", vec!["main"]),
            (
                "pdflatex",
                vec!["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            ),
            (
                "pdflatex",
                vec!["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            ),
        ] {
            let output = Command::new(program)
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{program}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        let log = fs::read_to_string(root.join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        let bbl = fs::read_to_string(root.join("main.bbl")).unwrap();
        assert!(bbl.contains("Inherited title"), "{bbl}");
        bbl
    };
    let original = compile();
    for (old, new) in [
        ("old", "O\"Connor"),
        ("O\"Connor", "Next\"Key"),
        ("Next\"Key", "plain"),
    ] {
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = Url::from_file_path(root.join("main.tex")).unwrap();
        workspace.refresh_project(&main);
        let bib = Url::from_file_path(root.join("refs.bib")).unwrap();
        let source = &workspace.documents[&bib].text.source;
        let pos = at(source, old);
        let changes = changes(
            features::rename(&workspace, &bib, pos, new)
                .unwrap()
                .unwrap(),
        );
        assert_eq!(changes.len(), 2);
        for (uri, edits) in changes {
            let path = uri.to_file_path().unwrap();
            fs::write(&path, apply(&fs::read_to_string(&path).unwrap(), &edits)).unwrap();
        }
        assert_eq!(compile(), original.replace("{old}", &format!("{{{new}}}")));
        assert!(fs::read_to_string(root.join("refs.bib"))
            .unwrap()
            .contains(&format!("crossref={{{new}}}")));
    }
}

#[test]
fn native_crossref_diagnostics_keep_quotes_in_child_and_parent_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let bib = root.join("refs.bib");
    let uri = Url::from_file_path(&bib).unwrap();
    std::fs::write(&main, "\\bibliography{refs}").unwrap();
    let source =
        "% 😀\r\n@misc{Child\"Key,crossref={ 父\"😀 }}\r\n@misc{父\"😀,crossref={next}}\r\n";
    std::fs::write(&bib, source).unwrap();
    let text = Text::new(source.into());
    for (record, code, severity) in [
        ("A bad cross reference---entry \"Child\"Key\"\nrefers to entry \"父\"😀\", which doesn't exist\n", "bad-bibliography-crossref", DiagnosticSeverity::ERROR),
        ("Warning--you've nested cross references--entry \"Child\"Key\"\nrefers to entry \"父\"😀\", which also refers to something\n", "nested-bibliography-crossref", DiagnosticSeverity::WARNING),
    ] {
        let diagnostics = build_log::parse_bibliography(&main, &format!("Database file #1: refs.bib\n{record}"));
        let diagnostic = diagnostics[&uri].iter().find(|d| d.code == Some(NumberOrString::String(code.into()))).unwrap();
        let offset = source.find("父\"😀").unwrap();
        assert_eq!(diagnostic.range, text.range(offset, offset + "父\"😀".len()));
        assert_eq!(diagnostic.severity, Some(severity));
        let related = diagnostic.related_information.as_ref().unwrap();
        assert_eq!(related.len(), 1);
        let offset = source.rfind("父\"😀").unwrap();
        assert_eq!(related[0].location, Location::new(uri.clone(), text.range(offset, offset + "父\"😀".len())));
    }
}
