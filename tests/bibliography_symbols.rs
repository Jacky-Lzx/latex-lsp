use latex_lsp::{
    language::{features, navigation},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

fn doc(source: &str) -> Document {
    Document::new(
        &Url::parse("file:///symbols/refs.bib").unwrap(),
        source.into(),
        Some(1),
    )
}

fn slice(document: &Document, range: Range) -> &str {
    &document.text.source
        [document.text.offset(range.start).unwrap()..document.text.offset(range.end).unwrap()]
}

fn contained(symbols: &[DocumentSymbol], parent: Option<Range>) {
    for symbol in symbols {
        assert!(symbol.range.start <= symbol.selection_range.start);
        assert!(symbol.selection_range.end <= symbol.range.end);
        assert!(symbol.range.start < symbol.range.end);
        if let Some(parent) = parent {
            assert!(parent.start <= symbol.range.start && symbol.range.end <= parent.end);
        }
        if let Some(children) = &symbol.children {
            contained(children, Some(symbol.range));
        }
    }
}

#[test]
fn static_ids_aliases_are_nested_under_the_field_and_searchable_as_key_symbols() {
    let source="% 😀\r\n@book{owner,Ids={ old😀 , O\"Alias },title={Book}}\r\n@book(second,IDS=\"left,right\",title={Other})\r\n";
    let doc = doc(source);
    let symbols = features::symbols(&doc);
    contained(&symbols, None);
    for (entry, expected) in symbols
        .iter()
        .zip([vec!["old😀", "O\"Alias"], vec!["left", "right"]])
    {
        let field = &entry.children.as_ref().unwrap()[0];
        assert!(field.name.eq_ignore_ascii_case("ids"));
        assert_eq!(field.kind, SymbolKind::PROPERTY);
        let aliases = field.children.as_ref().unwrap();
        assert_eq!(
            aliases
                .iter()
                .map(|alias| alias.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        for alias in aliases {
            assert_eq!(alias.kind, SymbolKind::KEY);
            assert_eq!(slice(&doc, alias.range), alias.name);
            assert_eq!(alias.range, alias.selection_range);
            assert!(alias.detail.as_ref().unwrap().contains(&entry.name));
        }
    }
    let uri = Url::parse("file:///symbols/refs.bib").unwrap();
    let flat = features::flat_symbols(&doc, &uri);
    for name in ["old😀", "O\"Alias", "left", "right"] {
        let alias = flat.iter().find(|item| item.name == name).unwrap();
        assert_eq!(alias.kind, SymbolKind::KEY);
        assert!(alias
            .container_name
            .as_ref()
            .unwrap()
            .eq_ignore_ascii_case("ids"));
        assert_eq!(slice(&doc, alias.location.range), name);
    }
    let mut workspace = Workspace::default();
    workspace.documents.insert(uri.clone(), doc);
    let found = navigation::workspace_symbols(&workspace, "old😀");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "old😀");
    assert_eq!(found[0].location.uri, uri);
    assert!(navigation::workspace_symbols(&workspace, "ids").is_empty());
}

#[test]
fn alias_symbols_use_the_static_index_and_keep_duplicate_declarations_distinct() {
    for source in [
        "@book{owner,ids=legacy}",
        "@book{owner,ids={legacy} # {suffix}}",
        "@book{owner,ids={legacy},IDS={other}}",
        "@book{owner,ids={legacy}",
        "@book{owner,title={ids={legacy}}}",
        "@comment{@book{owner,ids={legacy}}}",
    ] {
        let doc = doc(source);
        let uri = Url::parse("file:///symbols/refs.bib").unwrap();
        assert!(
            features::flat_symbols(&doc, &uri)
                .iter()
                .all(|symbol| symbol.name != "legacy"),
            "{source}"
        );
    }
    let doc = doc("@book{owner,ids={same,same}} @book{other,ids={same}}");
    let uri = Url::parse("file:///symbols/refs.bib").unwrap();
    let flat = features::flat_symbols(&doc, &uri);
    let aliases: Vec<_> = flat.iter().filter(|symbol| symbol.name == "same").collect();
    assert_eq!(aliases.len(), 3);
    assert_ne!(aliases[0].location.range, aliases[1].location.range);
    assert_ne!(aliases[1].location.range, aliases[2].location.range);
    contained(&features::symbols(&doc), None);
}

#[test]
fn long_single_line_alias_lists_keep_every_original_utf16_selection_range() {
    let names: Vec<_> = (0..2000).map(|index| format!("alias{index}é😀")).collect();
    let source = format!(
        "% 😀\r\n@book{{owner,IDS={{{}}},title={{Book}}}}",
        names.join(", ")
    );
    let doc = doc(&source);
    let symbols = features::symbols(&doc);
    let aliases = symbols[0].children.as_ref().unwrap()[0]
        .children
        .as_ref()
        .unwrap();
    assert_eq!(aliases.len(), names.len());
    let mut start = source.find("alias0").unwrap();
    for (alias, name) in aliases.iter().zip(&names) {
        assert_eq!(&alias.name, name);
        assert_eq!(
            alias.selection_range,
            doc.text.range(start, start + name.len())
        );
        start += name.len() + 2;
    }
}

#[test]
fn outlines_cover_constructs_and_fields_with_exact_utf16_crlf_ranges() {
    let source = "% 😀\r\n  @ custom_type\r\n(entry😀,\r\n  TITLE = {A 😀 title},\r\n  author = \"Writer, A\",\r\n  year = 2026 \r\n)\r\n";
    let doc = doc(source);
    let symbols = features::symbols(&doc);
    contained(&symbols, None);
    assert_eq!(symbols.len(), 1);
    let entry = &symbols[0];
    assert_eq!(entry.name, "entry😀");
    assert_eq!(entry.kind, SymbolKind::KEY);
    assert_eq!(
        slice(&doc, entry.range),
        source[source.find('@').unwrap()..].trim_end()
    );
    assert_eq!(slice(&doc, entry.selection_range), "entry😀");
    assert!(entry.detail.as_ref().unwrap().contains("Title: A 😀 title"));
    let fields = entry.children.as_ref().unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["TITLE", "author", "year"]
    );
    for (field, expected) in fields.iter().zip([
        "TITLE = {A 😀 title}",
        "author = \"Writer, A\"",
        "year = 2026",
    ]) {
        assert_eq!(field.kind, SymbolKind::PROPERTY);
        assert_eq!(slice(&doc, field.range), expected);
        assert_eq!(slice(&doc, field.selection_range), field.name);
    }
    let offset = source.find("TITLE").unwrap();
    assert_eq!(
        fields[0].selection_range,
        Text::new(source.into()).range(offset, offset + 5)
    );
}

#[test]
fn string_symbols_cover_assignments_and_remain_separate_from_entry_fields() {
    let source = "@STRING (Prefix = {Journal})\n@misc{Prefix,title=Prefix # \" extra\"}\n@string{Next = Prefix # { next}}\n";
    let doc = doc(source);
    let symbols = features::symbols(&doc);
    contained(&symbols, None);
    assert_eq!(
        symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind))
            .collect::<Vec<_>>(),
        [
            ("Prefix", SymbolKind::CONSTANT),
            ("Prefix", SymbolKind::KEY),
            ("Next", SymbolKind::CONSTANT)
        ]
    );
    assert_eq!(
        slice(&doc, symbols[0].range),
        "@STRING (Prefix = {Journal})"
    );
    assert_eq!(
        slice(&doc, symbols[2].range),
        "@string{Next = Prefix # { next}}"
    );
    assert!(symbols[0].children.is_none());
    assert!(symbols[2].children.is_none());
    let fields = symbols[1].children.as_ref().unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(slice(&doc, fields[0].range), "title=Prefix # \" extra\"");
}

