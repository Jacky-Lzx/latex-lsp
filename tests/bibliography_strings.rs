use latex_lsp::{
    editing, features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn workspace(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///strings/refs.bib").unwrap();
    let mut workspace = Workspace::default();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    (workspace, uri)
}

fn at(source: &str, marker: &str) -> Position {
    Text::new(source.into()).position(source.rfind(marker).unwrap() + marker.len())
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let text = Text::new(source.into());
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| edit.range.start);
    let mut result = source.to_owned();
    for edit in edits.iter().rev() {
        result.replace_range(
            text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
            &edit.new_text,
        );
    }
    result
}

#[test]
fn declarations_and_concatenation_operands_navigate_case_insensitively_without_literal_leaks() {
    let source = "@STRING (Prefix = {Journal})\r\n@string{full = preFIX # \" of \" # 2026}\r\n@preamble{PREFIX # { text}}\r\n@book{Prefix,title=pReFiX,author={Prefix},publisher=full,year=2026,related={Prefix}}\r\n@comment{Prefix # PREFIX}\r\n% Prefix\r\n";
    let (workspace, uri) = workspace(source);
    let target = at(source, "title=pRe");
    let definitions = features::definitions(&workspace, &uri, target);
    assert_eq!(definitions.len(), 1);
    assert_eq!(
        definitions[0].range,
        Text::new(source.into()).range(
            source.find("Prefix").unwrap(),
            source.find("Prefix").unwrap() + 6
        )
    );
    assert_eq!(
        features::references(&workspace, &uri, target, false).len(),
        3
    );
    assert_eq!(
        features::references(&workspace, &uri, target, true).len(),
        4
    );
    let highlights = editing::highlights(&workspace, &uri, target);
    assert_eq!(
        highlights
            .iter()
            .filter(|item| item.kind == Some(DocumentHighlightKind::WRITE))
            .count(),
        1
    );
    assert_eq!(
        highlights
            .iter()
            .filter(|item| item.kind == Some(DocumentHighlightKind::READ))
            .count(),
        3
    );
    let hover = features::hover(&workspace, &uri, target).unwrap();
    let HoverContents::Markup(markup) = hover.contents else {
        panic!()
    };
    assert!(markup
        .value
        .contains("BibTeX string abbreviation = {Journal}"));
    let symbols = features::symbols(&workspace.documents[&uri]);
    assert_eq!(
        symbols
            .iter()
            .map(|symbol| (symbol.name.as_str(), symbol.kind))
            .collect::<Vec<_>>(),
        vec![
            ("Prefix", SymbolKind::CONSTANT),
            ("full", SymbolKind::CONSTANT),
            ("Prefix", SymbolKind::KEY)
        ]
    );
}

#[test]
fn abbreviation_completions_replace_identifiers_and_fill_empty_concatenation_operands() {
    let header = "% 😀\r\n@string{Journal={Journal}}\r\n";
    for (body, marker, expected) in [
        ("@book{key,title=JoBAD}", "=Jo", "@book{key,title=Journal}"),
        (
            "@book{key,title=2026 # JoBAD}",
            "# Jo",
            "@book{key,title=2026 # Journal}",
        ),
        (
            "@book{key,title=\"Text\" # }",
            "# ",
            "@book{key,title=\"Text\" # Journal}",
        ),
        ("@book{key,title=", "title=", "@book{key,title=Journal"),
        (
            "@book{key,title= % note\r\n  }",
            "\r\n  ",
            "@book{key,title= % note\r\n  Journal}",
        ),
        (
            "@preamble{\"text\" # jOBAD}",
            "# jO",
            "@preamble{\"text\" # Journal}",
        ),
        (
            "@string{combined=joBAD # { text}}",
            "=jo",
            "@string{combined=Journal # { text}}",
        ),
    ] {
        let source = format!("{header}{body}");
        let (mut workspace, uri) = workspace(&source);
        workspace.snippets = true;
        let items = features::completion(&workspace, &uri, at(&source, marker));
        let journal = items
            .iter()
            .find(|item| item.label == "Journal")
            .unwrap_or_else(|| panic!("missing Journal for {body:?}: {items:?}"));
        assert_eq!(journal.kind, Some(CompletionItemKind::CONSTANT));
        assert_eq!(
            journal.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT)
        );
        let Some(CompletionTextEdit::Edit(edit)) = &journal.text_edit else {
            panic!()
        };
        assert_eq!(
            apply(&source, std::slice::from_ref(edit)),
            format!("{header}{expected}")
        );
    }
}

