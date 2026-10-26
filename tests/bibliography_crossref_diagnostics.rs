use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{compilation::build_log, text::Text};

const CODE: &str = "bad-bibliography-crossref";
const NESTED_CODE: &str = "nested-bibliography-crossref";

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn error(child: &str, parent: &str) -> String {
    format!("A bad cross reference---entry \"{child}\"\nrefers to entry \"{parent}\", which doesn't exist\n")
}

fn warning(child: &str, parent: &str) -> String {
    format!("Warning--you've nested cross references--entry \"{child}\"\nrefers to entry \"{parent}\", which also refers to something\n")
}

fn parse(root: &Path, files: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\bibliography{refs}",
    )
    .unwrap();
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    build_log::parse_bibliography(&root.join("main.tex"), log)
}

fn diagnostic(found: &BTreeMap<Url, Vec<Diagnostic>>, target: &Url) -> Diagnostic {
    diagnostic_for(found, target, CODE)
}

fn diagnostic_for(found: &BTreeMap<Url, Vec<Diagnostic>>, target: &Url, code: &str) -> Diagnostic {
    found[target]
        .iter()
        .find(|item| item.code == Some(NumberOrString::String(code.into())))
        .unwrap()
        .clone()
}

#[test]
fn nested_warnings_highlight_the_child_operand_and_link_the_reported_parent_in_utf16() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@misc(child😀,title={😀},CROSSREF = \" 父😀 \" )\r\n";
    let parent = "% 😀\r\n@misc{父😀,crossref=Next}\r\n";
    let found = parse(
        root,
        &[
            ("refs.bib", source),
            ("defs.bib", parent),
            ("unread.bib", "@misc{child😀,crossref={父😀}}"),
        ],
        &format!(
            "Database file #1: defs.bib\nDatabase file #2: refs.bib\n{}",
            warning("child😀", "父😀")
        ),
    );
    assert_eq!(found.len(), 1);
    let item = diagnostic_for(&found, &uri(root, "refs.bib"), NESTED_CODE);
    assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(item.source.as_deref(), Some("latex-build"));
    let offset = source.find("父😀").unwrap();
    assert_eq!(
        item.range,
        Text::new(source.into()).range(offset, offset + "父😀".len())
    );
    assert_eq!(item.message, "BibTeX: you've nested cross references--entry \"child😀\" refers to entry \"父😀\", which also refers to something");
    assert_eq!(item.data.as_ref().unwrap()["key"], "child😀");
    assert_eq!(item.data.as_ref().unwrap()["parent"], "父😀");
    let related = item.related_information.unwrap();
    assert_eq!(related.len(), 1);
    let offset = parent.find("父😀").unwrap();
    assert_eq!(
        related[0].location,
        Location::new(
            uri(root, "defs.bib"),
            Text::new(parent.into()).range(offset, offset + "父😀".len())
        )
    );
    assert!(related[0].message.contains("nested crossref chain"));
    assert!(!related[0].message.contains("skip an uncited parent"));
    assert!(!related[0].message.contains("did not resolve"));
}

#[test]
fn nested_self_references_and_cycles_keep_native_warning_severity_and_related_keys() {
    for (source, parent) in [
        ("@misc{child,crossref={child}}", "child"),
        (
            "@misc{child,crossref={parent}}\n@misc{parent,crossref={child}}",
            "parent",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[("refs.bib", source)],
            &format!("Database file #1: refs.bib\n{}", warning("child", parent)),
        );
        let item = diagnostic_for(&found, &uri(root, "refs.bib"), NESTED_CODE);
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        let text = Text::new(source.into());
        let operand = source.find(&format!("crossref={{{parent}}}")).unwrap() + "crossref={".len();
        assert_eq!(item.range, text.range(operand, operand + parent.len()));
        let declaration = source.find(&format!("@misc{{{parent},")).unwrap() + "@misc{".len();
        let related = item.related_information.unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(
            related[0].location,
            Location::new(
                uri(root, "refs.bib"),
                text.range(declaration, declaration + parent.len())
            )
        );
    }
}

