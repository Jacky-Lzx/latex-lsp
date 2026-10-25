use latex_lsp::{features, text::Text, workspace::Workspace};
use lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:patched-strings.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    (workspace, uri)
}

fn source(patch: &str) -> String {
    format!(
        r"\documentclass{{article}}\usepackage{{etoolbox}}\newcommand\word{{\typeout{{LSP-WORD}}}}{patch}\begin{{document}}\ifblank{{\word}}{{\word}}{{\word}}\label{{safe}}\ref{{safe}}\end{{document}}"
    )
}

fn patches() -> [(&'static str, usize); 11] {
    [
        (r"\pretocmd{\ifblank}{#1}{}{}", 2),
        (r"\pretocmd\ifblank{#1}{}{}", 2),
        (
            r"\patchcmd{\ifblank}{\expandafter\ifx}{#1\expandafter\ifx}{}{}",
            2,
        ),
        (
            r"\patchcmd[\long]{\ifblank}{\expandafter\ifx}{#1\expandafter\ifx}{}{}",
            2,
        ),
        (r"\csuse{pretocmd}{\ifblank}{#1}{}{}", 2),
        (r"\def\patcher{\pretocmd}\patcher{\ifblank}{#1}{}{}", 2),
        (r"\csname pretocmd\endcsname{\ifblank}{#1}{}{}", 2),
        (r"\let\patcher\pretocmd\patcher{\ifblank}{#1}{}{}", 2),
        (
            r"\def\other{}\def\identity#1{#1}\pretocmd{\other}{Text}{\let\ifblank\identity}{}",
            3,
        ),
        (
            r"\def\other{}\def\identity#1{#1}\appto{\other}{\let\ifblank\identity}\other",
            3,
        ),
        (
            r"\def\other{}\def\identity#1{#1}\let\savedblank\ifblank\cspreto{other}{\let\ifblank\identity}\let\ifblank\savedblank\other",
            3,
        ),
    ]
}

fn checked_macro_edits(workspace: &Workspace, uri: &Url, source: &str) -> Option<WorkspaceEdit> {
    let at = workspace.documents[uri]
        .text
        .position(source.rfind("word").unwrap() + 1);
    match features::rename(workspace, uri, at, "term") {
        Ok(Some(edit)) => {
            assert_eq!(edit.changes.as_ref().unwrap()[uri].len(), 4, "{source}");
            Some(edit)
        }
        _ => None,
    }
}

#[test]
fn macro_patches_never_return_partial_string_reader_edits() {
    for (patch, _) in patches() {
        let source = source(patch);
        let (workspace, uri) = setup(&source);
        checked_macro_edits(&workspace, &uri, &source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("safe").unwrap() + 1);
        assert_eq!(
            features::rename(&workspace, &uri, at, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            2
        );
    }
}

#[test]
fn macro_patch_targets_preserve_other_readers_and_explicit_restoration() {
    for patch in [
        r"\pretocmd{\other}{Text}{}{}",
        r"\let\savedblank\ifblank\pretocmd{\ifblank}{#1}{}{}\let\ifblank\savedblank",
        r"\newcommand\later{\pretocmd{\ifblank}{##1}{}{}}",
    ] {
        let source = source(patch);
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        assert_eq!(
            features::rename(&workspace, &uri, at, "term")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            3,
            "{source}"
        );
    }
}

#[test]
fn shared_provider_aliases_and_unsaved_patch_removal_refresh_string_meanings() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = main.join("patches.tex").unwrap();
    let declarations = r"\def\patcher{\pretocmd}";
    let source = source(r"\input{patches}\patcher{\ifblank}{#1}{}{}");
    std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), declarations).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), source.clone(), 5);
    checked_macro_edits(&workspace, &main, &source);
    workspace.open(provider.clone(), r"\def\patcher#1#2#3#4{}".into(), 12);
    let at = workspace.documents[&main]
        .text
        .position(source.rfind("word").unwrap() + 1);
    assert_eq!(
        features::rename(&workspace, &main, at, "term")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&main]
            .len(),
        3
    );
    workspace.open(provider, declarations.into(), 13);
    checked_macro_edits(&workspace, &main, &source);
}

#[test]
fn patched_string_copies_keep_captured_meanings_and_guard_mutated_destinations() {
    let source = source(r"\let\savedblank\ifblank\pretocmd{\ifblank}{#1}{}{}")
        .replace(r"\begin{document}\ifblank", r"\begin{document}\savedblank");
    let (workspace, uri) = setup(&source);
    let at = workspace.documents[&uri]
        .text
        .position(source.rfind("word").unwrap() + 1);
    assert_eq!(
        features::rename(&workspace, &uri, at, "term")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&uri]
            .len(),
        3
    );
    let source = source.replace(r"\pretocmd{\ifblank}", r"\pretocmd{\savedblank}");
    let (workspace, uri) = setup(&source);
    checked_macro_edits(&workspace, &uri, &source);
}

