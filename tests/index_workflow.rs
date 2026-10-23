#![cfg(unix)]
use latex_lsp::{
    build,
    config::{BuildConfig, Config, IndexToolConfig},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn script(directory: &Path, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_str().unwrap().into()
}
const COMPILER: &str = r#"
output="$1"; job="$2"; source="$3"
mkdir -p "$output"
printf x >> "$output/passes"
printf '\\indexentry{Apple}{1}\n' > "$output/$job.idx"
printf '\\glossaryentry{Banana}{1}\n' > "$output/$job.glo"
printf '%%PDF-new\n' > "$output/$job.pdf"
printf '(./main.tex\n' > "$output/$job.log"
if test "$(wc -c < "$output/passes")" -eq 1; then
    printf 'Package transient Warning: First pass only.\n' >> "$output/$job.log"
fi
printf 'Output written on %s.pdf (1 page, 9 bytes).\n)\n' "$job" >> "$output/$job.log"
printf 'PWD %s\nINPUT %s\n' "$output" "$source" > "$output/$job.fls"
"#;
struct Fixture {
    directory: tempfile::TempDir,
    root: PathBuf,
    config: BuildConfig,
}
impl Fixture {
    fn new(tool: &str) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("index %j fixtures ")
            .tempdir()
            .unwrap();
        let root = directory.path().join("main.tex");
        std::fs::write(&root, "Text").unwrap();
        let compiler = script(directory.path(), "compiler", COMPILER);
        let tool = script(directory.path(), "index-tool", tool);
        let arguments = vec!["%o".into(), "%j".into(), "%f".into()];
        let config = BuildConfig {
            executable: compiler,
            arguments: Some(arguments.clone()),
            rebuild_arguments: Some(arguments),
            job_name: Some("index draft".into()),
            output_directory: Some("out %f directory".into()),
            index_tools: vec![IndexToolConfig {
                executable: tool,
                arguments: vec!["-o".into(), "%t".into(), "%i".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        Self {
            directory,
            root,
            config,
        }
    }
    fn output(&self) -> PathBuf {
        self.config.output_path(&self.root).unwrap()
    }
    fn artifact(&self, extension: &str) -> PathBuf {
        self.config.artifact_path(&self.root, extension).unwrap()
    }
    fn old_index(&self) {
        std::fs::create_dir_all(self.output()).unwrap();
        std::fs::write(self.artifact("ind"), "old index").unwrap();
    }
    fn run(&self) -> build::Outcome {
        build::run(&self.root, &self.config, &AtomicBool::new(false))
    }
    fn passes(&self) -> usize {
        std::fs::read(self.output().join("passes")).map_or(0, |bytes| bytes.len())
    }
    fn no_staging_files(&self) {
        assert!(!std::fs::read_dir(self.output()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("latex-lsp-index-")));
    }
}

#[test]
fn configured_pipeline_processes_real_inputs_and_final_log_supersedes_initial_warnings() {
    let fixture = Fixture::new("test \"$1\" = -o\ncat \"$3\" > \"$2\"");
    fixture.old_index();
    let result = fixture.run();
    assert_eq!(
        result.status, "success",
        "{}\n{}",
        result.message, result.output
    );
    assert_eq!(fixture.passes(), 2);
    assert_eq!(
        std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
        "\\indexentry{Apple}{1}\n"
    );
    assert!(result
        .diagnostics
        .values()
        .flatten()
        .all(|item| !item.message.contains("First pass only")));
    assert_eq!(
        result.analysis.unwrap()["processedIndexTools"][0]["outputUri"],
        tower_lsp::lsp_types::Url::from_file_path(fixture.artifact("ind"))
            .unwrap()
            .as_str()
    );
    fixture.no_staging_files();
}

#[test]
fn regenerated_inputs_are_processed_until_stable_with_a_bounded_round_count() {
    for stable in [true, false] {
        let mut fixture = Fixture::new("cat \"$3\" > \"$2\"");
        let extra = if stable {
            "if test \"$(wc -c < \"$1/passes\")\" -gt 1; then printf 'updated entry' > \"$1/$2.idx\"; fi"
        } else {
            "cat \"$1/passes\" > \"$1/$2.idx\""
        };
        fixture.config.executable = script(
            fixture.directory.path(),
            "changing-compiler",
            &format!("{COMPILER}\n{extra}"),
        );
        let result = fixture.run();
        assert_eq!(
            result.status,
            if stable { "success" } else { "error" },
            "{}",
            result.message
        );
        assert_eq!(fixture.passes(), if stable { 3 } else { 5 });
        if stable {
            assert_eq!(result.analysis.unwrap()["indexProcessingRounds"], 2);
        } else {
            assert!(result.pdf.is_none());
            assert!(result.message.contains("stabilize"));
        }
    }
}

#[test]
fn multiple_explicit_tasks_and_named_indexes_use_independent_outputs() {
    let mut fixture = Fixture::new("cat \"$3\" > \"$2\"");
    let mut glossary = fixture.config.index_tools[0].clone();
    glossary.input_extension = "glo".into();
    glossary.output_extension = "gls".into();
    fixture.config.index_tools.push(glossary);
    let mut named = fixture.config.index_tools[0].clone();
    named.job_name = Some("people".into());
    fixture.config.index_tools.push(named);
    std::fs::create_dir_all(fixture.output()).unwrap();
    std::fs::write(fixture.output().join("people.idx"), "Named input").unwrap();
    let result = fixture.run();
    assert_eq!(result.status, "success", "{}", result.message);
    assert_eq!(
        result.analysis.unwrap()["processedIndexTools"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(fixture.artifact("gls").is_file());
    assert_eq!(
        std::fs::read_to_string(fixture.output().join("people.ind")).unwrap(),
        "Named input"
    );
}

#[test]
fn zero_exit_without_output_and_failed_tools_preserve_existing_index_and_skip_final_tex() {
    for tool in ["exit 0", "printf partial > \"$2\"\nexit 7"] {
        let fixture = Fixture::new(tool);
        fixture.old_index();
        let result = fixture.run();
        assert_eq!(result.status, "error", "{}", result.message);
        assert!(result.pdf.is_none());
        assert!(result.pages.is_none());
        assert!(result.analysis.is_none());
        assert_eq!(fixture.passes(), 1);
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
            "old index"
        );
        fixture.no_staging_files();
    }
}

#[test]
fn initial_compiler_errors_and_missing_index_inputs_do_not_run_tools() {
    for fault in [
        "exit 1",
        "printf 'main.tex:1: Undefined control sequence.\\n' >> \"$1/$2.log\"",
        "rm \"$1/$2.idx\"",
    ] {
        let mut fixture = Fixture::new("printf ran > ran\ncat \"$3\" > \"$2\"");
        fixture.config.executable = script(
            fixture.directory.path(),
            "faulty-compiler",
            &format!("{COMPILER}\n{fault}"),
        );
        fixture.old_index();
        let result = fixture.run();
        assert_eq!(result.status, "error", "{fault}: {}", result.message);
        assert!(!fixture.output().join("ran").exists());
        assert_eq!(fixture.passes(), 1);
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
            "old index"
        );
    }
}

#[test]
fn symbolic_output_links_are_rejected_without_changing_the_target() {
    let fixture = Fixture::new("cat \"$3\" > \"$2\"");
    std::fs::create_dir_all(fixture.output()).unwrap();
    let target = fixture.directory.path().join("keep.tex");
    std::fs::write(&target, "preserve source").unwrap();
    std::os::unix::fs::symlink(&target, fixture.artifact("ind")).unwrap();
    assert_eq!(fixture.run().status, "error");
    assert_eq!(std::fs::read_to_string(target).unwrap(), "preserve source");
}

#[test]
fn missing_styles_or_invalid_pipeline_are_rejected_before_compiling() {
    let mut fixture = Fixture::new("cat \"$3\" > \"$2\"");
    fixture.config.index_tools[0].dependencies = vec!["missing.ist".into()];
    assert_eq!(fixture.run().status, "error");
    assert_eq!(fixture.passes(), 0);
    assert!(!fixture.output().exists());
    fixture.config.index_tools[0].dependencies.clear();
    fixture.config.rebuild_arguments = None;
    assert_eq!(fixture.run().status, "error");
    assert!(!fixture.output().exists());
    fixture.config.rebuild_arguments = fixture.config.arguments.clone();
    fixture
        .config
        .index_tools
        .push(fixture.config.index_tools[0].clone());
    assert_eq!(fixture.run().status, "error");
    assert!(!fixture.output().exists());
}

#[test]
fn changed_input_and_style_dependencies_do_not_install_the_tool_output() {
    for style in [false, true] {
        let body = if style {
            "printf changed > \"$3\"\ncat \"$1\" > \"$2\""
        } else {
            "printf changed >> \"$1\"\ncat \"$1\" > \"$2\""
        };
        let mut fixture = Fixture::new(body);
        fixture.config.index_tools[0].arguments =
            vec!["%i".into(), "%t".into(), "%r/style.ist".into()];
        fixture.config.index_tools[0].dependencies = vec!["style.ist".into()];
        std::fs::write(fixture.directory.path().join("style.ist"), "style").unwrap();
        fixture.old_index();
        let result = fixture.run();
        assert_eq!(result.status, "error", "{}", result.message);
        assert!(result.message.contains("changed"));
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
            "old index"
        );
        assert_eq!(fixture.passes(), 1);
        fixture.no_staging_files();
    }
}

#[test]
fn cancellation_and_timeouts_kill_descendants_and_preserve_previous_outputs() {
    for timeout in [false, true] {
        let mut fixture =
            Fixture::new("printf ready > started\n(sleep 2; printf orphan > orphan) &\nwait");
        fixture.config.index_tools[0].timeout_seconds = if timeout { 1 } else { 10 };
        fixture.old_index();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let root = fixture.root.clone();
        let config = fixture.config.clone();
        let worker = std::thread::spawn(move || build::run(&root, &config, &worker_cancel));
        let started = Instant::now();
        while !fixture.output().join("started").exists()
            && started.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(fixture.output().join("started").exists());
        if !timeout {
            cancel.store(true, Ordering::Relaxed);
        }
        let result = worker.join().unwrap();
        assert_eq!(result.status, if timeout { "timeout" } else { "cancelled" });
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
            "old index"
        );
        assert_eq!(fixture.passes(), 1);
        fixture.no_staging_files();
        std::thread::sleep(Duration::from_millis(2100));
        assert!(!fixture.output().join("orphan").exists());
    }
}

#[test]
fn pre_cancelled_pipeline_does_not_create_outputs_or_start_tools() {
    let fixture = Fixture::new("exit 99");
    let result = build::run(&fixture.root, &fixture.config, &AtomicBool::new(true));
    assert_eq!(result.status, "cancelled");
    assert!(!fixture.output().exists());
}

#[test]
fn configuration_validates_tool_extensions_placeholders_limits_and_custom_rebuilds() {
    assert!(Config::parse(serde_json::json!({"build":{"indexTools":[{}]}})).is_ok());
    for tool in [
        serde_json::json!({"inputExtension":"../tex"}),
        serde_json::json!({"outputExtension":"tex"}),
        serde_json::json!({"arguments":["%i"]}),
        serde_json::json!({"arguments":["%t"]}),
        serde_json::json!({"timeoutSeconds":0}),
        serde_json::json!({"jobName":"../main"}),
        serde_json::json!({"executable":" "}),
        serde_json::json!({"dependencies":[""]}),
    ] {
        assert!(Config::parse(serde_json::json!({"build":{"indexTools":[tool]}})).is_err());
    }
    assert!(Config::parse(serde_json::json!({"build":{"indexTools":[{},{}]}})).is_err());
    assert!(
        Config::parse(serde_json::json!({"build":{"arguments":["%f"],"indexTools":[{}]}})).is_err()
    );
}

#[test]
#[ignore = "requires pdflatex, makeindex and makeidx"]
fn real_index_only_and_glossary_workflows_produce_updated_entries_and_final_pdfs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let mut config = BuildConfig {
        executable: "pdflatex".into(),
        index_tools: vec![IndexToolConfig::default()],
        ..Default::default()
    };
    std::fs::write(&root, "\\documentclass{article}\\usepackage{makeidx}\\makeindex\\begin{document}\\makeatletter\\immediate\\write\\@indexfile{\\string\\indexentry{Apple}{1}}\\makeatother\\printindex\\end{document}").unwrap();
    let result = build::run(&root, &config, &AtomicBool::new(false));
    assert_eq!(
        result.status, "success",
        "{}\n{}",
        result.message, result.output
    );
    assert!(config.artifact_path(&root, "ind").unwrap().is_file());
    assert!(result.pdf.unwrap().metadata().unwrap().len() > 0);
    std::fs::write(
        directory.path().join("glossary.ist"),
        "keyword \"\\\\glossaryentry\"\n",
    )
    .unwrap();
    config.index_tools.push(IndexToolConfig {
        input_extension: "glo".into(),
        output_extension: "gls".into(),
        arguments: vec![
            "-s".into(),
            "%r/glossary.ist".into(),
            "-t".into(),
            "%l".into(),
            "-o".into(),
            "%t".into(),
            "%i".into(),
        ],
        dependencies: vec!["glossary.ist".into()],
        ..Default::default()
    });
    for word in ["Banana", "Cherry"] {
        let source = format!("\\documentclass{{article}}\\usepackage{{makeidx}}\\makeindex\\makeglossary\\begin{{document}}Text\\index{{{word}}}\\glossary{{{word}}}\\printindex\\InputIfFileExists{{\\jobname.gls}}{{}}{{}}\\end{{document}}");
        std::fs::write(&root, source).unwrap();
        let result = build::run(&root, &config, &AtomicBool::new(false));
        assert_eq!(
            result.status, "success",
            "{}\n{}",
            result.message, result.output
        );
        for extension in ["ind", "gls"] {
            let contents =
                std::fs::read_to_string(config.artifact_path(&root, extension).unwrap()).unwrap();
            assert!(contents.contains(word), "{extension}: {contents}");
            if word == "Cherry" {
                assert!(!contents.contains("Banana"));
            }
        }
        assert_eq!(
            result.analysis.unwrap()["processedIndexTools"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}

#[test]
#[ignore = "requires makeindex"]
fn real_makeindex_rejected_entries_fail_even_with_a_zero_exit_code_and_quiet_output() {
    for quiet in [false, true] {
        let mut fixture = Fixture::new("exit 99");
        fixture.config.executable = script(
            fixture.directory.path(),
            "malformed-compiler",
            &format!("{COMPILER}\nprintf '\\\\indexentry{{Broken}}\\n' >> \"$1/$2.idx\""),
        );
        fixture.config.index_tools[0] = IndexToolConfig::default();
        if quiet {
            fixture.config.index_tools[0]
                .arguments
                .insert(0, "-q".into());
        }
        fixture.old_index();
        let result = fixture.run();
        assert_eq!(
            result.status, "error",
            "{}\n{}",
            result.message, result.output
        );
        assert!(result.message.contains("rejected 1"), "{}", result.message);
        assert!(
            result.output.contains("Missing arguments"),
            "{}",
            result.output
        );
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
            "old index"
        );
        assert_eq!(fixture.passes(), 1);
        fixture.no_staging_files();
    }
}

#[test]
fn failed_makeindex_processes_keep_new_transcripts_and_precise_input_diagnostics() {
    let mut fixture = Fixture::new("exit 99");
    fixture.config.index_tools[0] = IndexToolConfig {
        executable: script(
            fixture.directory.path(),
            "makeindex",
            r#"printf generated > "$2"
printf '!! Input index error (file = %s, line = 1):\n   -- Failed index entry.\ndone (0 entries accepted, 1 rejected).\n' "$5" > "$4"
exit 2
"#,
        ),
        ..Default::default()
    };
    fixture.old_index();
    let result = fixture.run();
    assert_eq!(result.status, "error");
    assert_eq!(result.exit_code, Some(2));
    assert_eq!(
        std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
        "old index"
    );
    let input = tower_lsp::lsp_types::Url::from_file_path(fixture.artifact("idx")).unwrap();
    assert_eq!(result.diagnostics[&input][0].range.start.line, 0);
    assert_eq!(
        result.diagnostics[&input][0].source.as_deref(),
        Some("makeindex")
    );
    assert!(std::fs::read_to_string(fixture.artifact("ilg"))
        .unwrap()
        .contains("Failed index entry"));
    fixture.no_staging_files();
}

#[test]
#[ignore = "requires makeindex"]
fn real_makeindex_style_errors_fail_at_zero_exit_and_conflicts_remain_located_warnings() {
    for style_error in [false, true] {
        let mut fixture = Fixture::new("exit 99");
        let style = fixture.directory.path().join("malformed.ist");
        fixture.config.index_tools[0] = IndexToolConfig::default();
        if style_error {
            std::fs::write(&style, "unknown_option \"x\"\n").unwrap();
            fixture.config.index_tools[0]
                .arguments
                .splice(0..0, ["-s".into(), "%r/malformed.ist".into()]);
            fixture.config.index_tools[0].dependencies = vec!["malformed.ist".into()];
        } else {
            fixture.config.executable = script(fixture.directory.path(), "conflicting-compiler", &format!("{COMPILER}\nprintf '\\\\indexentry{{Apple|textbf}}{{1}}\\n\\\\indexentry{{Apple|textit}}{{1}}\\n' >> \"$1/$2.idx\"\n"));
        }
        fixture.old_index();
        let result = fixture.run();
        assert_eq!(
            result.status,
            if style_error { "error" } else { "success" },
            "{}\n{}",
            result.message,
            result.output
        );
        let uri = tower_lsp::lsp_types::Url::from_file_path(if style_error {
            style
        } else {
            fixture.artifact("idx")
        })
        .unwrap();
        let diagnostics = &result.diagnostics[&uri];
        assert_eq!(
            diagnostics[0].severity,
            Some(if style_error {
                tower_lsp::lsp_types::DiagnosticSeverity::ERROR
            } else {
                tower_lsp::lsp_types::DiagnosticSeverity::WARNING
            })
        );
        if style_error {
            assert!(result.message.contains("Unknown specifier"));
            assert_eq!(
                std::fs::read_to_string(fixture.artifact("ind")).unwrap(),
                "old index"
            );
            assert_eq!(fixture.passes(), 1);
        } else {
            assert!(diagnostics[0].message.contains("Conflicting entries"));
            assert_eq!(fixture.passes(), 2);
        }
        assert!(fixture.artifact("ilg").is_file());
        fixture.no_staging_files();
    }
}