#[test]
fn missing_grandparents_keep_independent_warning_and_error_positions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@misc{child,crossref={parent}}\n@misc{parent,crossref={missing}}\n";
    let found = parse(
        root,
        &[("refs.bib", source)],
        &format!(
            "Database file #1: refs.bib\n{}{}",
            warning("child", "parent"),
            error("parent", "missing")
        ),
    );
    let refs = uri(root, "refs.bib");
    assert_eq!(found[&refs].len(), 2);
    let nested = diagnostic_for(&found, &refs, NESTED_CODE);
    let missing = diagnostic(&found, &refs);
    assert_eq!(nested.severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(missing.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(
        nested.range,
        Range::new(Position::new(0, 22), Position::new(0, 28))
    );
    assert_eq!(
        missing.range,
        Range::new(Position::new(1, 23), Position::new(1, 30))
    );
    assert_eq!(
        nested.related_information.unwrap()[0].location.range,
        Range::new(Position::new(1, 6), Position::new(1, 12))
    );
    assert!(missing.related_information.is_none());
}

#[test]
fn incomplete_and_wrong_kind_continuations_do_not_consume_independent_records() {
    for continuation in [
        "",
        "\n",
        "refers to entry \"parent\"\n",
        "refers to entry \"parent\", which also refers to something extra\n",
        "refers to entry \"parent\", which doesn't exist\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "@misc{child,crossref={parent}}\n@misc{parent,crossref={missing}}";
        let log = format!("Database file #1: refs.bib\nWarning--you've nested cross references--entry \"child\"\n{continuation}{}Warning--empty title in parent\n",error("parent", "missing"));
        let found = parse(root, &[("refs.bib", source)], &log);
        let nested = diagnostic_for(&found, &uri(root, "main.tex"), NESTED_CODE);
        assert_eq!(nested.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(nested.data.unwrap()["unresolvedSource"], true);
        assert!(nested.related_information.is_none());
        assert_eq!(
            diagnostic(&found, &uri(root, "refs.bib")).severity,
            Some(DiagnosticSeverity::ERROR)
        );
        assert!(found
            .values()
            .flatten()
            .any(|item| item.message == "BibTeX: empty title in parent"));
    }
}

#[test]
fn incomplete_missing_parent_errors_do_not_consume_nested_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@misc{child,crossref={parent}}\n@misc{parent,crossref={missing}}";
    let found = parse(
        root,
        &[("refs.bib", source)],
        &format!(
            "Database file #1: refs.bib\nA bad cross reference---entry \"parent\"\n{}",
            warning("child", "parent")
        ),
    );
    unresolved(&found, root);
    assert_eq!(
        diagnostic_for(&found, &uri(root, "refs.bib"), NESTED_CODE).severity,
        Some(DiagnosticSeverity::WARNING)
    );
}

#[test]
fn nested_warnings_with_uncertain_sources_stay_warnings_on_the_root() {
    for (history, source) in [
        ("", "@misc{child,crossref={parent}}"),
        (
            "Database file #2: refs.bib\n",
            "@misc{child,crossref={parent}}",
        ),
        (
            "Database file #1: refs.bib\n[texquill: middle of compiler output omitted]\n",
            "@misc{child,crossref={parent}}",
        ),
        (
            "Database file #1: missing.bib\nDatabase file #2: refs.bib\n",
            "@misc{child,crossref={parent}}",
        ),
        (
            "Database file #1: refs.bib\n",
            "@misc{child,crossref=macro}",
        ),
        (
            "Database file #1: refs.bib\n",
            "@misc{child,crossref={parent},crossref=macro}",
        ),
        (
            "Database file #1: refs.bib\n",
            "@misc{child,crossref={parent}} @misc{CHILD,title={Duplicate}}",
        ),
        (
            "Database file #1: refs.bib\n",
            "@misc{child,crossref={parent}",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[("refs.bib", source)],
            &format!("{history}{}", warning("child", "parent")),
        );
        let item = diagnostic_for(&found, &uri(root, "main.tex"), NESTED_CODE);
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.data.unwrap()["unresolvedSource"], true);
        assert!(item.related_information.is_none());
        assert!(item.message.ends_with("which also refers to something"));
    }
}

