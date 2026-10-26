use lsp_types::*;
use texquill::{
    compilation::build_log,
    config::{BibliographyBackend, BibliographyToolConfig, IndexToolConfig},
    editing,
    project::workspace::Workspace,
};

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
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: literals,
                apply_edits,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        let diagnostics = build_log::parse(&main, &format!("(./main.tex\n{RERUN}\n)\n"));
        let diagnostic = diagnostics[&root][0].clone();
        workspace
            .project
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
        project.workspace.project.build_diagnostics.insert(
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
            .project
            .build_diagnostics
            .insert(root, diagnostics);
    }
    for root in &roots {
        project.workspace.reload(root);
    }
    project.workspace.reload(&shared);
    assert!(project.workspace.root_document(&shared).is_err());
    let requested = project.workspace.project.build_diagnostics[&project.root][&shared].clone();
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
            project.workspace.project.build_diagnostics.insert(
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
                        project.workspace.project.build_diagnostics.insert(
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

#[test]
fn index_processing_requests_report_the_tool_without_offering_an_incomplete_rebuild() {
    for executable in [
        "latexmk", "pdflatex", "xelatex", "lualatex", "tectonic", "custom",
    ] {
        for kind in ["index", "glossary"] {
            let mut project = Project::new(true, false);
            project.workspace.config.build.executable = executable.into();
            project.workspace.config.build.rebuild_arguments = Some(vec!["%f".into()]);
            let mut diagnostic = project.diagnostic.clone();
            let header = if kind == "glossary" {
                "Package  rerunfilecheck Warning:"
            } else {
                "Package rerunfilecheck Warning:"
            };
            diagnostic.message = format!("{header} File `main.track' has changed. (rerunfilecheck) Rerun LaTeX/makeindex to get {kind} right.");
            let diagnostics = [(project.root.clone(), vec![diagnostic.clone()])].into();
            let analysis = build_log::compilation_analysis(&diagnostics, false);
            assert_eq!(
                analysis["requiredIndexTools"],
                serde_json::json!(["makeindex"])
            );
            assert_eq!(analysis["rerunRequired"], true);
            project
                .workspace
                .project
                .build_diagnostics
                .insert(project.root.clone(), diagnostics);
            assert!(
                project.actions(vec![diagnostic]).is_empty(),
                "{executable}/{kind}"
            );
        }
    }
}

fn index_warning(project: &mut Project, file: &str, kind: &str) {
    let log = format!("(./main.tex\nPackage rerunfilecheck Warning: File `{file}' has changed. (rerunfilecheck) Rerun LaTeX/makeindex to get {kind} right.\n)\n");
    let diagnostics = build_log::parse(&project.root.to_file_path().unwrap(), &log);
    project.diagnostic = diagnostics[&project.root][0].clone();
    project
        .workspace
        .project
        .build_diagnostics
        .insert(project.root.clone(), diagnostics);
}

#[test]
fn index_actions_match_configured_input_names_paths_and_processing_kind() {
    for literals in [false, true] {
        let mut project = Project::new(literals, false);
        project.workspace.config.build.index_tools = vec![IndexToolConfig::default()];
        let absolute = project.directory.path().join("build/texquill/main.idx");
        for (file, kind, allowed) in [
            ("main.idx", "index", true),
            ("./main.idx", "index", true),
            ("build/texquill/main.idx", "index", true),
            (absolute.to_str().unwrap(), "index", true),
            ("main.idx", "glossary", false),
            ("main.glo", "glossary", false),
            ("main.track", "index", false),
            ("other.idx", "index", false),
            ("elsewhere/main.idx", "index", false),
        ] {
            index_warning(&mut project, file, kind);
            let actions = project.actions(vec![project.diagnostic.clone()]);
            assert_eq!(
                actions.len(),
                usize::from(allowed),
                "{literals}/{file}/{kind}: {actions:?}"
            );
            if allowed {
                assert_eq!(command(&actions[0]).command, "latex.rebuild");
            }
        }
    }
}

#[test]
fn index_actions_follow_job_overrides_glossary_extensions_and_updated_configuration() {
    let mut project = Project::new(true, false);
    project.workspace.config.build.job_name = Some("draft  中文".into());
    project.workspace.config.build.index_tools = vec![IndexToolConfig::default()];
    index_warning(&mut project, "draft  中文.idx", "index");
    assert_eq!(project.actions(vec![]).len(), 1);
    // Filename whitespace is data, not warning-header normalization.
    index_warning(&mut project, "draft 中文.idx", "index");
    assert!(project.actions(vec![]).is_empty());
    project.workspace.config.build.index_tools[0].job_name = Some("people".into());
    index_warning(&mut project, "draft  中文.idx", "index");
    assert!(project.actions(vec![]).is_empty());
    index_warning(&mut project, "people.idx", "index");
    assert_eq!(project.actions(vec![]).len(), 1);
    for (input, output) in [("glo", "gls"), ("acn", "acr"), ("nlo", "nls")] {
        project.workspace.config.build.index_tools[0].input_extension = input.into();
        project.workspace.config.build.index_tools[0].output_extension = output.into();
        index_warning(&mut project, &format!("people.{input}"), "glossary");
        assert_eq!(project.actions(vec![]).len(), 1);
        index_warning(&mut project, &format!("people.{input}"), "index");
        assert!(project.actions(vec![]).is_empty());
    }
    index_warning(&mut project, "people.nlo", "glossary");
    project.workspace.config.build.index_tools.clear();
    assert!(project.actions(vec![]).is_empty());
}

#[test]
fn shared_source_index_actions_use_each_roots_own_task_configuration() {
    let mut project = Project::new(true, false);
    let other = project.directory.path().join("other.tex");
    let shared = project.directory.path().join("shared.tex");
    std::fs::write(&shared, "Shared text.\n").unwrap();
    for name in ["main.tex", "other.tex"] {
        let path = project.directory.path().join(name);
        std::fs::write(&path, "\\documentclass{article}\\input{shared}").unwrap();
        let diagnostics = build_log::parse(&path, &format!("(./{name}\n(./shared.tex\nPackage rerunfilecheck Warning: File `people.idx' has changed. Rerun LaTeX/makeindex to get index right.\n)\n)\n"));
        project
            .workspace
            .project
            .build_diagnostics
            .insert(Url::from_file_path(&path).unwrap(), diagnostics);
    }
    std::fs::write(project.directory.path().join(".texquill.json"), r#"{"projects":{"main.tex":{"build":{"indexTools":[{"jobName":"people"}]}},"other.tex":{"build":{"indexTools":[{}]}}}}"#).unwrap();
    project.workspace.reload(&project.root);
    project
        .workspace
        .reload(&Url::from_file_path(&other).unwrap());
    let shared_uri = Url::from_file_path(&shared).unwrap();
    project.workspace.reload(&shared_uri);
    let diagnostics =
        project.workspace.project.build_diagnostics[&project.root][&shared_uri].clone();
    let actions = editing::code_actions(
        &project.workspace,
        &shared_uri,
        diagnostics[0].range,
        &CodeActionContext {
            diagnostics,
            only: None,
            trigger_kind: None,
        },
    );
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(
        command(&actions[0]).arguments.as_ref().unwrap()[0]["uri"],
        serde_json::json!(project.root)
    );
}

#[test]
fn bibliography_actions_require_the_configured_backend_and_the_single_root_job() {
    for backend in [BibliographyBackend::Bibtex, BibliographyBackend::Biber] {
        let mut project = Project::new(true, false);
        project.workspace.config.build.executable = "pdflatex".into();
        project.workspace.config.build.bibliography_tool = Some(BibliographyToolConfig {
            backend,
            ..Default::default()
        });
        for (requested, field, expected) in [
            ("BibTeX", "file(s):", backend == BibliographyBackend::Bibtex),
            ("Biber", "file:", backend == BibliographyBackend::Biber),
        ] {
            for (job, matches) in [("main", true), ("other", false), ("main, other", false)] {
                let mut diagnostic = project.diagnostic.clone();
                diagnostic.message = format!("Package biblatex Warning: Please (re)run {requested} on the {field} {job} and rerun LaTeX afterwards.");
                project.workspace.project.build_diagnostics.insert(
                    project.root.clone(),
                    [(project.root.clone(), vec![diagnostic])].into(),
                );
                assert_eq!(
                    project.actions(vec![]).len(),
                    usize::from(expected && matches),
                    "{backend:?}/{job}/{requested}"
                );
            }
        }
    }
}
