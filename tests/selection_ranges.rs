use latex_lsp::{editing, project::workspace::Document};
use lsp_types::*;

fn doc(name: &str, source: &str) -> Document {
    Document::new(
        &Url::parse(&format!("file:///selection-ranges/{name}")).unwrap(),
        source.into(),
        Some(3),
    )
}

fn chain(doc: &Document, cursor: usize) -> Vec<Range> {
    let position = doc.text.position(cursor);
    let selection = editing::selection_ranges(doc, &[position])
        .unwrap()
        .remove(0);
    let mut current = &selection;
    let mut result = vec![];
    loop {
        assert!(current.range.start <= position && position <= current.range.end);
        assert!(doc.text.offset(current.range.start).is_some());
        assert!(
            doc.text.offset(current.range.end).is_some(),
            "invalid end {:?}: {:?}",
            current.range,
            doc.text.source
        );
        result.push(current.range);
        let Some(parent) = &current.parent else {
            break;
        };
        assert!(parent.range.start <= current.range.start && current.range.end <= parent.range.end);
        assert_ne!(parent.range, current.range);
        current = parent;
    }
    assert_eq!(
        *result.last().unwrap(),
        doc.text.range(0, doc.text.source.len())
    );
    result
}

fn contains(doc: &Document, ranges: &[Range], expected: &str) {
    let source = &doc.text.source;
    let start = source.find(expected).unwrap();
    let expected_range = doc.text.range(start, start + expected.len());
    assert!(
        ranges.contains(&expected_range),
        "missing {expected:?} in {:?}",
        ranges
            .iter()
            .map(|range| &source
                [doc.text.offset(range.start).unwrap()..doc.text.offset(range.end).unwrap()])
            .collect::<Vec<_>>()
    );
}

#[test]
fn ordinary_citation_lists_select_complete_punctuated_keys_before_the_list() {
    for key in ["O\"Connor", "book(old)", "(中文😀)", "a/b?key", "\""] {
        for name in ["cite", "citet", "nocite", "citetalias"] {
            let call = format!("\\{name}{{other,% keep\r\n  {key}% tail\r\n,last}}");
            let source = format!("% 😀\r\n{call} Following prose\r\n");
            let document = doc("main.tex", &source);
            let cursor = source.find(key).unwrap();
            let ranges = chain(&document, cursor);
            contains(&document, &ranges, key);
            contains(
                &document,
                &ranges,
                &format!("other,% keep\r\n  {key}% tail\r\n,last"),
            );
            contains(&document, &ranges, &call);
        }
    }
}

#[test]
fn structured_citation_selection_includes_every_argument_and_stops_before_following_text() {
    for call in [
        "\\parencites(Global note with enough text to cross the last line)(After)[Before]{other}[After]{other,O\"Connor,last}",
        "\\volcite[Before]{42}[1--2]{other,O\"Connor,last}",
        "\\volcites[Before]{42}[1--2]{first}[After]{7}[3]{other,O\"Connor,last}",
        "\\citefield[Note]{other,O\"Connor,last}[format]{title}",
        "\\citelist{other,O\"Connor,last}{publisher}",
        "\\citename{other,O\"Connor,last}{author}",
        "\\addtocategory{category}{other,O\"Connor,last}",
    ] {
        for newline in ["", "\r\n"] {
            let call = call.replace("{other,O", &format!("{{other,{newline}O"));
            let source = format!("% 😀\r\n prefix {call} Following text \\relax{{body}}\r\n");
            let document = doc("main.tex", &source);
            let ranges = chain(&document, source.find("Connor").unwrap() + 2);
            contains(&document, &ranges, "O\"Connor");
            contains(&document, &ranges, &format!("other,{newline}O\"Connor,last"));
            contains(&document, &ranges, &call);
            let start = source.find(&call).unwrap();
            assert!(!ranges.contains(&document.text.range(start, start + call.len() + 1)), "lookahead must not be selected: {source}");
        }
    }
}

#[test]
fn single_citations_keep_comments_and_alias_display_inside_the_full_call() {
    for call in [
        "\\bibentry{% keep\r\nO\"Connor% tail\r\n}",
        "\\defcitealias{% keep\r\nO\"Connor% tail\r\n}{Displayed {name}}",
        "\\bibitem[Display]{% keep\r\nO\"Connor% tail\r\n}",
    ] {
        let source = format!("% 😀\r\n{call} Body\r\n");
        let document = doc("main.tex", &source);
        let ranges = chain(&document, source.find("Connor").unwrap() + 1);
        contains(&document, &ranges, "O\"Connor");
        contains(&document, &ranges, call);
    }
}

