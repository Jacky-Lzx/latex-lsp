use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{hyperref,etoolbox}}\\newcommand\\unicodeword{{😀}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source, 5);
    (directory, workspace, uri)
}

fn blocked(workspace: &Workspace, uri: &Url, key: &str) {
    let text = &workspace.documents[uri].text;
    let at = text.position(text.source.find(key).unwrap() + 1);
    let prepare = features::prepare_rename(workspace, uri, at);
    let rename = features::rename(workspace, uri, at, "fresh");
    assert!(
        !matches!(prepare, Ok(Some(_))),
        "{}: {prepare:?}",
        text.source
    );
    assert!(
        !matches!(rename, Ok(Some(_))),
        "{}: {rename:?}",
        text.source
    );
}

fn aliases() -> [(&'static str, &'static str); 7] {
    [
        (r"\def\jump{\hyperlink}", r"\jump{here}{Link}"),
        (r"\let\jump\hyperlink", r"\jump{here}{Link}"),
        ("", r"\csuse{hyperlink}{here}{Link}"),
        ("", r"\csname hyperlink\endcsname{here}{Link}"),
        (
            r"\def\jump{\hyperlink}\def\forward{\jump}",
            r"\forward{here}{Link}",
        ),
        (
            r"\let\jump\hyperlink\def\forward{\jump}",
            r"\csuse{forward}{here}{Link}",
        ),
        (r"\NewCommandCopy\jump\hyperlink", r"\jump{here}{Link}"),
    ]
}

#[test]
fn data_declarations_do_not_offer_pdf_destination_edits() {
    for body in [
        r"\typeout{\unexpanded{\hypertarget{ghost}{Text}}}",
        r"\message{\unexpanded{\hypertarget{ghost}{Text}}}",
        r"\newcommand\discard[1]{}\discard{\hypertarget{ghost}{Text}}",
        r"\newcommand\literal[1]{\detokenize{#1}}\literal{\hypertarget{ghost}{Text}}",
        r"\newcommand\unknown[1]{\textbf{#1}}\unknown{\hypertarget{ghost}{Text}}",
        r"\renewcommand\hypertarget[2]{#2}\hypertarget{ghost}{Text}",
        r"\typeout{\unexpanded{\hyperdef{category}{ghost}{Text}}}",
        r"\message{\unexpanded{\pdfbookmark{Title}{ghost}}}",
    ] {
        let (_directory, workspace, uri) = setup(body);
        blocked(&workspace, &uri, "ghost");
    }
}

#[test]
fn data_references_cannot_rewrite_log_or_display_text() {
    for call in [
        r"\typeout{\unexpanded{\hyperlink{here}{Text}}}",
        r"\message{\unexpanded{\hyperlink{here}{Text}}}",
        r"\newcommand\literal[1]{\detokenize{#1}}\literal{\hyperlink{here}{Text}}",
        r"\renewcommand\hyperlink[2]{#2}\hyperlink{here}{Text}",
    ] {
        let (_directory, workspace, uri) = setup(&format!(r"\hypertarget{{here}}{{Target}}{call}"));
        blocked(&workspace, &uri, "here");
    }
}

#[test]
fn indirect_and_captured_links_never_receive_partial_destination_renames() {
    for (preamble, call) in aliases() {
        let (_directory, workspace, uri) =
            setup(&format!(r"{preamble}\hypertarget{{here}}{{Target}}{call}"));
        blocked(&workspace, &uri, "here");
    }
}

#[test]
fn direct_native_text_identity_and_deferred_declarations_keep_complete_edits() {
    for body in [
        r"\hypertarget{here}{Target}\hyperlink{here}{Link}",
        r"\textbf{\hypertarget{here}{Target}}\hyperlink{here}{Link}",
        r"\newcommand\one[1]{#1}\one{\hypertarget{here}{Target}}\hyperlink{here}{Link}",
        r"\def\one#1;{#1}\one\hypertarget{here}{Target};\hyperlink{here}{Link}",
        r"\newcommand\later{\hypertarget{here}{Target}}\later\hyperlink{here}{Link}",
        r"\message{\unexpanded{\hypertarget{ghost}{Text}}}\hypertarget{here}{Target}\hyperlink{here}{Link}",
    ] {
        let (_directory, workspace, uri) = setup(body);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find("here").unwrap() + 1);
        assert!(
            features::prepare_rename(&workspace, &uri, at)
                .unwrap()
                .is_some(),
            "{body}"
        );
        let changes = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes[&uri].len(), 2, "{body}");
        let mut edits = changes[&uri].clone();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let changed = text
            .clone()
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
            .unwrap();
        assert_eq!(changed.source, text.source.replace("{here}", "{fresh}"));
    }
}

