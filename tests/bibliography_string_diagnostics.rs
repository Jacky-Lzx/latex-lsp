use latex_lsp::{compilation::build_log, editing, project::workspace::Workspace, text::Text};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "undefined-bibliography-string";

fn uri(root: &Path, file: &str) -> Url {
    Url::from_file_path(root.join(file)).unwrap()
}

fn warning(name: &str, line: u32, file: &str) -> String {
    format!("Warning--string name \"{name}\" is undefined\n--line {line} of file {file}\n")
}

fn project(root: &Path, source: &str, definitions: &str, log: &str) -> (Workspace, Url) {
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\bibliography{defs,refs,later}",
    )
    .unwrap();
    std::fs::write(root.join("defs.bib"), definitions).unwrap();
    std::fs::write(root.join("refs.bib"), source).unwrap();
    std::fs::write(root.join("later.bib"), "@string{Journals={Later}}").unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            versioned_edits: true,
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    workspace.project.build_diagnostics.insert(
        uri(root, "main.tex"),
        build_log::parse_bibliography(&root.join("main.tex"), log),
    );
    (workspace, uri(root, "refs.bib"))
}

fn history(tail: &str) -> String {
    format!(
        "The style file: plain.bst\nDatabase file #1: defs.bib\nDatabase file #2: refs.bib\n{tail}"
    )
}

fn diagnostics(workspace: &Workspace, target: &Url) -> Vec<Diagnostic> {
    workspace
        .project
        .build_diagnostics
        .values()
        .flat_map(|map| map.get(target).into_iter().flatten())
        .filter(|diagnostic| diagnostic.code == Some(NumberOrString::String(CODE.into())))
        .cloned()
        .collect()
}

fn actions(workspace: &Workspace, target: &Url) -> Vec<CodeAction> {
    editing::code_actions(
        workspace,
        target,
        Range::new(Position::default(), Position::new(u32::MAX, 0)),
        &CodeActionContext {
            diagnostics: diagnostics(workspace, target),
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
    .into_iter()
    .filter_map(|action| match action {
        CodeActionOrCommand::CodeAction(action) if action.title.contains("abbreviation") => {
            Some(action)
        }
        _ => None,
    })
    .collect()
}

fn replacement(action: &CodeAction, target: &Url) -> String {
    let Some(DocumentChanges::Edits(edits)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!("expected versioned edit")
    };
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].text_document.uri, *target);
    assert_eq!(edits[0].text_document.version, Some(7));
    assert_eq!(edits[0].edits.len(), 1);
    let OneOf::Left(edit) = &edits[0].edits[0] else {
        panic!()
    };
    edit.new_text.clone()
}

#[test]
fn two_line_warnings_use_exact_utf16_ranges_and_preserve_other_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let source = "@misc{entry,title={😀} # JoUrAnL}\r\n@preamble{\"text\" # jouranl}\r\n@string{full=jouranl # { text}}\r\n";
    let log = history(&format!(
        "{}{}{}Warning--empty author in entry\n",
        warning("jouranl", 1, "refs.bib"),
        warning("jouranl", 2, "./refs.bib"),
        warning("jouranl", 3, "refs.bib")
    ));
    let (workspace, target) = project(directory.path(), source, "@string{Journal={Text}}", &log);
    let found = diagnostics(&workspace, &target);
    assert_eq!(found.len(), 3);
    let text = Text::new(source.into());
    for (diagnostic, at) in found.iter().zip([
        source.find("JoUrAnL").unwrap(),
        source.find("jouranl").unwrap(),
        source.rfind("jouranl").unwrap(),
    ]) {
        assert_eq!(diagnostic.range, text.range(at, at + 7));
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(diagnostic.source.as_deref(), Some("latex-build"));
        assert_eq!(
            diagnostic.message,
            "BibTeX: string name \"jouranl\" is undefined"
        );
        assert!(diagnostic.data.as_ref().unwrap()["bibliographyInputs"].is_array());
    }
    assert!(workspace.project.build_diagnostics.values().any(|map| map
        .values()
        .flatten()
        .any(|diagnostic| diagnostic.message == "BibTeX: empty author in entry")));
    let available = actions(&workspace, &target);
    assert_eq!(available.len(), 3);
    assert!(available
        .iter()
        .all(|action| replacement(action, &target) == "Journal"));
}

