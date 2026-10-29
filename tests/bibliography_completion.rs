use lsp_types::*;
use texquill::{
    language::features,
    project::workspace::{Document, Workspace},
    text::Text,
};

fn workspace(source: &str, snippets: bool) -> (Workspace, Url) {
    let uri = Url::parse("file:///bib-completion/refs.bib").unwrap();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            snippets,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    (workspace, uri)
}

fn position(source: &str, marker: &str) -> Position {
    Text::new(source.into()).position(source.find(marker).unwrap() + marker.len())
}

fn item(source: &str, marker: &str, label: &str, snippets: bool) -> CompletionItem {
    let (workspace, uri) = workspace(source, snippets);
    features::completion(&workspace, &uri, position(source, marker))
        .into_iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("missing {label} in {source:?} at {marker:?}"))
}

fn edited(source: &str, item: &CompletionItem) -> String {
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    let text = Text::new(source.into());
    let mut result = source.to_owned();
    result.replace_range(
        text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
        &edit.new_text,
    );
    result
}

#[test]
fn entry_types_complete_empty_and_partial_prefixes_without_replacing_the_at_sign() {
    for (source, marker) in [
        ("@", "@"),
        ("@  ArTBAD", "ArT"),
        ("😀 @\r\n  artBAD", "art"),
    ] {
        let result = item(source, marker, "article", false);
        assert_eq!(result.kind, Some(CompletionItemKind::CLASS));
        assert_eq!(
            result.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT)
        );
        let expected = if source.ends_with("ArTBAD") {
            source.replace("ArTBAD", "article")
        } else if source.ends_with("artBAD") {
            source.replace("artBAD", "article")
        } else {
            "@article".into()
        };
        assert_eq!(edited(source, &result), expected);
    }
    let (workspace, uri) = workspace("@", false);
    let completions = features::completion(&workspace, &uri, Position::new(0, 1));
    for name in [
        "article", "book", "online", "software", "xdata", "comment", "string", "preamble",
    ] {
        assert!(completions.iter().any(|item| item.label == name));
    }
}

#[test]
fn bare_at_before_another_entry_preserves_both_completion_contexts() {
    for whitespace in ["", "\n", "\r\n", " \n\n"] {
        for snippets in [false, true] {
            let prefix = "@book{existing, title={Keep me}}\n\n";
            let following = "@book{following, title={Also keep me}}\n";
            let source = format!("{prefix}@{whitespace}{following}");
            let (workspace, uri) = workspace(&source, snippets);
            let at = Text::new(source.clone()).position(prefix.len() + 1);
            let items = features::completion(&workspace, &uri, at);
            let article = items.iter().find(|item| item.label == "article").unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &article.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.range, Range::new(at, at));
            assert_eq!(
                article.insert_text_format,
                Some(if snippets {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                })
            );
            let completed = edited(&source, article);
            assert!(completed.starts_with(&format!("{prefix}@article")));
            assert!(completed.ends_with(&format!("{whitespace}{following}")));
            assert!(items.iter().all(|item| item.label != "@book"));
            let following_at =
                Text::new(source.clone()).position(prefix.len() + 1 + whitespace.len() + 1);
            assert!(features::completion(&workspace, &uri, following_at)
                .iter()
                .any(|item| item.label == "book"));
        }
    }
}

#[test]
fn bare_at_completes_before_trailing_whitespace_without_consuming_it() {
    for prefix in ["", "@book{existing, title={Keep me}}\n\n"] {
        for suffix in ["\n", "\r\n", "  \n\n"] {
            for snippets in [false, true] {
                let source = format!("{prefix}@{suffix}");
                let (workspace, uri) = workspace(&source, snippets);
                let items = features::completion(
                    &workspace,
                    &uri,
                    Text::new(source.clone()).position(prefix.len() + 1),
                );
                let article = items.iter().find(|item| item.label == "article").unwrap();
                let edited = edited(&source, article);
                assert!(edited.starts_with(&format!("{prefix}@article")));
                assert!(edited.ends_with(suffix), "{edited:?}");
                assert_eq!(article.kind, Some(CompletionItemKind::CLASS));
                assert_eq!(
                    article.insert_text_format,
                    Some(if snippets {
                        InsertTextFormat::SNIPPET
                    } else {
                        InsertTextFormat::PLAIN_TEXT
                    })
                );
            }
        }
    }
}

