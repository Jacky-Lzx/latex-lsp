use latex_lsp::{build, config::BuildConfig, workspace::Workspace};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

#[test]
#[cfg(unix)]
#[ignore = "requires latexmk, pdflatex, xelatex and lualatex"]
fn custom_job_names_locate_pdf_logs_recorder_inputs_and_synctex_for_all_tex_engines() {
    for executable in ["latexmk", "pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(
            &main,
            "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("chapter.tex"),
            "\\PackageWarning{example}{Job name warning}Chapter.",
        )
        .unwrap();
        let compiler = if executable == "lualatex" {
            let wrapper = script(root, "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"");
            let named = root.join("lualatex");
            std::fs::rename(wrapper, &named).unwrap();
            named.to_string_lossy().into_owned()
        } else {
            executable.into()
        };
        let config = BuildConfig {
            executable: compiler,
            job_name: Some("论文 draft.v2".into()),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{executable}: {}",
            outcome.output
        );
        assert_eq!(outcome.pdf, config.artifact_path(&main, "pdf"));
        assert_eq!(outcome.log, config.artifact_path(&main, "log"));
        let child = Url::from_file_path(root.join("chapter.tex")).unwrap();
        assert!(outcome.inputs.unwrap().contains(&child));
        assert!(
            outcome.diagnostics[&child]
                .iter()
                .any(|diagnostic| diagnostic.message.contains("Job name warning")),
            "{:?}",
            outcome.diagnostics
        );
        let mut workspace = Workspace::default();
        workspace.config.build = config;
        workspace.add_root(root.into());
        assert!(
            latex_lsp::synctex::prepare(&workspace, &Url::from_file_path(&main).unwrap()).is_ok(),
            "{executable}"
        );
        assert!(!root.join("build/latex-lsp/main.pdf").exists());
    }
}

#[test]
#[ignore = "requires latexmk and BibTeX"]
fn bibliography_builds_with_custom_job_names_still_resolve_and_analyze_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n").unwrap();
    std::fs::write(
        root.join("references.bib"),
        "@book{example, title={Example}, year={2026}, publisher={P}}\n",
    )
    .unwrap();
    let config = BuildConfig {
        job_name: Some("论文 draft.v2".into()),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert!(outcome.pdf.unwrap().is_file());
    let bib = Url::from_file_path(root.join("references.bib")).unwrap();
    assert!(
        outcome.diagnostics[&bib]
            .iter()
            .any(|diagnostic| diagnostic.message.contains("empty author")),
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        !outcome
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("undefined")),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn engine_directives_use_the_root_comment_and_respect_explicit_configuration() {
    use latex_lsp::{config::Engine, workspace::Document};
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    std::fs::write(
        &main,
        "% !TeX program = xelatex\n\\documentclass{article}\n",
    )
    .unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    assert_eq!(
        workspace.build_config(&uri).effective_engine(),
        Some(Engine::XeLatex)
    );
    workspace.config.build.engine = Some(Engine::LuaLatex);
    assert_eq!(
        workspace.build_config(&uri).effective_engine(),
        Some(Engine::LuaLatex)
    );
    workspace.config.build.engine = None;
    workspace.config.build.executable = "pdflatex".into();
    assert_eq!(
        workspace.build_config(&uri).effective_engine(),
        Some(Engine::PdfLatex)
    );
    workspace.config.build.executable = "latexmk".into();
    for (source, expected) in [
        ("% !TeX TS-program = \"LuaLaTeX\"\n\\documentclass{article}", Engine::LuaLatex),
        ("\\begin{verbatim}\n% !TeX program = xelatex\n\\end{verbatim}\n\\documentclass{article}", Engine::PdfLatex),
        ("% !TeX program = sh -c anything\n\\documentclass{article}", Engine::PdfLatex),
    ] {
        workspace.documents.insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
        assert_eq!(workspace.build_config(&uri).effective_engine(), Some(expected));
    }
}

#[test]
#[cfg(unix)]
fn unchanged_pdf_is_accepted_only_when_it_is_not_older_than_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let output = directory.path().join("build/latex-lsp");
    std::fs::create_dir_all(&output).unwrap();
    let pdf = output.join("main.pdf");
    std::fs::write(&main, "Original\n").unwrap();
    std::fs::write(&pdf, "%PDF-1.4\n").unwrap();
    let now = std::time::SystemTime::now();
    std::fs::File::open(&main)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(120)))
        .unwrap();
    std::fs::File::open(&pdf)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(60)))
        .unwrap();
    let config = BuildConfig {
        executable: "/bin/sh".into(),
        arguments: Some(vec!["-c".into(), "exit 0".into()]),
        ..Default::default()
    };
    let fresh = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(fresh.status, "success", "{}", fresh.message);
    assert!(!fresh.pdf_updated);
    std::fs::write(&main, "Changed before compilation\n").unwrap();
    let outdated = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outdated.status, "error");
    assert!(outdated.pdf.is_none());
    assert!(outdated.message.contains("outdated PDF"));
    assert!(outdated
        .diagnostics
        .values()
        .flatten()
        .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR)));
    assert!(
        pdf.is_file(),
        "the previous PDF should remain available on disk"
    );
}