#[test]
fn shared_consumers_and_unsaved_replacements_revoke_only_affected_anchor_edits() {
    let directory = tempfile::tempdir().unwrap();
    let a = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let b = a.join("new-unsaved.tex").unwrap();
    let unrelated = a.join("other-unsaved.tex").unwrap();
    let shared = a.join("shared.tex").unwrap();
    let provider = a.join("commands.sty").unwrap();
    let source = r"\documentclass{article}\input{commands.sty}\begin{document}\input{shared}\hyperlink{here}{Link}\end{document}";
    for (uri, text) in [
        (&a, source),
        (&shared, r"\hypertarget{here}{Target}"),
        (&provider, ""),
    ] {
        std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(a.clone(), source.into(), 5);
    let allowed = |workspace: &Workspace| {
        let text = &workspace.documents[&a].text;
        let edit = features::rename(
            workspace,
            &a,
            text.position(source.find("here").unwrap() + 1),
            "fresh",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            edit.changes.unwrap().values().map(Vec::len).sum::<usize>(),
            2
        );
    };
    allowed(&workspace);
    workspace.open(unrelated.clone(), r"\documentclass{article}\def\jump{\hyperlink}\begin{document}\jump{here}{Link}\end{document}".into(), 8);
    allowed(&workspace);
    workspace.open(b.clone(), r"\documentclass{article}\def\jump{\hyperlink}\begin{document}\input{shared}\jump{here}{Link}\end{document}".into(), 9);
    blocked(&workspace, &a, "here");
    workspace.documents.remove(&b);
    workspace.refresh_project(&a);
    allowed(&workspace);
    workspace.open(
        provider.clone(),
        r"\renewcommand\hyperlink[2]{#2}".into(),
        12,
    );
    blocked(&workspace, &a, "here");
    workspace.open(provider, "".into(), 13);
    allowed(&workspace);
    assert_eq!(workspace.documents[&a].version, Some(5));
}

#[test]
fn normal_macro_names_remain_editable_inside_destination_data() {
    let (_directory, workspace, uri) =
        setup(r"\newcommand\word{Text}\typeout{\unexpanded{\hypertarget{ghost}{\word}}}\word");
    let text = &workspace.documents[&uri].text;
    let at = text.position(text.source.rfind("word").unwrap() + 1);
    let changes = features::rename(&workspace, &uri, at, "term")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&uri].len(), 3);
}