#[test]
fn entry_snippets_use_the_document_newline_and_existing_bodies_are_preserved() {
    let source = "% 中文😀\r\n@art";
    let result = item(source, "@art", "article", true);
    assert_eq!(result.insert_text_format, Some(InsertTextFormat::SNIPPET));
    let expected = "% 中文😀\r\n@article{${1:key},\r\n  author = {${2:}},\r\n  title = {${3:}},\r\n  journal = {${4:}},\r\n  year = {${5:}},\r\n}$0";
    assert_eq!(edited(source, &result), expected);
    for source in [
        "@artBAD{key,title={Keep me}}",
        "@artBAD (key,title={Keep me})",
        "@artBAD% comment\n{key,title={Keep me}}",
    ] {
        let result = item(source, "@art", "article", true);
        assert_eq!(
            result.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT)
        );
        assert_eq!(edited(source, &result), source.replace("artBAD", "article"));
    }
}

#[test]
fn field_completion_replaces_whole_names_and_keeps_assignments_values_and_comments() {
    for source in [
        "@book{key, AUTHbad = {Writer}}",
        "@book(key, AUTHbad% keep comment\r\n = \"Writer\")",
        "@book{😀, title={标题}, AUTHbad = {Writer}}",
    ] {
        for snippets in [false, true] {
            let result = item(source, "AUTH", "author", snippets);
            assert_eq!(
                result.insert_text_format,
                Some(InsertTextFormat::PLAIN_TEXT)
            );
            assert_eq!(edited(source, &result), source.replace("AUTHbad", "author"));
        }
    }
}

#[test]
fn fields_complete_after_commas_and_in_unfinished_entries_with_snippet_capability() {
    for (source, marker, expected) in [
        ("@book{key, auth", "auth", "@book{key, author = {${1:}},"),
        ("@book{key, auth}", "auth", "@book{key, author = {${1:}},}"),
        ("@book{key, auth,}", "auth", "@book{key, author = {${1:}},}"),
        (
            "@book{key,\n  }",
            "\n  ",
            "@book{key,\n  author = {${1:}},}",
        ),
        (
            "@book{key,\n % note\n  }",
            "\n  ",
            "@book{key,\n % note\n  author = {${1:}},}",
        ),
        (
            "@book{key, = {Writer}}",
            "key, ",
            "@book{key, author= {Writer}}",
        ),
    ] {
        let result = item(source, marker, "author", true);
        assert_eq!(edited(source, &result), expected, "{source:?}");
    }
    let source = "@book{key, auth}";
    let result = item(source, "auth", "author", false);
    assert_eq!(edited(source, &result), "@book{key, author}");
}

#[test]
fn assigned_fields_are_excluded_case_insensitively_but_the_current_name_can_be_completed() {
    let source = "@article{key, AUTHOR={Writer}, title={Title}, \n  }";
    let (workspace, uri) = workspace(source, true);
    let items = features::completion(&workspace, &uri, position(source, "\n  "));
    assert!(!items
        .iter()
        .any(|item| item.label == "author" || item.label == "title"));
    let journal = items.iter().find(|item| item.label == "journal").unwrap();
    let note = items.iter().find(|item| item.label == "note").unwrap();
    assert!(journal.sort_text < note.sort_text);
    let result = item(source, "AUTH", "author", true);
    assert_eq!(edited(source, &result), source.replace("AUTHOR", "author"));
}

#[test]
fn names_are_not_completed_inside_values_keys_comments_or_special_entry_bodies() {
    for (source, marker) in [
        ("@book{key, title={auth}}", "auth"),
        ("@book{key, title=\"@art\"}", "@art"),
        ("@book{key, title={x, auth}}", "auth"),
        ("@book{key, url={https://example.org/@art}}", "@art"),
        ("@book{key, title=prefix # {auth}}", "auth"),
        ("@book{auth,title={Title}}", "{auth"),
        ("% @art", "@art"),
        ("@book{key, % auth\n}", "auth"),
        ("@comment{ @art, auth }", "@art"),
        ("@string{auth=\"value\"}", "{auth"),
        ("@preamble{\"@art\"}", "@art"),
        ("@book{key, title={unfinished auth", "auth"),
    ] {
        let (workspace, uri) = workspace(source, true);
        assert!(
            features::completion(&workspace, &uri, position(source, marker)).is_empty(),
            "{source:?}"
        );
    }
}

