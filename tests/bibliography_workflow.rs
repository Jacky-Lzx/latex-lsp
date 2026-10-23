#![cfg(unix)]
use latex_lsp::{
    build,
    config::{BibliographyBackend, BibliographyToolConfig, BuildConfig, Config, IndexToolConfig},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

const COMPILER: &str = r#"set -eu
output="$1"; job="$2"; source="$3"
printf x >> "$output/passes"
mkdir -p "$output/chapters"
printf '\\citation{key}\n\\bibdata{refs}\n\\bibstyle{custom}\n\\@input{chapters/chapter.aux}\n' > "$output/$job.aux"
printf '\\citation{child}\n' > "$output/chapters/chapter.aux"
printf 'bcf fixture\n' > "$output/$job.bcf"
printf 'index entry\n' > "$output/$job.idx"
printf '%%PDF-bibliography\n' > "$output/$job.pdf"
printf '(./main.tex\n' > "$output/$job.log"
if test ! -f "$output/$job.bbl"; then
  printf 'Package biblatex Warning: Please (re)run BibTeX on the file(s): main and rerun LaTeX afterwards.\n' >> "$output/$job.log"
fi
printf 'Output written on main.pdf (1 page, 18 bytes).\n)\n' >> "$output/$job.log"
printf 'PWD %s\nINPUT %s\n' "$PWD" "$source" > "$output/$job.fls"
"#;
const TOOL: &str = r#"set -eu
job=${1#./}; job=${job%.aux}
test -f "$job.aux"
test -f latex-lsp-aux/1.aux
printf 'bibliography entries\n' > "$job.bbl"
printf 'This is BibTeX, Version 0.99d\nThe top-level auxiliary file: %s\nThe style file: custom.bst\nDatabase file #1: refs.bib\n' "$1" > "$job.blg"
"#;
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    directory: tempfile::TempDir,
    root: PathBuf,
    config: BuildConfig,
}
impl Fixture {
    fn new(tool: &str) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("bibliography %j fixtures ")
            .tempdir()
            .unwrap();
        let root = directory.path().join("main.tex");
        std::fs::write(&root, "Source\n").unwrap();
        std::fs::write(
            directory.path().join("refs.bib"),
            "@book{key,title={One}}\n@book{key,title={Two}}\n",
        )
        .unwrap();
        std::fs::write(directory.path().join("custom.bst"), "style").unwrap();
        let compiler = directory.path().join("compiler.sh");
        write_executable(&compiler, COMPILER);
        let processor = directory.path().join("processor.sh");
        std::fs::write(&processor, tool).unwrap();
        let arguments = vec!["%o".into(), "%j".into(), "%f".into()];
        let config = BuildConfig {
            executable: compiler.to_string_lossy().into_owned(),
            arguments: Some(arguments.clone()),
            rebuild_arguments: Some(arguments),
            job_name: Some(".draft name".into()),
            output_directory: Some(".out %f directory".into()),
            bibliography_tool: Some(BibliographyToolConfig {
                executable: Some("/bin/sh".into()),
                arguments: vec![processor.to_string_lossy().into_owned()],
                dependencies: vec!["refs.bib".into(), "custom.bst".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        Self {
            directory,
            root,
            config,
        }
    }
    fn artifact(&self, extension: &str) -> PathBuf {
        self.config.artifact_path(&self.root, extension).unwrap()
    }
    fn output(&self) -> PathBuf {
        self.config.output_path(&self.root).unwrap()
    }
    fn run(&self) -> build::Outcome {
        build::run(&self.root, &self.config, &AtomicBool::new(false))
    }
    fn passes(&self) -> usize {
        std::fs::read(self.output().join("passes")).map_or(0, |bytes| bytes.len())
    }
    fn old_output(&self) {
        std::fs::create_dir_all(self.output()).unwrap();
        std::fs::write(self.artifact("bbl"), "old bibliography").unwrap();
    }
    fn no_temporary_files(&self) {
        assert!(!std::fs::read_dir(self.output()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("latex-lsp-bibliography-")));
    }
}

#[test]
fn bibtex_pipeline_stages_auxiliary_trees_preserves_names_and_reports_final_results() {
    let fixture = Fixture::new(TOOL);
    let result = fixture.run();
    assert_eq!(
        result.status, "success",
        "{}\n{}",
        result.message, result.output
    );
    assert_eq!(fixture.passes(), 3);
    assert_eq!(
        std::fs::read_to_string(fixture.artifact("bbl")).unwrap(),
        "bibliography entries\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.artifact("aux")).unwrap(),
        "\\citation{key}\n\\bibdata{refs}\n\\bibstyle{custom}\n\\@input{chapters/chapter.aux}\n"
    );
    let analysis = result.analysis.unwrap();
    assert_eq!(analysis["rerunRequired"], false);
    assert_eq!(analysis["bibliographyProcessingRounds"], 1);
    assert_eq!(analysis["processedBibliographyTool"]["backend"], "bibtex");
    assert_eq!(
        analysis["processedBibliographyTool"]["outputUri"],
        Url::from_file_path(fixture.artifact("bbl"))
            .unwrap()
            .as_str()
    );
    let sources = result.bibliography_inputs.unwrap();
    assert!(
        sources.contains(&Url::from_file_path(fixture.directory.path().join("refs.bib")).unwrap())
    );
    assert!(sources
        .contains(&Url::from_file_path(fixture.directory.path().join("custom.bst")).unwrap()));
    fixture.no_temporary_files();
}

#[test]
fn biber_receives_controlled_input_output_directories_and_root_search_paths() {
    let mut fixture = Fixture::new(
        r#"set -eu
test "$1" = --input-directory
test "$3" = --output-directory
test "$2" = "$4"
test "$5" = latex-lsp-bibliography
test -f "$2/$5.bcf"
test "$PWD" = "${BIBINPUTS%%:*}"
test -f "$PWD/refs.bib"
printf 'biber bibliography\n' > "$4/$5.bbl"
printf "INFO - This is Biber 2.20\nINFO - Found BibTeX data source 'refs.bib'\n" > "$4/$5.blg"
"#,
    );
    fixture.config.bibliography_tool.as_mut().unwrap().backend = BibliographyBackend::Biber;
    let result = fixture.run();
    assert_eq!(
        result.status, "success",
        "{}\n{}",
        result.message, result.output
    );
    assert_eq!(fixture.passes(), 3);
    assert_eq!(
        result.analysis.unwrap()["processedBibliographyTool"]["backend"],
        "biber"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.artifact("bbl")).unwrap(),
        "biber bibliography\n"
    );
    fixture.no_temporary_files();
}

#[test]
fn failures_missing_outputs_and_zero_exit_errors_preserve_previous_bibliography() {
    for fault in [
        "exit 7",
        "rm \"$job.bbl\"",
        "rm \"$job.blg\"",
        "printf 'Repeated entry---line 2 of file refs.bib\\n' >> \"$job.blg\"",
    ] {
        let fixture = Fixture::new(&format!("{TOOL}\n{fault}\n"));
        fixture.old_output();
        let result = fixture.run();
        assert_eq!(result.status, "error", "{fault}: {}", result.message);
        assert!(result.pdf.is_none());
        assert_eq!(fixture.passes(), 1);
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("bbl")).unwrap(),
            "old bibliography"
        );
        if fault.contains("Repeated entry") {
            let bib = Url::from_file_path(fixture.directory.path().join("refs.bib")).unwrap();
            assert!(result.diagnostics[&bib]
                .iter()
                .any(
                    |diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                        && diagnostic.range.start.line == 1
                ));
            assert!(result.bibliography_inputs.unwrap().contains(&bib));
        }
        fixture.no_temporary_files();
    }
}

#[test]
fn auxiliary_and_explicit_dependency_changes_do_not_install_generated_output() {
    for kind in ["aux", "dependency", "symlink"] {
        let mut fixture = Fixture::new(&format!("{TOOL}\ncase \"$2\" in\naux) printf changed >> \"$3\" ;;\ndependency) printf changed >> \"$4\" ;;\nsymlink) rm \"$4\"; ln -s \"$5\" \"$4\" ;;\nesac\n"));
        let replacement = fixture.directory.path().join("replacement.bib");
        std::fs::copy(fixture.directory.path().join("refs.bib"), &replacement).unwrap();
        let values = [
            kind.to_string(),
            fixture.artifact("aux").to_string_lossy().into_owned(),
            fixture
                .directory
                .path()
                .join("refs.bib")
                .to_string_lossy()
                .into_owned(),
            replacement.to_string_lossy().into_owned(),
        ];
        // The backend's fixed input operand follows these literal prefix arguments.
        let body = fixture.config.bibliography_tool.as_mut().unwrap();
        let script = body.arguments[0].clone();
        body.arguments = vec![script, "./latex-lsp-bibliography.aux".into()];
        body.arguments.extend(values);
        // TOOL consumes the supplied first operand; the server still appends its own.
        fixture.old_output();
        let result = fixture.run();
        assert_eq!(
            result.status, "error",
            "{kind}: {}\n{}",
            result.message, result.output
        );
        assert!(result.message.contains("changed"), "{}", result.message);
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("bbl")).unwrap(),
            "old bibliography"
        );
    }
}