#[test]
fn nested_warning_examples_in_other_records_do_not_acquire_native_codes() {
    for prefix in [
        "The style file: ",
        "Database file #1: ",
        "The top-level auxiliary file: ",
        " : ",
        "INFO - ",
        "[0] Utils.pm:395> INFO - ",
        "WARN - ",
        "ERROR - ",
        "Warning--Example: ",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[("refs.bib", "@misc{child,crossref={parent}}")],
            &format!("{prefix}{}", warning("child", "parent")),
        );
        assert!(found
            .values()
            .flatten()
            .all(|item| item.code != Some(NumberOrString::String(NESTED_CODE.into()))));
    }
}

fn unresolved(found: &BTreeMap<Url, Vec<Diagnostic>>, root: &Path) {
    let item = diagnostic(found, &uri(root, "main.tex"));
    assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(item.range, Range::default());
    assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    assert!(item.related_information.is_none());
}

#[test]
fn native_two_line_errors_highlight_only_the_owning_literal_crossref_in_utf16() {
    for (child, parent) in [
        ("child", "parent"),
        ("child😀", "父😀"),
        ("book(old)", "O'Connor2026"),
    ] {
        for (open, close, value) in [
            ("{", "}", format!("{{{parent}}}")),
            ("(", ")", format!("\"{parent}\"")),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("% 😀\r\n@misc{open}unrelated,crossref={value}{close}\r\n@inproceedings{open}{child},\r\n title={{{parent}}}, CROSSREF = {value}{close}\r\n");
            let found = parse(
                root,
                &[("refs.bib", &source)],
                &format!("Database file #1: refs.bib\n{}", error(child, parent)),
            );
            let item = diagnostic(&found, &uri(root, "refs.bib"));
            let offset = source.rfind(parent).unwrap();
            assert_eq!(
                item.range,
                Text::new(source).range(offset, offset + parent.len())
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(item.data.as_ref().unwrap()["key"], child);
            assert_eq!(item.data.as_ref().unwrap()["parent"], parent);
            assert!(item.related_information.is_none());
            assert!(item
                .message
                .contains(&format!("refers to entry \"{parent}\"")));
        }
    }
}

#[test]
fn earlier_parents_have_related_locations_and_a_conditional_order_explanation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@proceedings{PARENT,title={Earlier}}\r\n@misc{child,crossref={parent}}\r\n@proceedings{parent,title={Later}}";
    let found = parse(
        root,
        &[
            ("defs.bib", "@proceedings{parent,title={First}}"),
            ("refs.bib", source),
            ("unread.bib", "@misc{parent}"),
        ],
        &format!(
            "Database file #1: defs.bib\nDatabase file #2: refs.bib\n{}",
            error("child", "parent")
        ),
    );
    let item = diagnostic(&found, &uri(root, "refs.bib"));
    let related = item.related_information.unwrap();
    assert_eq!(related.len(), 3);
    assert_eq!(related[0].location.uri, uri(root, "defs.bib"));
    assert_eq!(
        related[1].location.range,
        Range::new(Position::new(0, 13), Position::new(0, 19))
    );
    assert!(related[0].message.contains("can skip an uncited parent"));
    assert!(related[1].message.contains("declared before the child"));
    assert!(related[2].message.contains("did not resolve"));
    assert!(!related[2].message.contains("declared before"));
}

