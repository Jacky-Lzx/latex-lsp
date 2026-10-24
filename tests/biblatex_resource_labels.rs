use latex_lsp::{features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(marked: &str) -> (tempfile::TempDir, Workspace, Url, String, Position) {
    let dir = tempfile::tempdir().unwrap();
    let marked = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource[label=default]{{default.bib}}\addglobalbib[label=everywhere]{{global.bib}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}{marked}\end{{document}}"
    );
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    for (name, contents) in [
        ("main.tex", source.as_str()),
        ("default.bib", "@book{paper,title={Default}}"),
        ("global.bib", "@book{always,title={Global}}"),
        ("section.bib", "@book{chosen,title={Selected}}"),
        ("spare.bib", "@book{spare,title={Spare}}"),
        (
            "chosen.bib",
            "@book{chosen,title={Wrong filename fallback}}",
        ),
    ] {
        std::fs::write(dir.path().join(name), contents).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source.clone(), 5);
    let position = ws.documents[&main].text.position(offset);
    (dir, ws, main, source, position)
}
fn apply(source: &str, item: &CompletionItem) -> String {
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("{item:?}")
    };
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
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
fn label_target(ws: &Workspace, uri: &Url, at: Position, label: &str, filename: &str) {
    let source = &ws.documents[uri].text.source;
    let start = source.find(&format!("label={label}")).unwrap() + 6;
    let declaration = Location::new(
        uri.clone(),
        ws.documents[uri].text.range(start, start + label.len()),
    );
    assert_eq!(
        features::definitions(ws, uri, at),
        std::slice::from_ref(&declaration)
    );
    assert_eq!(
        features::definitions(ws, uri, declaration.range.start),
        [declaration]
    );
    let hover = features::hover(ws, uri, at).unwrap();
    let HoverContents::Markup(contents) = hover.contents else {
        panic!("{hover:?}")
    };
    assert!(
        contents.value.contains(label) && contents.value.contains(filename),
        "{contents:?}"
    );
}
#[test]
fn registered_resource_labels_navigate_and_hover_independently_of_resource_roles() {
    for (label, file) in [
        ("default", "default.bib"),
        ("everywhere", "global.bib"),
        ("chosen", "section.bib"),
    ] {
        let body = format!(r"\newrefsection[{label}¦]");
        let (_dir, ws, main, _, at) = setup(&body);
        label_target(&ws, &main, at, label, file);
    }
}
#[test]
fn label_completion_replaces_the_current_whole_item_and_preserves_other_choices() {
    let (_dir, ws, main, source, at) = setup(r"\newrefsection[everywhere, ch¦BAD, spare.bib]");
    let item = features::completion(&ws, &main, at)
        .into_iter()
        .find(|item| item.label == "chosen")
        .unwrap();
    assert!(item.detail.as_ref().unwrap().contains("section.bib"));
    assert_eq!(apply(&source, &item), source.replace("chBAD", "chosen"));
}
#[test]
fn empty_selection_completion_combines_registered_labels_and_literal_filenames() {
    let (_dir, ws, main, _, at) = setup(r"\newrefsection[¦]");
    let items = features::completion(&ws, &main, at);
    for name in ["default", "everywhere", "chosen", "spare.bib"] {
        assert!(
            items.iter().any(|item| item.label == name),
            "{name}: {items:?}"
        );
    }
}
#[test]
fn named_environment_and_forwarded_section_headers_keep_their_label_context() {
    for call in [
        r"\csuse{newrefsection}",
        r"\csname newrefsection\endcsname",
        r"\begin{refsection}",
        r"\csuse{begin}{refsection}",
        r"\choose",
    ] {
        let body = format!(r"\def\choose{{\newrefsection}}{call}[ch¦osen]");
        let (_dir, ws, main, _, at) = setup(&body);
        label_target(&ws, &main, at, "chosen", "section.bib");
    }
}
#[test]
fn braced_resource_labels_preserve_precise_unicode_and_crlf_source_ranges() {
    let (_dir, mut ws, main, source, _) = setup("😀\r\n\\newrefsection[chosen¦]");
    let source = source.replace("label=chosen", "label = {chosen}");
    ws.open(main.clone(), source.clone(), 6);
    let start = source.find("{chosen}").unwrap() + 1;
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    let expected = Location::new(main.clone(), Text::new(source).range(start, start + 6));
    assert_eq!(features::definitions(&ws, &main, at), [expected]);
}
#[test]
fn resource_labels_do_not_take_over_latex_labels_or_citation_keys() {
    let (_dir, ws, main, source, at) =
        setup(r"\newrefsection[ch¦osen]\label{chosen}\ref{chosen}\cite{chosen}");
    label_target(&ws, &main, at, "chosen", "section.bib");
    let text = Text::new(source.clone());
    let cite = text.position(source.find(r"\cite{chosen}").unwrap() + 7);
    assert_eq!(
        features::definitions(&ws, &main, cite)[0].uri,
        main.join("section.bib").unwrap()
    );
    let label = text.position(source.find(r"\ref{chosen}").unwrap() + 6);
    let target = features::definitions(&ws, &main, label);
    let start = source.find(r"\label{chosen}").unwrap() + 7;
    assert_eq!(
        target,
        [Location::new(main.clone(), text.range(start, start + 6))]
    );
}
#[test]
fn literal_data_and_redefined_section_readers_do_not_expose_resource_label_symbols() {
    for body in [
        r"\detokenize{\newrefsection[ch¦osen]}",
        r"\newcommand\discard[1]{}\discard{\newrefsection[ch¦osen]}",
        r"\renewcommand\newrefsection[1][]{}\newrefsection[ch¦osen]",
    ] {
        let (_dir, ws, main, _, at) = setup(body);
        assert!(features::definitions(&ws, &main, at).is_empty(), "{body}");
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "chosen"),
            "{body}"
        );
    }
}
#[test]
fn unchanged_consumers_refresh_when_an_unsaved_label_provider_changes() {
    let (dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
    let provider = main.join("resources.tex").unwrap();
    let declaration = r"\addsectionbib[label=chosen]{section.bib}";
    std::fs::write(dir.path().join("resources.tex"), declaration).unwrap();
    let source = source.replace(declaration, r"\input{resources}");
    ws.open(main.clone(), source.clone(), 5);
    let at =
        Text::new(source).position(ws.documents[&main].text.source.rfind("chosen]").unwrap() + 2);
    let target = features::definitions(&ws, &main, at);
    assert_eq!(target.len(), 1);
    assert_eq!(target[0].uri, provider);
    ws.open(
        provider.clone(),
        declaration.replace("label=chosen", "label=other"),
        3,
    );
    assert_eq!(ws.documents[&main].version, Some(5));
    assert!(features::definitions(&ws, &main, at)
        .iter()
        .all(|location| location.uri != main && location.uri != provider));
    ws.open(provider, declaration.into(), 4);
    assert_eq!(features::definitions(&ws, &main, at).len(), 1);
}

#[test]
fn unsaved_shared_consumers_with_different_resource_label_bindings_are_not_guessed() {
    let (dir, mut ws, main, source, _) = setup(r"\input{shared}¦");
    let shared = main.join("shared.tex").unwrap();
    let child = r"\newrefsection[chosen]";
    std::fs::write(dir.path().join("shared.tex"), child).unwrap();
    ws.open(main.clone(), source.clone(), 5);
    ws.open(shared.clone(), child.into(), 3);
    let at = Position::new(0, 17);
    assert_eq!(features::definitions(&ws, &shared, at).len(), 1);
    let other = main.join("new-root.tex").unwrap();
    ws.open(
        other.clone(),
        source.replace("{section.bib}", "{spare.bib}"),
        8,
    );
    assert_eq!(ws.documents[&shared].version, Some(3));
    assert!(features::definitions(&ws, &shared, at).is_empty());
    assert!(!features::completion(&ws, &shared, at)
        .iter()
        .any(|item| item.label == "chosen"));
    ws.documents.remove(&other);
    ws.open(main, source, 5);
    assert_eq!(features::definitions(&ws, &shared, at).len(), 1);
}

#[test]
#[ignore = "requires pdflatex, biber and biblatex"]
fn real_resource_label_completion_selects_its_registered_bibliography() {
    let (dir, ws, main, source, at) =
        setup(r"\newrefsection[ch¦BAD]\cite{chosen,always}\printbibliography");
    let item = features::completion(&ws, &main, at)
        .into_iter()
        .find(|item| item.label == "chosen")
        .unwrap();
    let completed = apply(&source, &item);
    assert_eq!(completed, source.replace("chBAD", "chosen"));
    std::fs::write(dir.path().join("main.tex"), completed).unwrap();
    let latex = || {
        let output = std::process::Command::new("pdflatex")
            .args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    };
    latex();
    let bcf = std::fs::read_to_string(dir.path().join("main.bcf")).unwrap();
    let selected = bcf
        .split_once("<bcf:bibdata section=\"1\">")
        .unwrap()
        .1
        .split_once("</bcf:bibdata>")
        .unwrap()
        .0;
    assert!(
        selected.contains("section.bib")
            && selected.contains("global.bib")
            && !selected.contains("default.bib"),
        "{selected}"
    );
    let output = std::process::Command::new("biber")
        .arg("main")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    latex();
    latex();
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
    let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
    assert!(
        bbl.contains("Selected") && bbl.contains("Global") && !bbl.contains("Default"),
        "{bbl}"
    );
}

#[test]
fn only_the_final_label_option_binds_the_resource_and_preserves_its_inner_span() {
    let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
    let source = source.replace("label=chosen", "label=ignored,label={chosen}");
    ws.open(main.clone(), source.clone(), 6);
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    let start = source.find("{chosen}").unwrap() + 1;
    assert_eq!(
        features::definitions(&ws, &main, at),
        [Location::new(
            main.clone(),
            Text::new(source.clone()).range(start, start + 6)
        )]
    );
    let ignored = Text::new(source.clone()).position(source.find("label=ignored").unwrap() + 7);
    assert!(features::definitions(&ws, &main, ignored).is_empty());
    let source = source.replace("[chosen]", "[]");
    ws.open(main.clone(), source.clone(), 7);
    let at = Text::new(source.clone()).position(source.rfind('[').unwrap() + 1);
    let items = features::completion(&ws, &main, at);
    assert!(items.iter().any(|item| item.label == "chosen"), "{items:?}");
    assert!(
        !items.iter().any(|item| item.label == "ignored"),
        "{items:?}"
    );
}

#[test]
fn registrations_must_be_reached_unconditionally_in_the_preamble() {
    let registration = r"\addsectionbib[label=chosen]{section.bib}";
    for prefix in [
        r"\iffalse\addsectionbib[label=chosen]{section.bib}\fi",
        r"{\addsectionbib[label=chosen]{section.bib}}",
        r"\def\later{\addsectionbib[label=chosen]{section.bib}}",
    ] {
        let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
        let source = source.replace(registration, prefix);
        ws.open(main.clone(), source.clone(), 6);
        let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
        assert!(
            features::definitions(&ws, &main, at)
                .iter()
                .all(|location| location.uri != main),
            "{prefix}"
        );
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "chosen"),
            "{prefix}"
        );
    }
    let (_dir, mut ws, main, source, _) =
        setup(r"\addsectionbib[label=late]{section.bib}\newrefsection[la¦te]");
    let source = source.replace(registration, "");
    ws.open(main.clone(), source.clone(), 6);
    let at = Text::new(source.clone()).position(source.rfind("late]").unwrap() + 2);
    assert!(features::definitions(&ws, &main, at).is_empty());
    assert!(!features::completion(&ws, &main, at)
        .iter()
        .any(|item| item.label == "late"));
}