#[test]
fn configuration_and_missing_dependencies_fail_before_the_compiler_runs() {
    assert!(Config::parse(serde_json::json!({"build":{"bibliographyTool":{}}})).is_ok());
    for settings in [
        serde_json::json!({"backend":"unknown"}),
        serde_json::json!({"timeoutSeconds":0}),
        serde_json::json!({"timeoutSeconds":3601}),
        serde_json::json!({"executable":" "}),
        serde_json::json!({"dependencies":[""]}),
    ] {
        assert!(Config::parse(serde_json::json!({"build":{"bibliographyTool":settings}})).is_err());
    }
    let mut fixture = Fixture::new(TOOL);
    fixture
        .config
        .bibliography_tool
        .as_mut()
        .unwrap()
        .dependencies
        .push("missing.bst".into());
    assert_eq!(fixture.run().status, "error");
    assert_eq!(fixture.passes(), 0);
    assert!(!fixture.output().exists());
    fixture
        .config
        .bibliography_tool
        .as_mut()
        .unwrap()
        .dependencies
        .pop();
    fixture.config.rebuild_arguments = None;
    assert_eq!(fixture.run().status, "error");
    assert_eq!(fixture.passes(), 0);
}

#[test]
fn cancellation_timeout_and_pre_cancel_preserve_outputs_and_clean_children() {
    for timeout in [false, true] {
        let mut fixture = Fixture::new("sleep 30 &\nwait\n");
        fixture
            .config
            .bibliography_tool
            .as_mut()
            .unwrap()
            .timeout_seconds = if timeout { 1 } else { 10 };
        fixture.old_output();
        let cancel = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            if !timeout {
                scope.spawn(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    cancel.store(true, Ordering::Relaxed);
                });
            }
            build::run(&fixture.root, &fixture.config, &cancel)
        });
        assert_eq!(result.status, if timeout { "timeout" } else { "cancelled" });
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("bbl")).unwrap(),
            "old bibliography"
        );
        fixture.no_temporary_files();
    }
    let fixture = Fixture::new(TOOL);
    assert_eq!(
        build::run(&fixture.root, &fixture.config, &AtomicBool::new(true)).status,
        "cancelled"
    );
    assert!(!fixture.output().exists());
}

