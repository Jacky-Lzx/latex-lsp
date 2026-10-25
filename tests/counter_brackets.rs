use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///counter-brackets/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(4)));
    uri
}

fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
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
fn literal_bracket_counter_references_survive_independent_and_repeated_renames() {
    let mut workspace = Workspace::default();
    let declaration = "\\newcounter{counter[literal}\n\\newcounter{other}\n";
    let definitions = insert(&mut workspace, "definitions.tex", declaration);
    let source = "😀 \\newcounter{child}[counter[literal]\n\\newtheorem{shared}[counter[literal]{Title counter[literal}\n\\newtheorem{within}{Title counter[literal}[counter[literal]\n\\stepcounter{counter[literal}\n% \\stepcounter{counter[literal}\n\\verb|\\stepcounter{counter[literal}|\n";
    let main = insert(&mut workspace, "main.tex", source);
    let at = Text::new(declaration.into()).position("\\newcounter{counter[li".len());
    assert_eq!(
        features::references(&workspace, &definitions, at, false).len(),
        4
    );
    for marker in [
        "{child}[counter[li",
        "{shared}[counter[li",
        "{Title counter[literal}[counter[li",
    ] {
        let point = Text::new(source.into()).position(source.find(marker).unwrap() + marker.len());
        let locations = features::definitions(&workspace, &main, point);
        assert_eq!(locations.len(), 1, "{marker}: {locations:?}");
        assert_eq!(locations[0].uri, definitions);
    }
    for name in ["renamed", "counter]new"] {
        let edit = features::rename(&workspace, &definitions, at, name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        let shared = if name.contains(']') {
            format!("{{{name}}}")
        } else {
            name.into()
        };
        let within = if name.contains(']') {
            format!("\\detokenize{{{name}}}")
        } else {
            name.into()
        };
        let expected = source
            .replace("{child}[counter[literal]", &format!("{{child}}[{shared}]"))
            .replace(
                "{shared}[counter[literal]",
                &format!("{{shared}}[{shared}]"),
            )
            .replace(
                "{Title counter[literal}[counter[literal]",
                &format!("{{Title counter[literal}}[{within}]"),
            )
            .replacen(
                "\\stepcounter{counter[literal}",
                &format!("\\stepcounter{{{name}}}"),
                1,
            );
        let changed = apply(source, edit[&main].clone());
        assert_eq!(changed, expected, "{name}");
        assert_eq!(
            apply(declaration, edit[&definitions].clone()),
            declaration.replace("{counter[literal}", &format!("{{{name}}}"))
        );
        if name.contains(']') {
            let mut updated = Workspace::default();
            let definitions = insert(
                &mut updated,
                "definitions.tex",
                &format!("\\newcounter{{{name}}}\n\\newcounter{{other}}\n"),
            );
            let main = insert(&mut updated, "main.tex", &changed);
            let at = Text::new(changed.clone())
                .position(changed.find("\\stepcounter{").unwrap() + "\\stepcounter{cou".len());
            assert_eq!(features::references(&updated, &main, at, false).len(), 4);
            let edit = features::rename(&updated, &main, at, "final")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(
                apply(&changed, edit[&main].clone()),
                changed.replace(name, "final")
            );
            assert_eq!(
                apply(
                    &updated.documents[&definitions].text.source,
                    edit[&definitions].clone()
                ),
                "\\newcounter{final}\n\\newcounter{other}\n"
            );
        }
    }
}

#[test]
fn counter_completion_uses_the_protection_needed_by_each_optional_argument() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "definitions.tex",
        "\\newcounter{counter[literal}\n\\newcounter{counter]new}\n",
    );
    for (marked, name, expected) in [
        (
            r"\newcounter{child}[cou¦BAD]",
            "counter]new",
            r"\newcounter{child}[{counter]new}]",
        ),
        (
            r"\newtheorem{shared}[cou¦BAD]{Title}",
            "counter]new",
            r"\newtheorem{shared}[{counter]new}]{Title}",
        ),
        (
            r"\newtheorem{within}{Title}[cou¦BAD]",
            "counter]new",
            r"\newtheorem{within}{Title}[\detokenize{counter]new}]",
        ),
        (
            r"\newtheorem{within}{Title}[{cou¦BAD}]",
            "counter]new",
            r"\newtheorem{within}{Title}[{\detokenize{counter]new}}]",
        ),
        (
            r"\newtheorem{within}{Title}[{{cou¦BAD}}]",
            "counter]new",
            r"\newtheorem{within}{Title}[{\detokenize{counter]new}}]",
        ),
        (
            r"\newtheorem{within}{Title}[{{cou¦BAD}}",
            "counter]new",
            r"\newtheorem{within}{Title}[{\detokenize{counter]new}}",
        ),
        (
            r"\newtheorem{within}{Title}[{\detokenize{cou¦BAD}}]",
            "counter]new",
            r"\newtheorem{within}{Title}[{\detokenize{counter]new}}]",
        ),
        (
            r"\newtheorem{shared}[counter[li¦BAD]{Title}",
            "counter[literal",
            r"\newtheorem{shared}[counter[literal]{Title}",
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        );
        let item = items
            .iter()
            .find(|item| item.label == name)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("{marked}")
        };
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        assert_eq!(apply(&source, edits), expected, "{marked}");
    }
}