fn apply_rename(workspace: &Workspace, uri: &Url, key: &str) -> String {
    let text = &workspace.documents[uri].text;
    let mut edits = features::rename(
        workspace,
        uri,
        text.position(text.source.find(key).unwrap() + 1),
        "fresh",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap()[uri]
        .clone();
    assert_eq!(edits.len(), 2);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    text.clone()
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

fn compile(directory: &std::path::Path, source: &str) -> (Vec<String>, String) {
    std::fs::write(
        directory.join("main.tex"),
        format!(r"\pdfcompresslevel=0\pdfobjcompresslevel=0{source}"),
    )
    .unwrap();
    for suffix in ["aux", "out"] {
        let path = directory.join(format!("main.{suffix}"));
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
    for _ in 0..2 {
        let output = std::process::Command::new("pdflatex")
            .args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ])
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let pdf = std::fs::read(directory.join("main.pdf")).unwrap();
    let pdf = String::from_utf8_lossy(&pdf);
    let names: Vec<_> = pdf
        .split("/Names")
        .skip(1)
        .filter_map(|part| {
            let part = part.trim_start().strip_prefix('[')?;
            Some(
                part.split_once(']')
                    .unwrap()
                    .0
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect();
    assert!(!names.is_empty(), "missing PDF destination table");
    let log = std::fs::read_to_string(directory.join("main.log")).unwrap();
    (names, log.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn native_message_destination_data_keeps_log_text_and_complete_pdf_names() {
    for reader in ["typeout", "message"] {
        let (_directory, workspace, uri) = setup(&format!(
            r"\{reader}{{LSP-OBS:\unexpanded{{\hypertarget{{ghost}}{{Text}}}}:END-OBS}}\hypertarget{{here}}{{Target}}\hyperlink{{here}}{{Link}}"
        ));
        let directory = _directory.path();
        let source = &workspace.documents[&uri].text.source;
        let observation = |log: &str| {
            log.split_once("LSP-OBS:")
                .unwrap()
                .1
                .split_once(":END-OBS")
                .unwrap()
                .0
                .to_owned()
        };
        let (before, log) = compile(directory, source);
        assert!(observation(&log).contains("ghost"));
        assert!(!before.join(" ").contains("(ghost)"));
        let (legacy, legacy_log) = compile(directory, &source.replace("ghost", "fresh"));
        assert_eq!(legacy, before);
        assert_eq!(
            observation(&legacy_log),
            observation(&log).replace("ghost", "fresh")
        );
        blocked(&workspace, &uri, "ghost");
        let (after, after_log) = compile(directory, &apply_rename(&workspace, &uri, "here"));
        assert_eq!(
            after.join(" ").replace("(fresh)", "(here)"),
            before.join(" ")
        );
        assert_eq!(observation(&after_log), observation(&log));
        assert!(!after_log.contains("has been referenced but does not exist"));
    }
}

#[test]
#[ignore = "requires pdflatex, hyperref and etoolbox"]
fn native_indirect_links_expose_legacy_partial_edits_and_reject_them() {
    for (preamble, call) in aliases().into_iter().take(4) {
        let (directory, workspace, uri) =
            setup(&format!(r"{preamble}\hypertarget{{here}}{{Target}}{call}"));
        let source = &workspace.documents[&uri].text.source;
        let (before, log) = compile(directory.path(), source);
        assert!(before.join(" ").contains("(here)"));
        assert!(!log.contains("has been referenced but does not exist"));
        let (legacy, legacy_log) =
            compile(directory.path(), &source.replacen("{here}", "{fresh}", 1));
        assert!(legacy.join(" ").contains("(fresh)"));
        assert!(
            legacy_log.contains("name{here} has been referenced but does not exist"),
            "{legacy_log}"
        );
        blocked(&workspace, &uri, "here");
        let (complete, complete_log) =
            compile(directory.path(), &source.replace("{here}", "{fresh}"));
        assert_eq!(
            complete.join(" ").replace("(fresh)", "(here)"),
            before.join(" ")
        );
        assert!(!complete_log.contains("has been referenced but does not exist"));
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn native_complete_direct_identity_and_deferred_edits_preserve_pdf_destinations() {
    for declaration in [
        r"\hypertarget{here}{Target}",
        r"\textbf{\hypertarget{here}{Target}}",
        r"\newcommand\one[1]{#1}\one{\hypertarget{here}{Target}}",
        r"\def\one#1;{#1}\one\hypertarget{here}{Target};",
        r"\newcommand\later{\hypertarget{here}{Target}}\later",
    ] {
        let (directory, workspace, uri) =
            setup(&format!(r"{declaration}\hyperlink{{here}}{{Link}}"));
        let (before, log) = compile(directory.path(), &workspace.documents[&uri].text.source);
        assert!(before.join(" ").contains("(here)"));
        assert!(!log.contains("has been referenced but does not exist"));
        let changed = apply_rename(&workspace, &uri, "here");
        let (after, after_log) = compile(directory.path(), &changed);
        assert_eq!(
            after.join(" ").replace("(fresh)", "(here)"),
            before.join(" ")
        );
        assert!(!after_log.contains("has been referenced but does not exist"));
    }
}