#[test]
fn quoted_nested_commented_and_special_entry_text_never_creates_extra_fields() {
    let source = concat!(
        "% @misc{comment,title={Hidden}}\n",
        "@comment{@misc{hidden,title={Hidden}}}\n",
        "@preamble{\"title = {Hidden}, @misc{fake,}\"}\n",
        "@string{Journal={author={Hidden}, @misc{fake,}}}\n",
        "@misc{visible,\n",
        " title = {A, title={Nested}, @misc{fake,}},\n",
        " note = \"url={literal}, @book{fake,} and % text\",\n",
        " % doi={comment}\n",
        " custom_field = {Part} # Journal # 2026\n",
        "}\n",
    );
    let doc = doc(source);
    let symbols = features::symbols(&doc);
    contained(&symbols, None);
    assert_eq!(
        symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Journal", "visible"]
    );
    let fields = symbols[1].children.as_ref().unwrap();
    assert_eq!(
        fields.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["title", "note", "custom_field"]
    );
    assert_eq!(
        slice(&doc, fields[2].range),
        "custom_field = {Part} # Journal # 2026"
    );
}

#[test]
fn incomplete_entries_fields_and_strings_keep_their_current_extent_without_folding() {
    for source in [
        "@book{entry,",
        "@book{entry, title",
        "@book{entry, title = ",
        "@book(entry, title = {Unfinished 😀\r\n  ",
        "@STRING{Journal = {Unfinished\n ",
    ] {
        let doc = doc(source);
        let symbols = features::symbols(&doc);
        contained(&symbols, None);
        assert_eq!(symbols.len(), 1, "{source}: {symbols:?}");
        assert_eq!(slice(&doc, symbols[0].range), source.trim_end());
        if source.contains("title") {
            let fields = symbols[0].children.as_ref().unwrap();
            assert_eq!(fields.len(), 1);
            assert_eq!(slice(&doc, fields[0].selection_range), "title");
            assert!(slice(&doc, fields[0].range).starts_with("title"));
        }
        assert!(navigation::folding_ranges(&doc, None).is_empty());
    }
    for source in [
        "@book",
        "@book{",
        "@comment{unfinished",
        "@preamble{unfinished",
    ] {
        assert!(features::symbols(&doc(source)).is_empty(), "{source}");
    }
}