#[test]
fn protected_counter_errors_select_the_name_inside_the_required_brace_layers() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for call in [
        r"\newcounter{child}[{counter]missing}]",
        r"\newtheorem{shared}[{counter]missing}]{Title}",
        r"\newtheorem{within}{Title}[{{counter]missing}}]",
        r"\newtheorem{within}{Title}[\detokenize{counter]missing}]",
        r"\newtheorem{within}{Title}[{\detokenize{counter]missing}}]",
    ] {
        let source = format!("😀 {call}\n");
        std::fs::write(&main, &source).unwrap();
        let log = format!(
            "{}:1: LaTeX Error: No counter 'counter]missing' defined.\nl.1 {}\n",
            main.display(),
            source.trim_end()
        );
        let diagnostics = latex_lsp::build_log::parse(&main, &log);
        let items = &diagnostics[&uri];
        assert_eq!(items.len(), 1, "{call}: {items:?}");
        let start = source.find("counter]missing").unwrap();
        assert_eq!(
            items[0].range,
            Text::new(source.clone()).range(start, start + "counter]missing".len()),
            "{call}"
        );
    }
}

#[test]
fn renaming_a_reset_counter_normalizes_excess_groups_without_touching_the_title() {
    for argument in [r"{{counter]old}}", r"{{\detokenize{counter]old}}}"] {
        let mut workspace = Workspace::default();
        let declaration = r"\newcounter{counter]old}";
        let definitions = insert(&mut workspace, "definitions.tex", declaration);
        let source = format!(r"\newtheorem{{within}}{{Title counter]old}}[{argument}]");
        let main = insert(&mut workspace, "main.tex", &source);
        let point = Text::new(declaration.into()).position("\\newcounter{cou".len());
        for (name, expected) in [
            ("final", r"\newtheorem{within}{Title counter]old}[{final}]"),
            (
                "counter]new",
                r"\newtheorem{within}{Title counter]old}[{\detokenize{counter]new}}]",
            ),
        ] {
            let edit = features::rename(&workspace, &definitions, point, name)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(apply(&source, edit[&main].clone()), expected);
        }
    }
}

#[test]
fn a_theorem_reset_counter_with_insufficient_braces_does_not_refer_to_the_whole_name() {
    let mut workspace = Workspace::default();
    let declaration = r"\newcounter{counter]new}";
    let definitions = insert(&mut workspace, "definitions.tex", declaration);
    let source = "\\newtheorem{unsafe}{Title}[{counter]new}]\n\\stepcounter{counter]new}\n";
    let main = insert(&mut workspace, "main.tex", source);
    let at = Text::new(declaration.into()).position("\\newcounter{cou".len());
    assert_eq!(
        features::references(&workspace, &definitions, at, false).len(),
        1
    );
    let invalid = Text::new(source.into()).position(source.find("counter]new").unwrap() + 3);
    assert!(features::definitions(&workspace, &main, invalid).is_empty());
    let changes = features::rename(&workspace, &definitions, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        apply(source, changes[&main].clone()),
        source.replace(r"\stepcounter{counter]new}", r"\stepcounter{renamed}")
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn repeated_bracket_counter_renames_preserve_real_theorem_numbering() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;

    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let mut source = concat!(
        "\\documentclass{article}\n",
        "\\newcounter{bucket[literal}\n",
        "\\newcounter{child}[bucket[literal]\n",
        "\\newtheorem{shared}[bucket[literal]{Shared}\n",
        "\\newtheorem{within}{Within}[bucket[literal]\n",
        "\\begin{document}\n",
        "\\setcounter{bucket[literal}{3}\n",
        "\\begin{shared}Shared body.\\end{shared}\n",
        "\\begin{within}Within body.\\end{within}\n",
        "\\stepcounter{child}\n",
        "\\typeout{COUNTER-NUMBERING=\\arabic{bucket[literal}/",
        "\\theshared/\\thewithin/\\arabic{child}}\n",
        "\\end{document}\n",
    )
    .to_string();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let mut workspace = Workspace::default();
    let mut current = "bucket[literal";
    for (version, name) in ["bucket[literal", "bucket]new", "final", "again]last"]
        .into_iter()
        .enumerate()
    {
        if name != current {
            let at = workspace.documents[&uri]
                .text
                .position(source.find(current).unwrap() + 3);
            let mut changes = features::rename(&workspace, &uri, at, name)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            source = apply(&source, changes.remove(&uri).unwrap());
        }
        std::fs::write(&main, &source).unwrap();
        workspace.documents.insert(
            uri.clone(),
            Document::new(&uri, source.clone(), Some(version as i32)),
        );
        let result = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(result.status, "success", "{name}: {}", result.output);
        assert!(
            result.output.contains("COUNTER-NUMBERING=4/4/4.1/1"),
            "{name}: {}",
            result.output
        );
        current = name;
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn completing_a_bracket_reset_counter_preserves_real_numbering_through_existing_groups() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;

    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    for layers in 0..=2 {
        let argument = format!("{}cou{}", "{".repeat(layers), "}".repeat(layers));
        let source = format!(
            "\\documentclass{{article}}\n\\newcounter{{counter]new}}\n\\newtheorem{{within}}{{Within}}[{argument}]\n\\begin{{document}}\n\\setcounter{{counter]new}}{{3}}\n\\begin{{within}}Body.\\end{{within}}\n\\typeout{{COUNTER-NUMBERING=\\thewithin}}\n\\end{{document}}\n"
        );
        let mut workspace = Workspace::default();
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(1)));
        let cursor = source.find('[').unwrap() + 1 + layers + 3;
        let items =
            features::completion(&workspace, &uri, Text::new(source.clone()).position(cursor));
        let item = items
            .iter()
            .find(|item| item.label == "counter]new")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing counter completion edit")
        };
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        std::fs::write(&main, apply(&source, edits)).unwrap();
        let result = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(result.status, "success", "{layers}: {}", result.output);
        assert!(
            result.output.contains("COUNTER-NUMBERING=3.1"),
            "{layers}: {}",
            result.output
        );
    }
}
