#[path = "support/biber.rs"]
mod biber;

use latex_lsp::{
    editing, features, semantic, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///biblatex-sets/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    uri
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
fn span(source: &str, marker: &str) -> Range {
    let start = source.find(marker).unwrap();
    Text::new(source.into()).range(start, start + marker.len())
}

#[test]
fn dynamic_set_declarations_and_member_references_rename_independently() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{alpha,title={Alpha}} @book{beta,title={Beta}}",
    );
    let source="% 😀\r\n\\defbibentryset% boundary\r\n{group😀}{alpha,% alpha and group😀 stay\r\n beta}\\cite{group😀}\\cite{alpha}\\label{group😀}\r\n\\verb|\\defbibentryset{fake}{alpha}|\r\n";
    let main = insert(&mut workspace, "main.tex", source);
    let doc = &workspace.documents[&main];
    let set_at = doc
        .text
        .position(source.find("\\cite{group😀").unwrap() + "\\cite{gr".len());
    assert_eq!(
        features::definitions(&workspace, &main, set_at),
        vec![Location::new(main.clone(), span(source, "group😀"))]
    );
    assert_eq!(
        features::references(&workspace, &main, set_at, true).len(),
        2
    );
    assert_eq!(editing::highlights(&workspace, &main, set_at).len(), 2);
    assert!(features::hover(&workspace, &main, set_at).is_some());
    let edit = features::rename(&workspace, &main, set_at, "new\"set😀")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edit.len(), 1);
    assert_eq!(
        apply(source, &edit[&main]),
        source
            .replacen("{group😀}{alpha", "{new\"set😀}{alpha", 1)
            .replace("\\cite{group😀}", "\\cite{new\"set😀}")
    );
    let member = doc.text.position(source.find("{alpha,%").unwrap() + 2);
    assert_eq!(features::definitions(&workspace, &main, member)[0].uri, bib);
    assert_eq!(
        features::references(&workspace, &main, member, true).len(),
        3
    );
    let edit = features::rename(&workspace, &main, member, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        apply(source, &edit[&main]),
        source
            .replacen("{alpha,%", "{renamed,%", 1)
            .replace("\\cite{alpha}", "\\cite{renamed}")
    );
    assert_eq!(edit[&bib][0].new_text, "renamed");
}

#[test]
fn dynamic_sets_complete_member_lists_and_report_both_signature_parameters() {
    for (marked, parameter, complete) in [
        ("\\defbibentryset{gr¦oup}{alpha}", 0, false),
        ("\\defbibentryset{group}{beta,al¦pha}", 1, true),
        ("\\defbibentryset{group}{beta,% alpha\r\n al¦", 1, true),
        ("\\defbibentryset{group} ¦", 1, false),
        ("\\defbibentryset ¦", 0, false),
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "refs.bib",
            "@book{alpha,title={Alpha}} @book{beta,title={Beta}}",
        );
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        let help = signature::help(&workspace, &main, at).unwrap();
        assert_eq!(help.signatures[0].label, "\\defbibentryset{set key}{keys}");
        assert_eq!(help.active_parameter, Some(parameter), "{marked}");
        let candidates = features::completion(&workspace, &main, at);
        assert_eq!(
            candidates.iter().any(|item| item.label == "alpha"),
            complete,
            "{marked}"
        );
        if complete {
            let item = candidates
                .iter()
                .find(|item| item.label == "alpha")
                .unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                marked.replace("al¦pha", "alpha").replace("al¦", "alpha")
            );
        }
    }
}

#[test]
fn set_keys_are_completion_candidates_and_semantic_declarations_with_selection_ranges() {
    let mut workspace = Workspace::default();
    let source = "% 😀\r\n\\defbibentryset{% keep\r\ngroup😀% keep\r\n}{alpha,beta}\r\n\\cite{gro}";
    let main = insert(&mut workspace, "main.tex", source);
    let doc = &workspace.documents[&main];
    let candidates = features::completion(
        &workspace,
        &main,
        doc.text.position(source.rfind("gro}").unwrap() + 3),
    );
    assert!(candidates.iter().any(|item| item.label == "group😀"));
    let expected = span(source, "group😀");
    let mut position = Position::new(0, 0);
    let mut found = false;
    for token in semantic::tokens(doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        found |= token.token_type == 3
            && token.token_modifiers_bitset == 1
            && position == expected.start
            && token.length == "group😀".encode_utf16().count() as u32;
    }
    assert!(found);
    let selections = editing::selection_ranges(doc, &[expected.start]).unwrap();
    let mut selection = Some(&selections[0]);
    let mut found = false;
    while let Some(current) = selection {
        found |= current.range == expected;
        selection = current.parent.as_deref();
    }
    assert!(found);
}