#[test]
fn incomplete_headers_can_complete_without_resolving_their_current_name() {
    let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦BAD]");
    let source = source[..source.rfind("chBAD").unwrap() + 2].to_string();
    let at = Text::new(source.clone()).position(source.len());
    ws.open(main.clone(), source.clone(), 6);
    let items = features::completion(&ws, &main, at);
    let item = items.iter().find(|item| item.label == "chosen").unwrap();
    assert_eq!(apply(&source, item), format!("{source}osen"));
}

#[test]
fn distinct_label_declarations_do_not_choose_an_arbitrary_source() {
    let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
    let source = source.replace(
        r"\begin{document}",
        r"\addsectionbib[label=chosen]{section.bib}\begin{document}",
    );
    ws.open(main.clone(), source.clone(), 6);
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    assert!(features::definitions(&ws, &main, at).is_empty());
    assert!(!features::completion(&ws, &main, at)
        .iter()
        .any(|item| item.label == "chosen"));
}

#[test]
fn forwarded_readers_cannot_acquire_the_meaning_of_a_later_declaration() {
    let (_dir, ws, main, _, at) = setup(r"\choose[ch¦osen]\def\choose{\newrefsection}");
    assert!(features::definitions(&ws, &main, at).is_empty());
    assert!(!features::completion(&ws, &main, at)
        .iter()
        .any(|item| item.label == "chosen"));
}