#[test]
fn candidates_follow_database_and_declaration_order_and_ignore_independent_projects() {
    let directory = tempfile::tempdir().unwrap();
    let source =
        "@string{Journal={Current}}\n@misc{entry,title=Journla}\n@string{Journals={Too late}}";
    let log = history(&format!(
        "{}Database file #3: later.bib\n",
        warning("journla", 2, "refs.bib")
    ));
    let (mut workspace, target) =
        project(directory.path(), source, "@string{JOURNAL={Earlier}}", &log);
    let other = uri(directory.path(), "other.bib");
    std::fs::write(other.to_file_path().unwrap(), "@string{Journlaa={Other}}").unwrap();
    workspace.open(other, "@string{Journlaa={Other}}".into(), 1);
    let available = actions(&workspace, &target);
    assert_eq!(available.len(), 1, "{available:?}");
    assert_eq!(replacement(&available[0], &target), "Journal");
}

#[test]
fn only_complete_valid_assignments_are_candidates() {
    for definition in [
        "@string{Journal=}",
        "@string{Journal={Text} # }",
        "@string{Journal=bad tokens}",
        "@string{Journal={Text},other={No}}",
        "@string{Journal={Unclosed}",
        "@comment{@string{Journal={No}}}",
        "% @string{Journal={No}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let log = history(&warning("jouranl", 1, "refs.bib"));
        let (workspace, target) = project(
            directory.path(),
            "@misc{entry,title=jouranl}",
            definition,
            &log,
        );
        assert!(actions(&workspace, &target).is_empty(), "{definition}");
    }
    for definition in [
        "@string{Journal={Text},}",
        "@string(Journal=2026 # {Text})",
        "@string{Journal=missing}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let log = history(&warning("jouranl", 1, "refs.bib"));
        let (workspace, target) = project(
            directory.path(),
            "@misc{entry,title=jouranl}",
            definition,
            &log,
        );
        assert_eq!(actions(&workspace, &target).len(), 1, "{definition}");
    }
}

#[test]
fn ambiguous_literal_comment_or_invalid_line_locations_never_offer_edits() {
    for (source, line, flag) in [
        (
            "@misc{entry,title=jouranl # JOURANL}",
            1,
            "unresolvedReference",
        ),
        ("@misc{entry,title={jouranl}}", 1, "unresolvedReference"),
        (
            "% jouranl\n@misc{entry,title={Text}}",
            1,
            "unresolvedReference",
        ),
        ("@misc{jouranl,title={Text}}", 1, "unresolvedReference"),
        ("@string{jouranl={Text}}", 1, "unresolvedReference"),
        ("@misc{entry,title=jouranl}", 0, "unresolvedLine"),
        ("@misc{entry,title=jouranl}", 900, "unresolvedLine"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let log = history(&warning("jouranl", line, "refs.bib"));
        let (workspace, target) =
            project(directory.path(), source, "@string{Journal={Text}}", &log);
        let found = diagnostics(&workspace, &target);
        assert_eq!(found.len(), 1, "{source}");
        assert_eq!(found[0].data.as_ref().unwrap()[flag], true, "{source}");
        assert_eq!(found[0].range.start.line, 0);
        assert!(actions(&workspace, &target).is_empty(), "{source}");
    }
}

#[test]
fn missing_or_unconfirmed_sources_and_locations_stay_on_the_root() {
    for tail in [
        warning("jouranl", 1, "missing.bib"),
        warning("jouranl", 1, "folder.bib"),
        warning("jouranl", 1, "main.aux"),
        "Warning--string name \"jouranl\" is undefined\n".into(),
        "Warning--string name \"jouranl\" is undefined\n--line 1x of file refs.bib\n".into(),
        "Warning--string name \"jouranl\" is undefined\n\n--line 1 of file refs.bib\n".into(),
    ] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("folder.bib")).unwrap();
        std::fs::write(
            directory.path().join("main.aux"),
            "@misc{entry,title=jouranl}",
        )
        .unwrap();
        let (workspace, target) = project(
            directory.path(),
            "@misc{entry,title=jouranl}",
            "@string{Journal={Text}}",
            &history(&tail),
        );
        let found = diagnostics(&workspace, &uri(directory.path(), "main.tex"));
        assert_eq!(found.len(), 1, "{tail}");
        assert_eq!(found[0].data.as_ref().unwrap()["unresolvedSource"], true);
        assert!(diagnostics(&workspace, &target).is_empty());
        assert!(actions(&workspace, &target).is_empty());
    }
}

