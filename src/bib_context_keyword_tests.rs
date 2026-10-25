use super::*;

fn keyword_setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let (dir, ws, uri) = category_setup(body, "");
    let path = dir.path().join("main.bbl");
    let bbl = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            r"\entry{a}{book}{}{}",
            r"\entry{a}{book}{}{}\keyw{topic,second,quantum gravity}",
        )
        .replace(r"\entry{z}{book}{}{}", r"\entry{z}{book}{}{}\keyw{second}");
    std::fs::write(path, bbl).unwrap();
    (dir, ws, uri)
}

#[test]
fn automatic_keywords_select_actual_bbl_members_before_categories_and_entries() {
    for (body, expected) in [
        (
            r"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a,z}",
            vec!["→ A3", "→ B9"],
        ),
        (
            r"\cite{a}\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws[labelprefix=B]{second}\cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws[labelprefix=B]{second}\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws*[labelprefix=B]{second}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws*[labelprefix=B]{topic}\cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextcats*[labelprefix=B]{x}\cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextcats[labelprefix=B]{x}\assignrefcontextentries*[labelprefix=A]{a}\cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{absent,*}\cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextkeyws*[labelprefix=A]{topic}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\cite{a}",
            vec!["→ A3", "→ B8", "→ A3"],
        ),
        (
            r"{\assignrefcontextkeyws[labelprefix=A]{topic}}\cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextkeyws[labelprefix=A]{}\cite{a,z}",
            vec!["→ B8", "→ B9"],
        ),
    ] {
        let (_dir, ws, uri) = keyword_setup(body);
        assert_eq!(hints(&ws, &uri), expected, "{body}");
    }
}

#[test]
fn automatic_keywords_keep_native_spaces_comments_and_literal_stars() {
    for (target, expected) in [
        ("quantum   gravity", "→ A3"),
        ("quantum\t\tgravity", "→ A3"),
        ("quantum\ngravity", "→ A3"),
        ("to% join\npic", "→ A3"),
        ("to% 中文注释\r\npic", "→ A3"),
        ("quantum % note\r\n  gravity", "→ A3"),
        (" topic", "→ A3"),
        ("topic ", "→ B8"),
        ("  topic  , second  ", "→ B8"),
        ("quantumgravity", "→ B8"),
    ] {
        let (_dir, ws, uri) = keyword_setup(&format!(
            r"\assignrefcontextkeyws[labelprefix=A]{{{target}}}\cite{{a}}"
        ));
        assert_eq!(hints(&ws, &uri), [expected], "{target:?}");
    }
    let (dir, ws, uri) = keyword_setup(r"\assignrefcontextkeyws[labelprefix=A]{*}\cite{a,z}");
    let path = dir.path().join("main.bbl");
    let bbl = std::fs::read_to_string(&path)
        .unwrap()
        .replace("topic,second,quantum gravity", "*");
    std::fs::write(path, bbl).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ A3", "→ B9"]);
}

#[test]
fn automatic_unicode_keywords_keep_literal_identity_without_normalizing_assignments() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for (member, assignment, expected) in [
            ("café", "café", "→ A3"),
            ("βeta", "βeta", "→ A3"),
            ("主题", "主题", "→ A3"),
            ("主题２", "主题２", "→ A3"),
            ("café", "cafe\u{301}", "→ B8"),
            ("cafe\u{301}", "cafe\u{301}", "→ A3"),
            ("cafe\u{301}", "café", "→ B8"),
            ("café", "Café", "→ B8"),
            ("café", " café", "→ A3"),
            ("café", "café ", "→ B8"),
            ("主题", "主% café\r\n题", "→ A3"),
            ("主题 文献", "主题\t\t文献", "→ A3"),
            ("主题 文献", "主题\n文献", "→ A3"),
        ] {
            let (dir, mut ws, uri) = keyword_setup(&format!(
                r"\assignrefcontextkeyws[labelprefix=A]{{{assignment}}}\cite{{a,z}}"
            ));
            ws.config.build.engine = crate::config::Engine::from_program(engine);
            let path = dir.path().join("main.bbl");
            let bbl = std::fs::read_to_string(&path)
                .unwrap()
                .replace("topic,second,quantum gravity", member);
            std::fs::write(path, bbl).unwrap();
            assert_eq!(
                hints(&ws, &uri),
                [expected, "→ B9"],
                "{engine} {member:?} {assignment:?}"
            );
        }
    }
}