#[test]
fn empty_and_padded_literals_preserve_the_full_message_and_use_visible_ranges() {
    for (literal, parent, expected) in [
        ("{}", "", "{}"),
        ("{ }", "", "{ }"),
        ("\"\"", "", "\"\""),
        ("{  parent  }", "parent", "parent"),
        ("\" parent \"", "parent", "parent"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("@misc{{child,crossref={literal}}}");
        let found = parse(
            root,
            &[("refs.bib", &source)],
            &format!("Database file #1: refs.bib\n{}", error("child", parent)),
        );
        let item = diagnostic(&found, &uri(root, "refs.bib"));
        let offset = source.rfind(expected).unwrap();
        assert_eq!(
            item.range,
            Text::new(source).range(offset, offset + expected.len())
        );
        assert_eq!(item.data.unwrap()["parent"], parent);
        assert!(item.message.ends_with(&format!(
            "refers to entry \"{parent}\", which doesn't exist"
        )));
    }
}

#[test]
fn duplicate_children_and_case_mismatches_do_not_borrow_an_operand() {
    for source in [
        "@misc{child,crossref={parent}} @misc{child,crossref={parent}}",
        "@misc{child,crossref={parent}} @misc{CHILD,title={Duplicate}}",
        "@misc{CHILD,crossref={parent}}",
        "@misc{other,crossref={parent}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[("refs.bib", source)],
            &format!("Database file #1: refs.bib\n{}", error("child", "parent")),
        );
        unresolved(&found, root);
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let found = parse(
        root,
        &[
            ("refs.bib", "@misc{child,crossref={parent}}"),
            ("other.bib", "@misc{child,title={Duplicate}}"),
        ],
        &format!(
            "Database file #1: refs.bib\nDatabase file #2: other.bib\n{}",
            error("child", "parent")
        ),
    );
    unresolved(&found, root);
}

#[test]
fn dynamic_duplicate_incomplete_and_unrelated_fields_stay_on_the_root() {
    for source in [
        "@misc{child,crossref=parent}",
        "@misc{child,crossref={par} # {ent}}",
        "@misc{child,crossref={parent},crossref=macro}",
        "@misc{child,crossref={parent},CROSSREF={other}}",
        "@misc{child,crossref={parent}",
        "@misc{child,crossref={{parent}}}",
        "@misc{child,title={crossref={parent}}}",
        "@misc{child,xref={parent},related={parent}}",
        "@misc{child,crossref={PARENT}}",
        "@comment{@misc{child,crossref={parent}}}",
        "% @misc{child,crossref={parent}}",
        "@string{child={parent}}",
        "@misc{child,title={Text}} @misc{other,crossref={parent}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[("refs.bib", source)],
            &format!("Database file #1: refs.bib\n{}", error("child", "parent")),
        );
        unresolved(&found, root);
    }
}

#[test]
fn incomplete_unread_invalid_or_missing_database_histories_do_not_guess_locations() {
    for history in [
        "",
        "Database file #2: refs.bib\n",
        "Database file #x: refs.bib\n",
        "Database file #1: missing.bib\nDatabase file #2: refs.bib\n",
        "Database file #1: refs.bib\nDatabase file #2: refs.bib\n",
        "Database file #1: refs.bib\nExpected a comma---line 1 of file refs.bib\n",
        "Database file #1: refs.bib\n[texquill: middle of compiler output omitted]\n",
        "Database file #1: refs.data\n",
        "INFO - Found BibTeX data source 'refs.bib'\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[
                ("refs.bib", "@misc{child,crossref={parent}}"),
                ("refs.data", "@misc{child,crossref={parent}}"),
            ],
            &format!(
                "{history}{}Database file #1: refs.bib\n",
                error("child", "parent")
            ),
        );
        unresolved(&found, root);
    }
}

#[test]
fn broken_continuations_keep_the_error_and_do_not_swallow_the_next_record() {
    for continuation in [
        "",
        "\n",
        "refers to entry \"parent\"",
        "refers to entry \"parent\", which doesn't exist extra",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(root, &[("refs.bib", "@misc{child,crossref={parent}}")], &format!("Database file #1: refs.bib\nA bad cross reference---entry \"child\"\n{continuation}\nWarning--empty title in child\n"));
        unresolved(&found, root);
        assert!(found
            .values()
            .flatten()
            .any(|item| item.message == "BibTeX: empty title in child"));
    }
}

#[test]
fn metadata_echoes_and_biber_messages_do_not_create_native_crossref_errors() {
    for prefix in [
        "The style file: ",
        "Database file #1: ",
        "The top-level auxiliary file: ",
        " : ",
        "INFO - ",
        "[0] Utils.pm:395> INFO - ",
        "WARN - ",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[("refs.bib", "@misc{child,crossref={parent}}")],
            &format!("{prefix}{}", error("child", "parent")),
        );
        assert!(found
            .values()
            .flatten()
            .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
    }
}