#[test]
fn bibliography_and_index_stages_share_one_convergent_pipeline() {
    let mut fixture = Fixture::new(TOOL);
    let tool = fixture.directory.path().join("index.sh");
    write_executable(&tool, "cat \"$1\" > \"$2\"\n");
    fixture.config.index_tools = vec![IndexToolConfig {
        executable: tool.to_string_lossy().into_owned(),
        arguments: vec!["%i".into(), "%t".into()],
        ..Default::default()
    }];
    let result = fixture.run();
    assert_eq!(
        result.status, "success",
        "{}\n{}",
        result.message, result.output
    );
    let analysis = result.analysis.unwrap();
    assert_eq!(analysis["indexProcessingRounds"], 1);
    assert_eq!(analysis["bibliographyProcessingRounds"], 1);
    assert!(fixture.artifact("ind").is_file());
    assert!(fixture.artifact("bbl").is_file());
}

#[test]
fn changing_auxiliary_inputs_are_reprocessed_with_a_four_round_limit() {
    for converges in [false, true] {
        let fixture = Fixture::new(TOOL);
        let compiler = &fixture.config.executable;
        write_executable(Path::new(compiler), &format!("{COMPILER}\ncount=$(wc -c < \"$output/passes\")\nif test {converges} = false || test \"$count\" -lt 3; then printf '\\\\citation{{round-key-%s}}\\n' \"$count\" >> \"$output/$job.aux\"; fi\n"));
        let result = fixture.run();
        assert_eq!(
            result.status,
            if converges { "success" } else { "error" },
            "{}\n{}",
            result.message,
            result.output
        );
        if converges {
            assert_eq!(result.analysis.unwrap()["bibliographyProcessingRounds"], 2);
            assert_eq!(fixture.passes(), 5);
        } else {
            assert!(result.message.contains("four processing rounds"));
            assert!(result.pdf.is_none());
            assert_eq!(fixture.passes(), 9);
        }
    }
}

