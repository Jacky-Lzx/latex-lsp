use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;
#[test]
fn context_entry_assignments_participate_in_complete_key_renames() {
    let dir = tempfile::tempdir().unwrap();
    let source = r"\documentclass{article}\usepackage[style=numeric,sorting=none,defernumbers=true]{biblatex}\addbibresource{data.bib}\begin{document}\assignrefcontextentries[labelprefix=A]{a} Text \cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}\end{document}";
    std::fs::write(dir.path().join("main.tex"), source).unwrap();
    std::fs::write(dir.path().join("data.bib"), "@book{a,title={Alpha}}").unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("data.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source.into(), 5);
    let doc = &ws.documents[&bib];
    let at = doc.text.source.find("a,").unwrap();
    let pos = doc.text.position(at);
    let refs = features::references(&ws, &bib, pos, true);
    let edit = features::rename(&ws, &bib, pos, "fresh");
    let edits = edit.unwrap().unwrap().changes.unwrap();
    let at = source.find("{a} Text").unwrap() + 1;
    let assignment = ws.documents[&main].text.range(at, at + 1);
    assert!(
        edits[&main]
            .iter()
            .any(|edit| edit.range == assignment && edit.new_text == "fresh"),
        "native assignment key was omitted"
    );
    assert_eq!(refs.len(), 5); // header, assignment, cite and both explicit nocites
}

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage[style=numeric,sorting=none,defernumbers=true]{{biblatex}}\\addbibresource{{data.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(
        dir.path().join("data.bib"),
        "@book{alpha,title={Alpha}} @book{beta,title={Beta}} @book{中文😀,title={Unicode}}",
    )
    .unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("data.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}
fn key(ws: &Workspace, uri: &Url, marker: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let at = doc.text.source.find(marker).unwrap() + marker.len();
    assert_eq!(&doc.text.source[at..at + name.len()], name);
    Location::new(uri.clone(), doc.text.range(at, at + name.len()))
}
fn apply(source: &str, edits: &[TextEdit]) -> String {
    let text = latex_lsp::text::Text::new(source.into());
    let mut result = source.to_string();
    let mut edits = edits.iter().collect::<Vec<_>>();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for edit in edits {
        result.replace_range(
            text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
            &edit.new_text,
        );
    }
    result
}
#[test]
fn direct_starred_named_and_forwarded_assignments_keep_only_real_key_edits() {
    for call in [
        r"\assignrefcontextentries",
        r"\assignrefcontextentries*",
        r"\csuse{assignrefcontextentries}",
        r"\csname assignrefcontextentries\endcsname",
        r"\assign",
    ] {
        let (_dir, ws, main, bib) = setup(&format!(
            r"\def\assign{{\assignrefcontextentries}}{call}[labelprefix=alpha]{{alpha,beta}} Text \cite{{alpha}}\verb|\assignrefcontextentries[]{{alpha}}|"
        ));
        let assignment = key(&ws, &main, &format!("{call}[labelprefix=alpha]{{"), "alpha");
        let declaration = key(&ws, &bib, "@book{", "alpha");
        assert_eq!(
            features::definitions(&ws, &main, assignment.range.start),
            vec![declaration.clone()],
            "{call}"
        );
        assert!(features::hover(&ws, &main, assignment.range.start).is_some());
        assert_eq!(
            features::references(&ws, &main, assignment.range.start, true).len(),
            3,
            "{call}"
        );
        assert_eq!(
            latex_lsp::editing::highlights(&ws, &main, assignment.range.start).len(),
            2
        );
        let changes = features::rename(&ws, &main, assignment.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes[&main].len(), 2, "{call}");
        assert_eq!(changes[&bib].len(), 1);
        let changed = apply(&ws.documents[&main].text.source, &changes[&main]);
        assert!(
            changed.contains("[labelprefix=alpha]{fresh,beta}"),
            "{changed}"
        );
        assert!(changed.contains(r"\cite{fresh}"));
        assert!(changed.contains(r"\verb|\assignrefcontextentries[]{alpha}|"));
        assert!(apply(&ws.documents[&bib].text.source, &changes[&bib])
            .contains("@book{fresh,title={Alpha}}"));
    }
}
#[test]
fn assignment_lists_complete_full_keys_and_signature_help_requires_exact_native_arguments() {
    for following in ["", "[labelprefix=A]{alpha}", "*[]{alpha}"] {
        let name = "assignrefcontextentries";
        let body = format!("\\{name}{following}");
        let (_dir, mut ws, main, _bib) = setup(&body);
        ws.snippets = true;
        let doc = &ws.documents[&main];
        let at = doc.text.source.find(&body).unwrap() + name.len() + 1;
        let items = features::completion(&ws, &main, doc.text.position(at));
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(
            edit.new_text,
            if following.is_empty() {
                "assignrefcontextentries[${1:options}]{${2:keys}}"
            } else {
                name
            }
        );
    }
    for (tail, active) in [
        ("¦", 0),
        ("[label¦prefix=A]{alpha}", 0),
        ("[labelprefix=A]{al¦pha,beta}", 1),
        ("*[]{alpha,be¦ta}", 1),
    ] {
        let marked = format!(r"\assignrefcontextentries{tail}");
        let body = marked.replace('¦', "");
        let (_dir, mut ws, main, _bib) = setup(&body);
        if tail == "¦" {
            let source = ws.documents[&main].text.source.clone();
            let end = source.find(&body).unwrap() + body.len();
            ws.open(main.clone(), source[..end].into(), 6);
        }
        let doc = &ws.documents[&main];
        let at = doc.text.source.find(&body).unwrap() + marked.find('¦').unwrap();
        let help = latex_lsp::language::signature::help(&ws, &main, doc.text.position(at))
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(
            help.signatures[0].label,
            r"\assignrefcontextentries[options]{keys}"
        );
        assert_eq!(help.active_parameter, Some(active), "{marked}");
    }
    for call in [
        r"\assignrefcontextentries",
        r"\csuse{assignrefcontextentries}",
    ] {
        let (_dir, ws, main, _bib) = setup(&format!(r"{call}*[]{{beta,alphaSuffix}} Text"));
        let query = key(&ws, &main, &format!("{call}*[]{{beta,"), "alphaSuffix");
        let items = features::completion(&ws, &main, query.range.start);
        let item = items.iter().find(|item| item.label == "alpha").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.range, query.range);
        assert_eq!(edit.new_text, "alpha");
    }
    for body in [
        r"\assignrefcontextentries{alpha}",
        r"\assignrefcontextentries[][unexpected]{alpha}",
    ] {
        let (_dir, ws, main, _bib) = setup(body);
        let query = key(&ws, &main, body.rsplit_once("alpha").unwrap().0, "alpha");
        assert!(
            features::definitions(&ws, &main, query.range.start).is_empty(),
            "{body}"
        );
    }
    let (_dir, ws, main, _bib) = setup(r"\assignrefcontextentries[labelprefix=alpha]{beta}");
    let options = key(
        &ws,
        &main,
        r"\assignrefcontextentries[labelprefix=",
        "alpha",
    );
    assert!(features::definitions(&ws, &main, options.range.start).is_empty());
    assert!(features::completion(&ws, &main, options.range.start)
        .iter()
        .all(|item| item.label != "alpha"));
}
#[test]
fn nonloading_assignments_do_not_warn_for_missing_keys() {
    let (_dir, ws, main, _bib) = setup(
        r"\assignrefcontextentries*[]{missing,alpha,*} Text \cite{absent}\label{target}\ref{target}",
    );
    let missing: Vec<_> = ws
        .diagnostics(&main)
        .into_iter()
        .filter(|diag| diag.code == Some(NumberOrString::String("undefined-citation".into())))
        .map(|diag| diag.data.unwrap()["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(missing, ["absent"]);
}
#[test]
fn redefined_assignments_and_captured_or_operand_calls_keep_existing_reference_guards() {
    let (_dir, ws, main, bib) = setup(
        r"\renewcommand\assignrefcontextentries[2][]{}\assignrefcontextentries[]{alpha} Text \cite{alpha}",
    );
    let assignment = key(&ws, &main, r"\assignrefcontextentries[]{", "alpha");
    assert!(features::definitions(&ws, &main, assignment.range.start).is_empty());
    let declaration = key(&ws, &bib, "@book{", "alpha");
    let changes = features::rename(&ws, &bib, declaration.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&main].len(), 1);
    assert!(apply(&ws.documents[&main].text.source, &changes[&main])
        .contains(r"\assignrefcontextentries[]{alpha}"));
    for body in [
        r"\let\saved\assignrefcontextentries\renewcommand\assignrefcontextentries[2][]{}\saved[]{alpha} Text \cite{alpha}",
        r"\newcommand\take[1]{}\take\assignrefcontextentries[]{alpha} Text \cite{alpha}",
    ] {
        let (_dir, ws, _main, bib) = setup(body);
        let declaration = key(&ws, &bib, "@book{", "alpha");
        assert!(
            features::rename(&ws, &bib, declaration.range.start, "fresh").is_err(),
            "{body}"
        );
    }
}
#[test]
fn numeric_sections_keep_nonloading_keys_and_source_sets_refuse_partial_edits() {
    let (_dir, ws, main, bib) =
        setup(r"\newrefsection\assignrefcontextentries[]{alpha} Text \cite{alpha}");
    let declaration = key(&ws, &bib, "@book{", "alpha");
    let assignment = key(&ws, &main, r"\assignrefcontextentries[]{", "alpha");
    assert_eq!(
        features::definitions(&ws, &main, assignment.range.start),
        vec![declaration.clone()]
    );
    let changes = features::rename(&ws, &bib, declaration.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&main].len(), 2);
    for body in [
        r"\defbibentryset{collection}{beta}\assignrefcontextentries[]{alpha} Text \cite{alpha}",
        r"\defbibentryset{collection}{beta}\csuse{assignrefcontextentries}*[]{alpha} Text \cite{alpha}",
    ] {
        let (_dir, ws, main, bib) = setup(body);
        let declaration = key(&ws, &bib, "@book{", "alpha");
        assert!(
            features::rename(&ws, &bib, declaration.range.start, "fresh").is_err(),
            "{body}"
        );
        let assignment = key(&ws, &main, body.split_once("alpha").unwrap().0, "alpha");
        assert!(
            features::definitions(&ws, &main, assignment.range.start).is_empty(),
            "{body}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_complete_assignment_renames_preserve_actual_labels_and_all_bbl_records() {
    let printing = r"\begin{refcontext}[labelprefix=A]\nocite{alpha}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{alpha}\printbibliography\end{refcontext}";
    for (body,expected) in [
        (format!(r"\assignrefcontextentries[labelprefix=A]{{alpha,beta}} Text \cite{{alpha}}{printing}"),vec!["A:1"]),
        (format!(r"\csuse{{assignrefcontextentries}}[labelprefix=A]{{alpha}} Text \cite{{alpha}}{printing}"),vec!["A:1"]),
        (format!(r"\cite{{alpha}}\assignrefcontextentries[labelprefix=A]{{alpha}} Text \cite{{alpha}}{printing}"),vec!["B:1","A:1"]),
        (r"\assignrefcontextentries*[labelprefix=A]{alpha} Text \cite{alpha}\begin{refcontext}[labelprefix=A]\nocite{alpha}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\cite{alpha}\printbibliography\end{refcontext}\cite{alpha}".into(),vec!["A:1","B:1","A:1"]),
        (format!(r"\assignrefcontextentries[labelprefix=A]{{*}} Text \cite{{alpha}}{printing}"),vec!["A:1"]),
    ] {
        let (dir,mut ws,main,bib)=setup(&body);
        let source=ws.documents[&main].text.source.replace(r"\begin{document}",r"\AtEveryCitekey{\typeout{LSP-LABEL:\thefield{entrykey}:\thefield{labelprefix}:\thefield{labelnumber}}}\begin{document}");
        ws.open(main.clone(),source,6);
        // Unicode fixture keys must not enter the native bibliography parser.
        ws.open(bib.clone(),"@book{alpha,title={Alpha}} @book{beta,title={Beta}}".into(),7);
        let compile=|source: &str, library: &str| {
            for extension in ["aux","bcf","bbl","run.xml"] {let path=dir.path().join(format!("main.{extension}"));if path.exists(){std::fs::remove_file(path).unwrap();}}
            std::fs::write(dir.path().join("main.tex"),source).unwrap();std::fs::write(dir.path().join("data.bib"),library).unwrap();
            for program in ["pdflatex","biber","pdflatex","pdflatex","pdflatex"] {
                let args=if program=="biber"{vec!["main"]}else{vec!["-no-shell-escape","-interaction=nonstopmode","-halt-on-error","main.tex"]};
                let output=std::process::Command::new(program).args(args).current_dir(dir.path()).output().unwrap();assert!(output.status.success(),"{program}: {}\n{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
            }
            let log=std::fs::read_to_string(dir.path().join("main.log")).unwrap();let blg=std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
            assert!(!log.contains("undefined")&&!log.contains("Warning"),"{log}");assert!(!blg.contains("WARN")&&!blg.contains("ERROR"),"{blg}");
            let actual:Vec<_>=log.lines().filter_map(|line|line.strip_prefix("LSP-LABEL:")).map(str::to_string).collect();
            (std::fs::read_to_string(dir.path().join("main.bbl")).unwrap(),actual)
        };
        let before=compile(&ws.documents[&main].text.source,&ws.documents[&bib].text.source);
        let declaration=key(&ws,&bib,"@book{","alpha");let changes=features::rename(&ws,&bib,declaration.range.start,"fresh").unwrap().unwrap().changes.unwrap();
        let after=compile(&apply(&ws.documents[&main].text.source,&changes[&main]),&apply(&ws.documents[&bib].text.source,&changes[&bib]));
        assert_eq!(before.0,after.0.replace("{fresh}","{alpha}"),"{body}");
        assert_eq!(before.1,expected.iter().map(|label|format!("alpha:{label}")).collect::<Vec<_>>(),"{body}");
        assert_eq!(after.1,expected.iter().map(|label|format!("fresh:{label}")).collect::<Vec<_>>(),"{body}");
    }
}