#[test]
fn label_lists_range_endpoints_and_separate_labels_have_full_key_and_call_ranges() {
    for call in [
        "\\cref{other, fig(old), last}",
        "\\crefrange{first}{% keep\r\nfig(old)% tail\r\n}",
        "\\vrefrange[before]{first}{fig(old)}",
        "\\hyperref[% keep\r\nfig(old)% tail\r\n]{Displayed text}",
        "\\setcounterref{counter}{% keep\r\nfig(old)% tail\r\n}",
    ] {
        let source = format!("% 😀\r\n{call} Body\r\n");
        let document = doc("main.tex", &source);
        let ranges = chain(&document, source.find("old").unwrap() + 1);
        contains(&document, &ranges, "fig(old)");
        contains(&document, &ranges, call);
    }
}

#[test]
fn bibtex_relationships_select_keys_values_assignments_and_complete_entries() {
    for (name, value, inner) in [
        ("crossref", "\" book(old) \"", " book(old) "),
        ("xref", "{ book(old) }", " book(old) "),
        (
            "xdata",
            "\"other,\r\n book(old), last\"",
            "other,\r\n book(old), last",
        ),
        (
            "related",
            "{other,\r\n O\"Connor, last}",
            "other,\r\n O\"Connor, last",
        ),
        (
            "entryset",
            "\"other, book(old), last",
            "other, book(old), last",
        ),
    ] {
        let field = format!("{name} = {value}");
        let entry = format!(
            "@misc{{child,\r\n {field}{}",
            if name == "entryset" { "" } else { "\r\n}" }
        );
        let source = format!("% 😀\r\n{entry}");
        let document = doc("refs.bib", &source);
        let key = if name == "related" {
            "O\"Connor"
        } else {
            "book(old)"
        };
        let cursor = source
            .find(if name == "related" { "Connor" } else { "old" })
            .unwrap()
            + 1;
        let ranges = chain(&document, cursor);
        for expected in [key, inner, value, &field, &entry] {
            contains(&document, &ranges, expected);
        }
    }
}

#[test]
fn crossing_line_ranges_cannot_displace_a_multiline_command() {
    for call in [
        "\\textbf{Long introductory text\r\n short}",
        "\\parencites(Global note with many words)\r\n{other,O\"Connor,last}",
    ] {
        let source = format!("Before {call} following prose\r\nAfter\r\n");
        let document = doc("main.tex", &source);
        let cursor = source
            .find(if call.starts_with("\\textbf") {
                "short"
            } else {
                "Connor"
            })
            .unwrap()
            + 2;
        let ranges = chain(&document, cursor);
        contains(&document, &ranges, call);
        let line_start = source[..cursor].rfind('\n').unwrap() + 1;
        let line_end = source[cursor..].find('\r').unwrap() + cursor;
        assert!(!ranges.contains(&document.text.range(line_start, line_end)));
    }
}

#[test]
fn dynamic_literal_and_unsupported_commands_do_not_gain_static_key_selections() {
    for call in [
        "\\cite{other,\\chosen\"Connor,last}",
        "\\cite{other,O\"Con% splice\r\nnor,last}",
        "\\cite[one][two][extra]{other,O\"Connor,last}",
        "\\bibentry*{other,O\"Connor,last}",
        "\\parencites{other,\\chosen\"Connor,last}",
        "\\addtocategory*{category}{other,O\"Connor,last}",
        "\\defcitealias{other,O\"Connor,last}{Display}",
        "\\detokenize{\\cite{other,O\"Connor,last}}",
        "\\verb|\\cite{other,O\"Connor,last}|",
        "% \\cite{other,O\"Connor,last}",
        "\\meaning\\parencites{other,O\"Connor,last}",
    ] {
        let source = format!("{call}\r\nAfter\r\n");
        let document = doc("main.tex", &source);
        let cursor = source
            .find("Connor")
            .or_else(|| source.find("Con%"))
            .unwrap()
            + 1;
        let ranges = chain(&document, cursor);
        let key = if source.contains("\\chosen") {
            "\\chosen\"Connor"
        } else if source.contains("Con%") {
            "O\"Con% splice\r\nnor"
        } else {
            "O\"Connor"
        };
        let start = source.find(key).unwrap();
        assert!(
            !ranges.contains(&document.text.range(start, start + key.len())),
            "{source}"
        );
    }
}

