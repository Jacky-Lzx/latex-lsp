use lsp_types::{DocumentSymbol, Position, SymbolKind, Url};
use texquill::{language::features, project::workspace::Document};

fn document(source: &str) -> Document {
    Document::new(
        &Url::parse("file:///beamer-structure.tex").unwrap(),
        source.into(),
        Some(1),
    )
}
fn children(symbol: &DocumentSymbol) -> &[DocumentSymbol] {
    symbol.children.as_deref().unwrap_or_default()
}
fn contained(symbols: &[DocumentSymbol], parent: Option<&DocumentSymbol>) {
    for symbol in symbols {
        assert!(!symbol.name.trim().is_empty());
        assert!(
            symbol.range.start <= symbol.selection_range.start,
            "{symbol:?}"
        );
        assert!(symbol.selection_range.end <= symbol.range.end, "{symbol:?}");
        if let Some(parent) = parent {
            assert!(parent.range.start <= symbol.range.start);
            assert!(symbol.range.end <= parent.range.end);
        }
        contained(children(symbol), Some(symbol));
    }
}

#[test]
fn beamer_frames_lists_and_items_preserve_scopes_and_nesting() {
    let source = "\\section{Progress}\n\\begin{frame}<2->[fragile]{中文😀 title}\n\\begin{itemize}\n\\item Outer\n\\begin{enumerate}\n\\item<2->[A] Inner\n\\end{enumerate}\n\\item Next\n\\end{itemize}\n\\end{frame}\n\\begin{frame}{References}\n\\end{frame}\n";
    let doc = document(source);
    let symbols = features::symbols(&doc);
    assert_eq!(symbols.len(), 1);
    let frames = children(&symbols[0]);
    assert_eq!(
        frames.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["中文😀 title", "References"]
    );
    assert_eq!(frames[0].detail.as_deref(), Some("frame"));
    let list = &children(&frames[0])[0];
    assert_eq!(list.name, "itemize");
    assert_eq!(list.kind, SymbolKind::ARRAY);
    let items = children(list);
    assert_eq!(
        items.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Outer", "Next"]
    );
    assert_eq!(children(&children(&items[0])[0])[0].name, "Inner");
    assert_eq!(items[0].range.end, items[1].range.start);
    assert_eq!(frames[0].selection_range.start, Position::new(1, 27));
    contained(&symbols, None);
    let uri = Url::parse("file:///beamer-structure.tex").unwrap();
    let flat = features::flat_symbols(&doc, &uri);
    assert_eq!(
        flat.iter()
            .find(|s| s.name == "Inner")
            .unwrap()
            .container_name
            .as_deref(),
        Some("enumerate")
    );
    assert_eq!(features::symbols(&doc), symbols);
}

#[test]
fn frametitle_blocks_empty_titles_and_unicode_have_valid_selection_ranges() {
    let source = "\\begin{frame}[t]\r\n\\frametitle[Short]{😀 Long\n title}\n\\begin{block}{Claim}\ntext\n\\end{block}\n\\end{frame}\n\\begin{frame}{}\n\\end{frame}";
    let doc = document(source);
    let symbols = features::symbols(&doc);
    assert_eq!(symbols[0].name, "😀 Long title");
    assert_eq!(symbols[0].selection_range.start, Position::new(1, 19));
    assert!(!children(&symbols[0]).is_empty(), "{symbols:#?}");
    assert_eq!(children(&symbols[0])[0].name, "Claim");
    assert_eq!(symbols[1].name, "frame");
    contained(&symbols, None);
}

#[test]
fn comments_verbatim_declarations_and_quoted_data_do_not_invent_structure() {
    let source = r"% \begin{frame}{Comment}
\newcommand{\unused}{\begin{frame}{Deferred}\begin{itemize}\item Fake\end{itemize}\end{frame}}
\newenvironment{saved}{\begin{itemize}\item Deferred}{\end{itemize}}
\detokenize{\begin{frame}{Quoted}\end{frame}}
\begin{verbatim}
\begin{frame}{Literal}
\end{frame}
\end{verbatim}
\begin{frame}{Real}
\begin{description}
\item[Name] body % ignored suffix
\end{description}
\end{frame}";
    let symbols = features::symbols(&document(source));
    let frames: Vec<_> = symbols
        .iter()
        .filter(|s| s.detail.as_deref() == Some("frame"))
        .collect();
    assert_eq!(frames.len(), 1, "{symbols:?}");
    assert_eq!(frames[0].name, "Real");
    assert_eq!(children(&children(frames[0])[0])[0].name, "body");
    contained(&symbols, None);
}

#[test]
fn unclosed_and_mismatched_environments_remain_bounded_while_editing() {
    for source in [
        "\\section{Progress}\n\\begin{frame}{Editing}\n\\begin{itemize}\n\\item unfinished",
        "\\begin{frame}{Editing}\n\\begin{itemize}\n\\item unfinished\n\\end{frame}",
    ] {
        let doc = document(source);
        let symbols = features::symbols(&doc);
        contained(&symbols, None);
        assert!(
            features::flat_symbols(&doc, &Url::parse("file:///beamer-structure.tex").unwrap())
                .iter()
                .any(|s| s.name == "unfinished")
        );
        assert_eq!(symbols[0].range.end, doc.text.position(source.len()));
    }
}

#[test]
fn inline_items_and_long_previews_do_not_overlap_or_split_unicode() {
    let source = format!(
        "\\begin{{itemize}}\\item 第一项\\item {}\\end{{itemize}}",
        "😀".repeat(500)
    );
    let doc = document(&source);
    let symbols = features::symbols(&doc);
    let items = children(&symbols[0]);
    assert_eq!(items[0].name, "第一项");
    assert_eq!(items[1].name.chars().count(), 81);
    assert!(items[1].name.ends_with('…'));
    assert_eq!(items[0].range.end, items[1].range.start);
    contained(&symbols, None);
}

#[test]
fn empty_list_items_do_not_consume_their_nested_environment() {
    let source = "\\begin{frame}{Title}\n\\begin{itemize}\n\\item\n\\begin{enumerate}\n\\item Child\n\\end{enumerate}\n\\end{itemize}\n\\end{frame}";
    let symbols = features::symbols(&document(source));
    let list = &children(&symbols[0])[0];
    let item = &children(list)[0];
    assert_eq!(item.name, "\\item");
    assert_eq!(children(&children(item)[0])[0].name, "Child");
    contained(&symbols, None);
}
