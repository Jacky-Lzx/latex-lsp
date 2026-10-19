use latex_lsp::{editing, text::Text, workspace::Document};
use tower_lsp::lsp_types::*;

fn document(source: &str, extension: &str) -> Document {
    let uri = Url::parse(&format!("file:///linked-editing/main.{extension}")).unwrap();
    Document::new(&uri, source.into(), Some(7))
}

#[test]
fn paired_environment_names_return_identical_utf16_ranges_from_either_end() {
    for name in ["quote", "align*", "环境"] {
        let source = format!("😀 \\begin{{{name}}}\r\nBody\r\n\\end{{{name}}}\r\n");
        let doc = document(&source, "tex");
        let starts: Vec<_> = source
            .match_indices(&format!("{{{name}}}"))
            .map(|(offset, _)| offset + 1)
            .collect();
        let expected: Vec<_> = starts
            .iter()
            .map(|start| doc.text.range(*start, start + name.len()))
            .collect();
        for start in starts {
            for offset in [
                start,
                start + name.chars().next().unwrap().len_utf8(),
                start + name.len(),
            ] {
                let linked =
                    editing::linked_editing_ranges(&doc, doc.text.position(offset)).unwrap();
                assert_eq!(linked.ranges, expected, "{source}");
                assert!(linked.word_pattern.is_some());
            }
        }
    }
}

#[test]
fn nested_and_repeated_names_link_only_their_own_balanced_pair() {
    let source = r"\begin{panel}A\begin{panel}B\end{panel}C\end{panel}\begin{panel}D\end{panel}";
    let doc = document(source, "tex");
    let names: Vec<_> = source
        .match_indices("{panel}")
        .map(|(offset, _)| offset + 1)
        .collect();
    for (begin, end) in [(0, 3), (1, 2), (4, 5)] {
        let expected = vec![
            doc.text.range(names[begin], names[begin] + 5),
            doc.text.range(names[end], names[end] + 5),
        ];
        for index in [begin, end] {
            assert_eq!(
                editing::linked_editing_ranges(&doc, doc.text.position(names[index] + 2))
                    .unwrap()
                    .ranges,
                expected
            );
        }
    }
}

#[test]
fn raw_environment_delimiters_link_without_pairing_fake_commands_in_the_body() {
    for name in [
        "verbatim",
        "verbatim*",
        "lstlisting",
        "minted",
        "comment",
        "Verbatim",
    ] {
        let source = format!("\\begin{{quote}}\\begin{{{name}}}\r\n\\begin{{quote}}Fake\\end{{quote}}\r\n\\end{{{name}}}\\end{{quote}}");
        let doc = document(&source, "tex");
        let raw_begin = source.find(&format!("{{{name}}}")).unwrap() + 1;
        let raw_end = source.rfind(&format!("{{{name}}}")).unwrap() + 1;
        let expected = vec![
            doc.text.range(raw_begin, raw_begin + name.len()),
            doc.text.range(raw_end, raw_end + name.len()),
        ];
        for offset in [raw_begin, raw_end] {
            assert_eq!(
                editing::linked_editing_ranges(&doc, doc.text.position(offset))
                    .unwrap()
                    .ranges,
                expected,
                "{name}"
            );
        }
        let first_quote = source.find("{quote}").unwrap() + 1;
        let last_quote = source.rfind("{quote}").unwrap() + 1;
        assert_eq!(
            editing::linked_editing_ranges(&doc, doc.text.position(first_quote))
                .unwrap()
                .ranges,
            vec![
                doc.text.range(first_quote, first_quote + 5),
                doc.text.range(last_quote, last_quote + 5)
            ]
        );
        let fake_quote = source[first_quote + 5..].find("{quote}").unwrap() + first_quote + 5 + 1;
        assert!(
            editing::linked_editing_ranges(&doc, doc.text.position(fake_quote)).is_none(),
            "{name}"
        );
    }
}

#[test]
fn unbalanced_dynamic_declaration_comment_inline_raw_and_body_positions_have_no_link() {
    for marked in [
        r"\begin{pa¦nel}Unclosed",
        r"\begin*{pa¦nel}Body\end{panel}",
        r"\begin[Note]{pa¦nel}Body\end{panel}",
        r"\begin{panel}Body\end*{pa¦nel}",
        r"\begin{panel}Body\end[Note]{pa¦nel}",
        r"\begin{pa¦nel}Body\end*{panel}",
        r"\begin{panel}Body\end{panel¦% tail
}",
        r"\begin{outer}\begin{inner}x\end{out¦er}\end{inner}",
        r"\begin{out¦er}\begin{\chosen}\end{panel}\end{outer}",
        r"\begin{\chosen}Text\end{pa¦nel}",
        r"\begin{pa¦nel}x\end{other}",
        r"\begin{ panel }x\end{pa¦nel}",
        r"\begin{panel}x\end{ panel¦ }",
        r"\newenvironment{pa¦nel}{Body}{End}",
        r"\begin{panel}Bo¦dy\end{panel}",
        r"% \begin{pa¦nel}Text\end{panel}",
        r"\verb|\begin{pa¦nel}Text\end{panel}|",
        "\\begin{verbatim}\n\\begin{pa¦nel}Text\\end{panel}\\end{verbatim}",
        "\\begin{verbatim}\n\\begin{pa¦nel}Text\\end{panel}",
        r"\begin{verb¦atim}Unclosed",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let doc = document(&source, "tex");
        assert!(
            editing::linked_editing_ranges(&doc, doc.text.position(offset)).is_none(),
            "{marked}"
        );
    }
}

#[test]
fn linked_editing_refuses_non_tex_documents_and_invalid_utf16_positions() {
    let source = "😀 \\begin{quote}Body\\end{quote}";
    let doc = document(source, "tex");
    for at in [
        Position::new(0, 1),
        Position::new(100, 0),
        Position::new(0, u32::MAX),
    ] {
        assert!(editing::linked_editing_ranges(&doc, at).is_none());
    }
    for extension in ["bib", "bst"] {
        let doc = document(source, extension);
        assert!(editing::linked_editing_ranges(
            &doc,
            Text::new(source.into()).position(source.find("quote").unwrap())
        )
        .is_none());
    }
}