#[test]
fn selection_requests_preserve_order_utf16_boundaries_and_empty_document_behavior() {
    let source = "% 😀\r\n\\cite{other,(中文😀),last}\r\n";
    let document = doc("main.tex", source);
    let start = source.find("中文").unwrap();
    let positions = [
        document.text.position(start + 3),
        Position::default(),
        document.text.position(source.len()),
        document.text.position(start),
    ];
    let selections = editing::selection_ranges(&document, &positions).unwrap();
    assert_eq!(selections.len(), positions.len());
    for (selection, position) in selections.iter().zip(positions) {
        assert!(selection.range.start <= position && position <= selection.range.end);
    }
    assert_eq!(selections[0], selections[3]);
    let ranges = chain(&document, start + 3);
    contains(&document, &ranges, "(中文😀)");
    let emoji = document.text.position(source.find('😀').unwrap());
    assert!(editing::selection_ranges(
        &document,
        &[Position::new(emoji.line, emoji.character + 1)]
    )
    .is_none());
    assert!(editing::selection_ranges(&document, &[Position::new(99, 0)]).is_none());
    assert_eq!(
        editing::selection_ranges(&doc("empty.tex", ""), &[Position::default()]).unwrap()[0].range,
        Range::default()
    );
    assert!(editing::selection_ranges(&document, &[])
        .unwrap()
        .is_empty());
}

#[test]
fn crlf_comments_and_incomplete_structured_arguments_keep_valid_containment_chains() {
    for call in [
        "% citation O\"Connor\r\n",
        "\\parencites(Global note)[Before]{other,O\"Connor}[After]{",
        "\\volcite[Note]{42}{other,O\"Connor",
        "\\citefield{other,O\"Connor}[unfinished",
    ] {
        let document = doc("main.tex", call);
        let cursor = call.find("Connor").unwrap() + 1;
        let ranges = chain(&document, cursor);
        if call.starts_with('%') {
            contains(&document, &ranges, "% citation O\"Connor");
            let range = ranges
                .iter()
                .find(|range| range.start == Position::default())
                .unwrap();
            assert_eq!(
                range.end,
                Position::new(0, "% citation O\"Connor".encode_utf16().count() as u32)
            );
        } else if call.contains("[After]") || call.contains("[unfinished") {
            contains(&document, &ranges, "O\"Connor");
            contains(&document, &ranges, call);
        } else {
            let start = call.find("O\"Connor").unwrap();
            assert!(!ranges.contains(&document.text.range(start, start + "O\"Connor".len())));
        }
    }
    let document = doc("main.tex", "Word\r");
    let selection = editing::selection_ranges(&document, &[Position::new(0, 1)])
        .unwrap()
        .remove(0);
    assert_eq!(
        selection.range,
        Range::new(Position::default(), Position::new(0, 4))
    );
    assert!(selection.parent.is_none());
}

#[test]
fn structured_notes_volumes_pages_selectors_and_categories_share_the_full_call_parent() {
    for (call, needle, outer) in [
        (
            "\\parencites(Global note)[Before]{first}[After]{last}",
            "Global",
            "(Global note)",
        ),
        (
            "\\volcites[Before]{42}[1--2]{first}[After]{7}[3--4]{last}",
            "Before",
            "[Before]",
        ),
        (
            "\\volcites[Before]{42}[1--2]{first}[After]{7}[3--4]{last}",
            "42",
            "{42}",
        ),
        (
            "\\volcites[Before]{42}[1--2]{first}[After]{7}[3--4]{last}",
            "1--2",
            "[1--2]",
        ),
        (
            "\\volcites[Before]{42}[1--2]{first}[After]{7}[3--4]{last}",
            "After",
            "[After]",
        ),
        (
            "\\volcites[Before]{42}[1--2]{first}[After]{7}[3--4]{last}",
            "7",
            "{7}",
        ),
        (
            "\\volcites[Before]{42}[1--2]{first}[After]{7}[3--4]{last}",
            "3--4",
            "[3--4]",
        ),
        (
            "\\citefield[Note]{first}[format]{title}",
            "format",
            "[format]",
        ),
        (
            "\\citefield[Note]{first}[format]{title}",
            "title",
            "{title}",
        ),
        (
            "\\addtocategory{category}{first,last}",
            "category",
            "{category}",
        ),
    ] {
        let source = format!("% 😀\r\n{call} Body\r\n");
        let document = doc("main.tex", &source);
        let ranges = chain(&document, source.rfind(needle).unwrap());
        contains(&document, &ranges, outer);
        contains(&document, &ranges, call);
    }
}

