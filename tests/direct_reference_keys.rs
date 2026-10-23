use latex_lsp::{editing, features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    let uri = Url::parse("untitled:direct-keys.tex").unwrap();
    ws.open(uri.clone(), source.into(), 5);
    (ws, uri)
}

#[test]
fn redefined_reference_commands_leave_their_ordinary_arguments_unchanged() {
    for (declaration, command, args) in [
        (r"\label{paper}", "ref", "{paper}"),
        (r"\label{paper}", "crefrange", "{paper}{paper}"),
        (r"\label{paper}", "hyperref", "[paper]{paper}"),
        (r"\bibitem{paper}", "cite", "{paper}"),
        (r"\bibitem{paper}", "entrydata", "{paper}{paper}"),
        (
            r"\bibitem{paper}",
            "ifentrycategory",
            "{paper}{selected}{}{}",
        ),
    ] {
        let source = format!("😀 {declaration}\\def\\{command}#1{{}}\\{command}{args}");
        let (ws, uri) = setup(&source);
        let position = Text::new(source.clone()).position(source.rfind("paper").unwrap() + 2);
        assert!(
            features::definitions(&ws, &uri, position).is_empty(),
            "{source}"
        );
        assert!(
            features::prepare_rename(&ws, &uri, position)
                .unwrap()
                .is_none(),
            "{source}"
        );
        assert!(
            editing::highlights(&ws, &uri, position).is_empty(),
            "{source}"
        );
        assert!(
            !features::completion(&ws, &uri, position)
                .iter()
                .any(|item| item.label == "paper"),
            "{source}"
        );
        let position = Text::new(source.clone()).position(source.find("paper").unwrap() + 2);
        let edit = features::rename(&ws, &uri, position, "updated")
            .unwrap()
            .unwrap();
        assert_eq!(edit.changes.as_ref().unwrap()[&uri].len(), 1, "{source}");
    }
}

#[test]
fn reference_keys_in_parameters_and_operands_block_partial_renames() {
    for call in [
        r"\def\take#1{}\take{\ref{paper}}",
        r"\def\take#1{}\take{\cite{paper}}",
        r"\noexpand\ref{paper}",
        r"\noexpand\cite{paper}",
        r"\unknown{\crefrange{paper}{paper}}",
        r"\def\take#1{}\take{\ifentrycategory{paper}{selected}{}{}}",
    ] {
        let source = format!(r"\label{{paper}}\bibitem{{paper}}{call}");
        let (ws, uri) = setup(&source);
        let position = Text::new(source.clone()).position(source.rfind("paper").unwrap() + 2);
        assert!(
            features::definitions(&ws, &uri, position).is_empty(),
            "{source}"
        );
        assert!(
            !features::completion(&ws, &uri, position)
                .iter()
                .any(|item| item.label == "paper"),
            "{source}"
        );
        let declaration = if call.contains("cite") || call.contains("ifentrycategory") {
            source.find(r"\bibitem").unwrap() + 10
        } else {
            source.find("paper").unwrap() + 2
        };
        assert!(
            features::rename(
                &ws,
                &uri,
                Text::new(source.clone()).position(declaration),
                "updated"
            )
            .is_err(),
            "{source}"
        );
    }
}