#[test]
fn comments_literals_numbers_declaration_names_and_malformed_expressions_do_not_complete_abbreviations(
) {
    for (body, marker) in [
        ("@book{key,title={Jo}}", "{Jo"),
        ("@book{key,title=\"Jo\"}", "\"Jo"),
        ("@book{key,title={nested {Jo}}}", "{Jo"),
        ("@book{key,title=2026}", "202"),
        ("@book{key,title= % Jo\n}", "% Jo"),
        ("@book{key,title=Jo bad}", "=Jo"),
        ("@book{key,title=2026Jo}", "2026Jo"),
        ("@comment{Jo}", "{Jo"),
        ("@string{Jo = {Text}}", "{Jo"),
        ("@book{Jo,title={Text}}", "{Jo"),
    ] {
        let source = format!("@string{{Journal={{Text}}}}\n{body}");
        let (workspace, uri) = workspace(&source);
        assert!(
            !features::completion(&workspace, &uri, at(&source, marker))
                .iter()
                .any(|item| item.label == "Journal"),
            "{body}"
        );
    }
}

#[test]
fn month_abbreviations_offer_descriptions_without_requiring_source_definitions() {
    let source = "@book{entry,month=JaBAD}";
    let (workspace, uri) = workspace(source);
    let completion = features::completion(&workspace, &uri, at(source, "=Ja"));
    let jan = completion.iter().find(|item| item.label == "jan").unwrap();
    assert!(jan.detail.as_ref().unwrap().contains("January"));
    let source = "@book{entry,month=JAN}";
    let (workspace, uri) = self::workspace(source);
    let target = at(source, "=JA");
    let HoverContents::Markup(markup) = features::hover(&workspace, &uri, target).unwrap().contents
    else {
        panic!()
    };
    assert!(markup.value.contains("January"));
    assert!(features::definitions(&workspace, &uri, target).is_empty());
    assert!(features::prepare_rename(&workspace, &uri, target)
        .unwrap()
        .is_none());
}

#[test]
fn renames_preserve_literals_and_keys_and_support_case_only_normalization() {
    let source = "@string{Journal={Text}}\n@book{Journal,title=JOURNAL # {Journal},publisher=journal,related={Journal}}\n% JOURNAL";
    let (workspace, uri) = workspace(source);
    let target = at(source, "title=JOUR");
    for name in ["New:Journal_2026", "JOURNAL"] {
        let changes = features::rename(&workspace, &uri, target, name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&uri].len(), 3);
        let expected = format!("@string{{{name}={{Text}}}}\n@book{{Journal,title={name} # {{Journal}},publisher={name},related={{Journal}}}}\n% JOURNAL");
        assert_eq!(apply(source, &changes[&uri]), expected);
    }
    let prepared = features::prepare_rename(&workspace, &uri, target)
        .unwrap()
        .unwrap();
    let PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } = prepared else {
        panic!()
    };
    assert_eq!(placeholder, "JOURNAL");
    let key_edit = features::rename(&workspace, &uri, at(source, "@book{Jour"), "OtherKey")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(key_edit[&uri].len(), 2);
    assert_eq!(apply(source,&key_edit[&uri]),"@string{Journal={Text}}\n@book{OtherKey,title=JOURNAL # {Journal},publisher=journal,related={OtherKey}}\n% JOURNAL");
}

#[test]
fn abbreviation_completions_keep_snippet_metacharacters_literal_and_bound_long_previews() {
    let source = format!(
        "@string{{Cash$Name_1={{\n {}\n}}}}\n@book{{entry,title=CaBAD}}",
        "😀 ".repeat(1000)
    );
    let (mut workspace, uri) = workspace(&source);
    workspace.snippets = true;
    let items = features::completion(&workspace, &uri, at(&source, "title=Ca"));
    let item = items
        .iter()
        .find(|item| item.label == "Cash$Name_1")
        .unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
    assert!(item.detail.as_ref().unwrap().chars().count() < 300);
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.new_text, "Cash$Name_1");
    assert_eq!(
        apply(&source, std::slice::from_ref(edit)),
        source.replace("CaBAD", "Cash$Name_1")
    );
}