#[test]
fn parent_related_information_is_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@misc{parent,title={Repeated}}\n".repeat(33) + "@misc{child,crossref={parent}}";
    let found = parse(
        root,
        &[("refs.bib", &source)],
        &format!("Database file #1: refs.bib\n{}", error("child", "parent")),
    );
    assert_eq!(
        diagnostic(&found, &uri(root, "refs.bib"))
            .related_information
            .unwrap()
            .len(),
        16
    );
}

#[test]
fn entry_and_file_size_limits_keep_the_original_error_on_the_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!("Database file #1: refs.bib\n{}", error("child", "parent"));
    let many = (0..16_384)
        .map(|index| format!("@misc{{entry{index},title={{Other}}}}\n"))
        .collect::<String>()
        + "@misc{child,crossref={parent}}";
    let oversized = "%".repeat(2 * 1024 * 1024) + "\n@misc{child,crossref={parent}}";
    for source in [many, oversized] {
        let found = parse(root, &[("refs.bib", &source)], &log);
        unresolved(&found, root);
    }
}

#[test]
fn style_and_auxiliary_inputs_do_not_supply_children_or_related_parent_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let files = [
        ("refs.bib", "@misc{child,crossref={parent}}"),
        (
            "local.bst",
            "@misc{child,crossref={parent}} @misc{parent,title={Wrong role}}",
        ),
        ("main.aux", "@misc{child,crossref={parent}}"),
    ];
    let found = parse(root, &files, &format!("The top-level auxiliary file: main.aux\nThe style file: local.bst\nDatabase file #1: refs.bib\n{}", error("child", "parent")));
    let item = diagnostic(&found, &uri(root, "refs.bib"));
    assert!(item.related_information.is_none());
    assert_eq!(found.len(), 1);
}

