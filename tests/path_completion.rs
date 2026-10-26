use lsp_types::*;
use texquill::{
    language::features,
    project::workspace::{Document, Workspace},
    text::Text,
};

fn apply_completion(workspace: &mut Workspace, uri: &Url, marked: &str, label: &str) -> String {
    let offset = marked.find('|').unwrap();
    let source = marked.replace('|', "");
    workspace
        .documents
        .insert(uri.clone(), Document::new(uri, source.clone(), Some(1)));
    let text = Text::new(source);
    let items = features::completion(workspace, uri, text.position(offset));
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("missing {label}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!();
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    assert!(edit.range.start <= text.position(offset) && text.position(offset) <= edit.range.end);
    let typed = &text.source[text.offset(edit.range.start).unwrap()..offset];
    assert!(
        item.filter_text.as_ref().unwrap().starts_with(typed),
        "{marked}: {item:?}"
    );
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for pair in edits.windows(2) {
        assert!(pair[1].range.end <= pair[0].range.start);
    }
    text.changed(
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
fn input_completions_keep_non_tex_extensions_and_links_do_not_infer_them() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "main.tex",
        "commands.def",
        "defaults.clo",
        "chapter.ltx",
        "local.sty",
        "bibliography.bbx",
        "citations.cbx",
        "language.lbx",
        "datamodel.dbx",
        "font.fd",
        "locale.ldf",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for (marked, label, expected) in [
        (
            r"\input{comm|ands}",
            "commands.def",
            r"\input{commands.def}",
        ),
        (
            r"\input{defa|ults}",
            "defaults.clo",
            r"\input{defaults.clo}",
        ),
        (r"\input{chap|ter}", "chapter.ltx", r"\input{chapter.ltx}"),
        (r"\input{lo|cal}", "local.sty", r"\input{local.sty}"),
        (
            r"\input{biblio|graphy}",
            "bibliography.bbx",
            r"\input{bibliography.bbx}",
        ),
        (
            r"\input{cita|tions}",
            "citations.cbx",
            r"\input{citations.cbx}",
        ),
        (
            r"\input{lang|uage}",
            "language.lbx",
            r"\input{language.lbx}",
        ),
        (
            r"\input{data|model}",
            "datamodel.dbx",
            r"\input{datamodel.dbx}",
        ),
        (
            r"\InputIfFileExists{biblio|graphy}{}{}",
            "bibliography.bbx",
            r"\InputIfFileExists{bibliography.bbx}{}{}",
        ),
        (r"\input{fo|nt}", "font.fd", r"\input{font.fd}"),
        (
            r"\InputIfFileExists{loc|ale}{}{}",
            "locale.ldf",
            r"\InputIfFileExists{locale.ldf}{}{}",
        ),
    ] {
        assert_eq!(
            apply_completion(&mut workspace, &uri, marked, label),
            expected
        );
    }
    let source = "\\input{commands}\n\\input{commands.def}\n\\input{chapter}\n\\input{chapter.ltx}\n\\input{bibliography}\n\\input{bibliography.bbx}\n\\input{citations}\n\\input{citations.cbx}\n\\input{language}\n\\input{language.lbx}\n\\input{datamodel}\n\\input{datamodel.dbx}\n\\input{font}\n\\input{font.fd}\n\\input{locale}\n\\input{locale.ldf}\n";
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let links = texquill::language::navigation::document_links(&workspace, &uri);
    assert_eq!(links.len(), 8);
    assert_eq!(links[0].range.start.line, 1);
    assert_eq!(links[1].range.start.line, 3);
    for (index, name) in [
        "bibliography.bbx",
        "citations.cbx",
        "language.lbx",
        "datamodel.dbx",
        "font.fd",
        "locale.ldf",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(links[index + 2].range.start.line, 5 + index as u32 * 2);
        assert_eq!(
            links[index + 2].target,
            Some(Url::from_file_path(root.join(name)).unwrap())
        );
    }
}

#[test]
fn support_source_input_completions_preserve_native_suffix_priority() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    std::fs::write(root.join("main.tex"), "").unwrap();
    let mut workspace = Workspace::default();
    for extension in ["bbx", "cbx", "lbx", "dbx", "fd", "ldf"] {
        let name = format!("local.{extension}");
        let shadow = format!("{name}.tex");
        let exact = matches!(extension, "fd" | "ldf");
        std::fs::write(root.join(&name), "Exact").unwrap();
        std::fs::write(root.join(&shadow), "Shadow").unwrap();
        for loader in ["input", "InputIfFileExists"] {
            let tail = if loader == "input" { "" } else { "{}{}" };
            let marked = format!("\\{loader}{{local.{extension}¦}}{tail}");
            let source = marked.replace('¦', "");
            workspace.open(main.clone(), source.clone(), 1);
            let at = Text::new(source).position(marked.find('¦').unwrap());
            let items = features::completion(&workspace, &main, at);
            assert_eq!(
                items.iter().any(|item| item.label == name),
                exact,
                "{marked}: {items:?}"
            );
            assert_eq!(
                items.iter().any(|item| item.label == shadow),
                !exact,
                "{marked}: {items:?}"
            );
            let marked = marked.replace('¦', "|");
            let chosen = if exact { &name } else { &shadow };
            let completed = apply_completion(&mut workspace, &main, &marked, chosen);
            workspace.open(main.clone(), completed, 2);
            assert_eq!(
                texquill::language::navigation::document_links(&workspace, &main)[0].target,
                Some(Url::from_file_path(root.join(chosen)).unwrap())
            );
        }
    }
}

#[test]
fn unicode_input_completions_group_bare_and_quoted_paths_and_keep_directory_suffixes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), "").unwrap();
    for name in ["新章节.tex", "新 章节.tex", "my file.tex"] {
        std::fs::write(root.join(name), "").unwrap();
    }
    std::fs::create_dir(root.join("目录")).unwrap();
    std::fs::write(root.join("目录/child.tex"), "").unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (marked, label, expected) in [
        ("\\input |", "新章节.tex", "\\input {新章节}"),
        ("\\input\"|\"", "新章节.tex", "\\input{新章节}"),
        ("\\input\"|", "新章节.tex", "\\input{新章节}"),
        ("\\input{|}", "新章节.tex", "\\input{新章节}"),
        ("\\input\"|\"", "新 章节.tex", "\\input{新 章节}"),
        ("\\input\"my|\"", "my file.tex", "\\input\"my file\""),
        ("\\input\"目|/child\"", "目录/", "\\input{目录/child}"),
        ("\\input 目|/child", "目录/", "\\input {目录/child}"),
    ] {
        assert_eq!(
            apply_completion(&mut workspace, &main, marked, label),
            expected,
            "{marked}"
        );
    }
}

