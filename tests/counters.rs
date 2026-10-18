use latex_lsp::{features, signature, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(marked: &str) -> (tempfile::TempDir, Workspace, Url, Position) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newcounter{exercise}[section]\n\\newaliascnt{exercisealias}{exercise}\n\\newtheorem{claim}{Claim}\n\\newtheorem{shared}[claim]{Shared}\n\\newtheorem*{remark}{Remark}\n% \\newcounter{ignored}\n\\verb|\\newcounter{ignoredtoo}|\n";
    std::fs::write(root.join("definitions.tex"), definitions).unwrap();
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    std::fs::write(root.join("main.tex"), &source).unwrap();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let position = Text::new(source).position(offset);
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    (directory, workspace, uri, position)
}

#[test]
fn counter_names_complete_in_operations_reset_relationships_and_theorem_dependencies() {
    for (marked, target) in [
        (r"😀 \setcounter{ex¦ercise}{2}", "exercise"),
        (r"\addtocounter{ex¦}{2}", "exercise"),
        (r"\stepcounter{ex¦", "exercise"),
        (r"\value{  ex¦ercise  }", "exercise"),
        (r"\Roman{ex¦}", "exercise"),
        (r"\counterwithin*{exercise}{sec¦tion}", "section"),
        (r"\counterwithout[\roman]{exercise}{sec¦}", "section"),
        (r"\numberwithin{ex¦}{section}", "exercise"),
        (r"\newcounter{fresh}[ex¦]", "exercise"),
        (r"\newaliascnt{fresh}{ex¦}", "exercise"),
        (r"\aliascntresetthe{ex¦}", "exercisealias"),
        (r"\newtheorem{result}[ex¦]{Result}", "exercise"),
        (r"\newtheorem{result}{\textbf{Result}}[sec¦]", "section"),
    ] {
        let (_directory, workspace, uri, position) = setup(marked);
        let items = features::completion(&workspace, &uri, position);
        let item = items
            .iter()
            .find(|item| item.label == target)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        assert_eq!(item.kind, Some(CompletionItemKind::VARIABLE));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.new_text, target);
        let text = &workspace.documents[&uri].text;
        let changed = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap()
            .source;
        assert!(!changed.contains('¦'));
        if marked == r"\value{  ex¦ercise  }" {
            assert_eq!(changed, r"\value{  exercise  }");
        }
        if marked == r"😀 \setcounter{ex¦ercise}{2}" {
            assert_eq!(changed, r"😀 \setcounter{exercise}{2}");
        }
    }
    let (_directory, workspace, uri, position) = setup(r"\stepcounter{¦}");
    let items = features::completion(&workspace, &uri, position);
    for name in ["page", "section", "exercise", "exercisealias", "claim"] {
        assert!(items.iter().any(|item| item.label == name), "{name}");
    }
    for name in ["shared", "remark", "ignored", "ignoredtoo"] {
        assert!(!items.iter().any(|item| item.label == name), "{name}");
    }
}

#[test]
fn counter_completion_does_not_replace_declarations_values_formats_or_computed_names() {
    for marked in [
        r"\newcounter{ex¦}",
        r"\newaliascnt{ex¦}{exercise}",
        r"\setcounter{exercise}{ex¦}",
        r"\counterwithin[ar¦]{exercise}{section}",
        r"\newtheorem*{remark}[ex¦]{Remark}",
        r"\value{ex¦\suffix}",
        r"\value{ex¦\suffix",
        r"\value{ex¦% suffix",
        r"\value{\prefix ex¦}",
        "% \\value{ex¦}",
        r"\verb|\value{ex¦}|",
    ] {
        let (_directory, workspace, uri, position) = setup(marked);
        assert!(
            features::completion(&workspace, &uri, position).is_empty(),
            "{marked}"
        );
    }
}

#[test]
fn counter_completions_follow_saved_project_scopes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")), format!("\\documentclass{{article}}\n\\input{{{name}-counts}}\n\\begin{{document}}\\value{{local}}\\end{{document}}\n")).unwrap();
        std::fs::write(
            root.join(format!("{name}-counts.tex")),
            format!("\\newcounter{{local{name}}}\n\\newtheorem{{localtheorem{name}}}{{Theorem}}\n"),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    let text = &workspace.documents[&uri].text;
    let position = text.position(text.source.find("{local}").unwrap() + 6);
    let items = features::completion(&workspace, &uri, position);
    assert!(items.iter().any(|item| item.label == "locala"));
    assert!(items.iter().any(|item| item.label == "localtheorema"));
    assert!(!items
        .iter()
        .any(|item| item.label == "localb" || item.label == "localtheoremb"));
}

#[test]
fn counter_commands_offer_signatures_and_snippets_without_overwriting_existing_arguments() {
    for (marked, label, active) in [
        (r"\newcounter{fresh}[sec¦]", r"\newcounter{name}[within]", 1),
        (
            r"\newaliascnt{fresh}{ex¦}",
            r"\newaliascnt{alias}{counter}",
            1,
        ),
        (
            r"\setcounter{exercise}{2¦}",
            r"\setcounter{counter}{value}",
            1,
        ),
        (r"\value{exercise¦}", r"\value{counter}", 0),
        (
            r"\counterwithin*{exercise}{sec¦}",
            r"\counterwithin[format]{counter}{within}",
            2,
        ),
        (
            r"\numberwithin[\roman]{exercise}{sec¦}",
            r"\numberwithin[format]{counter}{within}",
            2,
        ),
    ] {
        let (_directory, workspace, uri, position) = setup(marked);
        let help = signature::help(&workspace, &uri, position).unwrap();
        assert_eq!(help.signatures[0].label, label);
        assert_eq!(help.active_parameter, Some(active));
    }
    for (marked, expected, format) in [
        (
            r"\setc¦",
            "setcounter{${1:counter}}{${2:0}}",
            InsertTextFormat::SNIPPET,
        ),
        (
            r"\setc¦ounter{exercise}{2}",
            "setcounter",
            InsertTextFormat::PLAIN_TEXT,
        ),
    ] {
        let (_directory, mut workspace, uri, position) = setup(marked);
        workspace.snippets = true;
        let items = features::completion(&workspace, &uri, position);
        let item = items
            .iter()
            .find(|item| item.label == "setcounter")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, expected);
        assert_eq!(item.insert_text_format, Some(format));
    }
}