#[test]
fn root_detection_follows_includes_magic_comments_and_explicit_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\input{chapter}",
    )
    .unwrap();
    std::fs::write(root.join("chapter.tex"), "Chapter").unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let chapter = Url::from_file_path(root.join("chapter.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert_eq!(workspace.root_document(&chapter).unwrap(), main);
    std::fs::write(
        root.join("other.tex"),
        r"\documentclass{article}\input{chapter}",
    )
    .unwrap();
    let other = Url::from_file_path(root.join("other.tex")).unwrap();
    workspace.reload(&other);
    assert!(workspace.root_document(&chapter).is_err());
    std::fs::write(root.join("chapter.tex"), "% !TeX root = main.tex\nChapter").unwrap();
    workspace.reload(&chapter);
    assert_eq!(workspace.root_document(&chapter).unwrap(), main);
    workspace.config.build.root_file = Some("other.tex".into());
    assert_eq!(workspace.root_document(&chapter).unwrap(), other);
    workspace.config.build.root_file = None;
    std::fs::write(root.join("main.tex"), "% !TeX root = chapter.tex\n").unwrap();
    workspace.reload(&main);
    assert!(workspace
        .root_document(&chapter)
        .unwrap_err()
        .contains("Cyclic"));
}

#[test]
fn root_magic_comments_inside_verbatim_are_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let other = directory.path().join("other.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{verbatim}\n% !TeX root = other.tex\n\\end{verbatim}\n",
    )
    .unwrap();
    std::fs::write(&other, "Other").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let uri = Url::from_file_path(&main).unwrap();
    assert_eq!(workspace.root_document(&uri).unwrap(), uri);
    assert!(latex_lsp::navigation::document_links(&workspace, &uri).is_empty());
}

#[test]
fn project_refresh_follows_updated_children_packages_and_root_relative_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("sections")).unwrap();
    for (name, source) in [
        (
            "main.tex",
            "\\documentclass{local}\\usepackage{macros}\\input{sections/child}",
        ),
        ("local.cls", "\\LoadClass{article}"),
        ("macros.sty", "\\RequirePackage{other}"),
        ("other.sty", "\\newcommand{\\hello}{Hello}"),
        ("sections/child.tex", "Chapter"),
        ("nested.v1.tex", "\\bibliography{refs\\_v1}"),
        ("refs_v1.bib", "@book{example,title={A}}"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    // Introduce a new include in a closed file after it has been indexed.
    std::fs::write(root.join("sections/child.tex"), "\\input{nested.v1}").unwrap();
    workspace.refresh_project(&main);
    let project = workspace.project_documents(&main);
    assert_eq!(project.len(), 7);
    for name in [
        "local.cls",
        "macros.sty",
        "other.sty",
        "nested.v1.tex",
        "refs_v1.bib",
    ] {
        let uri = Url::from_file_path(root.join(name)).unwrap();
        assert!(project.contains(&uri), "missing {name}");
        assert_eq!(workspace.root_document(&uri).unwrap(), main);
    }
}

#[test]
fn missing_compiler_produces_an_actionable_error_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, r"\documentclass{article}").unwrap();
    let config = BuildConfig {
        executable: directory
            .path()
            .join("missing-compiler")
            .to_string_lossy()
            .into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error");
    assert!(outcome.message.contains("Cannot start"));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(main).unwrap()][0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
}

#[cfg(unix)]
fn script(directory: &std::path::Path, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("fake compiler");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into()
}