#[test]
fn adjacent_entries_duplicate_keys_and_duplicate_fields_keep_distinct_owners() {
    let source = "@misc{same,title={First},TITLE=macro} @misc(same,year=2026)\n@misc{empty,}\n";
    let doc = doc(source);
    let symbols = features::symbols(&doc);
    contained(&symbols, None);
    assert_eq!(symbols.len(), 3);
    assert_eq!(
        symbols[0]
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["title", "TITLE"]
    );
    assert_eq!(symbols[1].children.as_ref().unwrap()[0].name, "year");
    assert!(symbols[2].children.is_none());
    assert!(symbols[0].range.end < symbols[1].range.start);
    assert_eq!(
        slice(&doc, symbols[0].range),
        "@misc{same,title={First},TITLE=macro}"
    );
    assert_eq!(slice(&doc, symbols[1].range), "@misc(same,year=2026)");
}

#[test]
fn legacy_outlines_flatten_fields_with_container_names_and_global_search_keeps_keys() {
    let uri = Url::parse("file:///symbols/refs.bib").unwrap();
    let doc = doc("@string{Journal={Value}}\n@misc{title,title={Text},year=2026}");
    let flat = features::flat_symbols(&doc, &uri);
    assert_eq!(
        flat.iter()
            .map(|s| (s.name.as_str(), s.container_name.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("Journal", None),
            ("title", None),
            ("title", Some("title")),
            ("year", Some("title"))
        ]
    );
    for symbol in &flat {
        assert_eq!(symbol.location.uri, uri);
        assert_eq!(slice(&doc, symbol.location.range), symbol.name);
    }
    let mut workspace = Workspace::default();
    workspace.documents.insert(uri, doc);
    let all = navigation::workspace_symbols(&workspace, "");
    assert_eq!(
        all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Journal", "title"]
    );
    assert_eq!(navigation::workspace_symbols(&workspace, "title").len(), 1);
    assert!(navigation::workspace_symbols(&workspace, "year").is_empty());
}

#[test]
fn language_identified_buffers_and_reopened_documents_refresh_outline_fields() {
    let uri = Url::parse("untitled:misleading.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.open_with_language(uri.clone(), "@misc{entry,title={Old}}".into(), 1, "bibtex");
    assert_eq!(
        features::symbols(&workspace.documents[&uri])[0]
            .children
            .as_ref()
            .unwrap()[0]
            .name,
        "title"
    );
    workspace.open_with_language(uri.clone(), "@misc{entry,year=2026}".into(), 2, "bib");
    assert_eq!(
        features::symbols(&workspace.documents[&uri])[0]
            .children
            .as_ref()
            .unwrap()[0]
            .name,
        "year"
    );
    workspace.open_with_language(
        uri.clone(),
        "\\section{Main}\\label{label}".into(),
        3,
        "latex",
    );
    let symbols = features::symbols(&workspace.documents[&uri]);
    assert_eq!(symbols[0].name, "Main");
    assert_eq!(symbols[0].children.as_ref().unwrap()[0].name, "label");
}

#[test]
fn large_bibliographies_keep_fields_in_their_own_entries() {
    let source = (0..4096)
        .map(|i| format!("@misc{{key{i},title={{Text {i}}},year=2026}}\n"))
        .collect::<String>();
    let doc = doc(&source);
    let symbols = features::symbols(&doc);
    assert_eq!(symbols.len(), 4096);
    contained(&symbols, None);
    for (i, symbol) in symbols.iter().enumerate() {
        assert_eq!(symbol.name, format!("key{i}"));
        let fields = symbol.children.as_ref().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(slice(&doc, fields[0].range), format!("title={{Text {i}}}"));
    }
}

#[test]
fn fieldless_braced_entries_are_outlined_and_keep_key_navigation_and_rename_ranges() {
    for (entry, key) in [
        ("@misc{empty}", "empty"),
        ("@custom_type{\r\nempty😀\r\n}", "empty😀"),
        ("@misc{book(old)}", "book(old)"),
        ("@misc{O\"Connor}", "O\"Connor"),
    ] {
        let uri = Url::parse("file:///symbols/refs.bib").unwrap();
        let source = format!("{entry}\n@misc{{child,crossref={{{key}}}}}\n");
        let doc = doc(&source);
        let symbols = features::symbols(&doc);
        assert!(!symbols.is_empty(), "{source}");
        assert_eq!(symbols[0].name, key);
        assert_eq!(slice(&doc, symbols[0].range), entry);
        assert_eq!(slice(&doc, symbols[0].selection_range), key);
        assert!(symbols[0].children.is_none());
        contained(&symbols, None);
        let position = doc.text.position(source.rfind(key).unwrap() + 1);
        let mut workspace = Workspace::default();
        workspace.documents.insert(uri.clone(), doc);
        let definitions = features::definitions(&workspace, &uri, position);
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].range, symbols[0].selection_range);
        let edit = features::rename(&workspace, &uri, position, "renamed")
            .unwrap()
            .unwrap();
        let edits = &edit.changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), 2);
        for edit in edits {
            assert_eq!(slice(&workspace.documents[&uri], edit.range), key);
        }
    }
    for invalid in ["@misc(empty)", "@misc{two words}", "@misc{}"] {
        assert!(features::symbols(&doc(invalid)).is_empty(), "{invalid}");
    }
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_fieldless_entries_match_outline_ranges_and_compiler_keys() {
    for (source, key) in [
        ("@misc{empty}\n", "empty"),
        ("@misc{\n empty\n}\n", "empty"),
        ("@misc{O\"Connor}\n", "O\"Connor"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("refs.bib"), source).unwrap();
        std::fs::write(
            root.join("main.aux"),
            format!("\\relax\n\\citation{{{key}}}\n\\bibstyle{{plain}}\n\\bibdata{{refs}}\n"),
        )
        .unwrap();
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let document = doc(source);
        let symbols = features::symbols(&document);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, key);
        assert_eq!(slice(&document, symbols[0].range), source.trim_end());
        assert!(symbols[0].children.is_none());
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        assert!(!log.contains("I didn't find a database entry"), "{log}");
    }
}

#[test]
fn quoted_entry_keys_do_not_capture_following_entries_or_value_fields() {
    for first in [
        "@misc{O\"Connor,title={First}}",
        "@misc(O\"Connor,title={First})",
    ] {
        let source = format!("{first}\n@misc{{next,year=2026}}\n");
        let doc = doc(&source);
        let symbols = features::symbols(&doc);
        contained(&symbols, None);
        assert_eq!(
            symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["O\"Connor", "next"]
        );
        assert_eq!(slice(&doc, symbols[0].range), first);
        assert_eq!(symbols[0].children.as_ref().unwrap()[0].name, "title");
        assert_eq!(symbols[1].children.as_ref().unwrap()[0].name, "year");
        assert_eq!(doc.syntax.citations[0].detail, "BibTeX misc · Title: First");
    }
}
