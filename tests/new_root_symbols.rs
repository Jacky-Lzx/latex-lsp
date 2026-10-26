use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace, text::Text};

struct Project {
    _dir: tempfile::TempDir,
    ws: Workspace,
    common: Url,
    shared: Url,
    other: Url,
    source: String,
}
impl Project {
    fn new(declarations: &str, calls: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source=r"\documentclass{article}\usepackage{etoolbox}\begin{document}\input{common}\input{shared}\end{document}".to_string();
        for (name, content) in [
            ("main.tex", source.as_str()),
            ("common.tex", declarations),
            ("shared.tex", calls),
        ] {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
        let shared = common.join("shared.tex").unwrap();
        let other = common.join("new.tex").unwrap();
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.add_root(dir.path().into());
        Self {
            _dir: dir,
            ws,
            common,
            shared,
            other,
            source,
        }
    }
    fn position(&self) -> Position {
        let doc = &self.ws.documents[&self.common];
        doc.text.position(doc.text.source.find("chosen").unwrap())
    }
    fn open_other(&mut self, source: String) {
        self.ws.open(self.other.clone(), source, 7);
    }
    fn rename(&self) -> texquill::rpc::Result<Option<WorkspaceEdit>> {
        features::rename(&self.ws, &self.common, self.position(), "fresh")
    }
}
#[test]
fn new_root_environment_redefinitions_refuse_editing_shared_literal_arguments() {
    for (calls, override_) in [
        (
            r"\csuse{begin}{chosen}Text\csuse{end}{chosen}",
            r"\renewcommand\csuse[2]{}",
        ),
        (
            r"\begin{chosen}Text\end{chosen}",
            r"\renewcommand\begin[1]{}",
        ),
        (r"\begin{chosen}Text\end{chosen}", r"\renewcommand\end[1]{}"),
        (
            r"\csname begin\endcsname{chosen}Text\csname end\endcsname{chosen}",
            r"\def\csname{}",
        ),
    ] {
        let mut p = Project::new(r"\newenvironment{chosen}{}{}", calls);
        assert!(p.rename().unwrap().is_some());
        p.open_other(p.source.replace(
            r"\input{shared}",
            &format!(r"{{{override_}\input{{shared}}}}"),
        ));
        assert!(p.rename().is_err(), "{calls}: {override_}");
        assert!(features::prepare_rename(&p.ws, &p.common, p.position()).is_err());
    }
}
#[test]
fn new_root_label_redefinitions_refuse_editing_shared_plain_arguments() {
    let mut p = Project::new(r"\label{chosen}", r"\ref{chosen}");
    p.open_other(p.source.replace(
        r"\input{shared}",
        r"{\renewcommand\ref[1]{#1}\input{shared}}",
    ));
    assert!(p.rename().is_err());
}
#[test]
fn new_roots_with_different_or_missing_symbol_declarations_prevent_shared_edits() {
    for (declaration, calls) in [
        (
            r"\newenvironment{chosen}{}{}",
            r"\begin{chosen}Text\end{chosen}",
        ),
        (r"\newcommand\chosen{Text}", r"\chosen"),
        (r"\label{chosen}", r"\ref{chosen}"),
        (
            r"\newcounter{chosen}",
            r"\setcounter{chosen}{2}\value{chosen}",
        ),
    ] {
        let mut p = Project::new(declaration, calls);
        for replacement in ["", declaration] {
            p.open_other(p.source.replace(r"\input{common}", replacement));
            assert!(p.rename().is_err(), "{declaration}: {replacement}");
        }
    }
}
#[test]
fn compatible_and_independent_new_roots_keep_complete_environment_edits() {
    let mut p = Project::new(
        r"\newenvironment{chosen}{}{}",
        r"\csuse{begin}{chosen}Text\csuse{end}{chosen}",
    );
    for source in [
        p.source.clone(),
        r"\documentclass{article}\newenvironment{chosen}{}{}".into(),
    ] {
        p.open_other(source);
        let changes = p.rename().unwrap().unwrap().changes.unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[&p.common].len(), 1);
        assert_eq!(changes[&p.shared].len(), 2);
    }
    p.open_other(
        p.source
            .replace(r"\input{common}", r"\renewcommand\csuse[2]{}\input{common}"),
    );
    assert!(p.rename().is_err());
    p.open_other(p.source.clone());
    assert_eq!(
        p.rename().unwrap().unwrap().changes.unwrap()[&p.shared].len(),
        2
    );
}
#[test]
fn new_root_target_name_capture_is_checked_without_merging_unrelated_projects() {
    let mut p = Project::new(
        r"\newenvironment{chosen}{}{}",
        r"\begin{chosen}Text\end{chosen}",
    );
    p.open_other(p.source.replace(
        r"\input{shared}",
        r"\newenvironment{fresh}{}{}\input{shared}",
    ));
    assert!(p.rename().is_err());
    p.open_other(r"\documentclass{article}\newenvironment{fresh}{}{}".into());
    assert!(p.rename().unwrap().is_some());
}
#[test]
fn rootless_unsaved_buffers_keep_existing_workspace_symbol_completion() {
    let mut p = Project::new(r"\label{chosen}", r"\ref{chosen}");
    let scratch = Url::parse("untitled:scratch.tex").unwrap();
    let source = r"\ref{chos";
    p.ws.open(scratch.clone(), source.into(), 8);
    let items = features::completion(
        &p.ws,
        &scratch,
        Text::new(source.into()).position(source.len()),
    );
    assert!(items.iter().any(|item| item.label == "chosen"));
}