#[test]
#[cfg(unix)]
fn bibliography_log_errors_are_reported_even_when_tex_produced_a_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("references.bib");
    std::fs::write(&main, "\\bibliography{references}\n").unwrap();
    std::fs::write(
        &bib,
        "@book{example, title={One}}\n@book{example, title={Two}}\n",
    )
    .unwrap();
    let executable = script(directory.path(), "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'Repeated entry---line 2 of file references.bib\\n' > \"$1/main.blg\"\n");
    let mut config = BuildConfig {
        executable,
        arguments: Some(vec!["%o".into()]),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error");
    assert!(outcome.pdf.is_none());
    assert!(outcome.message.contains("Repeated entry"));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(bib).unwrap()][0]
            .range
            .start
            .line,
        1
    );
    // A later compiler invocation that does not touch the old .blg must ignore it.
    config.executable = "/bin/sh".into();
    config.arguments = Some(vec!["-c".into(), "exit 0".into()]);
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success");
    assert!(outcome.diagnostics.is_empty());
}

#[test]
#[cfg(unix)]
fn arguments_are_passed_without_shell_interpolation_and_dotted_names_work() {
    let directory = tempfile::Builder::new()
        .prefix("latex lsp 中文 ")
        .tempdir()
        .unwrap();
    let main = directory.path().join("main.v1.tex");
    std::fs::write(&main, "source\nsource\n").unwrap();
    let executable = script(directory.path(), "[ \"$1\" = 'literal;$(false)' ] || exit 99\nprintf '%%PDF-1.4\n' > \"$2/main.v1.pdf\"\nprintf '%s:2: LaTeX Warning: example warning.\n' \"$3\"");
    let config = BuildConfig {
        executable,
        arguments: Some(vec!["literal;$(false)".into(), "%o".into(), "%f".into()]),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.unwrap().ends_with("main.v1.pdf"));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(main).unwrap()][0]
            .range
            .start
            .line,
        1
    );
    assert!(!directory.path().join("false").exists());
}