#[test]
#[cfg(unix)]
fn database_aliases_locate_physical_files_and_duplicate_aliases_remain_ambiguous() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("physical.data"), "@misc{child,crossref={parent}}").unwrap();
    symlink("physical.data", root.join("refs.bib")).unwrap();
    let found = parse(
        root,
        &[],
        &format!("Database file #1: refs.bib\n{}", error("child", "parent")),
    );
    assert_eq!(
        diagnostic(&found, &uri(root, "physical.data"))
            .data
            .unwrap()["parent"],
        "parent"
    );
    symlink("physical.data", root.join("second.bib")).unwrap();
    let found = parse(
        root,
        &[],
        &format!(
            "Database file #1: refs.bib\nDatabase file #2: second.bib\n{}",
            error("child", "parent")
        ),
    );
    unresolved(&found, root);
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_crossref_errors_distinguish_missing_and_earlier_uncited_parents() {
    let child = "@inproceedings{child,author={Writer, A},title={Child},crossref={parent}}\n";
    let parent = "@proceedings{parent,title={Parent},year=2026,editor={Editor, A}}\n";
    for (source, citations, bad, related) in [
        (child.to_owned(), "child", true, false),
        (format!("{parent}{child}"), "child", true, true),
        (format!("{child}{parent}"), "child", false, false),
        (format!("{parent}{child}"), "child,parent", false, false),
        (format!("{parent}{child}"), "*", false, false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        parse(root, &[("refs.bib", &source)], "");
        std::fs::write(
            root.join("main.aux"),
            format!("\\relax\n\\citation{{{citations}}}\n\\bibstyle{{plain}}\n\\bibdata{{refs}}\n"),
        )
        .unwrap();
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if bad { 2 } else { 0 }),
            "{output:?}"
        );
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let errors: Vec<_> = found
            .values()
            .flatten()
            .filter(|item| item.code == Some(NumberOrString::String(CODE.into())))
            .collect();
        assert_eq!(errors.len(), usize::from(bad), "{log}\n{found:?}");
        if bad {
            let item = diagnostic(&found, &uri(root, "refs.bib"));
            let offset = source.rfind("parent").unwrap();
            assert_eq!(item.range, Text::new(source).range(offset, offset + 6));
            assert_eq!(item.related_information.is_some(), related);
        }
    }
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_empty_and_padded_crossrefs_preserve_native_operands() {
    for (literal, parent, highlight) in [
        ("{}", "", "{}"),
        ("{ }", "", "{ }"),
        ("\"\"", "", "\"\""),
        ("{  parent  }", "parent", "parent"),
        ("\" parent \"", "parent", "parent"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("@misc{{child,crossref={literal}}}\n");
        parse(root, &[("refs.bib", &source)], "");
        std::fs::write(
            root.join("main.aux"),
            "\\relax\n\\citation{child}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
        )
        .unwrap();
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let item = diagnostic(&found, &uri(root, "refs.bib"));
        let offset = source.rfind(highlight).unwrap();
        assert_eq!(
            item.range,
            Text::new(source).range(offset, offset + highlight.len())
        );
        assert_eq!(item.data.unwrap()["parent"], parent);
    }
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_nested_crossrefs_preserve_chains_cycles_self_references_and_missing_ancestors() {
    for (source, parent, missing) in [
        ("@misc{child,title={Child},crossref={parent}}\n@misc{parent,crossref={grandparent}}\n@misc{grandparent,title={Top}}\n", "parent", false),
        ("@misc{child,title={Child},crossref={child}}\n", "child", false),
        ("@misc{child,title={Child},crossref={parent}}\n@misc{parent,crossref={child}}\n", "parent", false),
        ("@misc{child,title={Child},crossref={parent}}\n@misc{parent,crossref={missing}}\n", "parent", true),
        ("@string{Next={grandparent}}\n@misc{child,title={Child},crossref={parent}}\n@misc{parent,crossref=Next}\n@misc{grandparent,title={Top}}\n", "parent", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        parse(root, &[("refs.bib", source)], "");
        std::fs::write(root.join("main.aux"), "\\relax\n\\citation{child}\n\\bibstyle{plain}\n\\bibdata{refs}\n").unwrap();
        let output = std::process::Command::new("bibtex").arg("main").current_dir(root).output().unwrap();
        assert_eq!(output.status.code(), Some(if missing { 2 } else { 0 }), "{output:?}");
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let nested = diagnostic_for(&found, &uri(root, "refs.bib"), NESTED_CODE);
        assert_eq!(nested.severity, Some(DiagnosticSeverity::WARNING));
        let text = Text::new(source.into());
        let offset = source.find(&format!("crossref={{{parent}}}")).unwrap() + "crossref={".len();
        assert_eq!(nested.range, text.range(offset, offset + parent.len()));
        assert!(nested.message.ends_with(&format!("refers to entry \"{parent}\", which also refers to something")));
        let related = nested.related_information.unwrap();
        assert_eq!(related.len(), 1);
        let offset = source.find(&format!("@misc{{{parent},")).unwrap() + "@misc{".len();
        assert_eq!(related[0].location, Location::new(uri(root, "refs.bib"), text.range(offset, offset + parent.len())));
        let errors: Vec<_> = found.values().flatten().filter(|item| item.severity == Some(DiagnosticSeverity::ERROR)).collect();
        assert_eq!(errors.len(), usize::from(missing), "{log}\n{found:?}");
        if missing {
            let item = diagnostic(&found, &uri(root, "refs.bib"));
            let offset = source.find("{missing}").unwrap() + 1;
            assert_eq!(item.range, text.range(offset, offset + "missing".len()));
        }
        // The compiler owns the warning: removing the parent's crossref clears
        // it without inventing a static warning for an ordinary relationship.
        let repaired = "@misc{child,title={Child},crossref={parent}}\n@misc{parent,title={Parent}}\n";
        std::fs::write(root.join("refs.bib"), repaired).unwrap();
        let output = std::process::Command::new("bibtex").arg("main").current_dir(root).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        let cleared = build_log::parse_bibliography(&root.join("main.tex"), &log);
        assert!(cleared.values().flatten().all(|item| item.code != Some(NumberOrString::String(NESTED_CODE.into()))));
    }
}