#[test]
fn file_completions_replace_partial_names_preserve_lists_and_reuse_existing_slashes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("sections")).unwrap();
    for name in [
        "main.tex",
        "chapter.tex",
        "my file.tex",
        "references.bib",
        "local.sty",
        "custom.cls",
        "image.png",
        "sections/中文.tex",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    std::fs::create_dir(root.join("my dir")).unwrap();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for (marked, label, expected) in [
        (
            r"\input cha|pter trailing",
            "chapter.tex",
            r"\input chapter trailing",
        ),
        (r"\input my| next", "my file.tex", r"\input {my file}next"),
        (
            r#"\input"my f|ile" next"#,
            "my file.tex",
            r#"\input"my file" next"#,
        ),
        (
            r"\input my|/chapter next",
            "my dir/",
            r"\input {my dir/chapter}next",
        ),
        (r"\input |", "chapter.tex", r"\input chapter"),
        (r"\input{ cha|pter }", "chapter.tex", r"\input{ chapter }"),
        (r"\input{cha|pter}", "chapter.tex", r"\input{chapter}"),
        (
            r"\includeonly{other, cha|pter, last}",
            "chapter.tex",
            r"\includeonly{other, chapter, last}",
        ),
        (
            r"\input{cha|pter.tex}",
            "chapter.tex",
            r"\input{chapter.tex}",
        ),
        (r"\input{chapter.|}", "chapter.tex", r"\input{chapter.tex}"),
        (
            r"\input{se|ctions/chapter}",
            "sections/",
            r"\input{sections/chapter}",
        ),
        (
            r"😀 \input{sections/中|旧}",
            "sections/中文.tex",
            r"😀 \input{sections/中文}",
        ),
        (r"\input{my f|ile}", "my file.tex", r"\input{my file}"),
        (
            r"\bibliography{other, re|ferences}",
            "references.bib",
            r"\bibliography{other, references}",
        ),
        (
            r"\addbibresource{re|f}",
            "references.bib",
            r"\addbibresource{references.bib}",
        ),
        (
            r"\includegraphics[width=2cm]{im|age}",
            "image.png",
            r"\includegraphics[width=2cm]{image.png}",
        ),
        (
            r"\includegraphics[width=0.5\linewidth]{im|BAD}",
            "image.png",
            r"\includegraphics[width=0.5\linewidth]{image.png}",
        ),
        (
            r"\addbibresource[label={\emph{References}}]{re|BAD}",
            "references.bib",
            r"\addbibresource[label={\emph{References}}]{references.bib}",
        ),
        (
            r"\usepackage{other, lo|cal}",
            "local.sty",
            r"\usepackage{other, local}",
        ),
        (
            r"\documentclass{cu|stom}",
            "custom.cls",
            r"\documentclass{custom}",
        ),
    ] {
        assert_eq!(
            apply_completion(&mut workspace, &uri, marked, label),
            expected
        );
    }
}