#[test]
fn renames_reject_duplicate_definitions_capture_builtins_and_invalid_identifiers() {
    for extra in [
        "@string{JOURNAL={Other}}",
        "@string{other={Other}}",
        "@book{second,title=OTHER}",
    ] {
        let source = format!("@string{{Journal={{Text}}}}\n@book{{first,title=Journal}}\n{extra}");
        let (workspace, uri) = workspace(&source);
        assert!(
            features::rename(&workspace, &uri, at(&source, "title=Jour"), "other").is_err(),
            "{extra}"
        );
    }
    let source = "@string{Journal={Text}}\n@book{entry,title=Journal}";
    let (workspace, uri) = workspace(source);
    for new_name in [
        "", "2026name", "new name", "new#name", "new=name", "new{name", "new%name", "jan", "DEC",
    ] {
        assert!(
            features::rename(&workspace, &uri, at(source, "title=Jour"), new_name).is_err(),
            "{new_name}"
        );
    }
}

#[test]
fn abbreviation_self_references_are_renamed_with_their_declaration() {
    let source = "@string{Journal=JOURNAL # { extra}}\n@book{entry,title=Journal}";
    let (workspace, uri) = workspace(source);
    assert_eq!(
        features::definitions(&workspace, &uri, at(source, "title=Jour")).len(),
        1
    );
    assert!(
        features::prepare_rename(&workspace, &uri, at(source, "title=Jour"))
            .unwrap()
            .is_some()
    );
    let edit = features::rename(&workspace, &uri, at(source, "title=Jour"), "NewName")
        .unwrap()
        .unwrap();
    assert_eq!(edit.changes.unwrap()[&uri].len(), 3);
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_self_references_keep_their_value_after_rename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("fixture.bst"), "MACRO {journal} {\"Style\"}\nENTRY {title} {} {}\nFUNCTION {emit} {title write$ newline$}\nREAD\nITERATE {emit}\n").unwrap();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{entry}\n\\bibstyle{fixture}\n\\bibdata{refs}\n",
    )
    .unwrap();
    let source = "@string{Journal=JOURNAL # { extra}}\n@misc{entry,title=Journal}";
    std::fs::write(root.join("refs.bib"), source).unwrap();
    let compile = || {
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        std::fs::read_to_string(root.join("main.bbl")).unwrap()
    };
    let original = compile();
    // BibTeX initializes the declaration before reading its value: this is
    // a self reference, even when the style supplied an earlier abbreviation.
    assert_eq!(original.trim(), "extra");
    let (workspace, uri) = workspace(source);
    let edit = features::rename(&workspace, &uri, at(source, "title=Jour"), "NewName")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    std::fs::write(root.join("refs.bib"), apply(source, &changes[&uri])).unwrap();
    assert_eq!(compile(), original);
}

#[test]
fn string_names_and_edits_preserve_utf16_ranges_and_crlf() {
    let source = "% 😀\r\n@string{期刊😀_一 = {标题}}\r\n@book{条目,title=期刊😀_一}";
    let (workspace, uri) = workspace(source);
    let target = at(source, "title=期刊");
    let definitions = features::definitions(&workspace, &uri, target);
    assert_eq!(
        definitions[0].range,
        Range::new(Position::new(1, 8), Position::new(1, 14))
    );
    let changes = features::rename(&workspace, &uri, target, "期刊二")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        apply(source, &changes[&uri]),
        source.replace("期刊😀_一", "期刊二")
    );
}

