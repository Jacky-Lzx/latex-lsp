use latex_lsp::{features, signature, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

const SIMPLE: &[&str] = &[
    "AtBeginRefsection",
    "AtNextRefsection",
    "AtFollowingRefsections",
    "AtBeginBibliography",
    "AtBeginShorthands",
    "AtEveryBibitem",
    "AtEveryLositem",
    "AtNextBibliography",
    "AtEveryCite",
    "AtEveryCitekey",
    "AtEveryMultiCite",
    "AtNextCite",
    "AtEachCitekey",
    "AtNextCitekey",
    "AtNextMultiCite",
    "AtUsedriver",
    "AtVolcite",
];

fn setup(source: &str) -> (Workspace, Url) {
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.snippets = true;
    let uri = Url::parse("untitled:bib-hooks.tex").unwrap();
    ws.open(uri.clone(), source.into(), 5);
    (ws, uri)
}

fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
fn simple_hooks_keep_executable_keys_and_following_calls_available() {
    for name in SIMPLE {
        let source = format!(
            r"\label{{paper}}\DeclareBibliographyCategory{{selected}}\{name}{{\ref{{paper}}\ifcategory{{selected}}{{}}{{}}}}\ref{{paper}}"
        );
        let (ws, uri) = setup(&source);
        let text = Text::new(source.clone());
        for at in [
            source.rfind("paper").unwrap(),
            source.find(r"\ref{paper}").unwrap() + 5,
        ] {
            let position = text.position(at + 2);
            assert_eq!(
                features::definitions(&ws, &uri, position).len(),
                1,
                "{source}"
            );
            let changes = features::rename(&ws, &uri, position, "updated")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes[&uri].len(), 3, "{source}");
        }
        let position = text.position(source.rfind("selected").unwrap() + 2);
        assert_eq!(
            features::definitions(&ws, &uri, position).len(),
            1,
            "{source}"
        );
        let edit = features::rename(&ws, &uri, position, "new-category")
            .unwrap()
            .unwrap();
        assert_eq!(edit.changes.unwrap()[&uri].len(), 2, "{source}");
    }
}

#[test]
fn hook_signatures_track_selectors_code_branches_stars_and_unfinished_calls() {
    let mut cases: Vec<_> = SIMPLE
        .iter()
        .map(|name| (*name, "{co¦de}", 0, "{code}"))
        .collect();
    cases.extend([
        ("AtUsedriver", "*{co¦de}", 0, "{code}"),
        ("AtVolcite", "*{co¦de}", 0, "{code}"),
        ("AtDataInput", "[bo¦ok]{code}", 0, "[entry type]{code}"),
        ("AtDataInput", "{co¦de}", 1, "[entry type]{code}"),
        ("AtDataInput", "[book]{co¦", 1, "[entry type]{code}"),
        ("AtDataInput", "[book] ¦", 1, "[entry type]{code}"),
        (
            "AtBeginBiblist",
            "{shor¦thand}{code}",
            0,
            "{bibliography list}{code}",
        ),
        (
            "AtBeginBiblist",
            "{shorthand}{co¦de}",
            1,
            "{bibliography list}{code}",
        ),
        (
            "AtEveryBiblistitem",
            "{shorthand}{co¦de}",
            1,
            "{bibliography list}{code}",
        ),
        (
            "AtEveryEntrykey",
            "{co¦de}{success}{failure}",
            0,
            "{code}{success}{failure}",
        ),
        (
            "AtEveryEntrykey",
            "{code}{suc¦cess}{failure}",
            1,
            "{code}{success}{failure}",
        ),
        (
            "AtEveryEntrykey",
            "{code}{success}{fai¦lure}",
            2,
            "{code}{success}{failure}",
        ),
    ]);
    for (name, tail, active, label) in cases {
        for call in [
            format!("\\{name}"),
            format!("\\csuse{{{name}}}"),
            format!("\\csname {name}\\endcsname"),
        ] {
            let marked = format!("😀 {call}{tail}");
            let (ws, uri) = setup(&marked.replace('¦', ""));
            let position = ws.documents[&uri].text.position(marked.find('¦').unwrap());
            let help = signature::help(&ws, &uri, position).unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(
                help.signatures[0].label,
                format!("{call}{label}"),
                "{marked}"
            );
            assert_eq!(help.active_parameter, Some(active), "{marked}");
        }
    }
}