#[test]
fn irrelevant_auxiliary_changes_and_ignored_input_lines_do_not_repeat_the_backend() {
    let fixture = Fixture::new(TOOL);
    write_executable(
        Path::new(&fixture.config.executable),
        &format!(
            r#"{COMPILER}
count=$(wc -c < "$output/passes")
printf '%% timestamp %s\n\\newlabel{{dynamic}}{{{{%s}}{{1}}}}\n  \\@input{{missing.aux}}\n' "$count" "$count" >> "$output/$job.aux"
"#
        ),
    );
    let result = fixture.run();
    assert_eq!(
        result.status, "success",
        "{}\n{}",
        result.message, result.output
    );
    assert_eq!(fixture.passes(), 3);
    assert_eq!(result.analysis.unwrap()["bibliographyProcessingRounds"], 1);
}

#[test]
fn generated_database_changes_require_a_matching_bibliography_or_fail_to_converge() {
    for converges in [false, true] {
        let mut fixture = Fixture::new(
            r#"set -eu
output="$1"; shift
job=${1#./}; job=${job%.aux}
cat "$output/generated.bib" > "$job.bbl"
printf 'This is BibTeX, Version 0.99d\nThe top-level auxiliary file: %s\nDatabase file #1: generated.bib\n' "$1" > "$job.blg"
"#,
        );
        let output = fixture.output().to_string_lossy().into_owned();
        fixture
            .config
            .bibliography_tool
            .as_mut()
            .unwrap()
            .arguments
            .push(output);
        write_executable(
            Path::new(&fixture.config.executable),
            &format!(
                r#"{COMPILER}
count=$(wc -c < "$output/passes")
if test {converges} = true && test "$count" -ge 3; then count=stable; fi
printf 'data-%s\n' "$count" > "$output/generated.bib"
"#
            ),
        );
        let result = fixture.run();
        assert_eq!(
            result.status,
            if converges { "success" } else { "error" },
            "{}\n{}",
            result.message,
            result.output
        );
        if converges {
            assert_eq!(
                std::fs::read_to_string(fixture.artifact("bbl")).unwrap(),
                "data-stable\n"
            );
            assert_eq!(result.analysis.unwrap()["bibliographyProcessingRounds"], 2);
        } else {
            assert!(result.message.contains("four processing rounds"));
            assert!(result.pdf.is_none());
        }
    }
}

#[test]
fn symbolic_outputs_and_nonregular_tool_products_cannot_replace_bibliography() {
    for product in ["bbl", "blg"] {
        let fixture = Fixture::new(&format!(
            "{TOOL}\nrm \"$job.{product}\"\nmkfifo \"$job.{product}\"\n"
        ));
        fixture.old_output();
        assert_eq!(fixture.run().status, "error");
        assert_eq!(
            std::fs::read_to_string(fixture.artifact("bbl")).unwrap(),
            "old bibliography"
        );
    }
    let fixture = Fixture::new(TOOL);
    fixture.old_output();
    let target = fixture.directory.path().join("source-to-preserve");
    std::fs::write(&target, "original").unwrap();
    std::fs::remove_file(fixture.artifact("bbl")).unwrap();
    std::os::unix::fs::symlink(&target, fixture.artifact("bbl")).unwrap();
    assert_eq!(fixture.run().status, "error");
    assert_eq!(std::fs::read_to_string(target).unwrap(), "original");
}

fn real_document(root: &Path, entry: &str) {
    std::fs::create_dir_all(root.join("chapters")).unwrap();
    std::fs::write(root.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\include{chapters/one}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
    std::fs::write(
        root.join("chapters/one.tex"),
        "Text \\cite{key}.\\label{chapter}\n",
    )
    .unwrap();
    std::fs::write(root.join("refs.bib"), format!("@book{{key,author={{A. Author}},title={{{entry}}},year={{2026}},publisher={{Press}}}}\n")).unwrap();
}
#[test]
#[ignore = "requires pdflatex, bibtex and plain.bst"]
fn real_bibtex_handles_chapter_auxiliaries_hidden_output_directories_and_spaced_job_names() {
    let directory = tempfile::tempdir().unwrap();
    for output in ["build output", ".hidden"] {
        let config = BuildConfig {
            executable: "pdflatex".into(),
            output_directory: Some(output.into()),
            job_name: Some("draft name".into()),
            bibliography_tool: Some(BibliographyToolConfig::default()),
            ..Default::default()
        };
        for title in ["First bibliography", "Updated bibliography"] {
            real_document(directory.path(), title);
            let root = directory.path().join("main.tex");
            let result = build::run(&root, &config, &AtomicBool::new(false));
            assert_eq!(
                result.status, "success",
                "{output}/{title}: {}\n{}",
                result.message, result.output
            );
            assert_eq!(
                result.analysis.as_ref().unwrap()["bibliographyProcessingRounds"],
                1
            );
            let bbl = std::fs::read_to_string(config.artifact_path(&root, "bbl").unwrap()).unwrap();
            assert!(bbl.contains(title), "{bbl}");
            assert!(
                !result
                    .diagnostics
                    .values()
                    .flatten()
                    .any(|diagnostic| diagnostic.message.contains("undefined")),
                "{:?}",
                result.diagnostics
            );
            assert!(std::fs::read(config.artifact_path(&root, "pdf").unwrap())
                .unwrap()
                .starts_with(b"%PDF-"));
        }
    }
}

#[test]
#[ignore = "requires pdflatex, biber and biblatex"]
fn real_biber_resolves_root_resources_and_updates_a_biblatex_bibliography() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "\\documentclass{article}\n\\usepackage[backend=biber]{biblatex}\n\\addbibresource{refs.bib}\n\\begin{document}\nText \\cite{key}.\\printbibliography\n\\end{document}\n").unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        output_directory: Some("build output".into()),
        bibliography_tool: Some(BibliographyToolConfig {
            backend: BibliographyBackend::Biber,
            ..Default::default()
        }),
        ..Default::default()
    };
    for title in ["First entry", "Changed entry"] {
        std::fs::write(directory.path().join("refs.bib"), format!("@book{{key,author={{A. Author}},title={{{title}}},year={{2026}},publisher={{Press}}}}\n")).unwrap();
        let result = build::run(&root, &config, &AtomicBool::new(false));
        assert_eq!(
            result.status, "success",
            "{}\n{}",
            result.message, result.output
        );
        assert!(
            std::fs::read_to_string(config.artifact_path(&root, "bbl").unwrap())
                .unwrap()
                .contains(title)
        );
        assert!(
            !result
                .diagnostics
                .values()
                .flatten()
                .any(|diagnostic| diagnostic.message.contains("undefined")),
            "{:?}",
            result.diagnostics
        );
    }
}

#[test]
#[ignore = "requires pdflatex, bibtex and plain.bst"]
fn real_bibtex_can_read_databases_generated_by_tex_in_the_actual_output_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "\\documentclass{article}\n\\begin{filecontents*}[overwrite]{generated.bib}\n@book{key,author={A. Author},title={Generated bibliography},year={2026},publisher={Press}}\n\\end{filecontents*}\n\\begin{document}\nText \\cite{key}.\\bibliographystyle{plain}\\bibliography{generated}\n\\end{document}\n").unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        output_directory: Some("build output".into()),
        bibliography_tool: Some(BibliographyToolConfig::default()),
        ..Default::default()
    };
    let result = build::run(&root, &config, &AtomicBool::new(false));
    assert_eq!(
        result.status, "success",
        "{}\n{}",
        result.message, result.output
    );
    assert!(!directory.path().join("generated.bib").exists());
    assert!(config
        .output_path(&root)
        .unwrap()
        .join("generated.bib")
        .is_file());
    assert!(
        std::fs::read_to_string(config.artifact_path(&root, "bbl").unwrap())
            .unwrap()
            .contains("Generated bibliography")
    );
}