#[test]
fn incomplete_database_history_keeps_locations_but_disables_repairs() {
    for log in [
        warning("jouranl", 1, "refs.bib"),
        format!(
            "Database file #2: refs.bib\n{}",
            warning("jouranl", 1, "refs.bib")
        ),
        format!(
            "Database file #+1: refs.bib\n{}",
            warning("jouranl", 1, "refs.bib")
        ),
        history(&format!(
            "I was expecting an `='---line 1 of file defs.bib\n{}",
            warning("jouranl", 1, "refs.bib")
        )),
        format!(
            "Database file #1: absent.bib\nDatabase file #2: refs.bib\n{}",
            warning("jouranl", 1, "refs.bib")
        ),
        format!(
            "Database file #1: refs.bib\nDatabase file #2: ./refs.bib\n{}",
            warning("jouranl", 1, "refs.bib")
        ),
        format!(
            "{}\n{}\n",
            history(&warning("jouranl", 1, "refs.bib")),
            "[latex-lsp: middle of compiler output omitted]"
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (workspace, target) = project(
            directory.path(),
            "@misc{entry,title=jouranl}",
            "@string{Journal={Text}}",
            &log,
        );
        let found = diagnostics(&workspace, &target);
        assert_eq!(found.len(), 1, "{log}");
        assert_eq!(found[0].range.start.character, 18);
        assert!(actions(&workspace, &target).is_empty(), "{log}");
    }
}

#[test]
fn already_available_names_and_implicit_months_do_not_receive_repairs() {
    for (name, definitions) in [
        (
            "jouranl",
            "@string{Journal={Text}}\n@string{JOURANL={Present}}",
        ),
        ("jann", ""),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("@misc{{entry,title={name}}}");
        let (workspace, target) = project(
            directory.path(),
            &source,
            definitions,
            &history(&warning(name, 1, "refs.bib")),
        );
        assert!(actions(&workspace, &target).is_empty());
    }
}

#[test]
fn suggestions_are_ranked_bounded_and_deduplicated_case_insensitively() {
    let directory = tempfile::tempdir().unwrap();
    let definitions = "@string{aaaa={1}}\n@string{AAAB={2}}\n@string{aaab={3}}\n@string{aaac={4}}\n@string{aaad={5}}\n@string{aaae={6}}\n@string{aaaf={7}}";
    let (workspace, target) = project(
        directory.path(),
        "@misc{entry,title=aaaz}",
        definitions,
        &history(&warning("aaaz", 1, "refs.bib")),
    );
    assert_eq!(
        actions(&workspace, &target)
            .iter()
            .map(|action| replacement(action, &target))
            .collect::<Vec<_>>(),
        ["aaaa", "aaab", "aaac", "aaad", "aaae"]
    );
}

#[test]
fn disk_and_buffer_changes_in_any_confirmed_input_invalidate_old_repairs() {
    for file in ["defs.bib", "refs.bib", "local.bst"] {
        for buffer in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            std::fs::write(
                directory.path().join("local.bst"),
                "ENTRY {title} {} {}\nREAD",
            )
            .unwrap();
            let log = history(&warning("jouranl", 1, "refs.bib")).replace("plain.bst", "local.bst");
            let (mut workspace, target) = project(
                directory.path(),
                "@misc{entry,title=jouranl}",
                "@string{Journal={Text}}",
                &log,
            );
            assert_eq!(actions(&workspace, &target).len(), 1);
            let input = uri(directory.path(), file);
            let source = format!(
                "{}\n% changed",
                std::fs::read_to_string(input.to_file_path().unwrap()).unwrap()
            );
            if buffer {
                workspace.open(input, source, 8);
            } else {
                std::fs::write(input.to_file_path().unwrap(), source).unwrap();
            }
            assert!(
                actions(&workspace, &target).is_empty(),
                "{file}, buffer={buffer}"
            );
        }
    }
}

