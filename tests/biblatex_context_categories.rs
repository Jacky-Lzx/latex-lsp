use latex_lsp::{
    editing,
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;

fn setup(body: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:context-categories.tex").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), body.into(), 5);
    (ws, uri)
}
fn apply(source: &str, edits: &[TextEdit]) -> String {
    let text = Text::new(source.into());
    let mut result = source.to_string();
    let mut ordered = edits.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for edit in ordered {
        result.replace_range(
            text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
            &edit.new_text,
        );
    }
    result
}
#[test]
fn category_assignment_renames_include_every_native_target_and_preserve_entry_keys() {
    for call in [
        r"\assignrefcontextcats",
        r"\assignrefcontextcats*",
        r"\csuse{assignrefcontextcats}*",
        r"\csname assignrefcontextcats\endcsname",
        r"\assign",
    ] {
        let source=format!("% 😀\r\n\\DeclareBibliographyCategory{{picked}}\\DeclareBibliographyCategory{{other}}\\addtocategory{{picked}}{{picked}}\\def\\assign{{\\assignrefcontextcats}}{call}[labelprefix=picked]{{other,picked}} Text \\printbibliography[category=picked,title={{picked}}]\\verb|\\assignrefcontextcats[]{{picked}}|");
        let (ws, uri) = setup(&source);
        let text = Text::new(source.clone());
        let at = source.find("other,picked").unwrap() + "other,".len();
        let pos = text.position(at);
        let definitions = features::definitions(&ws, &uri, pos);
        assert_eq!(definitions.len(), 1, "{call}");
        assert_eq!(
            definitions[0].range,
            text.range(
                source.find("{picked}").unwrap() + 1,
                source.find("{picked}").unwrap() + 7
            )
        );
        assert!(format!("{:?}", features::hover(&ws, &uri, pos)).contains("biblatex category"));
        assert_eq!(
            features::references(&ws, &uri, pos, true).len(),
            4,
            "{call}"
        );
        assert_eq!(editing::highlights(&ws, &uri, pos).len(), 4);
        assert!(features::prepare_rename(&ws, &uri, pos).unwrap().is_some());
        let changes = features::rename(&ws, &uri, pos, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&uri].len(), 4);
        let changed = apply(&source, &changes[&uri]);
        assert!(changed.contains("[labelprefix=picked]{other,fresh}"));
        assert!(changed.contains(r"\addtocategory{fresh}{picked}"));
        assert!(changed.contains("title={picked}"));
        assert!(changed.contains(r"\verb|\assignrefcontextcats[]{picked}|"));
    }
}
#[test]
fn assignment_category_lists_complete_full_suffixes_comments_and_unfinished_items() {
    let prefix =
        "% 😀\r\n\\DeclareBibliographyCategory{picked}\\DeclareBibliographyCategory{other}\r\n";
    for (marked, expected) in [
        (
            r"\assignrefcontextcats[]{other,pi¦BAD}",
            r"\assignrefcontextcats[]{other,picked}",
        ),
        (
            r"\csuse{assignrefcontextcats}*[]{other,pi¦BAD}",
            r"\csuse{assignrefcontextcats}*[]{other,picked}",
        ),
        (
            "\\assignrefcontextcats*[]{other,% picked, comment\r\n pi¦BAD% trailing\r\n}",
            "\\assignrefcontextcats*[]{other,% picked, comment\r\n picked% trailing\r\n}",
        ),
        (
            r"\assignrefcontextcats[]{other,pi¦",
            r"\assignrefcontextcats[]{other,picked",
        ),
        (
            r"\assignrefcontextcats*[]{¦}",
            r"\assignrefcontextcats*[]{picked}",
        ),
        (
            "\\assignrefcontextcats[]{pi¦\n\\end{document}",
            "\\assignrefcontextcats[]{picked\n\\end{document}",
        ),
    ] {
        let at = prefix.len() + marked.find('¦').unwrap();
        let source = format!("{prefix}{}", marked.replace('¦', ""));
        let (ws, uri) = setup(&source);
        let items = features::completion(&ws, &uri, Text::new(source.clone()).position(at));
        let item = items
            .iter()
            .find(|item| item.label == "picked")
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        let mut edits = vec![edit.clone()];
        edits.extend(item.additional_text_edits.clone().unwrap_or_default());
        assert_eq!(apply(&source, &edits), format!("{prefix}{expected}"));
    }
    for marked in [
        r"\assignrefcontextcats[labelprefix=pi¦]{other}",
        r"\assignrefcontextentries[]{pi¦}",
        r"\assignrefcontextkeyws[]{pi¦}",
    ] {
        let at = prefix.len() + marked.find('¦').unwrap();
        let source = format!("{prefix}{}", marked.replace('¦', ""));
        let (ws, uri) = setup(&source);
        assert!(
            !features::completion(&ws, &uri, Text::new(source).position(at))
                .iter()
                .any(|item| item.label == "picked"),
            "{marked}"
        );
    }
}
#[test]
fn category_assignments_have_native_required_brackets_stars_and_command_snippets() {
    for (marked, active) in [
        (r"\assignrefcontextcats[lab¦elprefix=A]{picked}", 0),
        (r"\assignrefcontextcats*[]{pi¦cked,other}", 1),
        (r"\csuse{assignrefcontextcats}*[]{pi¦cked}", 1),
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (ws, uri) = setup(&source);
        let help = signature::help(&ws, &uri, Text::new(source).position(at))
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(
            help.signatures[0].label,
            if marked.starts_with(r"\csuse") {
                r"\csuse{assignrefcontextcats}[options]{categories}"
            } else {
                r"\assignrefcontextcats[options]{categories}"
            }
        );
        assert_eq!(help.active_parameter, Some(active));
    }
    for following in ["", "[labelprefix=A]{picked}", "*[]{picked}"] {
        let name = "assignrefcontextcats";
        let source = format!("\\{name}{following}");
        let (mut ws, uri) = setup(&source);
        ws.client.snippets = true;
        let items = features::completion(&ws, &uri, Text::new(source).position(name.len() + 1));
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(
            edit.new_text,
            if following.is_empty() {
                "assignrefcontextcats[${1:options}]{${2:categories}}"
            } else {
                name
            }
        );
    }
}
#[test]
fn ambiguous_category_lists_and_captured_calls_refuse_incomplete_renames() {
    for tail in [
        r"\assignrefcontextcats{picked}",
        r"\assignrefcontextcats[][oops]{picked}",
        r"\assignrefcontextcats[]{picked,\dynamic}",
        "\\assignrefcontextcats[]{pi% splice\ncked}",
        r"\assignrefcontextcats[]{picked",
        r"\def\take#1{}\take\assignrefcontextcats[]{picked}",
        r"\let\saved\assignrefcontextcats\renewcommand\assignrefcontextcats[2][]{}\saved[]{picked}",
    ] {
        let source = format!("\\DeclareBibliographyCategory{{picked}}{tail}");
        let (ws, uri) = setup(&source);
        let pos = Text::new(source.clone()).position(source.find("picked").unwrap() + 2);
        assert!(features::rename(&ws, &uri, pos, "fresh").is_err(), "{tail}");
    }
    for tail in [
        r"\assignrefcontextcats[]{fresh}",
        r"\assignrefcontextcats*[]{other,fresh}",
    ] {
        let source = format!("\\DeclareBibliographyCategory{{picked}}{tail}");
        let (ws, uri) = setup(&source);
        let pos = Text::new(source.clone()).position(source.find("picked").unwrap());
        assert!(features::rename(&ws, &uri, pos, "fresh").is_err(), "{tail}");
    }
    let source = r"\DeclareBibliographyCategory{picked}\let\unused\assignrefcontextcats\assignrefcontextcats[]{}\verb|\assignrefcontextcats[]{picked}|";
    let (ws, uri) = setup(source);
    let changes = features::rename(
        &ws,
        &uri,
        Text::new(source.into()).position(source.find("picked").unwrap()),
        "fresh",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(changes[&uri].len(), 1);
    let source = r"\DeclareBibliographyCategory{picked}\renewcommand\assignrefcontextcats[2][]{}\assignrefcontextcats[]{picked}\addtocategory{picked}{paper}";
    let (ws, uri) = setup(source);
    let changes = features::rename(
        &ws,
        &uri,
        Text::new(source.into()).position(source.find("picked").unwrap()),
        "fresh",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(changes[&uri].len(), 2);
    assert!(apply(source, &changes[&uri]).contains(r"\assignrefcontextcats[]{picked}"));
}
#[test]
fn shared_category_assignment_meanings_follow_unsaved_consumers_and_restored_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = r"\documentclass{article}\input{common}\input{shared}";
    for (name, source) in [
        ("a.tex", root),
        ("b.tex", root),
        ("common.tex", r"\DeclareBibliographyCategory{picked}"),
        (
            "shared.tex",
            r"\csuse{assignrefcontextcats}*[]{other,picked}",
        ),
        (
            "independent.tex",
            r"\documentclass{article}\DeclareBibliographyCategory{picked}",
        ),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
    let shared = common.join("shared.tex").unwrap();
    let b = common.join("b.tex").unwrap();
    let pos = ws.documents[&common]
        .text
        .position(ws.documents[&common].text.source.find("picked").unwrap());
    let changes = features::rename(&ws, &common, pos, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&common].len(), 1);
    assert_eq!(changes[&shared].len(), 1);
    ws.open(
        b.clone(),
        root.replace(
            r"\input{common}",
            r"\renewcommand\assignrefcontextcats[2][]{}\input{common}",
        ),
        8,
    );
    assert!(features::rename(&ws, &common, pos, "fresh").is_err());
    ws.open(b, root.into(), 9);
    assert_eq!(
        features::rename(&ws, &common, pos, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&shared]
            .len(),
        1
    );
}
#[test]
fn over_budget_category_assignment_lists_never_return_partial_edits() {
    let source = format!(
        "\\DeclareBibliographyCategory{{picked}}\\assignrefcontextcats[]{{{}picked}}",
        "other,".repeat(20_001)
    );
    let (ws, uri) = setup(&source);
    assert!(features::rename(
        &ws,
        &uri,
        Text::new(source.clone()).position(source.find("picked").unwrap()),
        "fresh"
    )
    .is_err());
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_category_assignment_renames_keep_actual_labels_membership_and_complete_bbl() {
    let printing = r"\begin{refcontext}[labelprefix=A]\nocite{alpha}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{alpha}\printbibliography\end{refcontext}";
    for (body,expected) in [
        (format!(r"\assignrefcontextcats[labelprefix=A]{{picked,other}} Text \cite{{alpha}}{printing}"),vec!["A:1"]),
        (format!(r"\csuse{{assignrefcontextcats}}[labelprefix=A]{{picked}} Text \cite{{alpha}}{printing}"),vec!["A:1"]),
        (format!(r"\cite{{alpha}}\assignrefcontextcats[labelprefix=A]{{picked}} Text \cite{{alpha}}{printing}"),vec!["B:1","A:1"]),
        (r"\assignrefcontextcats*[labelprefix=A]{picked} Text \cite{alpha}\begin{refcontext}[labelprefix=A]\nocite{alpha}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\cite{alpha}\printbibliography\end{refcontext}\cite{alpha}".into(),vec!["A:1","B:1","A:1"]),
        (format!(r"{{\assignrefcontextcats[labelprefix=A]{{picked}}}} Text \cite{{alpha}}{printing}"),vec!["A:1"]),
    ] {
        let dir=tempfile::tempdir().unwrap();let source=format!(r"\documentclass{{article}}\usepackage[style=numeric,sorting=none,defernumbers=true]{{biblatex}}\addbibresource{{data.bib}}\DeclareBibliographyCategory{{picked}}\DeclareBibliographyCategory{{other}}\addtocategory{{picked}}{{alpha}}\AtEveryCitekey{{\typeout{{LSP-LABEL:\thefield{{entrykey}}:\thefield{{labelprefix}}:\thefield{{labelnumber}}}}}}\begin{{document}}{body}\end{{document}}");
        std::fs::write(dir.path().join("main.tex"),&source).unwrap();std::fs::write(dir.path().join("data.bib"),"@book{alpha,title={Alpha}}").unwrap();
        let main=Url::from_file_path(dir.path().join("main.tex")).unwrap();let mut ws=Workspace::default();ws.config.distribution.enabled=false;ws.open(main.clone(),source.clone(),5);
        let compile=|content: &str| {
            for extension in ["aux","bcf","bbl","run.xml"] {let path=dir.path().join(format!("main.{extension}"));if path.exists(){std::fs::remove_file(path).unwrap();}}
            std::fs::write(dir.path().join("main.tex"),content).unwrap();
            for program in ["pdflatex","biber","pdflatex","pdflatex","pdflatex"] {let args=if program=="biber"{vec!["main"]}else{vec!["-no-shell-escape","-interaction=nonstopmode","-halt-on-error","main.tex"]};let output=std::process::Command::new(program).args(args).current_dir(dir.path()).output().unwrap();assert!(output.status.success(),"{program}: {}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));}
            let log=std::fs::read_to_string(dir.path().join("main.log")).unwrap();let blg=std::fs::read_to_string(dir.path().join("main.blg")).unwrap();assert!(!log.contains("undefined")&&!log.contains("Warning"),"{log}");assert!(!blg.contains("WARN")&&!blg.contains("ERROR"),"{blg}");
            let labels=log.lines().filter_map(|line|line.strip_prefix("LSP-LABEL:")).map(str::to_string).collect::<Vec<_>>();(std::fs::read_to_string(dir.path().join("main.bbl")).unwrap(),labels,std::fs::read_to_string(dir.path().join("main.aux")).unwrap())
        };
        let before=compile(&source);let pos=ws.documents[&main].text.position(source.find("picked").unwrap());let changes=features::rename(&ws,&main,pos,"fresh").unwrap().unwrap().changes.unwrap();assert_eq!(changes.len(),1);assert_eq!(changes[&main].len(),3);let changed=apply(&source,&changes[&main]);let after=compile(&changed);
        assert_eq!(before.0,after.0,"{body}");assert_eq!(before.1,after.1,"{body}");assert_eq!(before.1,expected.iter().map(|value|format!("alpha:{value}")).collect::<Vec<_>>(),"{body}");assert!(before.2.contains(r"\abx@aux@category{picked}{alpha}"));assert!(after.2.contains(r"\abx@aux@category{fresh}{alpha}"));
    }
}

#[test]
fn newly_opened_roots_certify_shared_meanings_and_category_declarations_before_renaming() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "main.tex",
            r"\documentclass{article}\input{common}\input{shared}",
        ),
        ("common.tex", r"\DeclareBibliographyCategory{picked}"),
        ("shared.tex", r"\assignrefcontextcats[]{picked}"),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
    let other = common.join("new.tex").unwrap();
    let pos = ws.documents[&common]
        .text
        .position(ws.documents[&common].text.source.find("picked").unwrap());
    for source in [
        r"\documentclass{article}\renewcommand\assignrefcontextcats[2][]{}\input{common}\input{shared}",
        r"\documentclass{article}\DeclareBibliographyCategory{picked}\input{shared}",
        r"\documentclass{article}\input{shared}",
    ] {
        ws.open(other.clone(), source.into(), 9);
        assert!(
            features::rename(&ws, &common, pos, "fresh").is_err(),
            "{source}"
        );
    }
    for source in [
        r"\documentclass{article}\input{common}\input{shared}",
        r"\documentclass{article}\DeclareBibliographyCategory{picked}",
    ] {
        ws.open(other.clone(), source.into(), 10);
        let changes = features::rename(&ws, &common, pos, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2, "{source}");
    }
}