#[test]
fn unicode_keyword_members_are_loaded_even_for_ascii_assignments_and_entry_overrides() {
    let (dir, mut ws, uri) = keyword_setup(
        r"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a,z}\assignrefcontextentries[labelprefix=B]{a}\cite{a}",
    );
    let path = dir.path().join("main.bbl");
    let bbl = std::fs::read_to_string(&path)
        .unwrap()
        .replace("topic,second,quantum gravity", "主题,topic,café");
    std::fs::write(&path, &bbl).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ A3", "→ B9", "→ B8"]);
    // Membership is the union of all loaded contexts, including Unicode names.
    let source = ws.documents[&uri].text.source.replace("{topic}", "{主题}");
    ws.open(uri.clone(), source, 9);
    std::fs::write(&path, bbl.replacen("主题,topic,café", "other", 1)).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ A3", "→ B9", "→ B8"]);
    std::fs::write(&path, bbl.replace("主题,topic,café", "other")).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ B8", "→ B9", "→ B8"]);
}

#[test]
fn unicode_keyword_whitespace_controls_and_expansion_remain_guarded() {
    for invalid in [
        "主\u{a0}题",
        "主\u{2003}题",
        "主\u{85}题",
        "主\u{200b}题",
        "主\u{feff}题",
        "主\0题",
        "主\u{1b}题",
        "主题😀",
        "主{题}",
        r"主\topic",
        "主#1",
        "主~题",
        "主^^题",
    ] {
        let (dir, ws, uri) = keyword_setup(&format!(
            r"\assignrefcontextkeyws[labelprefix=A]{{{invalid}}}\cite{{a}}"
        ));
        assert!(hints(&ws, &uri).is_empty(), "source {invalid:?}");
        let source = ws.documents[&uri].text.source.replace(invalid, "topic");
        let mut ws = ws;
        ws.open(uri.clone(), source, 8);
        let path = dir.path().join("main.bbl");
        let bbl = std::fs::read_to_string(&path)
            .unwrap()
            .replace("topic,second,quantum gravity", invalid);
        std::fs::write(path, bbl).unwrap();
        assert!(hints(&ws, &uri).is_empty(), "compiled {invalid:?}");
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and biber with biblatex"]
fn native_unicode_keyword_hints_match_literal_names_and_complete_bbl_in_three_engines() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for (keyword, first) in [
            ("café", "A1"),
            ("βeta", "A1"),
            ("主题", "A1"),
            ("主题２", "A1"),
            ("cafe\u{301}", "B1"),
        ] {
            let body = format!(
                r"\assignrefcontextkeyws[labelprefix=A]{{{keyword}}}\cite{{a,z}}\begin{{refcontext}}[labelprefix=A]\nocite{{a,z}}\printbibliography\end{{refcontext}}\begin{{refcontext}}[labelprefix=B]\nocite{{a,z}}\printbibliography\end{{refcontext}}"
            );
            let library = format!(
                "@book{{a,title={{Alpha}},keywords={{{keyword}}}}} @book{{z,title={{Zulu}}}}"
            );
            native_assignment_engine(
                &[(body.as_str(), vec![first, "B2"])],
                &[("data.bib", library.as_str())],
                "",
                engine,
            );
        }
    }
}

#[test]
fn automatic_keywords_use_union_of_loaded_contexts_but_keep_sections_separate() {
    let (dir, mut ws, uri) = keyword_setup(
        r"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}\begin{refsection}\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}\end{refsection}",
    );
    let a_list = list(A, "A", &["a"]).replace(r"\endentry", r"\keyw{topic}\endentry");
    let b_list = list(B, "B", &["a"]).replace(r"\endentry", r"\keyw{other}\endentry");
    std::fs::write(dir.path().join("main.bbl"), format!(r"{PROLOGUE}\refsection{{0}}{a_list}{b_list}\endrefsection\refsection{{1}}{}{}\endrefsection\endinput",list(A,"A",&["a"]),list(B,"B",&["a"]))).unwrap();
    let aux = number("a", A, 3)
        + &number("a", B, 8)
        + &default("a", B)
        + &number("a", A, 4).replace("{0}", "{1}")
        + &number("a", B, 9).replace("{0}", "{1}")
        + &default("a", B).replace("{0}", "{1}");
    std::fs::write(dir.path().join("main.aux"), aux).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ A3", "→ B9"]);
    // No source keyword field is needed: Biber's loaded output is authoritative.
    let bib = uri.join("data.bib").unwrap();
    ws.open(bib, r"@book{a,title={Alpha},keywords={other}}".into(), 7);
    assert_eq!(hints(&ws, &uri), ["→ A3", "→ B9"]);
}