#[test]
fn custom_fields_preserve_spelling_deduplicate_and_rank_after_builtin_fields() {
    let source = "@book{one, Custom:Field={Value}, custom:FIELD={Again}, AUTHOR={Writer}, Unassigned}\r\n@article{two, \r\n  }";
    let (workspace, uri) = workspace(source, false);
    let items = features::completion(&workspace, &uri, position(source, "\r\n  "));
    let custom: Vec<_> = items
        .iter()
        .filter(|item| item.label.eq_ignore_ascii_case("custom:field"))
        .collect();
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].label, "Custom:Field");
    assert_eq!(custom[0].kind, Some(CompletionItemKind::FIELD));
    assert!(custom[0].detail.as_ref().unwrap().contains("Custom"));
    let author = items.iter().find(|item| item.label == "author").unwrap();
    let note = items.iter().find(|item| item.label == "note").unwrap();
    assert!(author.sort_text < note.sort_text);
    assert!(note.sort_text < custom[0].sort_text);
    assert!(!items
        .iter()
        .any(|item| item.label == "AUTHOR" || item.label == "Unassigned"));
}

#[test]
fn custom_field_snippets_escape_names_and_plain_edits_keep_them_literal() {
    for (name, prefix) in [
        ("Cost$Field", "c"),
        (r"Path\Field", "p"),
        (r"Both\$Field", "b"),
        ("中😀Field", "中"),
        ("Cost$Field", "cost$"),
        (r"Path\Field", "path\\"),
        (r"Both\$Field", "both\\$"),
        ("中😀Field", "中😀"),
    ] {
        for snippets in [false, true] {
            for (tail, assigned, comma) in [
                ("", false, false),
                ("}", false, false),
                (",}", false, true),
                (" % keep\r\n = {Original}}", true, false),
            ] {
                let source =
                    format!("@book{{old,{name}={{Value}}}}\r\n@book{{😀,{prefix}BAD{tail}");
                let marker = format!("😀,{prefix}");
                let result = item(&source, &marker, name, snippets);
                let Some(CompletionTextEdit::Edit(edit)) = &result.text_edit else {
                    panic!()
                };
                assert_eq!(edit.range.start.line, 1);
                assert_eq!(edit.range.start.character, 9);
                assert_eq!(
                    edit.range.end.character as usize,
                    9 + prefix.encode_utf16().count() + 3
                );
                let snippet = snippets && !assigned;
                assert_eq!(
                    result.insert_text_format,
                    Some(if snippet {
                        InsertTextFormat::SNIPPET
                    } else {
                        InsertTextFormat::PLAIN_TEXT
                    })
                );
                let expected = if snippet {
                    format!(
                        "{} = {{${{1:}}}}{}",
                        name.replace('\\', "\\\\").replace('$', "\\$"),
                        if comma { "" } else { "," }
                    )
                } else {
                    name.into()
                };
                assert_eq!(edit.new_text, expected);
                assert_eq!(
                    edited(&source, &result),
                    source.replace(&format!("{prefix}BAD"), &expected)
                );
            }
        }
    }
}

#[test]
fn custom_fields_exclude_existing_assignments_and_allow_editing_the_current_field() {
    let source = "@book{old,Custom:Field={Value}}\n@book{new,CUSTOM:FIELD={Keep}, Cu}";
    let (workspace, uri) = workspace(source, true);
    let items = features::completion(&workspace, &uri, position(source, ", Cu"));
    assert!(!items
        .iter()
        .any(|item| item.label.eq_ignore_ascii_case("custom:field")));
    let result = item(source, "new,CU", "Custom:Field", true);
    assert_eq!(
        result.insert_text_format,
        Some(InsertTextFormat::PLAIN_TEXT)
    );
    assert_eq!(
        edited(source, &result),
        source.replace("CUSTOM:FIELD", "Custom:Field")
    );
}

#[test]
fn custom_fields_ignore_comments_special_constructs_and_invalid_or_unassigned_names() {
    let source = "% @book{fake,CommentField={Text}}\n@comment{@book{fake,HiddenField={Text}}}\n@string{StringName={Text}}\n@preamble{\"PreambleField={Text}\"}\n@book{invalid,123field={Value},AfterInvalid={Text}}\n@book{partial,Incomplete}\n@book{empty,EmptyField=}\n@book{known,KnownField={Value}}\n@book{query, \n  }";
    let (project, uri) = workspace(source, false);
    let items = features::completion(&project, &uri, position(source, "\n  "));
    for name in [
        "CommentField",
        "HiddenField",
        "StringName",
        "PreambleField",
        "123field",
        "AfterInvalid",
        "Incomplete",
    ] {
        assert!(!items.iter().any(|item| item.label == name), "{name}");
    }
    for name in ["KnownField", "EmptyField"] {
        assert!(items.iter().any(|item| item.label == name), "{name}");
    }
    for (tail, marker) in [
        ("@book{Cus,title={Text}}", "{Cus"),
        ("@book{key,title={Cus}}", "{Cus"),
        ("@book{key,title=\"Cus\"}", "\"Cus"),
        ("@book{key, % Cus\n}", "% Cus"),
        ("@comment{@book{key,Cus}}", "key,Cus"),
        ("@preamble{\"Cus\"}", "\"Cus"),
    ] {
        let source = format!("@book{{old,Custom:Field={{Value}}}}\n{tail}");
        let (workspace, uri) = workspace(&source, true);
        let items = features::completion(&workspace, &uri, position(&source, marker));
        assert!(
            !items.iter().any(|item| item.label == "Custom:Field"),
            "{source}"
        );
    }
}