#[test]
fn completions_find_project_root_files_parent_paths_and_prefer_build_root_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir_all(root.join("sections")).unwrap();
    for (name, source) in [
        (
            "main.tex",
            "\\documentclass{article}\\input{sections/child}",
        ),
        ("sections/child.tex", ""),
        ("image.pdf", "pdf"),
        ("local.tex", "root"),
        ("sections/local.tex", "near"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    std::fs::write(directory.path().join("outside.tex"), "outside").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.clone());
    let child = Url::from_file_path(root.join("sections/child.tex")).unwrap();
    assert_eq!(
        apply_completion(
            &mut workspace,
            &child,
            r"\includegraphics{im|}",
            "image.pdf"
        ),
        r"\includegraphics{image.pdf}"
    );
    let marked = r"\input{lo|}";
    apply_completion(&mut workspace, &child, marked, "local.tex");
    let items = features::completion(&workspace, &child, Position::new(0, 9));
    let locals: Vec<_> = items
        .iter()
        .filter(|item| item.label == "local.tex")
        .collect();
    assert_eq!(locals.len(), 1);
    assert_eq!(locals[0].detail.as_deref(), root.join("local.tex").to_str());
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    assert_eq!(
        apply_completion(&mut workspace, &main, r"\input{../ou|}", "../outside.tex"),
        r"\input{../outside}"
    );
}

#[test]
fn input_completion_omits_shadowed_files_and_deduplicates_identical_insertions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("sections")).unwrap();
    for (name, source) in [
        (
            "main.tex",
            "\\documentclass{article}\\input{sections/child}",
        ),
        ("chapter.ltx", "Shadowed."),
        ("chapter.ltx.tex", "Selected."),
        ("sections/child.tex", ""),
        (
            "sections/chapter.ltx",
            "Shadowed from the child directory too.",
        ),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let child = Url::from_file_path(root.join("sections/child.tex")).unwrap();
    assert_eq!(
        apply_completion(&mut workspace, &child, "\\input{chap|}", "chapter.ltx.tex"),
        "\\input{chapter.ltx}"
    );
    let source = "\\input{chap";
    workspace
        .documents
        .insert(child.clone(), Document::new(&child, source.into(), Some(2)));
    let items = features::completion(
        &workspace,
        &child,
        Text::new(source.into()).position(source.len()),
    );
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].label, "chapter.ltx.tex");
    assert_eq!(
        items[0].detail.as_deref(),
        root.join("chapter.ltx.tex").to_str()
    );
}

