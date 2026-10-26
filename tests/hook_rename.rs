use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;
use std::collections::BTreeMap;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:hook-rename.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    (workspace, uri)
}

fn changes(edit: WorkspaceEdit) -> BTreeMap<Url, Vec<TextEdit>> {
    if let Some(changes) = edit.changes {
        return changes.into_iter().collect();
    }
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected text edits")
    };
    documents
        .into_iter()
        .map(|document| {
            let edits = document
                .edits
                .into_iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    OneOf::Right(edit) => edit.text_edit,
                })
                .collect();
            (document.text_document.uri, edits)
        })
        .collect()
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

#[test]
fn literal_hook_renames_preserve_other_namespaces_and_registration_whitespace() {
    for template in [
        r"\NewHook{@@}\AddToHook{@@}[custom]{Text}\UseHook{@@}",
        r"\NewReversedHook{@@}\AddToHookNext{@@}{Text}\UseOneTimeHook{@@}",
        r"\NewHook{ @@ }\AddToHook{ @@ }{Text}\UseHook{@@}\UseHook{ custom }",
        r"\NewHookWithArguments{@@}{1}\AddToHookWithArguments{@@}{#1}\UseHookWithArguments{@@}{1}{custom}",
        r"\NewMirroredHookPair{@@}{partner}\AddToHook{@@}{Text}\UseHook{@@}\UseHook{partner}",
        r"\NewMirroredHookPairWithArguments{partner}{@@}{1}\AddToHookWithArguments{@@}{#1}\UseHook{@@}{custom}",
        r"\NewHook{@@}\DeclareHookRule{@@}{custom}{before}{second}\ClearHookRule{@@}{custom}{second}\RemoveFromHook{@@}[custom]\ClearHookNext{@@}\ShowHook{@@}\LogHook{@@}",
        r"\NewHook{@@}\IfHookExistsTF{@@}{Yes}{No}\IfHookEmptyTF{@@}{Yes}{No}\IfHookEmptyT{@@}{Yes}\IfHookEmptyF{@@}{No}",
        r"\csuse{NewHook}{@@}\csname AddToHook\endcsname{@@}{Text}\def\invoke{\UseHook}\invoke{@@}",
        r"\NewHook{@@}\newenvironment{invoke}{\UseHook}{}\invoke{@@}",
        r"\NewHook{@@}\newenvironment{invoke}{}{\UseHook}\endinvoke{@@}",
        r"\NewHook{@@}\newcommand\later{\UseHook{@@}}",
        r"\NewHook{@@}\csdef{invoke}{\UseHook}\invoke{@@}",
        r"\NewHook{@@}\NewDocumentCommand\invoke{}{\UseHook}\invoke{@@}",
        r"\NewHook{@@}\newcommand*{\invoke}{\UseHook}\invoke{@@}",
        r"\NewHook{@@}\NewDocumentEnvironment{invoke}{}{\UseHook}{}\invoke{@@}",
    ] {
        let template = format!("% UTF-16\r\n😀 {template}\\label{{custom}}\\ref{{custom}}\\typeout{{custom}}\\def\\custom{{custom}}");
        let source = template.replace("@@", "custom");
        let expected = template.replace("@@", "fresh");
        let (mut workspace, uri) = setup(&source);
        let start = template.find("@@").unwrap();
        let text = &workspace.documents[&uri].text;
        let at = text.position(start + 1);
        assert_eq!(
            features::prepare_rename(&workspace, &uri, at).unwrap(),
            Some(PrepareRenameResponse::RangeWithPlaceholder {
                range: text.range(start, start + 6),
                placeholder: "custom".into(),
            }),
            "{source}"
        );
        for versioned in [false, true] {
            workspace.versioned_edits = versioned;
            let edit = features::rename(&workspace, &uri, at, "fresh")
                .unwrap()
                .expect("complete hook rename");
            if versioned {
                let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
                    panic!("expected versioned edits")
                };
                assert_eq!(documents[0].text_document.version, Some(5));
            }
            let edits = changes(edit);
            assert_eq!(edits.len(), 1);
            assert_eq!(apply(&source, &edits[&uri]), expected, "{source}");
        }
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_opaque_names_and_generic_targets_expose_partial_edits_with_equal_aux() {
    for (prefix, extra, body, target, before_count, after_count) in [
        (
            "",
            "",
            r"\UseHook{custom}\ExplSyntaxOn\use:c{UseHook}{custom}\ExplSyntaxOff",
            "fresh",
            2,
            1,
        ),
        (
            "",
            "",
            r"\UseHook{custom}\csuse{use:c}{UseHook}{custom}",
            "fresh",
            2,
            1,
        ),
        (
            "",
            "",
            r"\UseHook{custom}\ExplSyntaxOn\cs_new_eq:NN\fire\UseHook\ExplSyntaxOff\fire{custom}",
            "fresh",
            2,
            1,
        ),
        (
            "",
            r"\def\which{custom}",
            r"\UseHook{\which}",
            "fresh",
            1,
            0,
        ),
        ("", "", r"\lowercase{\UseHook{CUSTOM}}", "fresh", 1, 0),
        (
            "",
            r"\DisableHook{custom}",
            r"\UseHook{custom}",
            "fresh",
            0,
            1,
        ),
        (
            "",
            r"\def\identity#1{#1}",
            r"{\let\detokenize\identity\detokenize{\UseHook {custom}}}",
            "fresh",
            1,
            0,
        ),
        (
            "",
            r"\newenvironment{invoke}{}{}",
            r"\begin{invoke}Text\end{invoke}\UseHook{custom}",
            "env/invoke/before",
            1,
            2,
        ),
        (
            r"\csdef{@customhook}{\typeout{EVENT:legacy}}",
            "",
            r"\UseHook{custom}",
            "fresh",
            2,
            1,
        ),
        (
            r"\csdef{@freshhook}{\typeout{EVENT:legacy}}",
            "",
            r"\UseHook{custom}",
            "fresh",
            1,
            2,
        ),
        (
            "",
            r"\NewHookWithArguments{capture}{1}\AddToHookWithArguments{capture}{\typeout{DATA:\detokenize{#1}}}",
            r"\UseHook{custom}\UseHook{capture}{\UseHook{custom}}",
            "fresh",
            1,
            1,
        ),
    ] {
        let source = format!(
            r"\documentclass{{article}}\usepackage{{etoolbox}}{prefix}\NewHook{{custom}}\AddToHook{{custom}}{{\typeout{{EVENT:run}}}}{extra}\begin{{document}}{body} Text\label{{safe}}\ref{{safe}}\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.find(r"\NewHook{custom}").unwrap() + r"\NewHook{".len());
        assert!(
            features::rename(&workspace, &uri, at, target).is_err(),
            "{source}"
        );
        let partial = source
            .replace(r"\NewHook{custom}", &format!(r"\NewHook{{{target}}}"))
            .replace(r"\AddToHook{custom}", &format!(r"\AddToHook{{{target}}}"))
            .replace(r"\UseHook{custom}", &format!(r"\UseHook{{{target}}}"));
        let directory = tempfile::tempdir().unwrap();
        let compile = |text: &str| {
            std::fs::write(directory.path().join("main.tex"), text).unwrap();
            let aux = directory.path().join("main.aux");
            if aux.exists() {
                std::fs::remove_file(&aux).unwrap();
            }
            for _ in 0..2 {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(directory.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{text}\n{}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            (
                log.lines()
                    .filter(|line| line.starts_with("EVENT:"))
                    .count(),
                std::fs::read_to_string(aux).unwrap(),
                log.lines()
                    .filter(|line| line.starts_with("DATA:"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
        };
        let before = compile(&source);
        let after = compile(&partial);
        assert_eq!((before.0, after.0), (before_count, after_count), "{source}");
        assert_eq!(before.1, after.1, "{source}");
        assert_ne!((before.0, before.2), (after.0, after.2), "{source}");
    }
}

#[test]
fn bare_hook_names_gain_groups_when_renamed_to_multiple_tokens() {
    let source = r"\NewMirroredHookPair ab\AddToHook a{Text}\UseHook a\UseHook b";
    let (workspace, uri) = setup(source);
    let at = workspace.documents[&uri]
        .text
        .position(source.find(" ab").unwrap() + 1);
    let edit = features::rename(&workspace, &uri, at, "custom.before")
        .unwrap()
        .expect("bare hook rename");
    assert_eq!(
        apply(source, &changes(edit)[&uri]),
        r"\NewMirroredHookPair {custom.before}b\AddToHook {custom.before}{Text}\UseHook {custom.before}\UseHook b"
    );
}

#[test]
fn dynamic_ambiguous_or_unindexed_calls_never_produce_partial_hook_edits() {
    for tail in [
        r"\def\hookname{custom}\UseHook{\hookname}",
        r"\NewHook{\hookname}",
        r"\UseHook{./custom}",
        r"\UseHook{custom",
        r"\UseHook",
        r"\NewHookWithArguments{other}{\count}",
        r"\UseHookWithArguments{custom}{\count}{Text}",
        r"\def\invoke#1{\UseHook}\invoke{Ignored}{custom}",
        r"\newenvironment{invoke}[1]{\UseHook}{}\invoke{Ignored}{custom}",
        r"\let\invoke\UseHook\invoke{custom}",
        r"\def\UseHook#1{}\UseHook{custom}",
        r"\def\which{UseHook}\csuse{\which}{custom}",
        r"\csname \which\endcsname{custom}",
        r"\ExplSyntaxOn\hook_use:n{custom}\ExplSyntaxOff",
        r"\ExplSyntaxOn\use:c{UseHook}{custom}\ExplSyntaxOff",
        r"\csuse{use:c}{UseHook}{custom}",
        r"\ExplSyntaxOn\cs_new_eq:NN\fire\UseHook\ExplSyntaxOff\fire{custom}",
        r"\csname __hook custom\endcsname",
        r"\NewHook{custom}",
        r"\input{\filename}",
        r"\def\which{UseHook}\csdef{\which}#1{}\UseHook{custom}",
        r"\def\which{UseHook}\expandafter\def\csname\which\endcsname#1{}\UseHook{custom}",
        r"\apptocmd{\UseHook}{Extra}{}{}\UseHook{custom}",
        r"\futurelet\UseHook\relax\UseHook{custom}",
        r"\scantokens{\UseHook{custom}}",
        r"\edef\fire{\unexpanded{\UseHook{custom}}}\fire",
        r"\csuse{csdef}{UseHook}#1{}\UseHook{custom}",
        r"\csuse{RenewDocumentCommand}\UseHook{m}{#1}\UseHook{custom}",
        r"\UseName{UseHook}{custom}",
        r"\let\fire\UseName\fire{UseHook}{custom}",
        r"\lowercase{\UseHook{CUSTOM}}",
        r"\let\convert\lowercase\convert{\UseHook{CUSTOM}}",
        r"\def\identity#1{#1}\let\detokenize\identity\detokenize{\UseHook{custom}}",
        r"\def\detokenize#1{#1}\detokenize{\UseHook{custom}}",
        r"\apptocmd{\detokenize}{Extra}{}{}\detokenize{\UseHook{custom}}",
        r"\let\patch\apptocmd\patch{\detokenize}{Extra}{}{}",
        r"\undef\detokenize\detokenize{\UseHook{custom}}",
        r"\def\identity#1{#1}\let\string\identity\string{\UseHook{custom}}",
        r"\def\verb#1{#1}\verb|\UseHook{custom}|",
        r"\apptocmd{\verb}{Extra}{}{}\verb|\UseHook{custom}|",
        r"\renewenvironment{verbatim}{}{}\begin{verbatim}\UseHook{custom}\end{verbatim}",
        r"\def\url#1{#1}\url{\UseHook{custom}}",
        r"\def\href#1{#1}\href{\UseHook{custom}}{Text}",
        r"\DisableHook{custom}",
        r"\ProvideHook{custom}",
        r"\ProvideReversedHook{custom}",
        r"\ProvideMirroredHookPair{custom}{other}",
        r"\let\disable\DisableHook\disable{custom}",
        r"\csuse{DisableHook}{custom}",
        r"\csdef{@customhook}{Text}",
        r"\csdef{@freshhook}{Text}",
        r"\csuse{@customhook}",
        r"\csappto{@freshhook}{Text}",
        r"\NewHookWithArguments{capture}{1}\AddToHookWithArguments{capture}{\typeout{\detokenize{#1}}}\UseHook{capture}{\UseHook{custom}}",
    ] {
        let source = format!(r"\NewHook{{custom}}\UseHook{{custom}}{tail}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.find("custom").unwrap() + 1);
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{source}"
        );
        assert!(
            !matches!(features::prepare_rename(&workspace, &uri, at), Ok(Some(_))),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires native pdflatex"]
fn native_global_rule_storage_name_cannot_become_a_custom_hook() {
    let source = r"\documentclass{article}\NewHook{custom}\AddToHook{custom}[alpha]{\typeout{EVENT:A}}\AddToHook{custom}[beta]{\typeout{EVENT:B}}\DeclareHookRule{custom}{alpha}{after}{beta}\begin{document}\UseHook{custom}Text\end{document}";
    let (workspace, uri) = setup(source);
    let at = workspace.documents[&uri]
        .text
        .position(source.find("custom").unwrap());
    assert!(features::rename(&workspace, &uri, at, "??").is_err());
    let directory = tempfile::tempdir().unwrap();
    for (text, valid) in [
        (source.to_owned(), true),
        (source.replace("{custom}", "{??}"), false),
    ] {
        std::fs::write(directory.path().join("main.tex"), text).unwrap();
        let output = std::process::Command::new("pdflatex")
            .args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
        if valid {
            assert_eq!(
                log.lines()
                    .filter(|line| line.starts_with("EVENT:"))
                    .collect::<Vec<_>>(),
                ["EVENT:B", "EVENT:A"]
            );
        } else {
            assert!(log.contains("g__hook_??_reversed_tl already defined"));
        }
    }
}

#[test]
fn target_names_cannot_capture_existing_uses_or_implicit_kernel_triggers() {
    for name in [
        "??",
        "begindocument",
        "begindocument/before",
        "enddocument/end",
        "label",
        "selectfont",
        "insertmark",
        "env/invoke/before",
        "cmd/invoke/after",
        "file/main.tex/before",
        "package/graphicx/after",
        "class/article/before",
        "include/chapter/end",
        "shipout/background",
        "para/begin",
        "build/page/before",
        "./fresh",
        "with space",
        "",
    ] {
        let source = r"\NewHook{custom}\UseHook{custom}";
        let (workspace, uri) = setup(source);
        let at = workspace.documents[&uri]
            .text
            .position(source.find("custom").unwrap());
        assert!(
            features::rename(&workspace, &uri, at, name).is_err(),
            "{name}"
        );
    }
    for tail in [
        r"\NewHook{fresh}",
        r"\UseHook{fresh}",
        r"\AddToHook{fresh}{Text}",
    ] {
        let source = format!(r"\NewHook{{custom}}\UseHook{{custom}}{tail}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.find("custom").unwrap());
        assert!(
            features::rename(&workspace, &uri, at, "fresh").is_err(),
            "{source}"
        );
    }
}

#[test]
fn shared_hook_identity_expands_all_consumers_and_refreshes_unsaved_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let a = Url::from_file_path(root.join("a.tex")).unwrap();
    let b = a.join("b.tex").unwrap();
    let shared = a.join("shared.tex").unwrap();
    let other = a.join("other.tex").unwrap();
    let definitions = r"\NewHook{custom}\AddToHook{custom}{Code}";
    let source = "\\documentclass{article}\\input{shared}\r\n😀 \\begin{document}\\UseHook{custom}\\label{custom}\\ref{custom}\\end{document}";
    let unrelated =
        r"\documentclass{article}\NewHook{custom}\begin{document}\UseHook{custom}\end{document}";
    for (uri, text) in [
        (&a, source),
        (&b, source),
        (&shared, definitions),
        (&other, unrelated),
    ] {
        std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(a.clone(), source.into(), 5);
    workspace.open(b.clone(), source.into(), 7);
    workspace.open(other.clone(), unrelated.into(), 9);
    let at = workspace.documents[&a]
        .text
        .position(source.find("{custom}").unwrap() + 2);
    let check = |workspace: &Workspace| {
        let edit = features::rename(workspace, &a, at, "fresh")
            .unwrap()
            .unwrap();
        let edits = changes(edit);
        assert_eq!(
            edits.keys().cloned().collect::<Vec<_>>(),
            [a.clone(), b.clone(), shared.clone()]
        );
        for uri in [&a, &b] {
            assert_eq!(
                apply(source, &edits[uri]),
                source.replace("UseHook{custom}", "UseHook{fresh}")
            );
        }
        assert_eq!(
            apply(definitions, &edits[&shared]),
            definitions.replace("custom", "fresh")
        );
        assert_eq!(workspace.documents[&a].version, Some(5));
    };
    check(&workspace);
    workspace.open(b.clone(), format!(r"\def\UseHook#1{{}}{source}"), 12);
    assert!(!matches!(
        features::rename(&workspace, &a, at, "fresh"),
        Ok(Some(_))
    ));
    workspace.open(b.clone(), source.into(), 13);
    check(&workspace);
    workspace.open(b.clone(), format!(r"\NewHook{{fresh}}{source}"), 14);
    assert!(features::rename(&workspace, &a, at, "fresh").is_err());
    workspace.open(b.clone(), source.into(), 15);
    let new_root = a.join("unsaved.tex").unwrap();
    workspace.open(new_root.clone(), format!(r"{source}\UseHook{{\which}}"), 11);
    assert!(!matches!(
        features::rename(&workspace, &a, at, "fresh"),
        Ok(Some(_))
    ));
    workspace.documents.remove(&new_root);
    workspace.reload(&new_root);
    check(&workspace);
    workspace.versioned_edits = true;
    let edit = features::rename(&workspace, &a, at, "fresh")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("versioned edit")
    };
    let versions: BTreeMap<_, _> = documents
        .into_iter()
        .map(|doc| (doc.text_document.uri, doc.text_document.version))
        .collect();
    assert_eq!(versions[&a], Some(5));
    assert_eq!(versions[&b], Some(15));
    assert_eq!(versions[&shared], None);
    assert!(!versions.contains_key(&other));
}

#[test]
fn generated_bibliography_code_and_incomplete_source_indexes_block_hook_rename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let database = main.join("refs.bib").unwrap();
    let source = r"\documentclass{article}\usepackage{biblatex}\addbibresource{refs.bib}\NewHook{custom}\begin{document}\UseHook{custom}\end{document}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("refs.bib"), "@book{one,title={One}}").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), source.into(), 5);
    let at = workspace.documents[&main]
        .text
        .position(source.find("custom").unwrap() + 1);
    assert!(features::rename(&workspace, &main, at, "fresh")
        .unwrap()
        .is_some());
    workspace.open(database.clone(), r"@preamble{\UseHook{custom}}".into(), 12);
    assert!(features::rename(&workspace, &main, at, "fresh").is_err());
    workspace.open(database, "@book{one,title={One}}".into(), 13);
    assert!(features::rename(&workspace, &main, at, "fresh")
        .unwrap()
        .is_some());
    workspace.index_truncated.insert("maxFiles".into());
    assert!(!matches!(
        features::rename(&workspace, &main, at, "fresh"),
        Ok(Some(_))
    ));
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_hook_name_edits_preserve_events_data_and_complete_aux() {
    for (preamble, body, old) in [
        (
            r"\NewHook{custom}\AddToHook{custom}[custom]{\typeout{EVENT:custom}}",
            r"\UseHook{custom}",
            "custom",
        ),
        (
            r"\NewHook{ custom }\AddToHook{ custom }{\typeout{EVENT:run}}",
            r"\UseHook{custom}\UseHook{ custom }",
            "custom",
        ),
        (
            r"\NewMirroredHookPair{custom}{partner}\AddToHook{custom}{\typeout{EVENT:before}}\AddToHook{partner}{\typeout{EVENT:after}}",
            r"\UseHook{custom}\UseHook{partner}",
            "custom",
        ),
        (
            r"\NewHook{custom}\AddToHook{custom}[first]{\typeout{EVENT:first}}\AddToHook{custom}[second]{\typeout{EVENT:second}}\DeclareHookRule{custom}{first}{after}{second}",
            r"\UseHook{custom}",
            "custom",
        ),
        (
            r"\csuse{NewHook}{custom}\csname AddToHook\endcsname{custom}{\typeout{EVENT:run}}\def\fire{\UseHook}",
            r"\fire{custom}",
            "custom",
        ),
        (
            r"\NewHookWithArguments{custom}{1}\AddToHookWithArguments{custom}{\typeout{EVENT:run}\typeout{DATA:\detokenize{#1}}}\def\custom{Text}",
            r"\UseHookWithArguments{custom}{1}\custom",
            "custom",
        ),
        (
            r"\NewReversedHook{custom}\AddToHook{custom}{\typeout{EVENT:once}}",
            r"\UseOneTimeHook{custom}\UseHook{custom}",
            "custom",
        ),
        (
            r"\NewMirroredHookPair ab\AddToHook a{\typeout{EVENT:before}}\AddToHook b{\typeout{EVENT:after}}",
            r"\UseHook a\UseHook b",
            "a",
        ),
    ] {
        let source = format!(
            r"\documentclass{{article}}\usepackage{{etoolbox}}{preamble}\begin{{document}}{body}Text\label{{custom}}\ref{{custom}}\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let start = source
            .find(if old == "a" { " ab" } else { "custom" })
            .unwrap()
            + usize::from(old == "a");
        let at = workspace.documents[&uri].text.position(start);
        let edit = features::rename(&workspace, &uri, at, "fresh.before")
            .unwrap()
            .expect("safe native rename");
        let edited = apply(&source, &changes(edit)[&uri]);
        let directory = tempfile::tempdir().unwrap();
        let compile = |text: &str| {
            std::fs::write(directory.path().join("main.tex"), text).unwrap();
            let aux = directory.path().join("main.aux");
            if aux.exists() {
                std::fs::remove_file(&aux).unwrap();
            }
            for _ in 0..2 {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(directory.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{text}\n{}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            let events: Vec<_> = log
                .lines()
                .filter(|line| line.starts_with("EVENT:") || line.starts_with("DATA:"))
                .map(str::to_owned)
                .collect();
            assert!(events.iter().any(|line| line.starts_with("EVENT:")));
            (events, std::fs::read_to_string(aux).unwrap())
        };
        assert_eq!(compile(&source), compile(&edited), "{source}\n{edited}");
    }
}