#[test]
fn literal_examples_remain_untouched_and_complete_macro_bodies_keep_real_keys() {
    let source = r"\label{paper}\bibitem{paper}\newcommand\later{\ref{paper}\cite{paper}}\emph{\ref{paper}\cite{paper}}\detokenize{\ref{paper}\cite{paper}}\verb|\ref{paper}\cite{paper}|";
    let (ws, uri) = setup(source);
    for at in [
        source.find("paper").unwrap(),
        source.find(r"\bibitem").unwrap() + 9,
    ] {
        let position = Text::new(source.into()).position(at + 1);
        let changes = features::rename(&ws, &uri, position, "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes[&uri].len(), 3);
        assert_eq!(features::references(&ws, &uri, position, false).len(), 2);
    }
}

#[test]
fn stable_copied_argument_shapes_keep_following_references_editable() {
    for prefix in [
        r"\let\strong\textbf\strong{Text}",
        r"\let\strong\textbf\csuse{strong}{Text}",
        r"\newcommand\take[1]{}\let\saved\take\saved{Text}",
    ] {
        let source = format!(r"\label{{paper}}{prefix}\ref{{paper}}");
        let (ws, uri) = setup(&source);
        let position = Text::new(source.clone()).position(source.rfind("paper").unwrap());
        assert_eq!(
            features::definitions(&ws, &uri, position).len(),
            1,
            "{source}"
        );
        assert_eq!(
            features::rename(&ws, &uri, position, "updated")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            2,
            "{source}"
        );
    }
}

#[test]
fn copied_shapes_preserve_data_and_unresolved_macro_protection() {
    for prefix in [
        r"\newcommand\take[1]{}\let\saved\take\saved{\ref{paper}}",
        r"\let\saved\unknown\saved{Text}\ref{paper}",
        r"\let\saved\take\newcommand\take[1]{}\saved{Text}\ref{paper}",
        r"\let\saved\textbf\let\saved\unknown\saved{Text}\ref{paper}",
    ] {
        let source = format!(r"\label{{paper}}{prefix}");
        let (ws, uri) = setup(&source);
        let position = Text::new(source.clone()).position(source.rfind("paper").unwrap());
        assert!(
            features::definitions(&ws, &uri, position).is_empty(),
            "{source}"
        );
        assert!(
            features::rename(&ws, &uri, Position::new(0, 9), "updated").is_err(),
            "{source}"
        );
    }
}

#[test]
fn shared_direct_keys_require_consistent_command_meanings_and_refresh_unsaved_definitions() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\input{common}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\def\ref#1{}\input{common}\input{shared}",
        ),
        ("common.tex", r"\label{paper}"),
        ("shared.tex", r"\ref{paper}"),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    assert!(features::definitions(&ws, &shared, Position::new(0, 7)).is_empty());
    assert!(features::rename(&ws, &common, Position::new(0, 9), "updated").is_err());
    ws.open(
        b,
        r"\documentclass{article}\input{common}\input{shared}".into(),
        2,
    );
    assert_eq!(
        features::definitions(&ws, &shared, Position::new(0, 7)).len(),
        1
    );
    assert_eq!(
        features::rename(&ws, &common, Position::new(0, 9), "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn direct_reference_analysis_limits_do_not_return_partial_edits() {
    let source = format!(r"\label{{paper}}{}", r"\ref{paper}".repeat(20_001));
    let (mut ws, uri) = setup(&source);
    assert!(features::rename(&ws, &uri, Position::new(0, 9), "updated").is_err());
    ws.index_truncated.insert("maxFiles".into());
    assert!(features::rename(&ws, &uri, Position::new(0, 9), "updated").is_err());
}

#[test]
fn standard_layout_commands_and_item_labels_do_not_consume_following_references() {
    for prefix in [
        r"\item",
        r"\item[Label \ref{paper}]",
        r"\maketitle",
        r"\tableofcontents",
        r"\listoffigures",
        r"\listoftables",
        r"\appendix",
        r"\newpage",
        r"\clearpage",
        r"\cleardoublepage",
        r"\newblock",
        r"\centering",
        r"\raggedright",
        r"\raggedleft",
        r"\TeX",
        r"\LaTeX",
        r"\LaTeXe",
        r"\quad",
        r"\qquad",
        r"\enspace",
        r"\thinspace",
    ] {
        let source = format!(r"\label{{paper}}{prefix}\ref{{paper}}");
        let (ws, uri) = setup(&source);
        let position = Text::new(source.clone()).position(source.rfind("paper").unwrap() + 2);
        assert_eq!(
            features::definitions(&ws, &uri, position).len(),
            1,
            "{source}"
        );
        let count = source.matches("paper").count();
        assert_eq!(
            features::rename(&ws, &uri, position, "updated")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            count,
            "{source}"
        );
    }
    let source = r"\label{paper}\def\item#1{}\item\ref{paper}";
    let (ws, uri) = setup(source);
    assert!(features::definitions(
        &ws,
        &uri,
        Text::new(source.into()).position(source.rfind("paper").unwrap() + 2)
    )
    .is_empty());
    assert!(features::rename(&ws, &uri, Position::new(0, 9), "updated").is_err());
}

#[test]
#[ignore = "requires pdflatex"]
fn real_label_rename_preserves_redefined_reference_argument_output() {
    let source = r"\documentclass{article}\begin{document}Text\label{paper}\def\ref#1{\typeout{ORDINARY-ARGUMENT:#1}}\ref{paper}\end{document}";
    let (ws, uri) = setup(source);
    let position = Text::new(source.into()).position(source.find("paper").unwrap() + 2);
    let mut edits = features::rename(&ws, &uri, position, "updated")
        .unwrap()
        .unwrap()
        .changes
        .unwrap()
        .remove(&uri)
        .unwrap();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let updated = Text::new(source.into())
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
        .source;
    let dir = tempfile::tempdir().unwrap();
    for text in [source, &updated] {
        std::fs::write(dir.path().join("main.tex"), text).unwrap();
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
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(log.contains("ORDINARY-ARGUMENT:paper"), "{log}");
        assert!(!log.contains("ORDINARY-ARGUMENT:updated"), "{log}");
    }
    assert!(std::fs::read_to_string(dir.path().join("main.aux"))
        .unwrap()
        .contains(r"\newlabel{updated}"));
}
