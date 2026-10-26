use latex_lsp::{
    compilation::build_log,
    editing,
    language::{features, semantic},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

fn project(root: &Path, bibliography: &str) -> (Workspace, Url, Url) {
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let bib = Url::from_file_path(root.join("refs.bib")).unwrap();
    std::fs::write(root.join("main.tex"),"\\documentclass{article}\n\\addbibresource{refs.bib}\n😀 \\cite{legacy😀,main,other} \\citefield{legacy😀}{title}\n").unwrap();
    std::fs::write(root.join("refs.bib"), bibliography).unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    (workspace, main, bib)
}
fn at(doc: &Document, marker: &str) -> Position {
    doc.text
        .position(doc.text.source.find(marker).unwrap() + marker.len())
}
fn span(source: &str, marker: &str) -> Range {
    let start = source.find(marker).unwrap();
    Text::new(source.into()).range(start, start + marker.len())
}
fn changed(workspace: &Workspace, edit: &WorkspaceEdit) -> BTreeMap<Url, String> {
    let mut edits = BTreeMap::<Url, Vec<TextEdit>>::new();
    if let Some(DocumentChanges::Edits(documents)) = &edit.document_changes {
        for document in documents {
            assert_eq!(document.text_document.version, Some(7));
            for edit in &document.edits {
                let OneOf::Left(edit) = edit else { panic!() };
                edits
                    .entry(document.text_document.uri.clone())
                    .or_default()
                    .push(edit.clone());
            }
        }
    } else if let Some(changes) = &edit.changes {
        edits = changes.clone().into_iter().collect();
    }
    edits
        .into_iter()
        .map(|(uri, mut edits)| {
            let doc = &workspace.documents[&uri];
            let mut result = doc.text.source.clone();
            edits.sort_by_key(|edit| edit.range.start);
            for edit in edits.into_iter().rev() {
                result.replace_range(
                    doc.text.offset(edit.range.start).unwrap()
                        ..doc.text.offset(edit.range.end).unwrap(),
                    &edit.new_text,
                );
            }
            (uri, result)
        })
        .collect()
}

#[test]
fn ids_aliases_complete_navigate_hover_and_rename_independently_of_primary_keys() {
    let directory = tempfile::tempdir().unwrap();
    let source="% 😀\r\n@book{main,title={Literal legacy😀},IDS={ legacy😀 , other }}\r\n@book{child,crossref={legacy😀}}\r\n";
    let (workspace, main, bib) = project(directory.path(), source);
    let position = at(&workspace.documents[&main], "\\cite{legacy😀");
    let definitions = features::definitions(&workspace, &main, position);
    assert_eq!(
        definitions,
        vec![Location::new(
            bib.clone(),
            Text::new(source.into()).range(
                source.find("IDS={ ").unwrap() + "IDS={ ".len(),
                source.find("IDS={ ").unwrap() + "IDS={ ".len() + "legacy😀".len()
            )
        )]
    );
    let hover = features::hover(&workspace, &main, position).unwrap();
    let HoverContents::Markup(hover) = hover.contents else {
        panic!()
    };
    assert_eq!(hover.kind, MarkupKind::Markdown);
    assert_eq!(
        hover.value,
        "```bibtex\n@book{main,title={Literal legacy😀},IDS={ legacy😀 , other }}\n```"
    );
    let completion = features::completion(&workspace, &main, position)
        .into_iter()
        .find(|item| item.label == "legacy😀")
        .unwrap();
    assert!(completion.detail.is_none());
    assert_eq!(
        completion.documentation,
        Some(Documentation::MarkupContent(hover))
    );
    let relation = at(&workspace.documents[&bib], "crossref={legacy😀");
    assert!(features::completion(&workspace, &bib, relation)
        .iter()
        .any(|item| item.label == "legacy😀"));
    let references = features::references(&workspace, &main, position, true);
    assert_eq!(references.len(), 4, "{references:?}");
    assert_eq!(
        features::references(&workspace, &main, position, false).len(),
        3
    );
    let edit = features::rename(&workspace, &main, position, "renamed😀")
        .unwrap()
        .unwrap();
    let changed = changed(&workspace, &edit);
    assert_eq!(
        changed[&bib],
        source
            .replace("IDS={ legacy😀", "IDS={ renamed😀")
            .replace("crossref={legacy😀}", "crossref={renamed😀}")
    );
    assert_eq!(
        changed[&main],
        workspace.documents[&main]
            .text
            .source
            .replace("legacy😀", "renamed😀")
    );
    assert!(changed[&bib].contains("@book{main,"));
    assert!(changed[&bib].contains("Literal legacy😀"));
    assert_eq!(workspace.documents[&bib].syntax.citations.len(), 2);
}

#[test]
fn quoted_ids_lists_are_braced_when_an_alias_is_renamed_to_a_key_with_quotes() {
    let directory = tempfile::tempdir().unwrap();
    let source="@book{main,IDS=\"before, legacy😀 ,after\",title={Literal legacy😀}} @book{child,crossref=\"legacy😀\"}";
    let (workspace, main, bib) = project(directory.path(), source);
    let position = at(&workspace.documents[&main], "\\cite{legacy😀");
    let edit = features::rename(&workspace, &main, position, "O\"Connor😀")
        .unwrap()
        .unwrap();
    let changed = changed(&workspace, &edit);
    assert_eq!(changed[&bib], "@book{main,IDS={before, O\"Connor😀 ,after},title={Literal legacy😀}} @book{child,crossref={O\"Connor😀}}");
}

#[test]
fn primary_key_renames_keep_ids_declarations_and_alias_references_intact() {
    let directory = tempfile::tempdir().unwrap();
    let source = "@book{main,ids={legacy😀,other},title={Parent}} @book{child,crossref={legacy😀}}";
    let (workspace, main, bib) = project(directory.path(), source);
    let position = at(&workspace.documents[&main], "legacy😀,main");
    let edit = features::rename(&workspace, &main, position, "newmain")
        .unwrap()
        .unwrap();
    let changed = changed(&workspace, &edit);
    assert_eq!(
        changed[&bib],
        source.replacen("@book{main,", "@book{newmain,", 1)
    );
    assert_eq!(
        changed[&main],
        workspace.documents[&main]
            .text
            .source
            .replace("legacy😀,main,other", "legacy😀,newmain,other")
    );
    for target in ["legacy😀", "other"] {
        assert!(features::rename(&workspace, &main, position, target).is_err());
    }
}

#[test]
fn dynamic_duplicate_incomplete_and_lookalike_ids_fields_do_not_guess_aliases() {
    let directory = tempfile::tempdir().unwrap();
    for source in [
        "@string{names={legacy😀}} @book{main,ids=names}",
        "@book{main,ids={legacy} # {😀}}",
        "@book{main,ids={legacy😀},IDS={other}}",
        "@book{main,ids={other},IDS={legacy😀}}",
        "@book{main,ids={legacy😀},IDS=}",
        "@book{main,ids={legacy😀}",
        "@book{main,ids={{legacy😀}}}",
        "@book{main,ids={legacy😀\\relax}}",
        "@book{main,ids={legacy😀% comment\n}}",
        "@book{main,IDſ={legacy😀}}",
        "@book{main,related={legacy😀}}",
        "@comment{@book{main,ids={legacy😀}}}",
    ] {
        let (workspace, main, _) = project(directory.path(), source);
        let position = at(&workspace.documents[&main], "\\cite{legacy😀");
        assert!(
            features::definitions(&workspace, &main, position).is_empty(),
            "{source}"
        );
        assert!(
            features::completion(&workspace, &main, position)
                .iter()
                .all(|item| item.label != "legacy😀"),
            "{source}"
        );
    }
    let source = "@book{bad,ids=macro} @book{main,Ids % keep\r\n = \" before , legacy😀 ,other\"}";
    let (workspace, main, bib) = project(directory.path(), source);
    let position = at(&workspace.documents[&main], "\\cite{legacy😀");
    let definitions = features::definitions(&workspace, &main, position);
    assert_eq!(
        definitions,
        vec![Location::new(bib, span(source, "legacy😀"))]
    );
}

#[test]
fn duplicate_alias_and_primary_collisions_refuse_ambiguous_renames() {
    let directory = tempfile::tempdir().unwrap();
    for source in [
        "@book{main,ids={legacy😀,legacy😀}}",
        "@book{main,ids={legacy😀}} @book{second,ids={legacy😀}}",
        "@book{main,ids={legacy😀}} @book{legacy😀,title={Actual}}",
    ] {
        let (workspace, main, _) = project(directory.path(), source);
        let position = at(&workspace.documents[&main], "\\cite{legacy😀");
        assert_eq!(
            features::definitions(&workspace, &main, position).len(),
            2,
            "{source}"
        );
        assert!(
            features::prepare_rename(&workspace, &main, position).is_err(),
            "{source}"
        );
        assert!(
            features::rename(&workspace, &main, position, "renamed").is_err(),
            "{source}"
        );
    }
}

#[test]
fn alias_positions_highlights_and_semantic_declarations_refresh_after_edits() {
    let directory = tempfile::tempdir().unwrap();
    let source="% 😀\r\n@book{main,ids={legacy😀,other},title={legacy😀}} @book{child,crossref={legacy😀}}";
    let (mut workspace, main, bib) = project(directory.path(), source);
    for (version, source) in [
        (8, source.to_string()),
        (9, source.replace("ids={legacy😀", "ids={\r\n legacy😀")),
    ] {
        workspace.open_with_language(bib.clone(), source.clone(), version, "bibtex");
        let start = source.find("ids={").unwrap() + "ids={".len();
        let alias_start = start + source[start..].find("legacy😀").unwrap();
        let range = Text::new(source.clone()).range(alias_start, alias_start + "legacy😀".len());
        let definitions = features::definitions(
            &workspace,
            &main,
            at(&workspace.documents[&main], "\\cite{legacy😀"),
        );
        assert_eq!(definitions, vec![Location::new(bib.clone(), range)]);
        let highlights = editing::highlights(&workspace, &bib, range.start);
        assert_eq!(highlights.len(), 2, "{highlights:?}");
        assert!(highlights
            .iter()
            .any(|item| item.range == range && item.kind == Some(DocumentHighlightKind::WRITE)));
        let selections =
            editing::selection_ranges(&workspace.documents[&bib], &[range.start]).unwrap();
        let mut selection = Some(&selections[0]);
        let mut found = false;
        while let Some(current) = selection {
            found |= current.range == range;
            selection = current.parent.as_deref();
        }
        assert!(found);
        let mut position = Position::new(0, 0);
        let mut aliases = Vec::new();
        for token in semantic::tokens(&workspace.documents[&bib], None)
            .unwrap()
            .data
        {
            position = Position::new(
                position.line + token.delta_line,
                if token.delta_line == 0 {
                    position.character + token.delta_start
                } else {
                    token.delta_start
                },
            );
            if token.token_type == 3 && token.token_modifiers_bitset == 1 {
                aliases.push(Range::new(
                    position,
                    Position::new(position.line, position.character + token.length),
                ));
            }
        }
        assert!(aliases.contains(&range), "{aliases:?}");
        assert_eq!(aliases.len(), 4);
    }
    workspace.open_with_language(bib.clone(), "@book{main,ids=dynamic}".into(), 10, "bibtex");
    assert!(features::definitions(
        &workspace,
        &main,
        at(&workspace.documents[&main], "\\cite{legacy😀")
    )
    .is_empty());
}

#[test]
fn ids_do_not_claim_biber_database_entry_warnings_or_unrelated_project_symbols() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("unrelated.bib"),
        "@book{external,ids={legacy😀},year={bad}} ",
    )
    .unwrap();
    let source = "@book{main,ids={legacy😀},year={bad}}";
    let (workspace, main, bib) = project(root, source);
    let position = at(&workspace.documents[&main], "\\cite{legacy😀");
    assert_eq!(features::definitions(&workspace, &main, position).len(), 1);
    assert!(features::rename(&workspace, &main, position, "renamed")
        .unwrap()
        .is_some());
    let log="INFO - Found BibTeX data source 'refs.bib'\nWARN - legacy year field 'bad' in entry 'legacy😀' is not an integer - this will probably not sort properly.\nWARN - legacy year field 'bad' in entry 'main' is not an integer - this will probably not sort properly.\n";
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), log);
    assert_eq!(diagnostics[&bib][0].range, span(source, "bad"));
    assert_eq!(
        diagnostics[&main][0].data.as_ref().unwrap()["unresolvedSource"],
        true
    );
}

