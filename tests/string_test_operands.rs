use latex_lsp::{
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    for (name, source) in files {
        std::fs::write(directory.path().join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    (directory, workspace, main)
}

fn rewritten(workspace: &Workspace, edit: &WorkspaceEdit, uri: &Url) -> String {
    let mut edits = edit.changes.as_ref().unwrap()[uri].clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[uri]
        .text
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

#[test]
fn local_and_imported_parameter_grammars_keep_predicate_operands_from_reading_later_inputs() {
    for predicate in ["ifblank", "notblank", "ifstrempty", "ifstrequal"] {
        for imported in [false, true] {
            for (definition, call) in [
                (r"\newcommand{\discard}[1]{}", r"\discard"),
                (r"\newcommand{\discard}[2]{}", r"\discard{Unused}"),
                (r"\newcommand{\discard}[2][Default]{}", r"\discard[note]"),
                (
                    r"\NewDocumentCommand{\discard}{O{Default} m}{}",
                    r"\discard[note]",
                ),
                (r"\def\discard#1!{}", r"\discard"),
            ] {
                let declaration = if imported {
                    r"\input{defs}"
                } else {
                    definition
                };
                let delimiter = if definition.contains("#1!") { "!" } else { "" };
                let extra = if predicate == "ifstrequal" {
                    "{Never}"
                } else {
                    ""
                };
                let source = format!("\\documentclass{{article}}\\newbool{{draft}}{declaration}{call}\\{predicate}{delimiter}\\input{{after}}\\ifbool{{imported}}{{Y}}{{N}}\\{predicate}{{\\boolfalse{{draft}}}}{extra}{{Y}}{{N}}");
                let (directory, mut workspace, main) = setup(&[
                    ("main.tex", &source),
                    ("defs.tex", definition),
                    ("after.tex", r"\booltrue{draft}\newbool{imported}"),
                ]);
                workspace.open(main.clone(), source.clone(), 2);
                let after = Url::from_file_path(directory.path().join("after.tex")).unwrap();
                assert!(
                    workspace.project_documents(&main).contains(&after),
                    "{predicate}/{imported}/{definition}"
                );
                let at = Text::new(source.clone())
                    .position(source.find("ifbool{imported}").unwrap() + 8);
                assert_eq!(
                    features::definitions(&workspace, &main, at).len(),
                    1,
                    "{predicate}/{imported}/{definition}"
                );
                let at = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
                let edit = features::rename(&workspace, &main, at, "release")
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    rewritten(&workspace, &edit, &main),
                    source.replacen("newbool{draft}", "newbool{release}", 1),
                    "{predicate}/{imported}/{definition}"
                );
                assert_eq!(
                    rewritten(&workspace, &edit, &after),
                    r"\booltrue{release}\newbool{imported}"
                );
            }
        }
    }
}

#[test]
fn imported_zero_and_one_argument_wrappers_keep_each_shared_context_and_guard_differing_data() {
    let (directory, workspace, _) = setup(&[
        (
            "a.tex",
            r"\documentclass{article}\input{flags}\input{one}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\input{flags}\input{zero}\input{shared}",
        ),
        ("one.tex", r"\newcommand{\discard}[1]{}"),
        ("zero.tex", r"\newcommand{\discard}{}"),
        ("flags.tex", r"\newbool{draft}\newbool{other}"),
        (
            "shared.tex",
            r"\discard\ifblank{\booltrue{draft}}{Y}{N}\boolfalse{other}",
        ),
    ]);
    let flags = Url::from_file_path(directory.path().join("flags.tex")).unwrap();
    let text = &workspace.documents[&flags].text;
    assert!(features::rename(
        &workspace,
        &flags,
        text.position(text.source.find("draft").unwrap() + 1),
        "release"
    )
    .is_err());
    assert!(features::rename(
        &workspace,
        &flags,
        text.position(text.source.find("other").unwrap() + 1),
        "another"
    )
    .unwrap()
    .is_some());
}

#[test]
fn an_unknown_wrapper_does_not_prove_that_a_saved_predicate_is_an_operand() {
    let source = r"\documentclass{article}\newbool{draft}\let\saved\ifblank\unknown\saved{\booltrue{draft}\input{ghost}}{Y}{N}";
    let (directory, workspace, main) =
        setup(&[("main.tex", source), ("ghost.tex", r"\newbool{ghost}")]);
    let ghost = Url::from_file_path(directory.path().join("ghost.tex")).unwrap();
    assert!(!workspace.project_documents(&main).contains(&ghost));
    let at = Text::new(source.into()).position(source.rfind("draft").unwrap() + 1);
    assert!(features::definitions(&workspace, &main, at).is_empty());
}

#[test]
fn a_predicate_control_operand_uses_its_imported_wrappers_signature() {
    let source = r"\documentclass{article}\input{defs}\discard\ifblank";
    let (_directory, workspace, main) = setup(&[
        ("main.tex", source),
        ("defs.tex", r"\newcommand{\discard}[1]{}"),
    ]);
    let help = signature::help(
        &workspace,
        &main,
        Text::new(source.into()).position(source.len()),
    )
    .unwrap();
    assert_eq!(help.signatures[0].label, r"\discard{#1}");
    assert_eq!(help.active_parameter, Some(0));
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_imported_predicate_operands_preserve_macro_and_boolean_renames_in_three_engines() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for predicate in ["ifblank", "notblank", "ifstrempty", "ifstrequal"] {
            for (definition, call, delimiter) in [
                (r"\newcommand{\discard}[1]{}", r"\discard", ""),
                (
                    r"\newcommand{\discard}[2][Default]{}",
                    r"\discard[note]",
                    "",
                ),
                (
                    r"\NewDocumentCommand{\discard}{O{Default} m}{}",
                    r"\discard[note]",
                    "",
                ),
                (r"\def\discard#1!{}", r"\discard", "!"),
            ] {
                let extra = if predicate == "ifstrequal" {
                    "{Never}"
                } else {
                    ""
                };
                let (yes, no) = if predicate == "notblank" {
                    ("literal", "wrong")
                } else {
                    ("wrong", "literal")
                };
                let source = format!("\\documentclass{{article}}\\usepackage{{etoolbox}}\\newbool{{draft}}\\input{{defs}}\\begin{{document}}{call}\\{predicate}{delimiter}\\input{{after}}\\{predicate}{{\\boolfalse{{draft}}}}{extra}{{\\typeout{{OPERAND:{yes}}}}}{{\\typeout{{OPERAND:{no}}}}}\\ifbool{{draft}}{{\\typeout{{OPERAND:true}}}}{{\\typeout{{OPERAND:wrong}}}}Text.\\end{{document}}");
                let (directory, mut workspace, main) = setup(&[
                    ("main.tex", &source),
                    ("defs.tex", definition),
                    ("after.tex", r"\booltrue{draft}"),
                ]);
                let wrapper = directory.path().join("engine");
                std::fs::write(&wrapper, format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
                let config = BuildConfig {
                    executable: wrapper.to_string_lossy().into(),
                    arguments: Some(vec![
                        "-interaction=nonstopmode".into(),
                        "-no-shell-escape".into(),
                        "-file-line-error".into(),
                        "-recorder".into(),
                        "-output-directory=%o".into(),
                        "%f".into(),
                    ]),
                    timeout_seconds: 20,
                    ..Default::default()
                };
                let before =
                    build::run(&main.to_file_path().unwrap(), &config, &Default::default());
                assert_eq!(
                    before.status, "success",
                    "{engine}/{predicate}/{definition}: {}",
                    before.output
                );
                let at = Text::new(source.clone()).position(source.find("discard").unwrap() + 1);
                let edit = features::rename(&workspace, &main, at, "ignore")
                    .unwrap()
                    .unwrap();
                let renamed: Vec<_> = edit
                    .changes
                    .as_ref()
                    .unwrap()
                    .keys()
                    .map(|uri| (uri.clone(), rewritten(&workspace, &edit, uri)))
                    .collect();
                assert_eq!(renamed.len(), 2);
                for (uri, text) in renamed {
                    std::fs::write(uri.to_file_path().unwrap(), &text).unwrap();
                    workspace.open(uri, text, 2);
                }
                let text = &workspace.documents[&main].text;
                let at = text.position(text.source.find("draft").unwrap() + 1);
                let edit = features::rename(&workspace, &main, at, "release")
                    .unwrap()
                    .unwrap();
                assert!(rewritten(&workspace, &edit, &main).contains(r"\boolfalse{draft}"));
                let after_uri = Url::from_file_path(directory.path().join("after.tex")).unwrap();
                assert_eq!(
                    rewritten(&workspace, &edit, &after_uri),
                    r"\booltrue{release}"
                );
                for uri in edit.changes.as_ref().unwrap().keys() {
                    std::fs::write(
                        uri.to_file_path().unwrap(),
                        rewritten(&workspace, &edit, uri),
                    )
                    .unwrap();
                }
                let after = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{predicate}/{definition}: {}",
                    after.output
                );
                let states = |output: &str| {
                    output
                        .lines()
                        .filter(|line| line.starts_with("OPERAND:"))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    states(&before.output),
                    ["OPERAND:literal", "OPERAND:true"],
                    "{engine}/{predicate}/{definition}"
                );
                assert_eq!(states(&after.output), states(&before.output));
                assert_eq!(before.pages, Some(1));
                assert_eq!(after.pages, before.pages);
            }
        }
    }
}