#[test]
fn ordinary_bibtex_fields_select_literal_contents_delimiters_assignments_and_entries() {
    for (value, inner, needle) in [
        ("\"Some title\"", "Some title", "title"),
        ("{Some title}", "Some title", "title"),
        (
            "\"中文😀 {Nested text} tail\"",
            "中文😀 {Nested text} tail",
            "Nested",
        ),
        (
            "{中文😀 {Nested text} tail}",
            "中文😀 {Nested text} tail",
            "Nested",
        ),
        (
            "\"A {protected \"quote\"} 100% literal\"",
            "A {protected \"quote\"} 100% literal",
            "literal",
        ),
        (
            "{A {protected \"quote\"} 100% literal}",
            "A {protected \"quote\"} 100% literal",
            "literal",
        ),
        (
            "\"A \\{literal\\} brace\"",
            "A \\{literal\\} brace",
            "brace",
        ),
        ("2026", "2026", "2026"),
        ("Journal/Name", "Journal/Name", "Name"),
    ] {
        for (open, close) in [('{', '}'), ('(', ')')] {
            let assignment = format!("TITLE = {value}");
            let entry = format!("@misc{open}entry,\r\n {assignment},\r\n year=2026\r\n{close}");
            let source = format!("% 😀\r\n{entry}\r\n");
            let document = doc("refs.bib", &source);
            let cursor = source.find(value).unwrap() + value.find(needle).unwrap();
            let ranges = chain(&document, cursor);
            for expected in [inner, value, &assignment, &entry] {
                contains(&document, &ranges, expected);
            }
        }
    }
}

#[test]
fn concatenated_bibtex_values_select_each_operand_and_the_whole_unexpanded_expression() {
    let expression = "Journal/Name # % comment with [ and }\r\n \"First % literal\" # {Second {nested} text} # 2026";
    let entry = format!("@misc{{entry,\r\n title = {expression},\r\n year={{2026}}\r\n}}");
    let source = format!("% 😀\r\n{entry}\r\n");
    let document = doc("refs.bib", &source);
    for (operand, inner, needle) in [
        ("Journal/Name", "Journal/Name", "Name"),
        ("\"First % literal\"", "First % literal", "literal"),
        ("{Second {nested} text}", "Second {nested} text", "nested"),
        ("2026", "2026", "2026"),
    ] {
        let cursor = source.find(operand).unwrap() + operand.find(needle).unwrap();
        let ranges = chain(&document, cursor);
        for expected in [
            inner,
            operand,
            expression,
            &format!("title = {expression}"),
            &entry,
        ] {
            contains(&document, &ranges, expected);
        }
    }
}

#[test]
fn string_assignments_and_preambles_have_full_construct_selection_with_both_delimiters() {
    let expression = "\"First words\" # Journal/Name # {Last words}";
    for (open, close) in [('{', '}'), ('(', ')')] {
        for declaration in [
            format!("@string{open}Journal = {expression}{close}"),
            format!("@preamble{open}{expression}{close}"),
        ] {
            let source = format!("% 😀\r\n{declaration}\r\n@misc{{next,title={{Kept}}}}\r\n");
            let document = doc("refs.bib", &source);
            let ranges = chain(&document, source.find("Name").unwrap());
            for expected in ["Journal/Name", expression, &declaration] {
                contains(&document, &ranges, expected);
            }
            if declaration.contains("@string") {
                contains(&document, &ranges, &format!("Journal = {expression}"));
            }
        }
    }
}

#[test]
fn unfinished_bibtex_literals_retain_trailing_content_and_full_assignment_ranges() {
    for opening in ["\"", "{"] {
        for trailing in ["", "   ", "\r\n  "] {
            let literal = format!("{opening}Some title{trailing}");
            let expression = format!("Journal # {literal}");
            for source in [
                format!("@misc{{entry,title = {expression}"),
                format!("@string{{Alias = {expression}"),
                format!("@preamble{{{expression}"),
            ] {
                let document = doc("refs.bib", &source);
                let ranges = chain(&document, source.find("Some").unwrap() + 2);
                for expected in [
                    &format!("Some title{trailing}"),
                    &literal,
                    &expression,
                    &source,
                ] {
                    contains(&document, &ranges, expected);
                }
                if source.starts_with("@misc") {
                    contains(&document, &ranges, &format!("title = {expression}"));
                } else if source.starts_with("@string") {
                    contains(&document, &ranges, &format!("Alias = {expression}"));
                }
            }
        }
    }
}