#[test]
fn automatic_keywords_guard_incomplete_compiled_data_without_breaking_entry_only_hints() {
    for invalid in [
        r"\keyw{\dynamic}",
        r"\keyw{{topic}}",
        r"\keyw{topic",
        r"\unknown{topic}",
    ] {
        let (dir, ws, uri) = keyword_setup(r"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}");
        let path = dir.path().join("main.bbl");
        let bbl = std::fs::read_to_string(&path)
            .unwrap()
            .replace(r"\keyw{topic,second,quantum gravity}", invalid);
        std::fs::write(path, bbl).unwrap();
        assert!(hints(&ws, &uri).is_empty(), "{invalid}");
    }
    let (dir, ws, uri) = keyword_setup(r"\assignrefcontextentries[labelprefix=A]{a}\cite{a}");
    let path = dir.path().join("main.bbl");
    let bbl = std::fs::read_to_string(&path)
        .unwrap()
        .replace(r"\keyw{topic,second,quantum gravity}", r"\keyw{\dynamic}");
    std::fs::write(path, bbl).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_automatic_keywords_match_actual_labels_and_complete_bbl() {
    // strong
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // before-after
    native_assignment_preamble(
        &[(
            r###"\cite{a}\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1", "A1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // last-matching-keyword
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws[labelprefix=B]{second}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // repeated-keyword
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws[labelprefix=B]{second}\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // weak-after-strong
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws*[labelprefix=B]{second}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1", "A1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // strong-same-keyword
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws*[labelprefix=B]{topic}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // category-precedence
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextcats*[labelprefix=B]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // entry-precedence
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextcats[labelprefix=B]{x}\assignrefcontextentries*[labelprefix=A]{a}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // missing-and-star
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{absent,*}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // weak-context-restoration
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws*[labelprefix=A]{topic}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B1", "A1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // multiword
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{quantum   gravity}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // inherited
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1"],
        )],
        &[(
            "data.bib",
            r###"@xdata{common,keywords={topic}} @book{a,title={Alpha},xdata={common}} @book{z,title={Zulu}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // global-order-across-sections
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws[labelprefix=B]{second}\cite{a}\begin{refsection}\assignrefcontextkeyws[labelprefix=A]{topic}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}\end{refsection}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1", "A1"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // keyword-trailing-space
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{topic }\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // keyword-leading-space
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{ topic}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // literal-star
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{*}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={*}} @book{z,title={Zulu},keywords={topic}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // empty-list
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // space-distinction
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{quantumgravity}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // linebreak
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{quantum
gravity}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // comment-join
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{to% join
pic}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // comment-word-space
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{quantum % join
  gravity}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // tabs
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{quantum		gravity}\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["A1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    // outer-csv-spaces
    native_assignment_preamble(
        &[(
            r###"\assignrefcontextkeyws[labelprefix=A]{  topic  , second  }\cite{a,z}\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}"###,
            vec!["B1", "B2"],
        )],
        &[(
            "data.bib",
            r###"@book{a,title={Alpha},keywords={topic,second,quantum gravity}} @book{z,title={Zulu},keywords={second}}"###,
        )],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    let printing = r"\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}";
    let named =
        format!(r"\csuse{{assignrefcontextkeyws}}[labelprefix=A]{{topic}}\cite{{a}}{printing}");
    let weak_named = format!(
        r"\csname assignrefcontextkeyws\endcsname*[labelprefix=A]{{topic}}\cite{{a}}\begin{{refcontext}}[labelprefix=A]\cite{{a}}\end{{refcontext}}{printing}"
    );
    let forwarded = format!("\\assign*[labelprefix=A]{{to% 中文注释\r\npic}}\\cite{{a}}{printing}");
    native_assignment_preamble(
        &[
            (named.as_str(), vec!["A1"]),
            (weak_named.as_str(), vec!["A1", "B1"]),
            (forwarded.as_str(), vec!["A1"]),
        ],
        &[(
            "data.bib",
            r"@book{a,title={Alpha},keywords={topic}} @book{z,title={Zulu}}",
        )],
        r"\def\assign{\innerassign}\def\innerassign{\assignrefcontextkeyws}",
    );
}

#[test]
fn keyword_assignments_have_native_parameter_help_and_preserve_keyword_namespaces() {
    for (marked, active) in [
        (r"\assignrefcontextkeyws[lab¦elprefix=A]{topic}", 0),
        (r"\assignrefcontextkeyws*[]{to¦pic}", 1),
        (r"\csuse{assignrefcontextkeyws}*[]{to¦pic}", 1),
        (r"\csname assignrefcontextkeyws\endcsname[]{to¦pic}", 1),
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (_dir, ws, uri) = keyword_setup(&source);
        let full = &ws.documents[&uri].text;
        let offset = full.source.find(&source).unwrap() + at;
        let help = crate::signature::help(&ws, &uri, full.position(offset)).unwrap();
        assert!(
            help.signatures[0].label.ends_with("[options]{keywords}"),
            "{marked}: {help:?}"
        );
        assert_eq!(help.active_parameter, Some(active));
        if active == 1 {
            assert!(crate::features::definitions(&ws, &uri, full.position(offset)).is_empty());
            assert!(
                crate::features::prepare_rename(&ws, &uri, full.position(offset))
                    .unwrap()
                    .is_none()
            );
        }
    }
    let name = "assignrefcontextkeyws";
    let source = format!("\\{name}");
    let (_dir, mut ws, uri) = keyword_setup(&source);
    ws.snippets = true;
    let pos = ws.documents[&uri]
        .text
        .position(ws.documents[&uri].text.source.find(&source).unwrap() + source.len());
    let items = crate::features::completion(&ws, &uri, pos);
    let item = items.iter().find(|item| item.label == name).unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(
        edit.new_text,
        "assignrefcontextkeyws[${1:options}]{${2:keywords}}"
    );
}

#[test]
fn automatic_keywords_keep_named_invocations_and_executed_preamble_forwards() {
    for (preamble, call) in [
        ("", r"\csuse{assignrefcontextkeyws}"),
        ("", r"\csname assignrefcontextkeyws\endcsname"),
        (r"\def\assign{\assignrefcontextkeyws}", r"\assign"),
        (
            r"\newcommand\assign{\innerassign}\def\innerassign{\assignrefcontextkeyws}",
            r"\assign",
        ),
    ] {
        let (_dir, mut ws, uri) =
            keyword_setup(&format!(r"{call}*[labelprefix=A]{{topic}}\cite{{a}}"));
        let source = ws.documents[&uri].text.source.replace(
            r"\begin{document}",
            &format!(r"{preamble}\begin{{document}}"),
        );
        ws.open(uri.clone(), source, 6);
        assert_eq!(hints(&ws, &uri), ["→ A3"], "{preamble} {call}");
    }
    for body in [
        r"\newcommand\later{\assignrefcontextkeyws[labelprefix=A]{topic}}\later\cite{a}",
        r"\AtBeginDocument{\assignrefcontextkeyws[labelprefix=A]{topic}}\cite{a}",
        r"\AddToHook{unused}[mine]{\assignrefcontextkeyws[labelprefix=A]{topic}}\cite{a}",
        r"\AddToHookNextWithArguments{unused}{\assignrefcontextkeyws[labelprefix=A]{topic}}\cite{a}",
        r"\def\forward{\AddToHook}\forward{unused}{\assignrefcontextkeyws[labelprefix=A]{topic}}\cite{a}",
        r"\assignrefcontextkeyws[labelprefix=A]{\dynamic}\cite{a}",
        r"\assignrefcontextkeyws{topic}\cite{a}",
        "\\assignrefcontextkeyws[labelprefix=A]{topic\n\nsecond}\\cite{a}",
        r"\let\saved\assignrefcontextkeyws\saved[labelprefix=A]{topic}\cite{a}",
    ] {
        let (_dir, ws, uri) = keyword_setup(body);
        assert!(hints(&ws, &uri).is_empty(), "{body}");
    }
}

#[test]
fn automatic_keywords_preserve_spaces_in_compiled_members_too() {
    let (dir, mut ws, uri) = keyword_setup(r"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}");
    let path = dir.path().join("main.bbl");
    let bbl = std::fs::read_to_string(&path)
        .unwrap()
        .replace("topic,second,quantum gravity", "topic ");
    std::fs::write(path, bbl).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ B8"]);
    let source = ws.documents[&uri]
        .text
        .source
        .replace("{topic}", "{topic }");
    ws.open(uri.clone(), source, 6);
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
fn automatic_keywords_keep_global_order_and_every_physical_execution() {
    let body = r"\assignrefcontextkeyws[labelprefix=A]{topic}\assignrefcontextkeyws[labelprefix=B]{second}\cite{a}\begin{refsection}\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}\end{refsection}\cite{a}";
    let (dir, mut ws, uri) = keyword_setup(body);
    let path = dir.path().join("main.bbl");
    let bbl = std::fs::read_to_string(&path).unwrap();
    let section = bbl
        .split_once(r"\refsection{0}")
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0;
    std::fs::write(
        path,
        format!(
            r"{}\refsection{{1}}{section}\endrefsection\endinput",
            bbl.strip_suffix(r"\endinput").unwrap()
        ),
    )
    .unwrap();
    let path = dir.path().join("main.aux");
    let aux = std::fs::read_to_string(&path).unwrap()
        + &number("a", A, 4).replace("{0}", "{1}")
        + &number("a", B, 9).replace("{0}", "{1}")
        + &default("a", B).replace("{0}", "{1}");
    std::fs::write(path, aux).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ B8", "→ A4", "→ A3"]);
    let source = ws.documents[&uri].text.source.replace(
        body,
        r"\input{chapter}\assignrefcontextkeyws[labelprefix=A]{topic}\input{chapter}",
    );
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{a}").unwrap();
    ws.open(uri.clone(), source.clone(), 6);
    let child = uri.join("chapter.tex").unwrap();
    assert!(hints(&ws, &child).is_empty());
    ws.open(
        uri.clone(),
        source.replace(
            r"\input{chapter}\assignrefcontextkeyws",
            r"\assignrefcontextkeyws",
        ),
        7,
    );
    assert_eq!(hints(&ws, &child), ["→ A3"]);
}

#[test]
fn automatic_keywords_bound_member_index_expansion_without_affecting_entry_labels() {
    let (dir, mut ws, uri) = keyword_setup(r"\assignrefcontextkeyws[labelprefix=A]{topic}\cite{a}");
    let names = (0..256)
        .map(|index| format!("tag{index:03}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut entries = list(A, "A", &["a"])
        .replace(r"\enddatalist", "")
        .replace(r"\endentry", r"\keyw{topic}\endentry");
    for index in 0..512 {
        let key = format!("k{index:04}{}", "x".repeat(123));
        entries += &format!(
            r"\entry{{{key}}}{{book}}{{}}{{}}\field{{labelprefix}}{{A}}\keyw{{{names}}}\endentry"
        );
    }
    entries += r"\enddatalist";
    let bbl = compiled(entries);
    assert!(bbl.len() < 2 * 1024 * 1024);
    std::fs::write(dir.path().join("main.bbl"), bbl).unwrap();
    assert!(hints(&ws, &uri).is_empty());
    let source = ws.documents[&uri]
        .text
        .source
        .replace("assignrefcontextkeyws", "assignrefcontextentries")
        .replace("{topic}", "{a}");
    ws.open(uri.clone(), source, 6);
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
fn keyword_assignment_parameters_do_not_create_executing_reference_hints() {
    let source = r"\assignrefcontextkeyws[]{\ref{fake}}\label{real}\ref{real}";
    let (dir, ws, uri) = keyword_setup(source);
    let path = dir.path().join("main.aux");
    let aux =
        std::fs::read_to_string(&path).unwrap() + r"\newlabel{fake}{{4}{1}}\newlabel{real}{{5}{2}}";
    std::fs::write(path, aux).unwrap();
    assert_eq!(hints(&ws, &uri), ["→ 5, p. 2"]);
}
