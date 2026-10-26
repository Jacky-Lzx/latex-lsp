use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace};

#[path = "support/completion.rs"]
mod completion;
fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}\\DeclareBibliographyCategory{{group}}\\addbibresource{{data.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(
        dir.path().join("data.bib"),
        "@book{parent,title={Raw parent}} @book{member,title={Member}} @book{other,title={Other}}",
    )
    .unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("data.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let at = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&doc.text.source[at..at + name.len()], name);
    Location::new(uri.clone(), doc.text.range(at, at + name.len()))
}

fn completions(ws: &Workspace, uri: &Url, prefix: &str) -> Vec<CompletionItem> {
    let doc = &ws.documents[uri];
    let at = doc.text.source.find(prefix).unwrap() + prefix.len();
    features::completion(ws, uri, doc.text.position(at))
}
fn candidate(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> CompletionItem {
    completions(ws, uri, prefix)
        .into_iter()
        .find(|item| item.label == name)
        .unwrap_or_else(|| panic!("missing {name} at {prefix}"))
}
#[test]
fn textual_predicate_completion_uses_final_source_identity_without_an_early_request() {
    for predicate in [
        r"\ifentryinbib{par}{Yes}{No}",
        r"\ifentrycategory{par}{group}{Yes}{No}",
    ] {
        let (_dir, ws, main, _bib) = setup(&format!(
            r"{predicate}\defbibentryset{{parent}}{{member}}\cite{{parent}}"
        ));
        let prefix = &predicate[..predicate.find("par}").unwrap() + 2];
        let item = candidate(&ws, &main, prefix, "parent");
        assert!(
            item.detail
                .as_ref()
                .unwrap()
                .contains("Dynamic bibliography set: member"),
            "{item:?}"
        );
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        let full = key(
            &ws,
            &main,
            &predicate[..predicate.find("par}").unwrap()],
            "par",
        );
        assert_eq!(edit.range, full.range);
        assert_eq!(edit.new_text, "parent");
    }
}
#[test]
fn named_category_predicate_completion_uses_final_sets_without_explicit_citations() {
    for predicate in [
        r"\csuse{ifentryinbib}{par}{存在}{缺失}",
        r"\csname ifentrycategory\endcsname{par}{group}{Yes}{No}",
    ] {
        let (_dir, ws, main, _bib) =
            setup(&format!(r"{predicate}\defbibentryset{{parent}}{{member}}"));
        let prefix = &predicate[..predicate.find("par}").unwrap() + 2];
        let item = candidate(&ws, &main, prefix, "parent");
        assert!(
            item.detail
                .as_ref()
                .unwrap()
                .contains("Dynamic bibliography set: member"),
            "{item:?}"
        );
    }
}
#[test]
fn early_real_requests_keep_raw_metadata_for_predicate_completion() {
    let (_dir, ws, main, _bib) =
        setup(r"\cite{parent}\ifentryinbib{par}{}{}\defbibentryset{parent}{member}\cite{parent}");
    let item = candidate(&ws, &main, r"\ifentryinbib{pa", "parent");
    assert_eq!(
        completion::preview(&item),
        "```bibtex\n@book{parent,title={Raw parent}}\n```"
    );
}

fn applied(ws: &Workspace, uri: &Url, item: CompletionItem) -> String {
    let doc = &ws.documents[uri];
    let Some(CompletionTextEdit::Edit(main)) = item.text_edit else {
        panic!()
    };
    let mut edits = item.additional_text_edits.unwrap_or_default();
    edits.push(main);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let mut source = doc.text.source.clone();
    let mut previous = source.len();
    for edit in edits {
        let start = doc.text.offset(edit.range.start).unwrap();
        let end = doc.text.offset(edit.range.end).unwrap();
        assert!(end <= previous);
        source.replace_range(start..end, &edit.new_text);
        previous = start;
    }
    source
}
#[test]
fn empty_suffix_and_commented_keys_keep_complete_edits_and_final_navigation() {
    for (predicate, prefix) in [
        (r"\ifentryinbib{}{}{}", r"\ifentryinbib{"),
        (r"\ifentryinbib{parSuffix}{Yes}{No}", r"\ifentryinbib{pa"),
        (
            "\\ifentryinbib{% 😀\r\npaSuffix% tail\r\n}{Yes}{No}",
            "\\ifentryinbib{% 😀\r\npa",
        ),
        (r"\ifentrycategory{}{group}{}{}", r"\ifentrycategory{"),
    ] {
        let (_dir, mut ws, main, _bib) =
            setup(&format!(r"{predicate}\defbibentryset{{parent}}{{member}}"));
        assert!(
            completions(&ws, &main, prefix)
                .iter()
                .any(|item| item.label == "parent"),
            "{predicate}"
        );
        let item = candidate(&ws, &main, prefix, "parent");
        let source = applied(&ws, &main, item);
        assert!(!source.contains("Suffix"));
        assert!(!source.contains("pa%"));
        ws.open(main.clone(), source, 6);
        let doc = &ws.documents[&main];
        let at = doc.text.source.find("parent").unwrap();
        let query = Location::new(main.clone(), doc.text.range(at, at + "parent".len()));
        let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![declaration]
        );
    }
}
#[test]
fn predicate_candidates_do_not_change_request_position_cite_candidates() {
    let (_dir, ws, main, _bib) =
        setup(r"\ifentryinbib{par}{}{}\cite{par}\defbibentryset{parent}{member}");
    let item = candidate(&ws, &main, r"\ifentryinbib{pa", "parent");
    assert!(item.detail.unwrap().contains("Dynamic bibliography set"));
    let item = candidate(&ws, &main, r"\cite{pa", "parent");
    assert_eq!(
        completion::preview(&item),
        "```bibtex\n@book{parent,title={Raw parent}}\n```"
    );
    for early in [r"\cite{collected}", r"\cite{collected}\nocite{*}"] {
        let (_dir, ws, main, _bib) = setup(&format!(
            r"{early}\ifentryinbib{{col}}{{}}{{}}\defbibentryset{{collected}}{{member}}"
        ));
        assert!(completions(&ws, &main, r"\ifentryinbib{col")
            .iter()
            .all(|item| item.label != "collected"));
    }
}
#[test]
fn selected_sections_use_their_final_set_or_their_real_early_book_metadata() {
    for early in [false, true] {
        let (dir, mut ws, main, _bib) = setup(&format!(
            r"\newrefsection[chosen]{}\ifentryinbib{{par}}{{}}{{}}\defbibentryset{{parent}}{{member}}",
            if early { r"\cite{parent}" } else { "" }
        ));
        std::fs::write(
            dir.path().join("selected.bib"),
            "@book{parent,title={Selected parent}} @book{member,title={Selected member}}",
        )
        .unwrap();
        let source = ws.documents[&main].text.source.replace(
            r"\begin{document}",
            r"\addsectionbib[label=chosen]{selected.bib}\begin{document}",
        );
        ws.open(main.clone(), source, 6);
        let item = candidate(&ws, &main, r"\ifentryinbib{pa", "parent");
        let detail = completion::preview(&item);
        assert!(
            detail.contains(if early {
                "Selected parent"
            } else {
                "Dynamic bibliography set: member"
            }),
            "{detail}"
        );
        assert!(!detail.contains("Raw parent"));
        if early {
            assert!(!detail.contains("selected.bib"));
        } else {
            assert!(!detail.contains("main.tex"));
        }
    }
}
#[test]
fn shared_queries_intersect_exact_physical_final_identities_and_refresh_inputs() {
    let (dir, mut ws, main, _bib) =
        setup(r"\input{query}\input{sets}\newrefsection\input{query}\input{sets}");
    let query = main.join("query.tex").unwrap();
    let sets = main.join("sets.tex").unwrap();
    std::fs::write(dir.path().join("query.tex"), r"\ifentryinbib{par}{}{}").unwrap();
    std::fs::write(
        dir.path().join("sets.tex"),
        r"\defbibentryset{parent}{member}",
    )
    .unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source.clone(), 6);
    assert!(candidate(&ws, &query, r"\ifentryinbib{pa", "parent")
        .detail
        .unwrap()
        .contains("Dynamic bibliography set"));
    let other = main.join("other.tex").unwrap();
    ws.open(other.clone(), source.clone(), 7);
    assert!(candidate(&ws, &query, r"\ifentryinbib{pa", "parent")
        .detail
        .unwrap()
        .contains("Dynamic bibliography set"));
    ws.open(
        other.clone(),
        source.replace(r"\input{sets}", r"\defbibentryset{parent}{member}"),
        8,
    );
    assert!(completions(&ws, &query, r"\ifentryinbib{pa")
        .iter()
        .all(|item| item.label != "parent"));
    ws.documents.remove(&other);
    assert!(candidate(&ws, &query, r"\ifentryinbib{pa", "parent")
        .detail
        .unwrap()
        .contains("Dynamic bibliography set"));
    ws.open(sets.clone(), r"\defbibentryset{parent}{other}".into(), 9);
    assert!(candidate(&ws, &query, r"\ifentryinbib{pa", "parent")
        .detail
        .unwrap()
        .contains("Dynamic bibliography set: other"));
    ws.open(sets, r"\defbibentryset{parent}{member}".into(), 10);
    assert!(candidate(&ws, &query, r"\ifentryinbib{pa", "parent")
        .detail
        .unwrap()
        .contains("Dynamic bibliography set: member"));
}
#[test]
fn unknown_predicate_meanings_branches_and_ids_cannot_offer_unproved_candidates() {
    for predicate in [
        r"\ifentryinbib{par}{\cite{other}}{}",
        r"\ifentrycategory{par}{group}{\newrefsection}{}",
        r"\ifentryinbib{par}{#1}{}",
        r"\ifentryinbib{par}{Unfinished}",
        r"\entrydata{par}{Literal}",
        r"\ifentryexists{par}{Yes}{No}",
        r"\newcommand\future{\ifentryinbib{par}{}{}}",
        r"\take\ifentryinbib{par}{}{}",
        r"\ifentryinbib{\dynamic par}{}{}",
    ] {
        let (_dir, ws, main, _bib) =
            setup(&format!(r"{predicate}\defbibentryset{{parent}}{{member}}"));
        let prefix = &predicate[..predicate.find("par}").unwrap() + 2];
        assert!(
            completions(&ws, &main, prefix)
                .iter()
                .all(|item| item.label != "parent"),
            "{predicate}"
        );
    }
    for definition in [
        r"\renewcommand\ifentryinbib[3]{}",
        r"\renewcommand\csuse[1]{}",
    ] {
        let (_dir, ws, main, _bib) = setup(&format!(
            r"{definition}\csuse{{ifentryinbib}}{{par}}{{}}{{}}\defbibentryset{{parent}}{{member}}"
        ));
        assert!(completions(&ws, &main, r"\csuse{ifentryinbib}{pa")
            .iter()
            .all(|item| item.label != "parent"));
    }
    let (_dir, mut ws, main, bib) = setup(r"\ifentryinbib{}{}{}\defbibentryset{parent}{member}");
    let original = ws.documents[&bib].text.source.clone();
    ws.open(
        bib.clone(),
        original.replace("title={Raw parent}", "ids={alias},title={Raw parent}"),
        8,
    );
    assert!(completions(&ws, &main, r"\ifentryinbib{")
        .iter()
        .all(|item| item.label != "parent"));
    ws.open(bib, original, 9);
    assert!(candidate(&ws, &main, r"\ifentryinbib{", "parent")
        .detail
        .unwrap()
        .contains("Dynamic bibliography set"));
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_applied_predicate_completions_keep_the_complete_bibliography_and_final_identity() {
    for (predicate, early, wildcard) in [
        (r"\ifentryinbib{par}{Yes}{No}", false, false),
        (r"\ifentrycategory{par}{group}{Yes}{No}", true, false),
        (r"\csuse{ifentryinbib}{par}{Yes}{No}", false, false),
        (
            r"\csname ifentrycategory\endcsname{par}{group}{Yes}{No}",
            true,
            false,
        ),
        (r"\ifentryinbib{par}{Yes}{No}", false, true),
    ] {
        let (dir, ws, main, _bib) = setup(&format!(
            r"{}{predicate}\defbibentryset{{parent}}{{member}}\printbibliography",
            if early {
                r"\cite{parent}"
            } else if wildcard {
                r"\nocite{*}"
            } else {
                ""
            }
        ));
        let prefix = &predicate[..predicate.find("par}").unwrap() + 2];
        let item = candidate(&ws, &main, prefix, "parent");
        assert!(item.detail.as_ref().unwrap().contains(if early {
            "Raw parent"
        } else {
            "Dynamic bibliography set: member"
        }));
        let changed = applied(&ws, &main, item);
        let compile = |source: &str| {
            for extension in ["aux", "bcf", "bbl", "run.xml"] {
                let path = dir.path().join(format!("main.{extension}"));
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            }
            std::fs::write(dir.path().join("main.tex"), source).unwrap();
            for program in ["pdflatex", "biber", "pdflatex", "pdflatex"] {
                let args = if program == "biber" {
                    vec!["main"]
                } else {
                    vec![
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ]
                };
                let output = std::process::Command::new(program)
                    .args(args)
                    .current_dir(dir.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{program}: {}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
            let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
            assert!(
                !log.contains("undefined") && !log.contains("Warning"),
                "{log}"
            );
            assert!(!blg.contains("WARN") && !blg.contains("ERROR"), "{blg}");
            std::fs::read_to_string(dir.path().join("main.bbl")).unwrap()
        };
        let before = compile(&ws.documents[&main].text.source);
        let after = compile(&changed);
        assert_eq!(before, after, "{predicate}, early={early}");
        let expected = if early {
            r"\entry{parent}{book}"
        } else {
            r"\entry{parent}{set}"
        };
        assert!(after.contains(expected), "{after}");
    }
}