#[test]
fn punctuation_groups_cannot_cross_bibtex_literal_or_comment_boundaries() {
    for (expression, inner) in [
        (
            "\"A long introductory [a\" # \"b] end\"",
            "A long introductory [a",
        ),
        (
            "{A long introductory [a} # {b] end}",
            "A long introductory [a",
        ),
        (
            "\"A long introductory [a\",\r\n other=\"b] end\"",
            "A long introductory [a",
        ),
        (
            "\"A long introductory [a\" % ] } comment\r\n # \"tail\"",
            "A long introductory [a",
        ),
    ] {
        let source = format!("@misc{{entry,\r\n title = {expression}\r\n}}\r\n");
        let document = doc("refs.bib", &source);
        let cursor = source.find("[a").unwrap() + 1;
        let ranges = chain(&document, cursor);
        contains(&document, &ranges, inner);
        for range in ranges {
            let selected = &source[document.text.offset(range.start).unwrap()
                ..document.text.offset(range.end).unwrap()];
            assert!(
                !selected.starts_with("[a") || !selected.ends_with(']'),
                "cross-value punctuation group: {selected:?}"
            );
        }
    }
}

#[test]
fn bibtex_expression_selections_do_not_create_references_in_literals_or_malformed_values() {
    use latex_lsp::{language::features, project::workspace::Workspace};
    let source = "@string{Journal={Defined}}\r\n@misc{entry,title=Journal # {Journal} # \"Journal\",year=2026}\r\n@misc{invalid,title=Journal other}\r\n@comment{title=Journal}\r\n@preamble{\"Journal\" # Journal}\r\n";
    let document = doc("refs.bib", source);
    let uri = Url::parse("file:///selection-ranges/refs.bib").unwrap();
    let mut workspace = Workspace::default();
    workspace.documents.insert(uri.clone(), document);
    let document = &workspace.documents[&uri];
    let declaration = document.text.position(source.find("Journal").unwrap());
    let references = features::references(&workspace, &uri, declaration, true);
    assert_eq!(references.len(), 3, "{references:?}");
    for needle in ["{Journal}", "\"Journal\"", "Journal other"] {
        let cursor = source.find(needle).unwrap() + usize::from(needle.starts_with(['{', '"']));
        assert!(
            features::definitions(&workspace, &uri, document.text.position(cursor)).is_empty(),
            "{needle}"
        );
    }
    let rename = features::rename(&workspace, &uri, declaration, "Renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(rename[&uri].len(), 3);
}

#[test]
fn empty_and_malformed_bibtex_expressions_keep_valid_structural_ranges() {
    for value in [
        "",
        " ",
        "\"\"",
        "{}",
        "Journal #",
        "\"",
        "{",
        "Journal # % comment\r\n ",
        "Journal other",
    ] {
        let source = format!("@misc{{entry,title={value}");
        let document = doc("refs.bib", &source);
        for (offset, _) in source.char_indices() {
            let position = document.text.position(offset);
            if document.text.offset(position) == Some(offset) {
                chain(&document, offset);
            }
        }
        chain(&document, source.len());
    }
}

#[test]
fn large_bibliographies_keep_literal_selection_groups_within_their_own_fields() {
    let mut source = String::new();
    for index in 0..1000 {
        source.push_str(&format!(
            "@misc{{key{index},title=\"Long [text{index}\",note=\"closing] Tail{index}\"}}\r\n"
        ));
    }
    let document = doc("refs.bib", &source);
    for index in [0, 499, 999] {
        let inner = format!("Long [text{index}");
        let entry = format!(
            "@misc{{key{index},title=\"Long [text{index}\",note=\"closing] Tail{index}\"}}"
        );
        let cursor = source.find(&format!("[text{index}\"")).unwrap() + 2;
        let ranges = chain(&document, cursor);
        for expected in [
            &inner,
            &format!("\"{inner}\""),
            &format!("title=\"{inner}\""),
            &entry,
        ] {
            contains(&document, &ranges, expected);
        }
    }
}
