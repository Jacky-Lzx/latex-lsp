use latex_lsp::{
    features, semantic,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn workspace(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///bibtex-field-names/refs.bib").unwrap();
    let mut workspace = Workspace::default();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    (workspace, uri)
}

fn position(source: &str, marker: &str) -> Position {
    Text::new(source.into()).position(source.find(marker).unwrap() + marker.len())
}

#[test]
fn legal_field_identifiers_keep_following_metadata_references_and_property_ranges() {
    for name in [
        "custom:field",
        ":field",
        "_field",
        "-field",
        "field.",
        "field$",
        "fieldé",
        "中😀",
        "field@",
        "field\\",
        "field[",
        "field|",
        "field1",
    ] {
        for (open, close) in [('{', '}'), ('(', ')')] {
            let source = format!("@string{{Full={{Text}}}}\r\n@book{open}entry, {name} = Full, title={{Kept}}, year=2026, crossref={{parent}}{close}\r\n@book{{parent}}");
            let (workspace, uri) = workspace(&source);
            let doc = &workspace.documents[&uri];
            let entry = &doc.syntax.citations[0];
            assert!(
                entry.detail.contains("Title: Kept"),
                "{source}: {}",
                entry.detail
            );
            assert!(entry.detail.contains("Year: 2026"));
            assert_eq!(doc.syntax.citation_references[0].name, "parent");
            let symbols = features::symbols(doc);
            let fields = symbols
                .iter()
                .find(|symbol| symbol.name == "entry")
                .unwrap()
                .children
                .as_ref()
                .unwrap();
            assert_eq!(
                fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>(),
                [name, "title", "year", "crossref"]
            );
            let range = fields[0].selection_range;
            assert_eq!(
                &source[doc.text.offset(range.start).unwrap()..doc.text.offset(range.end).unwrap()],
                name
            );
            let definition = features::definitions(&workspace, &uri, position(&source, "= Fu"));
            assert_eq!(definition.len(), 1, "{source}");
            let rename = features::rename(&workspace, &uri, position(&source, "= Fu"), "Renamed")
                .unwrap()
                .unwrap();
            assert_eq!(rename.changes.unwrap()[&uri].len(), 2);
            let tokens = semantic::tokens(doc, Some(range)).unwrap().data;
            assert_eq!(tokens.len(), 1);
            let legend = semantic::options().legend;
            assert_eq!(
                legend.token_types[tokens[0].token_type as usize],
                SemanticTokenType::PROPERTY
            );
            assert_eq!(tokens[0].length as usize, name.encode_utf16().count());
        }
    }
}

#[test]
fn completion_replaces_the_entire_identifier_and_preserves_its_assignment() {
    for name in ["AUTH:bad", "AUTH$bad", "AUTHébad", "AUTH@bad"] {
        let source = format!("@book{{😀, {name} % keep\r\n = {{Writer}}, title={{Text}}}}");
        let (workspace, uri) = workspace(&source);
        let item = features::completion(&workspace, &uri, position(&source, "AUTH"))
            .into_iter()
            .find(|item| item.label == "author")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        let text = Text::new(source.clone());
        assert_eq!(
            &source[text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap()],
            name
        );
        let mut updated = source.clone();
        updated.replace_range(
            text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
            &edit.new_text,
        );
        assert_eq!(updated, source.replace(name, "author"));
    }
}

#[test]
fn numeric_field_names_do_not_create_valid_assignments_or_abbreviation_references() {
    let source = "@string{Full={Text}}\n@book{entry,title={Kept},123field=Full,crossref={parent}}";
    let (workspace, uri) = workspace(source);
    let doc = &workspace.documents[&uri];
    let symbols = features::symbols(doc);
    let fields = symbols
        .iter()
        .find(|symbol| symbol.name == "entry")
        .unwrap()
        .children
        .as_ref()
        .unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["title"]
    );
    assert!(doc.syntax.citation_references.is_empty());
    assert!(features::definitions(&workspace, &uri, position(source, "=Fu")).is_empty());
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_custom_fields_and_following_titles_match_the_index() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{entry}\n\\bibstyle{fixture}\n\\bibdata{refs}\n",
    )
    .unwrap();
    for name in [
        "custom:field",
        ":field",
        "_field",
        "-field",
        "field.",
        "field$",
        "fieldé",
        "中😀",
        "field@",
        "field\\",
        "field[",
        "field|",
    ] {
        std::fs::write(root.join("fixture.bst"), format!("ENTRY {{{name} title}} {{}} {{}}\nFUNCTION {{emit}} {{{name} write$ \"|\" write$ title write$ newline$}}\nREAD\nITERATE {{emit}}\n")).unwrap();
        let source =
            format!("@string{{Full={{Value}}}}\n@misc{{entry, {name}=Full, title={{Kept}}}}\n");
        std::fs::write(root.join("refs.bib"), &source).unwrap();
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            std::fs::read_to_string(root.join("main.bbl"))
                .unwrap()
                .trim(),
            "Value|Kept"
        );
        let (workspace, uri) = workspace(&source);
        assert!(workspace.documents[&uri].syntax.citations[0]
            .detail
            .contains("Title: Kept"));
        assert_eq!(
            features::definitions(&workspace, &uri, position(&source, "=Fu")).len(),
            1
        );
        let rename = features::rename(&workspace, &uri, position(&source, "=Fu"), "Renamed")
            .unwrap()
            .unwrap();
        let mut edits = rename.changes.unwrap().remove(&uri).unwrap();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let text = Text::new(source.clone());
        let mut updated = source;
        for edit in edits {
            updated.replace_range(
                text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
                &edit.new_text,
            );
        }
        std::fs::write(root.join("refs.bib"), updated).unwrap();
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            std::fs::read_to_string(root.join("main.bbl"))
                .unwrap()
                .trim(),
            "Value|Kept"
        );
    }
}
