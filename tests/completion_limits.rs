use lsp_types::{CompletionItem, Url};
use texquill::{
    language::features,
    project::workspace::{Document, Workspace},
};

fn complete(source: &str, kind: &str, prefix: &str, limit: usize) -> Vec<CompletionItem> {
    let uri = Url::parse(&format!("file:///completion-limits/main.{kind}")).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.indexing.completion_limit = limit;
    let doc = Document::new(&uri, source.into(), Some(1));
    let position = doc
        .text
        .position(source.rfind(prefix).unwrap() + prefix.len());
    workspace.documents.insert(uri.clone(), doc);
    features::completion(&workspace, &uri, position)
}

#[test]
fn label_environment_and_command_budgets_apply_after_prefix_filtering_and_deduplication() {
    let mut labels = String::new();
    let mut environments = String::new();
    let mut commands = String::new();
    for index in 0..20 {
        labels.push_str(&format!(
            "\\label{{skip{index}}}\\label{{hit{index}}}\\label{{hit{index}}}\n"
        ));
        environments.push_str(&format!("\\newenvironment{{custom{index}}}{{}}{{}}\\begin{{custom{index}}}\\end{{custom{index}}}\n"));
        let suffix = char::from(b'a' + index);
        commands.push_str(&format!(
            "\\newcommand{{\\custom{suffix}}}{{}}\\newcommand{{\\custom{suffix}}}{{}}\n"
        ));
    }
    for (source, prefix) in [
        (format!("{labels}\\ref{{hi}}"), "hi"),
        (format!("{environments}\\begin{{custom}}"), "custom"),
        (format!("{commands}\\custom"), "custom"),
    ] {
        let items = complete(&source, "tex", prefix, 3);
        assert_eq!(items.len(), 4, "{items:?}");
        let names: std::collections::BTreeSet<_> = items.iter().map(|item| &item.label).collect();
        assert_eq!(names.len(), 4);
        assert!(items.iter().all(|item| item.label.starts_with(prefix)));
    }
}

#[test]
fn bibliography_types_fields_strings_and_relations_use_the_same_budget() {
    let mut types = String::new();
    let mut fields = String::new();
    let mut strings = String::new();
    let mut parents = String::new();
    for index in 0..20 {
        types.push_str(&format!("@custom{index}{{type{index}, title={{Text}}}}\n"));
        fields.push_str(&format!("@book{{field{index}, custom{index}={{Text}}}}\n"));
        strings.push_str(&format!("@string{{custom{index}=\"Text\"}}\n"));
        parents.push_str(&format!("@book{{parent{index},title={{Text}}}}\n"));
    }
    for (source, prefix) in [
        (format!("{types}@custom"), "custom"),
        (format!("{fields}@book{{last, custom"), "custom"),
        (format!("{strings}@book{{last,title=custom}}"), "custom"),
        (
            format!("{parents}@inbook{{child,crossref={{parent}}}}"),
            "parent",
        ),
    ] {
        let items = complete(&source, "bib", prefix, 3);
        assert_eq!(items.len(), 4, "{source}: {items:?}");
    }
}

#[test]
fn citation_budget_keeps_better_candidates_encountered_later() {
    let uri = Url::parse("file:///completion-limits/main.tex").unwrap();
    let bib = uri.join("refs.bib").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.indexing.completion_limit = 1;
    let source = r"\cite{needle}";
    let doc = Document::new(&uri, source.into(), Some(1));
    let position = doc.text.position(source.len() - 1);
    workspace.documents.insert(uri.clone(), doc);
    workspace.documents.insert(bib.clone(), Document::new(&bib,
        "@book{metadata,title={needle}}\n@book{needleZ,title={Last alphabetically}}\n@book{needleA,title={First alphabetically}}".into(), None));
    let items = features::completion(&workspace, &uri, position);
    assert_eq!(
        items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["needleA", "needleZ"]
    );
}

#[test]
fn path_budget_preserves_folder_priority_and_alphabetical_order() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["000.tex", "ccc.tex", "bbb.tex"] {
        std::fs::write(directory.path().join(name), "").unwrap();
    }
    for name in ["zzzfolder", "aaafolder"] {
        std::fs::create_dir(directory.path().join(name)).unwrap();
    }
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = r"\input{}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.indexing.completion_limit = 1;
    let doc = Document::new(&uri, source.into(), Some(1));
    let position = doc.text.position(source.len() - 1);
    workspace.documents.insert(uri.clone(), doc);
    let items = features::completion(&workspace, &uri, position);
    assert_eq!(
        items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["aaafolder/", "zzzfolder/"]
    );
}
