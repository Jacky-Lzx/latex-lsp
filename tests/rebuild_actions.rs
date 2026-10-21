use latex_lsp::{build_log, editing, workspace::Workspace};
use tower_lsp::lsp_types::*;

const RERUN: &str =
    "LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.";

struct Project {
    directory: tempfile::TempDir,
    workspace: Workspace,
    root: Url,
    diagnostic: Diagnostic,
}

impl Project {
    fn new(literals: bool, apply_edits: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(
            &main,
            "\\documentclass{article}\n\\begin{document}Text.\\end{document}\n",
        )
        .unwrap();
        let root = Url::from_file_path(&main).unwrap();
        let mut workspace = Workspace {
            code_action_literals: literals,
            apply_edits,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        let diagnostics = build_log::parse(&main, &format!("(./main.tex\n{RERUN}\n)\n"));
        let diagnostic = diagnostics[&root][0].clone();
        workspace
            .build_diagnostics
            .insert(root.clone(), diagnostics);
        Self {
            directory,
            workspace,
            root,
            diagnostic,
        }
    }

    fn actions(&self, diagnostics: Vec<Diagnostic>) -> CodeActionResponse {
        editing::code_actions(
            &self.workspace,
            &self.root,
            self.diagnostic.range,
            &CodeActionContext {
                diagnostics,
                only: Some(vec![CodeActionKind::QUICKFIX]),
                trigger_kind: None,
            },
        )
    }
}

fn command(action: &CodeActionOrCommand) -> &Command {
    match action {
        CodeActionOrCommand::CodeAction(action) => {
            assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
            assert!(action.edit.is_none());
            assert_ne!(action.is_preferred, Some(true));
            action.command.as_ref().unwrap()
        }
        CodeActionOrCommand::Command(command) => command,
    }
}

#[test]
fn confirmed_rerun_warnings_offer_rebuild_without_requiring_workspace_edit_support() {
    for (literals, apply_edits) in [(true, true), (true, false), (false, true), (false, false)] {
        let project = Project::new(literals, apply_edits);
        for requested in [vec![], vec![project.diagnostic.clone()]] {
            let actions = project.actions(requested);
            assert_eq!(actions.len(), 1, "{literals}/{apply_edits}: {actions:?}");
            assert_eq!(
                matches!(actions[0], CodeActionOrCommand::CodeAction(_)),
                literals
            );
            let command = command(&actions[0]);
            assert_eq!(command.command, "latex.rebuild");
            assert_eq!(
                command.arguments,
                Some(vec![serde_json::json!({
                    "uri":project.root,
                    "rebuildGuard":{
                        "uri":project.root,
                        "version":project.workspace.documents[&project.root].version,
                        "diagnostics":[project.diagnostic],
                    },
                })])
            );
        }
    }
}

#[test]
fn rerun_actions_match_current_diagnostics_selection_and_requested_kind() {
    let mut project = Project::new(true, false);
    let mut different = project.diagnostic.clone();
    different.message = "LaTeX Warning: There were undefined references.".into();
    assert!(project.actions(vec![different]).is_empty());
    let mut foreign = project.diagnostic.clone();
    foreign.source = Some("other-server".into());
    assert!(project.actions(vec![foreign]).is_empty());
    let mut moved = project.diagnostic.clone();
    moved.range = Range::new(Position::new(1, 0), Position::new(1, 1));
    assert!(project.actions(vec![moved]).is_empty());
    for kind in [CodeActionKind::REFACTOR, CodeActionKind::SOURCE] {
        let actions = editing::code_actions(
            &project.workspace,
            &project.root,
            project.diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![],
                only: Some(vec![kind]),
                trigger_kind: None,
            },
        );
        assert!(actions.is_empty());
    }
    let actions = editing::code_actions(
        &project.workspace,
        &project.root,
        Range::new(Position::new(1, 0), Position::new(1, 1)),
        &CodeActionContext {
            diagnostics: vec![],
            only: None,
            trigger_kind: None,
        },
    );
    assert!(actions.is_empty());
    project.workspace.clear_build_diagnostics(&project.root);
    assert!(project.actions(vec![project.diagnostic.clone()]).is_empty());
}

#[test]
fn ordinary_warning_text_errors_and_other_servers_do_not_create_rebuild_actions() {
    let mut project = Project::new(true, false);
    for (message, severity, source) in [
        (RERUN, DiagnosticSeverity::ERROR, "latex-build"),
        (RERUN, DiagnosticSeverity::WARNING, "other-server"),
        (
            "Package local Warning: Example: Rerun to get /PageLabels entry.",
            DiagnosticSeverity::WARNING,
            "latex-build",
        ),
        (
            "LaTeX Warning: There were undefined references.",
            DiagnosticSeverity::WARNING,
            "latex-build",
        ),
    ] {
        let mut diagnostic = project.diagnostic.clone();
        diagnostic.message = message.into();
        diagnostic.severity = Some(severity);
        diagnostic.source = Some(source.into());
        project.workspace.build_diagnostics.insert(
            project.root.clone(),
            [(project.root.clone(), vec![diagnostic.clone()])].into(),
        );
        assert!(project.actions(vec![diagnostic]).is_empty(), "{message}");
    }
}