#[test]
fn spaced_label_options_work_without_an_unrelated_unspaced_label() {
    let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
    let source = source.replace("label=", "label = ");
    ws.open(main.clone(), source.clone(), 6);
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    assert_eq!(features::definitions(&ws, &main, at).len(), 1);
    assert!(features::completion(&ws, &main, at)
        .iter()
        .any(|item| item.label == "chosen"));
}

#[test]
fn static_named_and_forwarded_resource_registrations_keep_their_declaration_identity() {
    for reader in [
        r"\csuse{addsectionbib}",
        r"\csname addsectionbib\endcsname",
        r"\register",
    ] {
        let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
        let source = source.replace(
            r"\addsectionbib[label=chosen]",
            &format!(r"\def\register{{\addsectionbib}}{reader}[label=chosen]"),
        );
        ws.open(main.clone(), source.clone(), 6);
        let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
        assert!(
            !features::definitions(&ws, &main, at).is_empty(),
            "{reader}"
        );
        label_target(&ws, &main, at, "chosen", "section.bib");
    }
}

#[test]
fn resource_registration_cannot_use_a_forwarder_declared_later() {
    let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
    let source = source.replace(
        r"\addsectionbib[label=chosen]{section.bib}",
        r"\register[label=chosen]{section.bib}\def\register{\addsectionbib}",
    );
    ws.open(main.clone(), source.clone(), 6);
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    assert!(features::definitions(&ws, &main, at).is_empty());
    assert!(!features::completion(&ws, &main, at)
        .iter()
        .any(|item| item.label == "chosen"));
}

