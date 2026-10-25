use latex_lsp::{navigation, workspace::Document};
use lsp_types::{
    FoldingRange, FoldingRangeCapability, FoldingRangeClientCapabilities, FoldingRangeKind, Url,
};

fn document(extension: &str, source: &str) -> Document {
    Document::new(
        &Url::parse(&format!("file:///folding/main.{extension}")).unwrap(),
        source.into(),
        Some(1),
    )
}

fn lines(folds: &[FoldingRange]) -> Vec<(u32, u32)> {
    folds
        .iter()
        .map(|fold| (fold.start_line, fold.end_line))
        .collect()
}

#[test]
fn bibtex_folds_complete_constructs_using_bibliography_grammar() {
    let source = concat!(
        "% bibliography\n",                                 // 0
        "  % notes\n",                                      // 1
        "@book{first,\n",                                   // 2
        "  title = {A ) and \"quoted\" title},\n",          // 3
        "  note = \"literal % and } and @book{fake,}\",\n", // 4
        "}\n",                                              // 5
        "@article(second)key,\n",                           // 6
        "  title = \"A ) parenthesis\",\n",                 // 7
        ")\n",                                              // 8
        "@STRING(\n",                                       // 9
        "  journal = \"A ) title\"\n",                      // 10
        ")\n",                                              // 11
        "@preamble{\n",                                     // 12
        "  \"a } character\" # {literal}\n",                // 13
        "}\n",                                              // 14
        "@Comment{\n",                                      // 15
        "  @book{fake, title={data}}\n",                    // 16
        "}\n",                                              // 17
        "@book{incomplete,\n",                              // 18
        "  title = {unfinished}\n",                         // 19
    );
    let doc = document("bib", source);
    let folds = navigation::folding_ranges(&doc, None);
    assert_eq!(
        lines(&folds),
        [(0, 1), (2, 5), (6, 8), (9, 11), (12, 14), (15, 17)]
    );
    assert_eq!(folds[0].kind, Some(FoldingRangeKind::Comment));
    assert_eq!(folds[5].kind, Some(FoldingRangeKind::Comment));
    assert_eq!(
        doc.syntax
            .citations
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "second)key", "incomplete"]
    );
}

#[test]
fn bibtex_does_not_scan_tex_groups_or_formula_delimiters_in_values_or_free_text() {
    let doc = document(
        "bib",
        concat!(
            "free text {\ntext\n}\n",
            "@book(one,\n title = {\n $\n \\[\n }\n)\n",
            "@book{two, title={single line}}\n",
        ),
    );
    assert_eq!(lines(&navigation::folding_ranges(&doc, None)), [(3, 8)]);
    assert!(navigation::folding_ranges(&document("bst", "{\n}\n$$\na\n$$"), None).is_empty());
}

#[test]
fn folds_all_four_multiline_formula_delimiters_and_preserves_group_folds() {
    let doc = document(
        "tex",
        concat!(
            "前😀 \\[\r\n",        // 0
            "x + \\text{$y$}\r\n", // 1
            "\\] 尾\r\n",          // 2
            "$$\r\n",              // 3
            "x_{\r\n",             // 4
            "y}\r\n",              // 5
            "$$\r\n",              // 6
            "\\(\r\n",             // 7
            "z\r\n",               // 8
            "\\)\r\n",             // 9
            "$\r\n",               // 10
            "w\r\n",               // 11
            "$\r\n",               // 12
            "$inline$ \\(inline\\) $$inline$$ \\[inline\\]\r\n",
        ),
    );
    let folds = navigation::folding_ranges_for_client(&doc, &Default::default());
    assert_eq!(lines(&folds), [(0, 2), (3, 6), (4, 5), (7, 9), (10, 12)]);
    assert_eq!(folds[0].start_character, Some(4));
    assert_eq!(folds[0].end_character, Some(2));
}