#[test]
fn file_completions_filter_types_hidden_files_and_ignored_regions_and_bound_results() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["main.tex", "chapter.tex", "wrong.png", ".hidden.tex"] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    let source = r"\input{";
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let items = features::completion(
        &workspace,
        &uri,
        Text::new(source.into()).position(source.len()),
    );
    assert!(items.iter().any(|item| item.label == "chapter.tex"));
    assert!(!items
        .iter()
        .any(|item| item.label == "wrong.png" || item.label == ".hidden.tex"));
    assert_eq!(
        apply_completion(&mut workspace, &uri, r"\input{.h|}", ".hidden.tex"),
        r"\input{.hidden}"
    );
    for source in [r"% \input{ch", "\\begin{verbatim}\n\\input{ch"] {
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
        assert!(features::completion(
            &workspace,
            &uri,
            Text::new(source.into()).position(source.len())
        )
        .is_empty());
    }
    std::fs::create_dir(root.join("many")).unwrap();
    for n in 0..270 {
        std::fs::write(root.join(format!("many/file{n}.tex")), "").unwrap();
    }
    let source = r"\input{many/";
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    assert_eq!(
        features::completion(
            &workspace,
            &uri,
            Text::new(source.into()).position(source.len())
        )
        .len(),
        // Keep one extra candidate for the protocol's incomplete-list response.
        workspace.config.indexing.completion_limit + 1
    );
}

#[test]
fn package_and_database_completions_match_compacted_names_and_replace_raw_utf16_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("mydir")).unwrap();
    for ext in ["sty", "cls", "bib"] {
        std::fs::write(root.join(format!("mydir/文献.{ext}")), "Source.").unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for (command, ext) in [
        ("usepackage", "sty"),
        ("RequirePackage", "sty"),
        ("RequirePackageWithOptions", "sty"),
        ("bibliography", "bib"),
    ] {
        for whitespace in [" ", "\t", "\n", "\r\n"] {
            let marked =
                format!("😀 \\{command}{{my{whitespace}dir/文{whitespace}|旧}}[2020/01/01]");
            let expected = format!("😀 \\{command}{{mydir/文献}}[2020/01/01]");
            let completed =
                apply_completion(&mut workspace, &uri, &marked, &format!("mydir/文献.{ext}"));
            assert_eq!(completed, expected, "{marked:?}");
            let offset = marked.find('|').unwrap();
            let source = Text::new(marked.replace('|', ""));
            let items = features::completion(&workspace, &uri, source.position(offset));
            let item = items
                .iter()
                .find(|item| item.label == format!("mydir/文献.{ext}"))
                .unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            let typed = &source.source[source.offset(edit.range.start).unwrap()..offset];
            assert!(item.filter_text.as_ref().unwrap().starts_with(typed));
            workspace.open(uri.clone(), completed, 2);
            assert_eq!(
                texquill::language::navigation::document_links(&workspace, &uri)[0].target,
                Some(Url::from_file_path(root.join(format!("mydir/文献.{ext}"))).unwrap())
            );
        }
    }
    assert_eq!(
        apply_completion(
            &mut workspace,
            &uri,
            "\\bibliography{prior, my dir/文|旧.b ib, next}",
            "mydir/文献.bib"
        ),
        r"\bibliography{prior, mydir/文献.bib, next}"
    );
}

#[test]
fn compacting_loaders_exclude_unrepresentable_directory_completions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in ["mydir", "my dir"] {
        std::fs::create_dir(root.join(folder)).unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for command in [
        "usepackage",
        "RequirePackageWithOptions",
        "bibliography",
        "bibliographystyle",
    ] {
        let source = format!("\\{command}{{my");
        workspace.open(uri.clone(), source.clone(), 1);
        let items = features::completion(
            &workspace,
            &uri,
            Text::new(source.clone()).position(source.len()),
        );
        assert!(
            items.iter().any(|item| item.label == "mydir/"),
            "{command}: {items:?}"
        );
        assert!(
            !items.iter().any(|item| item.label == "my dir/"),
            "{command}: {items:?}"
        );
    }
    for command in [
        "input",
        "addbibresource",
        "documentclass",
        "LoadClass",
        "LoadClassWithOptions",
    ] {
        assert_eq!(
            apply_completion(
                &mut workspace,
                &uri,
                &format!("\\{command}{{my|/file}}"),
                "my dir/"
            ),
            format!("\\{command}{{my dir/file}}")
        );
    }
}