#[test]
fn independent_projects_keep_abbreviation_definitions_completions_and_renames_separate() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("main.tex"),
            r"\documentclass{article}\bibliography{defs,refs}",
        )
        .unwrap();
        std::fs::write(
            root.join(name).join("defs.bib"),
            format!("@string{{Journal={{{name}}}}}\n@string{{{name}Only={{Private}}}}"),
        )
        .unwrap();
        std::fs::write(
            root.join(name).join("refs.bib"),
            format!("@book{{{name},title=JOURNAL,author=}}"),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("a/refs.bib")).unwrap();
    let source = &workspace.documents[&uri].text.source;
    let target = at(source, "title=JOUR");
    let definitions = features::definitions(&workspace, &uri, target);
    assert_eq!(definitions.len(), 1);
    assert_eq!(
        definitions[0].uri,
        Url::from_file_path(root.join("a/defs.bib")).unwrap()
    );
    let items = features::completion(&workspace, &uri, at(source, "author="));
    assert!(items.iter().any(|item| item.label == "aOnly"));
    assert!(!items.iter().any(|item| item.label == "bOnly"));
    let changes = features::rename(&workspace, &uri, target, "Renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert!(changes
        .keys()
        .all(|uri| uri.to_file_path().unwrap().starts_with(root.join("a"))));
}

#[test]
fn shared_abbreviations_expand_rename_scope_and_reject_other_consumer_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("shared.bib"), "@string{Journal={Shared}}").unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("main.tex"),
            r"\documentclass{article}\bibliography{../shared,refs}",
        )
        .unwrap();
        std::fs::write(
            root.join(name).join("refs.bib"),
            format!("@book{{{name},title=JOURNAL}}"),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("a/refs.bib")).unwrap();
    let target = at(&workspace.documents[&uri].text.source, "title=JOUR");
    assert_eq!(
        features::references(&workspace, &uri, target, true).len(),
        3
    );
    let changes = features::rename(&workspace, &uri, target, "Renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    let other = Url::from_file_path(root.join("b/refs.bib")).unwrap();
    workspace.open(
        other,
        "@string{journal={Other}}\n@book{b,title=Journal}".into(),
        2,
    );
    assert!(features::rename(&workspace, &uri, target, "Renamed").is_err());
}

#[test]
fn a_shared_reference_with_a_different_or_missing_definition_rejects_rename() {
    for other_definition in ["", "@string{JOURNAL={Other}}"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("shared.bib"), "@book{shared,title=Journal}").unwrap();
        for name in ["a", "b"] {
            std::fs::create_dir(root.join(name)).unwrap();
            std::fs::write(
                root.join(name).join("main.tex"),
                r"\documentclass{article}\bibliography{defs,../shared}",
            )
            .unwrap();
            std::fs::write(
                root.join(name).join("defs.bib"),
                if name == "a" {
                    "@string{Journal={First}}"
                } else {
                    other_definition
                },
            )
            .unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let uri = Url::from_file_path(root.join("a/defs.bib")).unwrap();
        let target = at(&workspace.documents[&uri].text.source, "{Jour");
        assert!(features::prepare_rename(&workspace, &uri, target).is_err());
        assert!(features::rename(&workspace, &uri, target, "Renamed").is_err());
    }
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_concatenation_and_preamble_output_is_identical_after_abbreviation_rename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "@string{Prefix={Journal}}\n@string{Full=PREFIX # { of } # 2026}\n@preamble{\"\\newcommand{\\fixture}{\" # pRefix # \"}\"}\n";
    let entries =
        "@book{entry,author={Writer, A},title=Full,publisher=prefix,year=2026,month=JAN}\n";
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\bibliography{defs,refs}",
    )
    .unwrap();
    std::fs::write(root.join("defs.bib"), definitions).unwrap();
    std::fs::write(root.join("refs.bib"), entries).unwrap();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{entry}\n\\bibstyle{plain}\n\\bibdata{defs,refs}\n",
    )
    .unwrap();
    let compile = || {
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::read_to_string(root.join("main.bbl")).unwrap()
    };
    let original = compile();
    assert!(original.contains("Journal of 2026"));
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("defs.bib")).unwrap();
    let edit = features::rename(&workspace, &uri, at(definitions, "{Pref"), "Pub:Name_2026")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2);
    for (uri, edits) in changes {
        let path = uri.to_file_path().unwrap();
        let source = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, apply(&source, &edits)).unwrap();
    }
    assert_eq!(compile(), original);
}