#[test]
fn malformed_incomplete_and_unsupported_set_declarations_do_not_define_guessed_keys() {
    for source in [
        "\\defbibentryset{group}",
        "\\defbibentryset{group}{alpha",
        "\\defbibentryset*{group}{alpha}",
        "\\defbibentryset[group]{group}{alpha}",
        "\\defbibentryset{group}[alpha]{alpha}",
        "\\defbibentryset{bad key}{alpha}",
        "\\defbibentryset{a,b}{alpha}",
        "\\defbibentryset{\\dynamic}{alpha}",
        "\\Defbibentryset{group}{alpha}",
        "% \\defbibentryset{group}{alpha}",
        "\\verb|\\defbibentryset{group}{alpha}|",
        "\\begin{verbatim}\\defbibentryset{group}{alpha}\\end{verbatim}",
    ] {
        let document = Document::new(
            &Url::parse("file:///sets.tex").unwrap(),
            source.into(),
            Some(1),
        );
        assert!(document.syntax.citations.is_empty(), "{source}");
    }
    for source in [
        "\\defbibentryset{group}{alpha} text {beta}",
        "\\defbibentryset{group}{alpha}{beta}",
    ] {
        let document = Document::new(
            &Url::parse("file:///sets.tex").unwrap(),
            source.into(),
            Some(1),
        );
        assert_eq!(document.syntax.citations.len(), 1);
        assert_eq!(
            document
                .syntax
                .citation_references
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha"]
        );
    }
}

#[test]
fn duplicate_dynamic_sets_and_database_collisions_reject_ambiguous_renames() {
    for duplicate in ["\\defbibentryset{group}{beta}", "\\bibitem{group}"] {
        let mut workspace = Workspace::default();
        let source = format!("\\defbibentryset{{group}}{{alpha}}{duplicate}\\cite{{group}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(source.rfind("group}").unwrap() + 1);
        assert_eq!(features::definitions(&workspace, &main, at).len(), 2);
        assert!(features::prepare_rename(&workspace, &main, at).is_err());
        assert!(features::rename(&workspace, &main, at, "new").is_err());
    }
    for bibliography in [
        "@book{group,title={Collision}} @book{alpha,title={Member}}",
        "@book{actual,ids={group}} @book{alpha,title={Member}}",
    ] {
        let mut workspace = Workspace::default();
        insert(&mut workspace, "refs.bib", bibliography);
        let source = "\\defbibentryset{group}{alpha}\\cite{group}";
        let main = insert(&mut workspace, "main.tex", source);
        let at = workspace.documents[&main]
            .text
            .position(source.rfind("group}").unwrap() + 1);
        assert!(features::prepare_rename(&workspace, &main, at).is_err());
    }
}

#[test]
fn set_commands_offer_snippets_and_existing_arguments_are_preserved() {
    for following in ["", "{group}{alpha}"] {
        let mut workspace = Workspace {
            snippets: true,
            ..Default::default()
        };
        let source = format!("\\defbibentryset{following}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position("\\defbibentryset".len());
        let candidates = features::completion(&workspace, &main, at);
        let item = candidates
            .iter()
            .find(|item| item.label == "defbibentryset")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(
            edit.new_text,
            if following.is_empty() {
                "defbibentryset{${1:set key}}{${2:keys}}"
            } else {
                "defbibentryset"
            }
        );
    }
}

#[test]
fn dynamic_set_declarations_refresh_after_buffer_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\r\n\\defbibentryset{group}{alpha}\\cite{group}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    for (version, current) in [
        (8, source.to_string()),
        (9, format!("% moved 😀\r\n{source}")),
        (10, source.replace("{group}{alpha}", "{\\dynamic}{alpha}")),
    ] {
        workspace.open_with_language(main.clone(), current.clone(), version, "latex");
        let at = workspace.documents[&main]
            .text
            .position(current.rfind("group}").unwrap() + 1);
        let definitions = features::definitions(&workspace, &main, at);
        assert_eq!(definitions.len(), usize::from(version < 10));
        if version < 10 {
            assert_eq!(definitions[0].range, span(&current, "group"));
        }
    }
    workspace.open_with_language(main.clone(), source.into(), 11, "latex");
    let at = workspace.documents[&main]
        .text
        .position(source.rfind("group}").unwrap() + 1);
    assert_eq!(features::definitions(&workspace, &main, at).len(), 1);
}