#[test]
fn style_completion_does_not_leave_invalid_whitespace_outside_the_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("local.bst"), "STYLE").unwrap();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for marked in [
        r"\bibliographystyle{ lo|cal}",
        r"\bibliographystyle{lo|cal }",
        r"\bibliographystyle{lo| cal}",
        "\\bibliographystyle{\tlo|cal}",
        "\\bibliographystyle{lo|cal\n}",
        r"\bibliographystyle{ lo|",
    ] {
        let offset = marked.find('|').unwrap();
        let source = marked.replace('|', "");
        workspace.open(uri.clone(), source.clone(), 1);
        let items = features::completion(&workspace, &uri, Text::new(source).position(offset));
        assert!(
            !items
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::FILE)),
            "{marked:?}: {items:?}"
        );
    }
}

#[test]
fn multiline_path_completions_remove_suffix_lines_and_avoid_overlapping_empty_insertions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("mydir")).unwrap();
    for name in ["local.sty", "refs.bib"] {
        std::fs::write(root.join("mydir").join(name), "Source.").unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for (marked, label, expected) in [
        (
            "😀 \\usepackage{my dir/lo| \r\ncal}",
            "mydir/local.sty",
            r"😀 \usepackage{mydir/local}",
        ),
        (
            "\\bibliography{first, my dir/re|fs.b \nib, last}",
            "mydir/refs.bib",
            r"\bibliography{first, mydir/refs.bib, last}",
        ),
    ] {
        assert_eq!(
            apply_completion(&mut workspace, &uri, marked, label),
            expected
        );
    }
    let marked = "\\usepackage{mydir/\n|\nlocal}";
    let offset = marked.find('|').unwrap();
    let source = marked.replace('|', "");
    workspace.open(uri.clone(), source.clone(), 1);
    let items = features::completion(&workspace, &uri, Text::new(source).position(offset));
    assert!(!items
        .iter()
        .any(|item| item.kind == Some(CompletionItemKind::FILE)));
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn completed_multiline_package_paths_load_the_suggested_file_in_real_compilers() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("mylibs")).unwrap();
        std::fs::write(
            root.join("mylibs/localpkg.sty"),
            r"\typeout{COMPLETED PACKAGE LOADED}",
        )
        .unwrap();
        let main = root.join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let marked = "% 😀\r\n\\documentclass{article}\r\n\\usepackage{my \r\n libs/lo ca|l}\r\n\\begin{document}Hello.\\end{document}\r\n";
        let completed = apply_completion(&mut workspace, &uri, marked, "mylibs/localpkg.sty");
        assert!(completed.contains(r"\usepackage{mylibs/localpkg}"));
        std::fs::write(&main, &completed).unwrap();
        workspace.open(uri.clone(), completed, 2);
        let outcome = texquill::compilation::build::run(
            &main,
            &texquill::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            },
            &Default::default(),
        );
        assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
        assert!(
            outcome.output.contains("COMPLETED PACKAGE LOADED"),
            "{}",
            outcome.output
        );
        assert_eq!(
            texquill::language::navigation::document_links(&workspace, &uri)[0].target,
            Some(Url::from_file_path(root.join("mylibs/localpkg.sty")).unwrap())
        );
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn unicode_input_completions_read_the_suggested_file_in_real_compilers() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("目录")).unwrap();
        std::fs::write(
            root.join("目录/child.tex"),
            "\\typeout{UNICODE COMPLETION READ}Chapter.\n",
        )
        .unwrap();
        let main = root.join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        let target = Url::from_file_path(root.join("目录/child.tex")).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        for command in [
            r"\input 目|/child",
            r#"\input"目|/child""#,
            r"\input{目|/child}",
        ] {
            // Complete a directory while retaining the existing filename suffix.
            let marked = format!("% 😀\n\\documentclass{{article}}\n\\begin{{document}}\n{command}\n\\end{{document}}\n");
            let completed = apply_completion(&mut workspace, &uri, &marked, "目录/");
            std::fs::write(&main, &completed).unwrap();
            workspace.open(uri.clone(), completed, 2);
            let outcome = texquill::compilation::build::run(
                &main,
                &texquill::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                },
                &Default::default(),
            );
            assert_eq!(
                outcome.status, "success",
                "{engine}, {command}: {}",
                outcome.output
            );
            assert!(
                outcome.output.contains("UNICODE COMPLETION READ"),
                "{}",
                outcome.output
            );
            assert!(outcome.inputs.unwrap().contains(&target));
            assert_eq!(
                texquill::language::navigation::document_links(&workspace, &uri)[0].target,
                Some(target.clone())
            );
        }
    }
}