#[test]
fn custom_field_suggestions_use_all_resources_in_the_project_and_prefer_current_spelling() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["first", "second"] {
        let root = directory.path().join(name);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("main.tex"),
            r"\documentclass{article}\bibliography{a-shared,z-query}",
        )
        .unwrap();
        std::fs::write(
            root.join("a-shared.bib"),
            if name == "first" {
                "@book{other,REMOTE:Field={Shared},local$field={Remote spelling}}"
            } else {
                "@book{other,FOREIGN:Field={Other project}}"
            },
        )
        .unwrap();
        std::fs::write(
            root.join("z-query.bib"),
            "@book{local,Local$Field={Local}}\r\n@book{query, \r\n  }",
        )
        .unwrap();
    }
    let source = std::fs::read_to_string(directory.path().join("first/z-query.bib")).unwrap();
    let uri = Url::from_file_path(directory.path().join("first/z-query.bib")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let items = features::completion(&workspace, &uri, position(&source, "\r\n  "));
    assert!(items.iter().any(|item| item.label == "REMOTE:Field"));
    assert!(items.iter().any(|item| item.label == "Local$Field"));
    assert!(!items
        .iter()
        .any(|item| item.label == "local$field" || item.label == "FOREIGN:Field"));
}

#[test]
fn custom_entry_types_in_the_project_are_available_and_snippet_metacharacters_are_escaped() {
    let source = "@Custom_Type{existing,title={Keep}}\n@Custom$Type{other,title={Other}}\n@cus";
    let (workspace, uri) = workspace(source, true);
    let items = features::completion(&workspace, &uri, position(source, "@cus"));
    for (label, escaped) in [
        ("Custom_Type", "Custom_Type"),
        ("Custom$Type", r"Custom\$Type"),
    ] {
        let result = items.iter().find(|item| item.label == label).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &result.text_edit else {
            panic!()
        };
        assert!(edit
            .new_text
            .starts_with(&format!("{escaped}{{${{1:key}},\n")));
    }
    assert_eq!(
        items
            .iter()
            .filter(|item| item.label == "Custom_Type")
            .count(),
        1
    );
}

#[test]
fn custom_type_suggestions_follow_bibliography_project_scope() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["first", "second"] {
        std::fs::create_dir(directory.path().join(name)).unwrap();
        std::fs::write(
            directory.path().join(name).join("main.tex"),
            r"\documentclass{article}\bibliography{refs}",
        )
        .unwrap();
    }
    let source = "@Local_Record{key,title={Local}}\n@";
    let local = directory.path().join("first/refs.bib");
    std::fs::write(&local, source).unwrap();
    std::fs::write(
        directory.path().join("second/refs.bib"),
        "@Foreign_Record{key,title={Other}}",
    )
    .unwrap();
    let uri = Url::from_file_path(local).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let items = features::completion(
        &workspace,
        &uri,
        Text::new(source.into()).position(source.len()),
    );
    assert!(items.iter().any(|item| item.label == "Local_Record"));
    assert!(!items.iter().any(|item| item.label == "Foreign_Record"));
}

#[test]
fn bibliography_type_and_field_hovers_use_exact_name_ranges_and_keep_key_hovers() {
    let source = "@Article{😀, AUTHOR={Writer}, title={Text author}, crossref={parent}}\n@book{parent,title={Parent}}";
    let (workspace, uri) = workspace(source, false);
    for (marker, expected, range_text) in [
        ("@Art", "journal", "Article"),
        ("AUTH", "separated", "AUTHOR"),
    ] {
        let hover = features::hover(&workspace, &uri, position(source, marker)).unwrap();
        let HoverContents::Markup(content) = hover.contents else {
            panic!()
        };
        assert!(content.value.contains(expected));
        let range = hover.range.unwrap();
        let text = Text::new(source.into());
        assert_eq!(
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            range_text
        );
    }
    let key = features::hover(&workspace, &uri, position(source, "crossref={par")).unwrap();
    let HoverContents::Markup(content) = key.contents else {
        panic!()
    };
    assert!(content.value.contains("Parent"));
    assert!(features::hover(&workspace, &uri, position(source, "Text auth")).is_none());
}

