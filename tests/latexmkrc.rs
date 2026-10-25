use latex_lsp::{
    compilation::{build, synctex},
    config::{BuildConfig, Config},
    project::workspace::Workspace,
};
use lsp_types::Url;
use std::sync::atomic::AtomicBool;

#[test]
fn project_rc_selection_matches_latexmk_and_is_refreshed_from_disk() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let workspace = Workspace::default();
    let output = || workspace.build_config(&uri).output_path(&main).unwrap();
    assert_eq!(output(), root.join("build/latex-lsp"));
    std::fs::write(root.join("latexmkrc"), "$out_dir = 'visible';").unwrap();
    assert_eq!(output(), root.join("visible"));
    std::fs::write(root.join(".latexmkrc"), "$out_dir = 'build';").unwrap();
    assert_eq!(output(), root.join("build"));
    std::fs::write(root.join(".latexmkrc"), "$out_dir = '输出 files';").unwrap();
    assert_eq!(output(), root.join("输出 files"));
    // latexmk reads only the first existing project rc, even if it does not
    // define an output directory. Do not borrow a value from the other file.
    std::fs::write(root.join(".latexmkrc"), "$pdf_mode = 1;").unwrap();
    assert_eq!(output(), root.join("build/latex-lsp"));
    std::fs::remove_file(root.join(".latexmkrc")).unwrap();
    assert_eq!(output(), root.join("visible"));
}

#[test]
fn explicit_output_settings_override_rc_including_the_previous_default() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(directory.path().join(".latexmkrc"), "$out_dir = 'build';").unwrap();
    for (value, expected) in [
        (serde_json::json!({}), "build"),
        (serde_json::json!({"outputDirectory":null}), "build"),
        (serde_json::json!({"outputDirectory":"custom"}), "custom"),
        (
            serde_json::json!({"outputDirectory":"build/latex-lsp"}),
            "build/latex-lsp",
        ),
    ] {
        let config = Config::parse(serde_json::json!({"latex":{"build":value}})).unwrap();
        assert_eq!(
            config.build.artifact_path(&main, "pdf").unwrap(),
            directory.path().join(expected).join("main.pdf")
        );
    }
    for value in ["", "  "] {
        assert!(Config::parse(serde_json::json!({"build":{"outputDirectory":value}})).is_err());
    }
}

#[test]
fn rc_is_read_beside_the_main_document_only_for_default_latexmk_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let sub = root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(root.join(".latexmkrc"), "$out_dir = 'parent';").unwrap();
    std::fs::write(sub.join("latexmkrc"), "$out_dir = '../output';").unwrap();
    let main = sub.join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let config = workspace.build_config(&uri);
    assert_eq!(config.output_path(&main).unwrap(), sub.join("../output"));
    assert!(config
        .expanded_arguments(&main)
        .unwrap()
        .contains(&"-bibfudge".into()));
    workspace.config.build.executable = "pdflatex".into();
    assert_eq!(
        workspace.build_config(&uri).output_path(&main).unwrap(),
        sub.join("build/latex-lsp")
    );
    workspace.config.build.executable = "latexmk".into();
    workspace.config.build.arguments = Some(vec!["-norc".into(), "%f".into()]);
    assert_eq!(
        workspace.build_config(&uri).output_path(&main).unwrap(),
        sub.join("build/latex-lsp")
    );
}

#[test]
fn unsupported_or_oversized_rc_settings_fall_back_without_running_perl() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let rc = directory.path().join(".latexmkrc");
    for source in [
        "$out_dir = $ENV{'BUILD_DIR'};".into(),
        "system('touch executed'); $out_dir = `echo build`;".into(),
        format!("{}\n$out_dir = 'build';", "# padding\n".repeat(7000)),
    ] {
        std::fs::write(&rc, source).unwrap();
        assert_eq!(
            BuildConfig::default().output_path(&main).unwrap(),
            directory.path().join("build/latex-lsp")
        );
        assert!(!directory.path().join("executed").exists());
    }
}

#[test]
#[cfg(unix)]
fn build_rebuild_logs_and_synctex_share_the_rc_output_directory() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "\\documentclass{article}\n\\include{chapters/part}\n";
    std::fs::create_dir(root.join("chapters")).unwrap();
    std::fs::write(root.join("chapters/part.tex"), "Chapter").unwrap();
    std::fs::write(&main, source).unwrap();
    std::fs::write(root.join(".latexmkrc"), "$out_dir = '输出 build';").unwrap();
    let compiler = root.join("latexmk");
    std::fs::write(
        &compiler,
        r##"#!/bin/sh
printf '%s\n' "$@" > arguments
for argument do
  case "$argument" in
    -outdir=*) output=${argument#-outdir=} ;;
  esac
done
test -d "$output/chapters" || exit 2
printf '%%PDF-1.4\n' > "$output/main.pdf"
printf 'Output written on main.pdf (1 page).\n' > "$output/main.log"
printf 'INPUT main.tex\nINPUT chapters/part.tex\n' > "$output/main.fls"
printf 'SyncTeX data\n' > "$output/main.synctex.gz"
"##,
    )
    .unwrap();
    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(uri.clone(), source.into(), 1);
    workspace.config.build.executable = compiler.to_string_lossy().into_owned();
    let config = workspace.build_config(&uri);
    for rebuild in [false, true] {
        let outcome = if rebuild {
            build::rebuild(&main, &config, &AtomicBool::new(false))
        } else {
            build::run(&main, &config, &AtomicBool::new(false))
        };
        assert_eq!(outcome.status, "success", "{}", outcome.message);
        assert_eq!(outcome.pdf, Some(root.join("输出 build/main.pdf")));
        assert_eq!(outcome.log, Some(root.join("输出 build/main.log")));
        assert_eq!(outcome.pages, Some(1));
        let arguments = std::fs::read_to_string(root.join("arguments")).unwrap();
        assert_eq!(arguments.lines().any(|line| line == "-g"), rebuild);
        let project = synctex::prepare(&workspace, &uri).unwrap();
        assert_eq!(project.pdf, root.join("输出 build/main.pdf"));
    }
    assert!(!root.join("build/latex-lsp").exists());
}

#[test]
#[ignore = "requires latexmk and pdfLaTeX"]
fn real_latexmk_uses_project_rc_output_and_preserves_explicit_overrides() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}Hello\\end{document}\n",
    )
    .unwrap();
    std::fs::write(root.join(".latexmkrc"), "$out_dir = 'build';").unwrap();
    for explicit in [None, Some("custom".into())] {
        let expected = explicit.as_deref().unwrap_or("build").to_owned();
        let config = BuildConfig {
            output_directory: explicit,
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{}\n{}",
            outcome.message, outcome.output
        );
        assert_eq!(outcome.pdf, Some(root.join(&expected).join("main.pdf")));
        for extension in ["log", "fls", "aux", "synctex.gz"] {
            assert!(root
                .join(&expected)
                .join(format!("main.{extension}"))
                .is_file());
        }
    }
    assert!(!root.join("build/latex-lsp").exists());
}