#[test]
fn repeated_warnings_group_by_the_build_root_even_in_a_shared_source() {
    let mut project = Project::new(true, false);
    let shared_path = project.directory.path().join("shared.tex");
    std::fs::write(&shared_path, "Shared text.\n").unwrap();
    let shared = Url::from_file_path(&shared_path).unwrap();
    let mut roots = vec![project.root.clone()];
    for name in ["main.tex", "other.tex"] {
        let path = project.directory.path().join(name);
        std::fs::write(
            &path,
            "\\documentclass{article}\\begin{document}\\input{shared}\\end{document}",
        )
        .unwrap();
        let root = Url::from_file_path(&path).unwrap();
        if !roots.contains(&root) {
            roots.push(root.clone());
        }
        let diagnostics = build_log::parse(
            &path,
            &format!("(./{name}\n(./shared.tex\n{RERUN}\n{RERUN}\n)\n)\n"),
        );
        project
            .workspace
            .build_diagnostics
            .insert(root, diagnostics);
    }
    for root in &roots {
        project.workspace.reload(root);
    }
    project.workspace.reload(&shared);
    assert!(project.workspace.root_document(&shared).is_err());
    let requested = project.workspace.build_diagnostics[&project.root][&shared].clone();
    let actions = editing::code_actions(
        &project.workspace,
        &shared,
        requested[0].range,
        &CodeActionContext {
            diagnostics: requested,
            only: None,
            trigger_kind: None,
        },
    );
    assert_eq!(actions.len(), 2, "{actions:?}");
    let mut selected: Vec<_> = actions
        .iter()
        .map(|action| command(action).arguments.clone().unwrap()[0]["uri"].clone())
        .collect();
    selected.sort_by_key(|uri| uri.to_string());
    roots.sort();
    assert_eq!(
        selected,
        roots
            .iter()
            .map(|root| serde_json::json!(root))
            .collect::<Vec<_>>()
    );
    assert_ne!(command(&actions[0]).title, command(&actions[1]).title);
}

#[test]
fn rerun_actions_do_not_offer_unavailable_or_redirected_builds() {
    let mut project = Project::new(true, false);
    project.workspace.config.build.executable = "custom-driver".into();
    project.workspace.config.build.arguments = Some(vec!["%f".into()]);
    assert!(project.actions(vec![]).is_empty());
    project.workspace.config.build.rebuild_arguments = Some(vec!["--force".into(), "%f".into()]);
    assert_eq!(project.actions(vec![]).len(), 1);
    let other = project.directory.path().join("other.tex");
    std::fs::write(&other, "\\documentclass{report}").unwrap();
    project.workspace.config.build.root_file = Some("other.tex".into());
    assert!(project.actions(vec![]).is_empty());
    project.workspace.config.build.root_file = None;
    std::fs::remove_file(project.root.to_file_path().unwrap()).unwrap();
    assert!(project.actions(vec![]).is_empty());
}

#[test]
fn biblatex_latex_only_requests_can_rebuild_with_direct_engines() {
    for executable in ["pdflatex", "xelatex", "lualatex", "tectonic", "latexmk"] {
        let mut project = Project::new(true, false);
        project.workspace.config.build.executable = executable.into();
        for message in [
            "Package biblatex Warning: Please rerun LaTeX.",
            "Package biblatex Warning: Please rerun LaTeX. (biblatex) Page breaks have changed.",
        ] {
            let mut diagnostic = project.diagnostic.clone();
            diagnostic.message = message.into();
            project.workspace.build_diagnostics.insert(
                project.root.clone(),
                [(project.root.clone(), vec![diagnostic.clone()])].into(),
            );
            assert_eq!(
                project.actions(vec![diagnostic]).len(),
                1,
                "{executable}: {message}"
            );
        }
    }
}

#[test]
fn bibliography_requests_offer_rebuild_only_for_the_known_latexmk_cycle() {
    for executable in [
        "latexmk",
        "/tools/latexmk",
        "pdflatex",
        "xelatex",
        "lualatex",
        "tectonic",
        "custom",
    ] {
        for custom_arguments in [false, true] {
            for custom_rebuild in [false, true] {
                for literals in [false, true] {
                    let mut project = Project::new(literals, false);
                    project.workspace.config.build.executable = executable.into();
                    project.workspace.config.build.arguments =
                        custom_arguments.then(|| vec!["%f".into()]);
                    project.workspace.config.build.rebuild_arguments =
                        custom_rebuild.then(|| vec!["%f".into()]);
                    for (tool, field) in [("BibTeX", "file(s):"), ("Biber", "file:")] {
                        let mut diagnostic = project.diagnostic.clone();
                        diagnostic.message = format!("Package biblatex Warning: Please (re)run {tool} on the {field} main and rerun LaTeX afterwards.");
                        project.workspace.build_diagnostics.insert(
                            project.root.clone(),
                            [(project.root.clone(), vec![diagnostic.clone()])].into(),
                        );
                        let actions = project.actions(vec![diagnostic]);
                        assert_eq!(actions.len(), usize::from(executable.ends_with("latexmk") && !custom_arguments && !custom_rebuild), "{executable}/{custom_arguments}/{custom_rebuild}/{literals}: {actions:?}");
                        if let Some(action) = actions.first() {
                            assert_eq!(command(action).command, "latex.rebuild");
                        }
                    }
                }
            }
        }
    }
}