#[test]
#[ignore = "requires biber"]
fn real_biber_alias_and_primary_renames_keep_ids_crossrefs_resolving_to_the_same_parent() {
    for (source, new_name, primary) in [
        ("@book{main,ids={legacy😀,other},title={Parent}} @book{child,crossref={legacy😀}}", "newalias😀", false),
        ("@book{main,ids={legacy😀,other},title={Parent}} @book{child,crossref={legacy😀}}", "newmain", true),
        ("@book{main,ids=\"before, legacy😀 ,other\",title={Parent}} @book{child,crossref=\"legacy😀\"}", "O\"Connor😀", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let (workspace, main, _) = project(root, source);
        let position = at(
            &workspace.documents[&main],
            if primary {
                "legacy😀,main"
            } else {
                "\\cite{legacy😀"
            },
        );
        let edit = features::rename(&workspace, &main, position, new_name)
            .unwrap()
            .unwrap();
        for (uri, text) in changed(&workspace, &edit) {
            std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
        }
        let output = std::process::Command::new("biber")
            .args([
                "--tool",
                "--output-resolve-crossrefs",
                "--output-file=processed.bib",
                "refs.bib",
            ])
            .current_dir(root)
            .output()
            .unwrap();
        let log = std::fs::read_to_string(root.join("refs.bib.blg")).unwrap();
        assert!(output.status.success(), "{log}");
        assert!(!log.contains("WARN -"), "{log}");
        let processed = std::fs::read_to_string(root.join("processed.bib")).unwrap();
        let doc = Document::new(
            &Url::from_file_path(root.join("processed.bib")).unwrap(),
            processed.clone(),
            None,
        );
        let child = doc
            .syntax
            .citations
            .iter()
            .find(|entry| entry.name == "child")
            .unwrap();
        assert!(child.detail.contains("Title: Parent"), "{processed}");
    }
}

#[test]
fn shared_alias_declarations_expand_rename_scope_and_check_every_consumer_for_collisions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("shared.bib"),
        "@book{main,ids={legacy😀},title={Shared}}",
    )
    .unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("main.tex"), "\\documentclass{article}\\addbibresource{../shared.bib}\\addbibresource{refs.bib}\\cite{legacy😀}").unwrap();
        std::fs::write(
            root.join(name).join("refs.bib"),
            format!("@book{{{name},title={{Local}}}}"),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
    let target = at(&workspace.documents[&main], "\\cite{legacy😀");
    assert_eq!(
        features::references(&workspace, &main, target, true).len(),
        3
    );
    let edit = features::rename(&workspace, &main, target, "newalias😀")
        .unwrap()
        .unwrap();
    let changed = changed(&workspace, &edit);
    assert_eq!(changed.len(), 3);
    assert!(changed.values().all(|source| source.contains("newalias😀")));
    assert!(
        changed[&Url::from_file_path(root.join("shared.bib")).unwrap()].contains("@book{main,")
    );
    let other = Url::from_file_path(root.join("b/refs.bib")).unwrap();
    for source in ["@book{second,ids={legacy😀}}", "@book{legacy😀,}"] {
        workspace.open_with_language(other.clone(), source.into(), 2, "bibtex");
        assert!(features::prepare_rename(&workspace, &main, target).is_err());
        assert!(features::rename(&workspace, &main, target, "newalias😀").is_err());
    }
    for source in ["@book{second,ids={newalias😀}}", "@book{newalias😀,}"] {
        workspace.open_with_language(other.clone(), source.into(), 3, "bibtex");
        assert!(features::rename(&workspace, &main, target, "newalias😀").is_err());
    }
}

#[test]
fn shared_citations_with_different_or_missing_alias_definitions_refuse_rename() {
    for other_definition in ["", "@book{second,ids={legacy😀}}", "@book{legacy😀,}"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("shared.tex"), "\\cite{legacy😀}").unwrap();
        for name in ["a", "b"] {
            std::fs::create_dir(root.join(name)).unwrap();
            std::fs::write(
                root.join(name).join("main.tex"),
                "\\documentclass{article}\\addbibresource{refs.bib}\\input{../shared}",
            )
            .unwrap();
            std::fs::write(
                root.join(name).join("refs.bib"),
                if name == "a" {
                    "@book{main,ids={legacy😀}}"
                } else {
                    other_definition
                },
            )
            .unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let bib = Url::from_file_path(root.join("a/refs.bib")).unwrap();
        let target = at(&workspace.documents[&bib], "ids={legacy😀");
        assert!(features::prepare_rename(&workspace, &bib, target).is_err());
        assert!(features::rename(&workspace, &bib, target, "newalias😀").is_err());
    }
}