#[test]
fn formulas_skip_comments_verbatim_urls_literal_conversions_and_token_operands() {
    let doc = document(
        "tex",
        concat!(
            "% \\[\n% \\]\n",
            "\\verb|$$|\n",
            "\\url{https://example.org/$\n$}\n",
            "\\detokenize{\\[\nx\n\\]}\n",
            "\\begin{verbatim}\n$$\nx\n$$\n\\end{verbatim}\n",
            "\\let\\open=\\[\n\\let\\close=\\]\n",
            "\\meaning\\[\n\\meaning\\]\n",
            "\\let\\shift=$\n\\meaning$\n",
            "\\let\\brace={\n\\meaning}\n",
            "\\[\nx\n% \\] $$\n\\]\n",
        ),
    );
    let folds = navigation::folding_ranges(&doc, None);
    assert_eq!(lines(&folds), [(0, 1), (3, 4), (8, 12), (21, 24)]);
}

#[test]
fn malformed_math_pairs_do_not_fold_across_groups_or_different_delimiters() {
    for source in [
        "\\[\nx\n\\)",
        "$$\nx\n$",
        "{\\[\nx\n}\n\\]",
        "{$\nx\n}\n$",
        "\\]\nx\n\\[",
        "\\[\nx\n",
    ] {
        let folds = navigation::folding_ranges(&document("tex", source), None);
        // A real brace group may still fold, but never the malformed formula.
        assert!(
            !folds.iter().any(|fold| fold.end_line == 3),
            "{source:?}: {folds:?}"
        );
        if !source.starts_with('{') {
            assert!(folds.is_empty(), "{source:?}: {folds:?}");
        }
    }
}

#[test]
fn clients_negotiate_columns_labels_limits_and_deduplication() {
    let doc = document("tex", "😀 {\\[\r\nx\r\n\\]}\r\n\\section{Next}\r\n尾😀\r\n");
    let capabilities = FoldingRangeClientCapabilities {
        folding_range: Some(FoldingRangeCapability {
            collapsed_text: Some(true),
        }),
        ..Default::default()
    };
    let folds = navigation::folding_ranges_for_client(&doc, &capabilities);
    assert_eq!(lines(&folds), [(0, 2), (0, 2), (3, 4)]);
    assert_eq!(folds[0].start_character, Some(3));
    assert_eq!(folds[1].start_character, Some(4));
    assert_eq!(folds[0].end_character, Some(3));
    assert_eq!(folds[1].end_character, Some(2));
    assert_eq!(folds[0].collapsed_text.as_deref(), Some("{\\[ …"));
    assert_eq!(folds[2].end_character, Some(3));
    let mut line_capabilities = capabilities.clone();
    line_capabilities.line_folding_only = Some(true);
    let line_folds = navigation::folding_ranges_for_client(&doc, &line_capabilities);
    assert_eq!(lines(&line_folds), [(0, 2), (3, 4)]);
    assert!(line_folds
        .iter()
        .all(|fold| fold.start_character.is_none() && fold.end_character.is_none()));
    line_capabilities.range_limit = Some(1);
    assert_eq!(
        navigation::folding_ranges_for_client(&doc, &line_capabilities).len(),
        1
    );
    line_capabilities.range_limit = Some(0);
    assert!(navigation::folding_ranges_for_client(&doc, &line_capabilities).is_empty());
    assert!(
        navigation::folding_ranges_for_client(&doc, &Default::default())
            .iter()
            .all(|fold| fold.collapsed_text.is_none())
    );
}

#[test]
fn collapsed_labels_are_bounded_and_do_not_split_unicode_characters() {
    let doc = document(
        "bib",
        &format!("@book{{{},\n title={{x}}\n}}", "😀".repeat(100)),
    );
    let folds = navigation::folding_ranges_for_client(
        &doc,
        &FoldingRangeClientCapabilities {
            folding_range: Some(FoldingRangeCapability {
                collapsed_text: Some(true),
            }),
            ..Default::default()
        },
    );
    assert_eq!(
        folds[0].collapsed_text.as_ref().unwrap().chars().count(),
        82
    );
    assert_eq!(folds[0].end_character, Some(1));
}