#[test]
fn dynamic_set_declarations_respect_project_scope_and_existing_key_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (folder, member) in [("a", "alpha"), ("b", "beta")] {
        std::fs::create_dir(root.join(folder)).unwrap();
        std::fs::write(root.join(folder).join("main.tex"),format!("\\documentclass{{article}}\\addbibresource{{refs.bib}}\\defbibentryset{{group}}{{{member}}}\\cite{{group}}" )).unwrap();
        std::fs::write(
            root.join(folder).join("refs.bib"),
            format!("@book{{{member},title={{Member}}}}"),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
    let doc = &workspace.documents[&main];
    let at = doc
        .text
        .position(doc.text.source.rfind("group}").unwrap() + 1);
    let edit = features::rename(&workspace, &main, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edit.len(), 1);
    assert_eq!(edit[&main].len(), 2);
    assert!(features::rename(&workspace, &main, at, "alpha").is_err());
}

#[test]
fn shared_dynamic_set_references_refuse_different_meanings_across_projects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("shared.tex"), "\\cite{group}").unwrap();
    for (folder, member) in [("a", "alpha"), ("b", "beta")] {
        std::fs::create_dir(root.join(folder)).unwrap();
        std::fs::write(root.join(folder).join("main.tex"),format!("\\documentclass{{article}}\\defbibentryset{{group}}{{{member}}}\\input{{../shared}}" )).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
    let doc = &workspace.documents[&main];
    let at = doc
        .text
        .position(doc.text.source.find("group}").unwrap() + 1);
    assert!(features::rename(&workspace, &main, at, "renamed").is_err());
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Feed the indexed dynamic declaration through Biber's normal control-file
/// interface. This checks set semantics without requiring biblatex.sty.
fn native_set(root: &std::path::Path, doc: &Document, bibliography: &str, control: &str) -> String {
    let owner = &doc.syntax.citations[0].name;
    let command = doc
        .syntax
        .commands
        .iter()
        .find(|command| command.name == "defbibentryset")
        .unwrap();
    let members = &command.argument.as_ref().unwrap().span;
    let members = doc
        .syntax
        .citation_references
        .iter()
        .filter(|entry| members.start <= entry.span.start && entry.span.end <= members.end)
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let citekey = format!(
        "<citekey order=\"1\" intorder=\"1\" type=\"set\" members=\"{}\">{}</citekey>",
        xml(&members),
        xml(owner)
    );
    let control = control.replace("<citekey order=\"1\" intorder=\"1\">*</citekey>", &citekey);
    std::fs::write(root.join("refs.data"), bibliography).unwrap();
    std::fs::write(root.join("main.bcf"), control).unwrap();
    let output = std::process::Command::new("biber")
        .args(["--noconf", "main"])
        .current_dir(root)
        .output()
        .unwrap();
    let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
    assert!(output.status.success(), "{log}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!log.contains("WARN -"), "{log}");
    std::fs::read_to_string(root.join("main.bbl")).unwrap()
}

#[test]
#[ignore = "requires biber"]
fn real_biber_dynamic_sets_preserve_members_and_output_after_independent_renames() {
    let directory = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{alpha,title={First}} @book{beta,title={Second}}",
    );
    let source = "\\defbibentryset{group😀}{alpha,beta}\\cite{group😀}";
    let main = insert(&mut workspace, "main.tex", source);
    let control = biber::control_file();
    let baseline = native_set(
        directory.path(),
        &workspace.documents[&main],
        &workspace.documents[&bib].text.source,
        &control,
    );
    assert!(baseline.contains("\\entry{group😀}{set}"));
    assert!(baseline.contains("\\set{alpha,beta}"));
    let mut expected = baseline;
    // Biber's BibTeX driver rejects quotes in database keys; dynamic keys
    // arrive through XML and do support quotes. Exercise both input paths.
    for (old, new) in [("group😀", "new\"set😀"), ("alpha", "new-member😀")] {
        let doc = &workspace.documents[&main];
        let at = doc.text.position(doc.text.source.find(old).unwrap() + 1);
        let edit = features::rename(&workspace, &main, at, new)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        for (target, edits) in edit {
            let doc = &workspace.documents[&target];
            let source = apply(&doc.text.source, &edits);
            workspace
                .documents
                .insert(target.clone(), Document::new(&target, source, Some(8)));
        }
        expected = expected.replace(old, new);
        let actual = native_set(
            directory.path(),
            &workspace.documents[&main],
            &workspace.documents[&bib].text.source,
            &control,
        );
        assert_eq!(actual, expected);
    }
}