#[test]
#[ignore = "requires pdflatex with etoolbox"]
fn native_new_root_symbol_guards_preserve_plain_arguments_and_complete_environment_edits() {
    let declarations = r"\newenvironment{chosen}{\typeout{LSP-ENV:enter}}{\typeout{LSP-ENV:exit}}";
    let shared = r"\csuse{begin}{chosen}Text\csuse{end}{chosen}";
    let mut p = Project::new(declarations, shared);
    let unsafe_source = p.source.replace(
        r"\input{shared}",
        r"{\renewcommand\csuse[2]{\typeout{LSP-ARG:#2}}\input{shared}}",
    );
    p.open_other(unsafe_source.clone());
    assert!(p.rename().is_err());
    let compile =
        |label: &str,
         other: &str,
         changes: Option<&std::collections::HashMap<Url, Vec<TextEdit>>>| {
            let dir = p._dir.path().join(label);
            std::fs::create_dir(&dir).unwrap();
            for (name, content) in [
                ("main.tex", p.source.as_str()),
                ("new.tex", other),
                ("common.tex", declarations),
                ("shared.tex", shared),
            ] {
                let mut changed = content.to_string();
                let uri = p.common.join(name).unwrap();
                if let Some(edits) = changes.and_then(|changes| changes.get(&uri)) {
                    let text = Text::new(content.into());
                    let mut edits = edits.iter().collect::<Vec<_>>();
                    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                    for edit in edits {
                        changed.replace_range(
                            text.offset(edit.range.start).unwrap()
                                ..text.offset(edit.range.end).unwrap(),
                            &edit.new_text,
                        );
                    }
                }
                std::fs::write(dir.join(name), changed).unwrap();
            }
            ["main", "new"].map(|root| {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        &format!("{root}.tex"),
                    ])
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                let log = std::fs::read_to_string(dir.join(format!("{root}.log"))).unwrap();
                assert!(
                    !log.contains("undefined") && !log.contains("Warning"),
                    "{log}"
                );
                log.lines()
                    .filter(|line| line.starts_with("LSP-"))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        };
    let unsafe_observed = compile("unsafe", &unsafe_source, None);
    assert_eq!(unsafe_observed[1], ["LSP-ARG:chosen", "LSP-ARG:chosen"]);
    let before = compile("before", &p.source, None);
    p.open_other(p.source.clone());
    let changes = p.rename().unwrap().unwrap().changes.unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&p.shared].len(), 2);
    // Recreate the compiler closure after updating the workspace's open root.
    let dir = p._dir.path().join("after");
    std::fs::create_dir(&dir).unwrap();
    for (name, content) in [
        ("main.tex", p.source.as_str()),
        ("new.tex", p.source.as_str()),
        ("common.tex", declarations),
        ("shared.tex", shared),
    ] {
        let uri = p.common.join(name).unwrap();
        let text = Text::new(content.into());
        let mut changed = content.to_string();
        if let Some(edits) = changes.get(&uri) {
            let mut edits = edits.iter().collect::<Vec<_>>();
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            for edit in edits {
                changed.replace_range(
                    text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
                    &edit.new_text,
                );
            }
        }
        std::fs::write(dir.join(name), changed).unwrap();
    }
    for (index, root) in ["main", "new"].into_iter().enumerate() {
        let output = std::process::Command::new("pdflatex")
            .args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                &format!("{root}.tex"),
            ])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let log = std::fs::read_to_string(dir.join(format!("{root}.log"))).unwrap();
        assert!(
            !log.contains("undefined") && !log.contains("Warning"),
            "{log}"
        );
        let observed = log
            .lines()
            .filter(|line| line.starts_with("LSP-"))
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(observed, before[index]);
        assert_eq!(observed, ["LSP-ENV:enter", "LSP-ENV:exit"]);
    }
}