#[test]
fn patches_guard_every_native_string_reader_and_dynamic_targets() {
    for (reader, data, count) in [
        ("ifblank", r"{\word}", 4),
        ("notblank", r"{\word}", 4),
        ("ifstrempty", r"{\word}", 4),
        ("ifstrequal", r"{\word}{\word}", 5),
    ] {
        let source = format!(
            r"\documentclass{{article}}\usepackage{{etoolbox}}\newcommand\word{{Text}}\pretocmd{{\{reader}}}{{#1}}{{}}{{}}\begin{{document}}\{reader}{data}{{\word}}{{\word}}\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        if let Ok(Some(edit)) = features::rename(&workspace, &uri, at, "term") {
            assert_eq!(edit.changes.unwrap()[&uri].len(), count, "{reader}");
        }
    }
    for patch in [
        r"\pretocmd{\csname ifblank\endcsname}{#1}{}{}",
        r"\patchcmd[\prefix]{\other}{Old}{New}{}{}",
    ] {
        let source = source(patch);
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "term"), Ok(Some(_))),
            "{patch}"
        );
    }
}

#[test]
fn patch_signatures_keep_native_prefix_command_and_code_boundaries() {
    for (name, tail, suffix, active) in [
        (
            "pretocmd",
            r"\ifblank{Co¦de}{}{}",
            "{command}{code}{success}{failure}",
            1,
        ),
        (
            "apptocmd",
            r"{\ifblank}{Code}{}{Fa¦ilure}",
            "{command}{code}{success}{failure}",
            3,
        ),
        (
            "patchcmd",
            r"[\lo¦ng]{\ifblank}{Search}{Replacement}{}{}",
            "[prefix]{command}{search}{replacement}{success}{failure}",
            0,
        ),
        (
            "patchcmd",
            r"{\ifblank}{Search}{Repla¦cement}{}{}",
            "[prefix]{command}{search}{replacement}{success}{failure}",
            3,
        ),
        ("appto", r"\word{Co¦de}", "{command}{code}", 1),
        ("cspreto", r"{word}{Co¦de}", "{command}{code}", 1),
    ] {
        for call in [
            format!(r"\{name}"),
            format!(r"\csuse{{{name}}}"),
            format!(r"\csname {name}\endcsname"),
            r"\forward".into(),
        ] {
            let marked = format!("% 😀\r\n\\def\\forward{{\\{name}}}{call}{tail}");
            let (workspace, uri) = setup(&marked.replace('¦', ""));
            let text = &workspace.documents[&uri].text;
            let help = latex_lsp::signature::help(
                &workspace,
                &uri,
                text.position(marked.find('¦').unwrap()),
            )
            .unwrap_or_else(|| panic!("missing signature: {marked}"));
            assert_eq!(
                help.signatures[0].label,
                format!("{call}{suffix}"),
                "{marked}"
            );
            assert_eq!(help.active_parameter, Some(active), "{marked}");
        }
    }
    for call in [
        r"\pretocmd{\word}{Text}{}{}",
        r"\patchcmd[\long]{\word}{Old}{New}{}{}",
        r"\appto\word{Text}",
        r"\cspreto{word}{Text}",
    ] {
        let source = format!(r"\newcommand\word{{Text}}\label{{paper}}{call}\ref{{paper}}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        assert_eq!(
            features::definitions(
                &workspace,
                &uri,
                text.position(source.rfind("paper").unwrap() + 1)
            )
            .len(),
            1,
            "{call}"
        );
    }
    let source = r"\label{paper}\patchcmd{\word}{\ref{paper}}{Text}{}{}\ref{paper}";
    let (workspace, uri) = setup(source);
    let text = &workspace.documents[&uri].text;
    assert!(features::definitions(
        &workspace,
        &uri,
        text.position(source.find(r"\ref{paper}").unwrap() + 6)
    )
    .is_empty());
    assert_eq!(
        features::definitions(
            &workspace,
            &uri,
            text.position(source.rfind("paper").unwrap() + 1)
        )
        .len(),
        1
    );
}

fn apply(source: &str, uri: &Url, edit: WorkspaceEdit) -> String {
    let text = Text::new(source.into());
    let mut edits: Vec<_> = edit
        .changes
        .unwrap()
        .remove(uri)
        .unwrap()
        .into_iter()
        .map(|edit| {
            (
                text.offset(edit.range.start).unwrap(),
                text.offset(edit.range.end).unwrap(),
                edit.new_text,
            )
        })
        .collect();
    edits.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut changed = source.to_string();
    for (start, end, value) in edits {
        changed.replace_range(start..end, &value);
    }
    changed
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_macro_patch_edits_keep_actual_expansions_and_independent_aux() {
    for (patch, count) in patches() {
        let directory = tempfile::tempdir().unwrap();
        let source = source(patch);
        let (workspace, uri) = setup(&source);
        let compile = |source: &str| {
            std::fs::write(directory.path().join("main.tex"), source).unwrap();
            let aux = directory.path().join("main.aux");
            if aux.exists() {
                std::fs::remove_file(&aux).unwrap();
            }
            for _ in 0..2 {
                let result = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(directory.path())
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{patch}: {}",
                    String::from_utf8_lossy(&result.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert!(
                !log.contains("undefined") && !log.contains("Warning"),
                "{log}"
            );
            assert_eq!(log.matches("LSP-WORD").count(), count, "{patch}");
            std::fs::read_to_string(aux).unwrap()
        };
        let before = compile(&source);
        let changed = checked_macro_edits(&workspace, &uri, &source)
            .map_or_else(|| source.clone(), |edit| apply(&source, &uri, edit));
        assert_eq!(compile(&changed), before);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("safe").unwrap() + 1);
        let safe = apply(
            &source,
            &uri,
            features::rename(&workspace, &uri, at, "fresh")
                .unwrap()
                .unwrap(),
        );
        assert_eq!(compile(&safe).replace("{fresh}", "{safe}"), before);
    }
}