#[test]
fn legacy_repairs_revalidate_versions_replacements_and_source_proofs() {
    let directory = tempfile::tempdir().unwrap();
    let source = "@misc{entry,title=jouranl}";
    let (mut workspace, target) = project(
        directory.path(),
        source,
        "@string{Journal={Text}}",
        &history(&warning("jouranl", 1, "refs.bib")),
    );
    workspace.client.code_action_literals = false;
    workspace.client.apply_edits = true;
    let diagnostic = diagnostics(&workspace, &target).remove(0);
    let request = |version, name: &str| editing::FixRequest {
        uri: target.clone(),
        version,
        diagnostic: diagnostic.clone(),
        replacement: Some(name.into()),
    };
    assert!(editing::resolve_fix(&workspace, request(Some(7), "Journal")).is_some());
    assert!(editing::resolve_fix(&workspace, request(Some(6), "Journal")).is_none());
    assert!(editing::resolve_fix(&workspace, request(Some(7), "Injected")).is_none());
    std::fs::write(directory.path().join("defs.bib"), "@string{Other={Text}}").unwrap();
    assert!(editing::resolve_fix(&workspace, request(Some(7), "Journal")).is_none());
}

#[test]
fn database_names_with_spaces_quotes_and_log_field_words_keep_their_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let file = "文献 ' line 700 of file refs.bib";
    let source = "@misc{entry,title=jouranl}";
    std::fs::write(root.join(file), source).unwrap();
    let log = format!("Database file #1: {file}\n{}", warning("jouranl", 1, file));
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    assert_eq!(
        found[&uri(root, file)][0].range,
        Text::new(source.into()).range(18, 25)
    );
}

#[test]
fn error_looking_style_paths_do_not_invalidate_confirmed_abbreviation_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let name = "style---line 2 of file decoy.bst";
    std::fs::write(root.join(name), "ENTRY {title} {} {}\nREAD").unwrap();
    let log = history(&warning("jouranl", 1, "refs.bib")).replace("plain.bst", name);
    let (workspace, target) = project(
        root,
        "@misc{entry,title=jouranl}",
        "@string{Journal={Text}}",
        &log,
    );
    let all: Vec<_> = workspace
        .project
        .build_diagnostics
        .values()
        .flat_map(|map| map.values().flatten())
        .collect();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(actions(&workspace, &target).len(), 1);
}