#[test]
fn local_or_quoted_alias_declarations_do_not_bind_later_resource_readers() {
    for body in [
        r"{\def\choose{\newrefsection}}\choose[ch¦osen]",
        r"\begin{quote}\def\choose{\newrefsection}\end{quote}\choose[ch¦osen]",
        r"\detokenize{\def\choose{\newrefsection}}\choose[ch¦osen]",
    ] {
        let (_dir, ws, main, _, at) = setup(body);
        assert!(
            features::definitions(&ws, &main, at)
                .iter()
                .all(|location| location.uri != main),
            "{body}"
        );
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "chosen"),
            "{body}"
        );
    }
}

#[test]
fn imported_forwarders_require_an_executed_definition_before_the_selection() {
    for reader in [r"\choose", r"\csuse{choose}", r"\csname choose\endcsname"] {
        let body = format!(r"\input{{reader}}{reader}[ch¦osen]");
        let (dir, mut ws, main, source, _) = setup(&body);
        std::fs::write(
            dir.path().join("reader.tex"),
            r"\def\choose{\newrefsection}",
        )
        .unwrap();
        ws.open(main.clone(), source.clone(), 5);
        let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
        label_target(&ws, &main, at, "chosen", "section.bib");
        let items = features::completion(&ws, &main, at);
        assert!(
            items.iter().any(|item| item.label == "chosen"),
            "{reader}: {items:?}"
        );
        let source = source
            .replace(r"\input{reader}", "")
            .replace(r"\end{document}", r"\input{reader}\end{document}");
        ws.open(main.clone(), source.clone(), 6);
        let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
        assert!(features::definitions(&ws, &main, at).is_empty(), "{reader}");
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "chosen"),
            "{reader}"
        );
    }
}

#[test]
fn local_formatting_forwarders_do_not_invalidate_resource_label_provenance() {
    let (_dir, ws, main, _, at) =
        setup(r"\begin{quote}\def\strong{\textbf}\strong{Text}\end{quote}\newrefsection[ch¦osen]");
    label_target(&ws, &main, at, "chosen", "section.bib");
    assert!(features::completion(&ws, &main, at)
        .iter()
        .any(|item| item.label == "chosen"));
}