#[test]
fn hook_selectors_are_data_and_all_entrykey_branches_are_code() {
    for call in [
        r"\AtDataInput[\ref{paper}]{\ref{paper}}",
        r"\AtBeginBiblist{\ref{paper}}{\ref{paper}}",
        r"\AtEveryBiblistitem{\ref{paper}}{\ref{paper}}",
    ] {
        let source = format!(r"\label{{paper}}{call}");
        let (ws, uri) = setup(&source);
        let text = Text::new(source.clone());
        let data_at = source.find(r"\ref{paper}").unwrap() + 7;
        assert!(
            features::definitions(&ws, &uri, text.position(data_at)).is_empty(),
            "{source}"
        );
        let code_at = source.rfind("paper").unwrap() + 2;
        assert_eq!(
            features::definitions(&ws, &uri, text.position(code_at)).len(),
            1,
            "{source}"
        );
        assert!(
            features::rename(&ws, &uri, Position::new(0, 9), "updated").is_err(),
            "{source}"
        );
    }
    let source = r"\label{paper}\AtEveryEntrykey{\ref{paper}}{\ref{paper}}{\ref{paper}}\ref{paper}";
    let (ws, uri) = setup(source);
    assert_eq!(
        features::rename(&ws, &uri, Position::new(0, 9), "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&uri]
            .len(),
        5
    );
}

#[test]
fn callback_aliases_and_redefinitions_share_the_existing_project_boundaries() {
    for call in [
        r"\def\hook{\AtEveryBibitem}\hook{\ifcategory{selected}{}{}}",
        r"\def\hook{\AtEveryBibitem}\csuse{hook}{\ifcategory{selected}{}{}}",
        r"\csuse{AtEveryBibitem}{\ifcategory{selected}{}{}}",
        r"\csname AtEveryBibitem\endcsname{\ifcategory{selected}{}{}}",
    ] {
        let source = format!(r"\DeclareBibliographyCategory{{selected}}{call}");
        let (ws, uri) = setup(&source);
        assert_eq!(
            features::rename(&ws, &uri, Position::new(0, 32), "updated")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            2,
            "{source}"
        );
    }
    for call in [
        r"\def\AtEveryBibitem#1{}\AtEveryBibitem{\ifcategory{selected}{}{}}",
        r"\def\take#1{}\take{\AtEveryBibitem{\ifcategory{selected}{}{}}}",
        r"\unknown{\AtEveryBibitem{\ifcategory{selected}{}{}}}",
    ] {
        let source = format!(r"\DeclareBibliographyCategory{{selected}}{call}");
        let (ws, uri) = setup(&source);
        assert!(
            features::rename(&ws, &uri, Position::new(0, 32), "updated").is_err(),
            "{source}"
        );
    }
}

#[test]
fn hook_snippets_keep_existing_code_and_preserve_optional_defaults() {
    for name in SIMPLE.iter().copied().chain([
        "AtDataInput",
        "AtBeginBiblist",
        "AtEveryBiblistitem",
        "AtEveryEntrykey",
    ]) {
        for tail in ["", "{existing}"] {
            let source = format!("\\{name}{tail}");
            let (ws, uri) = setup(&source);
            let at = Position::new(0, name.len() as u32 + 1);
            let item = features::completion(&ws, &uri, at)
                .into_iter()
                .find(|item| item.label == name)
                .unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            if tail.is_empty() {
                assert!(edit.new_text.contains("${1:"), "{name}: {}", edit.new_text);
                assert!(!edit.new_text.contains('['), "{name}: {}", edit.new_text);
            } else {
                assert_eq!(apply(&source, vec![edit]), source);
            }
        }
    }
}

#[test]
fn callbacks_shared_between_projects_require_the_same_code_meaning() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\input{common}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\def\AtEveryBibitem#1{}\input{common}\input{shared}",
        ),
        ("common.tex", r"\DeclareBibliographyCategory{selected}"),
        ("shared.tex", r"\AtEveryBibitem{\ifcategory{selected}{}{}}"),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    assert!(features::definitions(&ws, &shared, Position::new(0, 30)).is_empty());
    assert!(features::rename(&ws, &common, Position::new(0, 32), "updated").is_err());
    ws.open(
        b,
        r"\documentclass{article}\input{common}\input{shared}".into(),
        2,
    );
    assert_eq!(
        features::definitions(&ws, &shared, Position::new(0, 30)).len(),
        1
    );
    assert_eq!(
        features::rename(&ws, &common, Position::new(0, 32), "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .len(),
        2
    );
}