#[test]
fn auxiliary_working_directory_selects_the_database_before_source_and_output_fallbacks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("build")).unwrap();
    let source = "@misc{entry,title=jouranl}";
    std::fs::write(root.join("refs.bib"), format!("\n\n{source}")).unwrap();
    std::fs::write(root.join("build/refs.bib"), source).unwrap();
    for (auxiliary, selected, line) in [
        ("main.aux", "build/refs.bib", 1),
        ("build/main.aux", "refs.bib", 3),
    ] {
        let log = format!(
            "The top-level auxiliary file: {auxiliary}\nDatabase file #1: refs.bib\n{}",
            warning("jouranl", line, "refs.bib")
        );
        let found = build_log::parse_bibliography_log(
            &root.join("main.tex"),
            &root.join("build/main.blg"),
            &log,
        );
        assert_eq!(found.len(), 1);
        let diagnostic = &found[&uri(root, selected)][0];
        assert_eq!(diagnostic.range.start, Position::new(line - 1, 18));
        assert!(diagnostic.data.as_ref().unwrap()["bibliographyInputs"].is_array());
    }
}

#[test]
fn oversized_sources_keep_warnings_without_unbounded_reads_or_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = format!(
        "@misc{{entry,title=jouranl}}\n% {}",
        "x".repeat(2 * 1024 * 1024)
    );
    std::fs::write(root.join("refs.bib"), source).unwrap();
    let found = build_log::parse_bibliography(
        &root.join("main.tex"),
        &format!(
            "Database file #1: refs.bib\n{}",
            warning("jouranl", 1, "refs.bib")
        ),
    );
    let diagnostic = &found[&uri(root, "main.tex")][0];
    assert_eq!(diagnostic.code, Some(NumberOrString::String(CODE.into())));
    assert_eq!(diagnostic.data.as_ref().unwrap()["unresolvedSource"], true);
}

#[test]
#[cfg(unix)]
fn database_aliases_keep_roles_without_borrowing_auxiliary_alias_contents() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("physical.data"), "@misc{entry,title=jouranl}").unwrap();
    symlink("physical.data", root.join("alias.bib")).unwrap();
    symlink("physical.data", root.join("main.aux")).unwrap();
    let log = format!(
        "Database file #1: alias.bib\n{}",
        warning("jouranl", 1, "alias.bib")
    );
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    assert_eq!(
        found[&uri(root, "physical.data")][0].range.start.character,
        18
    );
    let found = build_log::parse_bibliography(
        &root.join("main.tex"),
        &format!(
            "Database file #1: alias.bib\n{}",
            warning("jouranl", 1, "main.aux")
        ),
    );
    assert_eq!(
        found[&uri(root, "main.tex")][0].data.as_ref().unwrap()["unresolvedSource"],
        true
    );
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_warning_corrections_restore_the_expected_bibliography() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        "@misc{entry,title={Prefix } #\n JOURANL # { suffix}}\n@string{Journals={Too late}}";
    let (mut workspace, target) = project(root, source, "@string{Journal={Expected}}", "");
    std::fs::write(
        root.join("fixture.bst"),
        "ENTRY {title} {} {}\nFUNCTION {emit} {title write$ newline$}\nREAD\nITERATE {emit}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{entry}\n\\bibstyle{fixture}\n\\bibdata{defs,refs,later}\n",
    )
    .unwrap();
    let compile = || {
        let result = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        std::fs::read_to_string(root.join("main.blg")).unwrap()
    };
    let log = compile();
    assert!(log.contains("--line 2 of file refs.bib"), "{log}");
    workspace.project.build_diagnostics = BTreeMap::from([(
        uri(root, "main.tex"),
        build_log::parse_bibliography(&root.join("main.tex"), &log),
    )]);
    let found = diagnostics(&workspace, &target);
    assert_eq!(found.len(), 1);
    let available = actions(&workspace, &target);
    assert_eq!(available.len(), 1, "{available:?}");
    assert_eq!(replacement(&available[0], &target), "Journal");
    std::fs::write(root.join("refs.bib"), source.replace("JOURANL", "Journal")).unwrap();
    let log = compile();
    assert!(!log.contains("is undefined"), "{log}");
    assert_eq!(
        std::fs::read_to_string(root.join("main.bbl"))
            .unwrap()
            .trim(),
        "Prefix Expected suffix"
    );
}