#[test]
fn comment_columns_exclude_crlf_and_section_folds_exclude_the_next_heading() {
    let doc = document(
        "tex",
        "  % 😀\r\n\t% 中😀\r\n\\section{First}\r\n正文😀\r\n\\section{Next}\r\nend",
    );
    let folds = navigation::folding_ranges_for_client(&doc, &Default::default());
    assert_eq!(lines(&folds), [(0, 1), (2, 3), (4, 5)]);
    assert_eq!(folds[0].start_character, Some(2));
    assert_eq!(folds[0].end_character, Some(6));
    assert_eq!(folds[1].end_character, Some(4));
    for range in folds {
        assert!(doc
            .text
            .offset(lsp_types::Position::new(
                range.end_line,
                range.end_character.unwrap()
            ))
            .is_some());
    }
}

#[test]
fn nested_formulas_in_text_groups_keep_the_surrounding_display_fold() {
    for (open, close) in [("$$", "$$"), ("\\[", "\\]"), ("$", "$"), ("\\(", "\\)")] {
        let source = format!("{open}\n\\text{{for $\nx\n$ and \\(\ny\n\\)}}\n{close}\n");
        let folds = navigation::folding_ranges(&document("tex", &source), None);
        assert_eq!(
            lines(&folds),
            [(0, 6), (1, 5), (1, 3), (3, 5)],
            "{source:?}"
        );
    }
}

#[test]
fn environment_folds_use_static_argument_grammar_and_do_not_treat_comments_as_verbatim() {
    let doc = document(
        "tex",
        concat!(
            "\\begin{itemize}% an ordinary comment\n", // 0
            "\\item content\n",                        // 1
            "\\end{itemize}\n",                        // 2
            "\\begin{quote% name comment\n",           // 3
            "}\n",                                     // 4
            "content\n",                               // 5
            "\\end{quote}\n",                          // 6
            "\\begin a\n",                             // 7
            "content\n",                               // 8
            "\\end a\n",                               // 9
            "\\begin{\\dynamic}\n",                    // 10
            "content\n",                               // 11
            "\\end{\\dynamic}\n",                      // 12
        ),
    );
    assert_eq!(
        lines(&navigation::folding_ranges(&doc, None)),
        [(0, 2), (3, 6), (3, 4), (7, 9)]
    );
}

#[test]
fn named_regions_pair_only_real_full_line_comments_and_support_nesting() {
    let source = concat!(
        "% region 外层😀\r\n",             // 0
        "text\r\n",                        // 1
        "  % #region 内层\r\n",            // 2
        "text\r\n",                        // 3
        "% #endregion\r\n",                // 4
        "% endregion\r\n",                 // 5
        "% endregion\r\n",                 // 6, unmatched
        "text % region inline\r\n",        // 7
        "text\r\n",                        // 8
        "% endregion\r\n",                 // 9
        "% regional is ordinary text\r\n", // 10
        "text\r\n",                        // 11
        "% endregion\r\n",                 // 12
        "% region unfinished\r\n",         // 13
        "text\r\n",                        // 14
    );
    for extension in ["tex", "bib"] {
        let folds = navigation::folding_ranges(&document(extension, source), None);
        let regions: Vec<_> = folds
            .iter()
            .filter(|fold| fold.kind == Some(FoldingRangeKind::Region))
            .cloned()
            .collect();
        assert_eq!(lines(&regions), [(0, 5), (2, 4)]);
    }
    let source = "\\begin{verbatim}\n% region\ntext\n% endregion\n\\end{verbatim}\n\\detokenize{% region\ntext\n% endregion\n}\n";
    let folds = navigation::folding_ranges(&document("tex", source), None);
    assert_eq!(lines(&folds), [(0, 4)]);
}

#[test]
fn comment_environments_are_categorized_as_comments_and_literal_url_data_is_not() {
    let doc = document(
        "tex",
        "\\begin{comment}\nignored\n\\end{comment}\n\\url{% literal\n% data}\n",
    );
    let folds = navigation::folding_ranges(&doc, None);
    assert_eq!(lines(&folds), [(0, 2), (3, 4)]);
    assert_eq!(folds[0].kind, Some(FoldingRangeKind::Comment));
    assert_eq!(folds[1].kind, Some(FoldingRangeKind::Region));
}