#[test]
#[ignore = "requires pdflatex, BibTeX and biblatex"]
fn real_biblatex_callbacks_keep_category_queries_after_entry_key_rename() {
    let dir = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = Url::from_file_path(dir.path().join("refs.bib")).unwrap();
    let source = "\\documentclass{article}\n\\usepackage[backend=bibtex,style=numeric]{biblatex}\n\\addbibresource{refs.bib}\n\\DeclareBibliographyCategory{selected}\n\\addtocategory{selected}{alpha}\n\\AtDataInput[book]{\\ifentrycategory{alpha}{selected}{\\typeout{DATA-HOOK-OK}}{\\errmessage{Data hook lost entry}}}\n\\AtEveryCitekey{\\ifentrycategory{alpha}{selected}{\\typeout{CITE-HOOK-OK}}{\\errmessage{Cite hook lost entry}}}\n\\AtEveryEntrykey{\\typeout{ENTRYKEY-HOOK:#1}}{\\typeout{APPENDED-HOOK-OK}}{\\errmessage{Hook append failed}}\n\\begin{document}\n\\cite{alpha}\n\\printbibliography\n\\end{document}";
    let completed = complete_input_hook_type(&source.replacen("[book]", "[BO¦ook]", 1));
    let source = completed.as_str();
    let bibliography =
        "@book{alpha,author={Doe, Jane},title={Hook book},year={2026},publisher={Example}}";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(bib.to_file_path().unwrap(), bibliography).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let position = Text::new(source.into()).position(source.rfind("alpha").unwrap() + 2);
    let changes = features::rename(&ws, &main, position, "updated")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    let updated = apply(source, changes[&main].clone());
    assert!(!updated.contains("alpha"), "{updated}");
    std::fs::write(main.to_file_path().unwrap(), updated).unwrap();
    std::fs::write(
        bib.to_file_path().unwrap(),
        apply(bibliography, changes[&bib].clone()),
    )
    .unwrap();
    for program in ["pdflatex", "bibtex", "pdflatex", "pdflatex"] {
        let mut command = std::process::Command::new(program);
        if program == "pdflatex" {
            command.args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ]);
        } else {
            command.arg("main");
        }
        let output = command.current_dir(dir.path()).output().unwrap();
        assert!(
            output.status.success(),
            "{program}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    for marker in [
        "DATA-HOOK-OK",
        "CITE-HOOK-OK",
        "ENTRYKEY-HOOK:updated",
        "APPENDED-HOOK-OK",
    ] {
        assert!(log.contains(marker), "{marker}: {log}");
    }
    assert!(!log.contains("undefined on input line"), "{log}");
}
fn complete_input_hook_type(marked: &str) -> String {
    let source = marked.replace('¦', "");
    let (ws, uri) = setup(&source);
    let at = ws.documents[&uri].text.position(marked.find('¦').unwrap());
    let item = features::completion(&ws, &uri, at)
        .into_iter()
        .find(|item| item.label == "book")
        .unwrap_or_else(|| panic!("missing book: {marked}"));
    assert_eq!(item.kind, Some(CompletionItemKind::CLASS));
    assert!(matches!(&item.documentation,Some(Documentation::String(value)) if !value.is_empty()));
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
        panic!()
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    let mut edits = item.additional_text_edits.unwrap_or_default();
    edits.push(edit);
    apply(&source, edits)
}