#[test]
fn existing_bibliography_key_completions_remain_available_after_field_name_completion() {
    let source = "@book{parent,title={Parent}}\n@book{child,related={other, parBAD, tail}}";
    let (workspace, uri) = workspace(source, true);
    let items = features::completion(&workspace, &uri, position(source, "related={other, par"));
    let parent = items.iter().find(|item| item.label == "parent").unwrap();
    assert_eq!(edited(source, parent), source.replace("parBAD", "parent"));
    assert_eq!(parent.kind, Some(CompletionItemKind::REFERENCE));
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_reads_completed_custom_fields_without_changing_assigned_values() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{entry}\n\\bibstyle{fixture}\n\\bibdata{refs}\n",
    )
    .unwrap();
    for name in ["Cost$Field", r"Path\Field", "Custom:Field", "中😀Field"] {
        std::fs::write(root.join("fixture.bst"), format!("ENTRY {{{name} title}} {{}} {{}}\nFUNCTION {{emit}} {{{name} write$ \"|\" write$ title write$ newline$}}\nREAD\nITERATE {{emit}}\n")).unwrap();
        let prefix = name.chars().next().unwrap().to_ascii_lowercase();
        let source = format!("@misc{{prototype,{name}={{Old}}}}\r\n@misc{{entry, {prefix}BAD = {{New}}, title={{Kept}}}}");
        let result = item(&source, &format!("entry, {prefix}"), name, true);
        assert_eq!(
            result.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT)
        );
        let updated = edited(&source, &result);
        assert!(updated.contains(&format!("{name} = {{New}}")));
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
            "New|Kept"
        );
    }
}

#[test]
#[ignore = "requires Neovim 0.10+ with vim.snippet"]
fn bibliography_snippets_expand_in_neovim_with_literal_braces_and_special_entry_grammars() {
    let mut cases = Vec::new();
    for (name, expected) in [
        (
            "article",
            "article{key,\n  author = {},\n  title = {},\n  journal = {},\n  year = {},\n}",
        ),
        (
            "online",
            "online{key,\n  author = {},\n  title = {},\n  url = {},\n  urldate = {},\n}",
        ),
        ("xdata", "xdata{key,\n  \n}"),
        ("comment", "comment{text}"),
        ("string", "string{name = {value}}"),
        ("preamble", "preamble{\"text\"}"),
    ] {
        let result = item("@", "@", name, true);
        let Some(CompletionTextEdit::Edit(edit)) = result.text_edit else {
            panic!()
        };
        cases.push(serde_json::json!({"snippet":edit.new_text,"expected":expected}));
    }
    for (source, marker, label, expected) in [
        ("@book{key, auth}", "auth", "author", "author = {},"),
        ("@book{key, auth,}", "auth", "author", "author = {}"),
        (
            "@Cash$Type{old,title={Old}}\n@Ca",
            "\n@Ca",
            "Cash$Type",
            "Cash$Type{key,\n  \n}",
        ),
        (
            "@book{old,Cost$Field={Old}}\n@book{new, co}",
            ", co",
            "Cost$Field",
            "Cost$Field = {},",
        ),
        (
            "@book{old,Path\\$Field={Old}}\n@book{new, pa,}",
            ", pa",
            "Path\\$Field",
            "Path\\$Field = {}",
        ),
    ] {
        let result = item(source, marker, label, true);
        let Some(CompletionTextEdit::Edit(edit)) = result.text_edit else {
            panic!()
        };
        cases.push(serde_json::json!({"snippet":edit.new_text,"expected":expected}));
    }
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("bibliography-snippets.lua");
    let source = format!(
        r#"
local cases = vim.json.decode([==[{}]==])
for _, case in ipairs(cases) do
  vim.cmd('enew!')
  vim.snippet.expand(case.snippet)
  local actual = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n')
  assert(actual == case.expected, vim.inspect({{ snippet = case.snippet, expected = case.expected, actual = actual }}))
  vim.snippet.stop()
end
vim.cmd('qa!')
"#,
        serde_json::to_string(&cases).unwrap()
    );
    std::fs::write(&script, source).unwrap();
    let output = std::process::Command::new("nvim")
        .args(["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l"])
        .arg(script)
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .env("XDG_STATE_HOME", directory.path().join("state"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