#[test]
#[cfg(unix)]
fn cancellation_and_timeout_terminate_the_compiler_process_group() {
    for cancelled in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "source").unwrap();
        let executable = script(
            directory.path(),
            "sleep 30 &\necho $! > \"$1/child.pid\"\nwait",
        );
        let config = BuildConfig {
            executable,
            arguments: Some(vec!["%o".into()]),
            timeout_seconds: 1,
            ..Default::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let started = Instant::now();
        let worker = std::thread::spawn(move || build::run(&main, &config, &worker_cancel));
        let pid_file = directory.path().join("build/latex-lsp/child.pid");
        while !pid_file.is_file() {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        if cancelled {
            cancel.store(true, Ordering::Relaxed);
        }
        let outcome = worker.join().unwrap();
        assert_eq!(
            outcome.status,
            if cancelled { "cancelled" } else { "timeout" }
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(pid_file).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            // SIGKILL delivery and the kernel's final process-state update are
            // asynchronous even after the parent has been reaped.
            while let Ok(state) = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
                if state
                    .split(')')
                    .nth(1)
                    .unwrap()
                    .trim_start()
                    .starts_with('Z')
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "compiler descendant was not terminated: {state}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_hyperref_urls_with_percent_escapes_compile_without_syntax_errors() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\documentclass{article}\n\\usepackage{hyperref}\n\\begin{document}\n\\url{https://example.com/a%20b?x=some_value}\n\\href{https://example.com/a%20b#section}{\\textbf{Link}}\n\\end{document}\n";
    std::fs::write(&main, source).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let document = latex_lsp::workspace::Document::new(&uri, source.into(), Some(1));
    assert!(latex_lsp::features::diagnostics(&document).is_empty());
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn real_bibliography_supports_configured_hidden_and_external_output_directories() {
    let directory = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main,"\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}").unwrap();
    std::fs::write(
        directory.path().join("references.bib"),
        "@book{example,author={A},title={Example},year={2025},publisher={P}}",
    )
    .unwrap();
    for output_directory in [
        ".custom-output".into(),
        external.path().to_string_lossy().into_owned(),
    ] {
        let config = BuildConfig {
            output_directory,
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{}\n{}",
            outcome.message, outcome.output
        );
        assert!(outcome.pdf.unwrap().is_file());
    }
}

#[test]
#[ignore = "requires XeLaTeX and a TeX installation"]
fn real_missing_glyphs_and_font_substitutions_are_warnings_even_with_a_valid_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\fontfamily{missingfont}\\selectfont\nHello \\char\"1F600.\n\\end{document}\n").unwrap();
    let config = BuildConfig {
        executable: "xelatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.unwrap().is_file());
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(&main).unwrap()];
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.starts_with("LaTeX Font Warning:")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(
            |diagnostic| diagnostic.message.starts_with("Missing character:")
                && diagnostic.message.contains("U+1F600")
        ),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING)),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn real_bibtex_failure_points_to_the_bib_file() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("references.bib");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n").unwrap();
    std::fs::write(&bib, "@book{example, author={A}, title={One}, year={2025}, publisher={P}}\n@book{example, title={Two}}\n").unwrap();
    let outcome = build::run(&main, &BuildConfig::default(), &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "error",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.is_none());
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(bib).unwrap()];
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 1
                && diagnostic.message.contains("Repeated entry")),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_latexmk_produces_pdf_and_uses_only_the_final_pass_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.v1.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\nSee~\\ref{sec:x}.\n\\end{document}\n").unwrap();
    std::fs::write(
        directory.path().join("chapter.tex"),
        r"\section{Hello}\label{sec:x}",
    )
    .unwrap();
    let outcome = build::run(&main, &BuildConfig::default(), &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.unwrap().is_file());
    assert!(outcome.log.unwrap().is_file());
    assert!(
        outcome
            .diagnostics
            .values()
            .flatten()
            .all(|diagnostic| !diagnostic.message.contains("undefined")),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
#[ignore = "requires pdflatex and a TeX installation"]
fn real_pdflatex_error_points_to_the_included_file_and_line() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let chapter = directory.path().join("chapter.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(&chapter, "Hello\n\\unknownLspCommand\n").unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error", "{}", outcome.output);
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(chapter).unwrap()];
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 1
                && diagnostic.message.contains("Undefined control sequence")),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_definition_file_warnings_with_spaces_and_parentheses_keep_their_source_context() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let definition = directory.path().join("settings (old).def");
    let options = directory.path().join("layout options.clo");
    let long_name = format!("settings {}(old).def", "long-name-".repeat(12));
    let long_definition = directory.path().join(&long_name);
    std::fs::write(&main,format!("\\documentclass{{article}}\n\\begin{{document}}\n\\input{{settings (old).def}}\n\\input{{layout options.clo}}\n\\PackageWarning{{audit}}{{Root warning}}\n\\input{{{long_name}}}\nText.\n\\end{{document}}\n")).unwrap();
    std::fs::write(&definition,"% configuration\n\\PackageWarning{audit}{Closing ) marker}\n\\PackageWarning{audit}{Still in definition}\n").unwrap();
    std::fs::write(
        &options,
        "% class options\n\\PackageWarning{audit}{Class options warning}\n",
    )
    .unwrap();
    std::fs::write(
        &long_definition,
        "\\PackageWarning{audit}{Wrapped filename warning}\n",
    )
    .unwrap();
    let outcome = build::run(&main, &Default::default(), &Default::default());
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    let definition_diagnostics = &outcome.diagnostics[&Url::from_file_path(&definition).unwrap()];
    assert_eq!(definition_diagnostics.len(), 2, "{:?}", outcome.diagnostics);
    assert!(definition_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.range.start.line == 2
            && diagnostic.message.contains("Still in definition")));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(&options).unwrap()][0]
            .range
            .start
            .line,
        1
    );
    assert!(outcome.diagnostics[&Url::from_file_path(&main).unwrap()]
        .iter()
        .any(|diagnostic| diagnostic.range.start.line == 4
            && diagnostic.message.contains("Root warning")));
    assert!(
        outcome.diagnostics[&Url::from_file_path(&long_definition).unwrap()][0]
            .message
            .contains("Wrapped filename warning")
    );
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_undefined_reference_and_citation_warnings_highlight_only_the_missing_keys() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let chapter = directory.path().join("chapter.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
    )
    .unwrap();
    let source = "See \\ref{missing} and \\cite{lost}.\n";
    std::fs::write(&chapter, source).unwrap();
    let outcome = build::run(&main, &Default::default(), &Default::default());
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(&chapter).unwrap()];
    let text = latex_lsp::text::Text::new(source.into());
    let values: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("undefined"))
        .map(|diagnostic| {
            &source[text.offset(diagnostic.range.start).unwrap()
                ..text.offset(diagnostic.range.end).unwrap()]
        })
        .collect();
    assert_eq!(values, vec!["missing", "lost"], "{:?}", outcome.diagnostics);
}