#[test]
fn input_hook_type_completions_and_hover_follow_direct_named_and_forwarding_calls() {
    for call in [
        r"\AtDataInput",
        r"\csuse{AtDataInput}",
        r"\csname AtDataInput\endcsname",
        r"\oninput",
    ] {
        let prefix = r"\def\oninput{\AtDataInput}😀 ";
        let marked = format!("{prefix}{call}[BO¦ook]{{code}}");
        let completed = complete_input_hook_type(&marked);
        assert_eq!(completed, format!("{prefix}{call}[book]{{code}}"));
        let (ws, uri) = setup(&completed);
        let at = ws.documents[&uri]
            .text
            .position(completed.rfind("book").unwrap() + 2);
        let hover = features::hover(&ws, &uri, at).unwrap();
        assert!(format!("{:?}", hover.contents).contains("book"));
        assert!(features::definitions(&ws, &uri, at).is_empty());
        assert!(features::prepare_rename(&ws, &uri, at).unwrap().is_none());
    }
}

#[test]
fn input_hook_type_edits_preserve_comments_unfinished_headers_and_following_code() {
    for (marked, expected) in [
        (
            "\\AtDataInput[% select [ ]😀\r\n BO¦ook % tail\r\n]{\\label{kept}}",
            "\\AtDataInput[% select [ ]😀\r\nbook% tail\r\n]{\\label{kept}}",
        ),
        (
            "\\AtDataInput[\r\n BO¦ook \r\n]{code}",
            "\\AtDataInput[book]{code}",
        ),
        (
            "\\AtDataInput[bo¦\n\\label{kept}",
            "\\AtDataInput[book\n\\label{kept}",
        ),
    ] {
        assert_eq!(complete_input_hook_type(marked), expected, "{marked}");
    }
}

#[test]
fn input_hook_type_candidates_exclude_code_data_literals_and_uncertain_meanings() {
    for marked in [
        r"\AtDataInput[book]{bo¦ok}",
        r"\AtDataInput[book]{code}bo¦ok",
        r"\def\AtDataInput#1{}\AtDataInput[bo¦ok]{code}",
        r"\def\take#1{}\take{\AtDataInput[bo¦ok]{code}}",
        r"\noexpand\AtDataInput[bo¦ok]{code}",
        r"\detokenize{\AtDataInput[bo¦ok]{code}}",
        r"\verb|\AtDataInput[bo¦ok]{code}|",
        r"\let\oninput\AtDataInput\oninput[bo¦ok]{code}",
        "\\AtDataInput[b% split\r\no¦ok]{code}",
        "\\AtDataInput[% bo¦ok\r\nbook]{code}",
        r"\AtDataInput[outer[bo¦ok]{code}",
    ] {
        let (ws, uri) = setup(&marked.replace('¦', ""));
        let at = ws.documents[&uri].text.position(marked.find('¦').unwrap());
        assert!(
            !features::completion(&ws, &uri, at)
                .iter()
                .any(|item| item.label == "book"),
            "{marked}"
        );
    }
    let marked = r"\AtDataInput[bo¦ok]{code}";
    let (mut ws, uri) = setup(&marked.replace('¦', ""));
    ws.index_truncated.insert("maxFiles".into());
    let at = ws.documents[&uri].text.position(marked.find('¦').unwrap());
    assert!(!features::completion(&ws, &uri, at)
        .iter()
        .any(|item| item.label == "book"));
}

#[test]
fn shared_input_hook_selectors_require_consistent_consumer_meanings_and_refresh() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\def\oninput{\AtDataInput}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\def\oninput#1{}\input{shared}",
        ),
        ("other.tex", r"\documentclass{article}\def\AtDataInput#1{}"),
        ("shared.tex", r"\oninput[book]{code}"),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    let at = Position::new(0, 11);
    assert!(!features::completion(&ws, &shared, at)
        .iter()
        .any(|item| item.label == "book"));
    ws.open(
        b,
        r"\documentclass{article}\def\oninput{\AtDataInput}\input{shared}".into(),
        8,
    );
    assert!(features::completion(&ws, &shared, at)
        .iter()
        .any(|item| item.label == "book"));
}
