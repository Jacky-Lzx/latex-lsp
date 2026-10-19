use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::{CompletionTextEdit, Position, TextDocumentContentChangeEvent, Url};

fn uri(name: &str) -> Url {
    Url::parse(&format!("file:///latex-lsp-tests/{name}")).unwrap()
}

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = uri(name);
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    uri
}

fn cursor(source: &str, needle: &str) -> Position {
    Text::new(source.into()).position(source.find(needle).unwrap() + needle.len())
}

#[test]
fn argument_completions_find_outer_keys_after_nested_commands_in_optional_notes() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "refs.bib",
        "@book{parent, title={Parent}}\n@book{other, title={Other}}\n",
    );
    insert(&mut workspace, "labels.tex", "\\label{sec:intro}");
    for (marked, label, expected) in [
        (
            r"\citep[\emph{see}][p.~2]{other, pa¦BAD}",
            "parent",
            r"\citep[\emph{see}][p.~2]{other, parent}",
        ),
        (
            r"\Citealp*[see \ref{sec:intro}][p.~2]{pa¦BAD}",
            "parent",
            r"\Citealp*[see \ref{sec:intro}][p.~2]{parent}",
        ),
        (
            r"\citep[see \citet{other}]{pa¦BAD}",
            "parent",
            r"\citep[see \citet{other}]{parent}",
        ),
        (
            r"\citep[{see ] \emph{also}}]{pa¦BAD}",
            "parent",
            r"\citep[{see ] \emph{also}}]{parent}",
        ),
        (
            r"\citep[\emph{see}]{pa¦",
            "parent",
            r"\citep[\emph{see}]{parent",
        ),
        (
            r"\citep[see \ref{sec:in¦BAD}]{parent}",
            "sec:intro",
            r"\citep[see \ref{sec:intro}]{parent}",
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        );
        let item = items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        let updated = Text::new(source)
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        assert_eq!(updated.source, expected);
    }
    for marked in [
        r"\citep[\emph{pa¦}]{parent}",
        r"\citep{parent} plain pa¦",
        r"\citep[\emph{see}]{parent} \textbf{pa¦}",
        r"\citep[see \ref{sec:intro}]{parent} pa¦",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(&workspace, &main, Text::new(source).position(offset));
        assert!(items.is_empty(), "{marked}: {items:?}");
    }
}

#[test]
fn pdfbookmark_targets_use_literal_levels_and_navigate_independently_of_titles_and_labels() {
    let mut workspace = Workspace::default();
    let source = concat!(
        "\\pdfbookmark{base.0}{base}\\label{base.0}\n",
        "\\pdfbookmark[01]{Title}{leading}\\pdfbookmark[-1]{Part}{part}\\pdfbookmark[+1]{Plus}{plus}\n",
        "\\pdfbookmark[\\level]{Dynamic}{ignored}\\pdfbookmark[ 1 ]{Spaced}{ignored}\n",
        "\\pdfbookmark{Dynamic}{\\name}\\pdfbookmark*{Starred}{ignored}\n",
        "\\currentpdfbookmark{State}{ignored}\\subpdfbookmark{State}{ignored}\n",
        "% \\pdfbookmark{Comment}{ignored}\n\\verb|\\pdfbookmark{Raw}{ignored}|\n",
    );
    let declaration = insert(&mut workspace, "bookmarks.tex", source);
    let doc = &workspace.documents[&declaration];
    assert_eq!(
        doc.syntax
            .anchors
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["base.0", "leading.01", "part.-1", "plus.+1"]
    );
    assert!(doc
        .syntax
        .anchors
        .iter()
        .all(|entry| !source[entry.span.clone()].contains('.')));
    assert!(features::flat_symbols(doc, &declaration)
        .iter()
        .any(|symbol| symbol.name == "leading.01"));
    let title = doc.text.position(source.find("base.0").unwrap() + 2);
    assert!(features::definitions(&workspace, &declaration, title).is_empty());
    let links = "😀 \\hyperlink{base.0}{Text}\\hyperref{}{}{base.0}{Compound}\\ref{base.0}\n\\hyperlink{base.";
    let main = insert(&mut workspace, "main.tex", links);
    let position = cursor(links, "\\hyperlink{base.");
    let definitions = features::definitions(&workspace, &main, position);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, declaration);
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        3
    );
    assert!(features::hover(&workspace, &main, position).is_some());
    let items = features::completion(
        &workspace,
        &main,
        Text::new(links.into()).position(links.len()),
    );
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].label, "base.0");
}

#[test]
fn labels_complete_and_navigate_across_documents() {
    let mut workspace = Workspace::default();
    let chapter = insert(
        &mut workspace,
        "chapter.tex",
        "\\section{中文}\n\\label{sec:intro}",
    );
    let source = "😀 \\ref{sec:intro}\n\\ref{sec:";
    let main = insert(&mut workspace, "main.tex", source);
    let completions = features::completion(&workspace, &main, cursor(source, "\\ref{sec:"));
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].label, "sec:intro");
    let definitions = features::definitions(&workspace, &main, cursor(source, "\\ref{sec:int"));
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, chapter);
    let refs = features::references(&workspace, &main, cursor(source, "\\ref{sec:int"), true);
    assert_eq!(refs.len(), 2);
    assert!(features::hover(&workspace, &main, cursor(source, "\\ref{sec:int")).is_some());
}

#[test]
fn citations_replace_only_the_current_key() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@article{alpha, title={A}}\n@book{beta, title={B}}",
    );
    let source = "\\citep[see][p.~2]{alpha, be";
    let main = insert(&mut workspace, "main.tex", source);
    let completions = features::completion(&workspace, &main, cursor(source, source));
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].label, "beta");
    let Some(CompletionTextEdit::Edit(edit)) = &completions[0].text_edit else {
        panic!("missing edit");
    };
    assert_eq!(edit.range.start.character as usize, source.len() - 2);
    let source = "\\cite{alpha, beta}";
    insert(&mut workspace, "main.tex", source);
    assert_eq!(
        features::definitions(&workspace, &main, cursor(source, "\\cite{alpha, be"))[0].uri,
        bib
    );
    assert_eq!(
        features::references(&workspace, &main, cursor(source, "\\cite{alpha, be"), false).len(),
        1
    );
}

#[test]
fn snippets_respect_client_capabilities() {
    let mut workspace = Workspace::default();
    let main = insert(&mut workspace, "main.tex", "\\sec");
    let plain = features::completion(&workspace, &main, Position::new(0, 4));
    let Some(CompletionTextEdit::Edit(edit)) = &plain[0].text_edit else {
        panic!();
    };
    assert_eq!(edit.new_text, "section");
    workspace.snippets = true;
    let snippets = features::completion(&workspace, &main, Position::new(0, 4));
    let Some(CompletionTextEdit::Edit(edit)) = &snippets[0].text_edit else {
        panic!();
    };
    assert!(edit.new_text.contains("${1:Title}"));
    assert_eq!(edit.range.start.character, 1);
}

#[test]
fn completion_in_the_middle_replaces_the_remaining_word() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "chapter.tex", r"\label{sec:中文}");
    insert(
        &mut workspace,
        "targets.tex",
        r"\hypertarget{dest:中文}{Text}",
    );
    insert(
        &mut workspace,
        "refs.bib",
        "@book{knuth1984, title={The TeXbook}}",
    );
    for (marked, label, expected) in [
        (
            r"\hyperlink{ dest:|旧 }{Text}",
            "dest:中文",
            r"\hyperlink{dest:中文}{Text}",
        ),
        (
            "\\hyperlink{\r\n dest:|旧 \r\n}{Text}",
            "dest:中文",
            "\\hyperlink{dest:中文}{Text}",
        ),
        (r"\Ref*{ sec:|旧 }", "sec:中文", r"\Ref*{sec:中文}"),
        (
            r"\autopageref{ sec:|旧 }",
            "sec:中文",
            r"\autopageref{sec:中文}",
        ),
        (
            r"\autopageref * { sec:|旧 }",
            "sec:中文",
            r"\autopageref * {sec:中文}",
        ),
        (
            r"\hyperref[{sec:|旧}]{Link}",
            "sec:中文",
            r"\hyperref[{sec:中文}]{Link}",
        ),
        (
            "\\hyperref[{\r\n sec:|旧 \r\n}]{Link}",
            "sec:中文",
            "\\hyperref[{sec:中文}]{Link}",
        ),
        (r"\hyperref[{sec:|", "sec:中文", r"\hyperref[{sec:中文"),
        (
            r"\hyperref[sec:|旧]{Link}",
            "sec:中文",
            r"\hyperref[sec:中文]{Link}",
        ),
        (
            r"\hyperref[ sec:|旧 ]{Link}",
            "sec:中文",
            r"\hyperref[sec:中文]{Link}",
        ),
        (
            "\\hyperref % note\r\n[\r\n sec:|旧 \r\n]{Link}",
            "sec:中文",
            "\\hyperref % note\r\n[sec:中文]{Link}",
        ),
        (r"\hyperref[sec:|", "sec:中文", r"\hyperref[sec:中文"),
        (r"\se|ction{Title}", "section", r"\section{Title}"),
        (r"\ref{sec:|旧}", "sec:中文", r"\ref{sec:中文}"),
        (r"\ref{ sec:|旧 }", "sec:中文", r"\ref{sec:中文}"),
        (r"\ref*{ sec:|旧 }", "sec:中文", r"\ref*{sec:中文}"),
        (
            r"\pageref * { sec:|旧 }",
            "sec:中文",
            r"\pageref * {sec:中文}",
        ),
        (
            r"\setcounterref{section}{ sec:|旧 }",
            "sec:中文",
            r"\setcounterref{section}{sec:中文}",
        ),
        (
            "\\addtocounterpageref{section}{\r\n sec:|旧 \r\n}",
            "sec:中文",
            r"\addtocounterpageref{section}{sec:中文}",
        ),
        (
            r"\getrefbykeydefault{ sec:|旧 }{page}{0}",
            "sec:中文",
            r"\getrefbykeydefault{sec:中文}{page}{0}",
        ),
        ("\\ref{\r\n sec:|旧 \r\n}", "sec:中文", r"\ref{sec:中文}"),
        (
            r"\cite{old, kn|uth1984}",
            "knuth1984",
            r"\cite{old, knuth1984}",
        ),
        (r"\begin{ali|gn}", "align", r"\begin{align}"),
        (r"\begin{ ali|gn }", "align", r"\begin{align}"),
        (r"\end{ ali|gn }", "align", r"\end{align}"),
        (
            "\\begin \r\n{\r\n ali|gn \r\n}x",
            "align",
            "\\begin \r\n{align}x",
        ),
        (
            "\\begin{ ali|gn  \r\nBody",
            "align",
            "\\begin{align\r\nBody",
        ),
    ] {
        let offset = marked.find('|').unwrap();
        let source = marked.replace('|', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let text = Text::new(source);
        let completions = features::completion(&workspace, &main, text.position(offset));
        let item = completions.iter().find(|item| item.label == label).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing completion edit");
        };
        assert_eq!(edit.range.start.line, edit.range.end.line);
        assert!(
            edit.range.start <= text.position(offset) && text.position(offset) <= edit.range.end
        );
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        for pair in edits.windows(2) {
            assert!(pair[1].range.end <= pair[0].range.start);
        }
        let result = text
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
            .unwrap();
        assert_eq!(result.source, expected);
    }
}

#[test]
fn hyperref_label_completion_ignores_urls_text_stars_and_dynamic_names() {
    for marked in [
        r"\hyperref{sec:¦}{category}{name}{Text}",
        r"\hyperref[key]{sec:¦}",
        r"\hyperref*[sec:¦]{Text}",
        r"\hyperref[sec:¦\suffix]{Text}",
        r"\hyperref[sec:¦,other]{Text}",
        r"\hyperref[{sec:¦}suffix]{Text}",
        r"\hyperref[ {sec:¦} ]{Text}",
    ] {
        let mut workspace = Workspace::default();
        insert(&mut workspace, "chapter.tex", r"\label{sec:intro}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        assert!(
            features::completion(&workspace, &main, Text::new(source).position(offset)).is_empty(),
            "{marked}"
        );
    }
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "chapter.tex",
        r"\label{sec:intro}\label{sec:[intro}\label{sec:intro]}",
    );
    let source = r"\hyperref[sec:";
    let main = insert(&mut workspace, "main.tex", source);
    let items = features::completion(
        &workspace,
        &main,
        Text::new(source.into()).position(source.len()),
    );
    assert_eq!(items.len(), 3);
    for item in items {
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(
            edit.new_text,
            if item.label == "sec:intro" {
                item.label.clone()
            } else {
                format!("{{{}}}", item.label)
            }
        );
    }
    let source = r"\hyperref[{sec:";
    let main = insert(&mut workspace, "main.tex", source);
    let items = features::completion(
        &workspace,
        &main,
        Text::new(source.into()).position(source.len()),
    );
    assert_eq!(items.len(), 3);
    for item in items {
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.new_text, item.label);
    }
}

#[test]
fn pdf_destination_index_and_completion_keep_labels_text_and_external_links_separate() {
    let mut workspace = Workspace::default();
    let source = concat!(
        "\\hypertarget{dest:good}{Text}\\label{labelonly}\n",
        "\\hyperdef[label]{section}{first}{Text}\n",
        "\\hypertarget{ dest:good }{Text}\\hypertarget*{ignored}{Text}\n",
        "\\hypertarget{\\prefix dynamic}{Text}\\hyperdef{\\category}{ignored}{Text}\n",
        "% \\hypertarget{commented}{Text}\n\\verb|\\hypertarget{raw}{Text}|\n",
        "\\hyperlink{dest:good}{Text}\\hyperref{}{section}{first}{Text}\n",
        "\\hyperref{https://example.com/a%20b}{section}{first}{Text}\n",
    );
    let document = insert(&mut workspace, "targets.tex", source);
    let doc = &workspace.documents[&document];
    assert_eq!(
        doc.syntax
            .anchors
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["dest:good", "section.first"]
    );
    assert_eq!(
        doc.syntax
            .anchor_references
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["dest:good", "section.first"]
    );
    for (marked, expected) in [
        (r"\hyperlink{¦}{Text}", 2),
        (r"\hyperlink{label¦}{Text}", 0),
        (r"\hypertarget{dest¦}{Text}", 0),
        (r"\hyperlink*{dest¦}{Text}", 0),
        (r"\hyperlink{dest:good}{dest¦}", 0),
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let uri = insert(&mut workspace, "main.tex", &source);
        assert_eq!(
            features::completion(&workspace, &uri, Text::new(source).position(at)).len(),
            expected,
            "{marked}"
        );
    }
}

#[test]
fn environment_name_completion_does_not_guess_computed_names_or_overlap_edits_on_empty_lines() {
    let mut workspace = Workspace::default();
    for marked in [
        r"\begin{ali|\suffix}",
        r"\begin{ali|gn,other}",
        "\\begin{\r\n|\r\n}",
    ] {
        let at = marked.find('|').unwrap();
        let source = marked.replace('|', "");
        let main = insert(&mut workspace, "main.tex", &source);
        assert!(
            features::completion(&workspace, &main, Text::new(source).position(at)).is_empty(),
            "{marked}"
        );
    }
    insert(
        &mut workspace,
        "definitions.tex",
        r"\newenvironment{ panel }{}{}",
    );
    let main = insert(&mut workspace, "main.tex", r"\begin{}");
    let items = features::completion(&workspace, &main, Position::new(0, 7));
    assert!(items.iter().any(|item| item.label == "align"));
    assert!(!items.iter().any(|item| item.label.contains("panel")));
}

#[test]
fn snippet_completion_preserves_existing_arguments() {
    let mut workspace = Workspace {
        snippets: true,
        ..Default::default()
    };
    for source in [r"\section{Title}", r"\section*[Short]{Title}"] {
        let main = insert(&mut workspace, "main.tex", source);
        let completions = features::completion(&workspace, &main, Position::new(0, 3));
        let item = completions
            .iter()
            .find(|item| item.label == "section")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing completion edit");
        };
        assert_eq!(edit.new_text, "section");
        let result = Text::new(source.into())
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        assert_eq!(result.source, source);
    }
}

#[test]
fn custom_commands_complete_hover_and_find_uses() {
    let mut workspace = Workspace::default();
    let macros = insert(
        &mut workspace,
        "macros.tex",
        r"\newcommand{\hello}[1]{Hello #1}",
    );
    let main = insert(&mut workspace, "main.tex", "\\hello{世界}\n\\hel");
    assert_eq!(
        features::completion(&workspace, &main, Position::new(1, 4))[0].label,
        "hello"
    );
    assert_eq!(
        features::definitions(&workspace, &main, Position::new(0, 3))[0].uri,
        macros
    );
    assert!(features::hover(&workspace, &main, Position::new(0, 3)).is_some());
    assert_eq!(
        features::references(&workspace, &main, Position::new(0, 3), false).len(),
        1
    );
    assert_eq!(
        features::references(&workspace, &main, Position::new(0, 3), true).len(),
        2
    );
}

#[test]
fn suppress_features_in_comments_verbatim_and_escaped_commands() {
    let mut workspace = Workspace::default();
    for source in [
        r"% \sec",
        r"\verb|\sec|",
        "\\begin{verbatim}\n\\sec",
        r"\\sec",
    ] {
        let main = insert(&mut workspace, "main.tex", source);
        let position = Text::new(source.into()).position(source.find("sec").unwrap() + 3);
        assert!(
            features::completion(&workspace, &main, position).is_empty(),
            "{source}"
        );
        assert!(
            features::hover(&workspace, &main, position).is_none(),
            "{source}"
        );
    }
}

#[test]
fn environment_completion_and_duplicate_diagnostics() {
    let mut workspace = Workspace::default();
    let main = insert(&mut workspace, "main.tex", r"\begin{ali");
    let completions = features::completion(&workspace, &main, Position::new(0, 10));
    assert!(completions.iter().any(|item| item.label == "align"));
    let main = insert(
        &mut workspace,
        "main.tex",
        r"\label{x}\label{x}\begin{equation}",
    );
    let diagnostics = features::diagnostics(&workspace.documents[&main]);
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics.iter().any(|d| d.message.contains("Duplicate")));
}

#[test]
fn document_symbols_include_headings_labels_and_macros() {
    let mut workspace = Workspace::default();
    let main = insert(
        &mut workspace,
        "main.tex",
        "\\section*{中文}\n\\label{sec:a}\n\\subsection{Details}\n\\newcommand{\\foo}{bar}",
    );
    let symbols = features::symbols(&workspace.documents[&main]);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "中文");
    let children = symbols[0].children.as_ref().unwrap();
    assert_eq!(
        children.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["sec:a", "Details"]
    );
    assert_eq!(children[1].children.as_ref().unwrap()[0].name, "foo");
    assert_eq!(symbols[0].selection_range.start, Position::new(0, 10));
}

#[test]
fn indexes_closed_files_and_keeps_unsaved_content() {
    let directory = tempfile::tempdir().unwrap();
    let chapter = directory.path().join("chapter.tex");
    std::fs::write(&chapter, r"\label{disk}").unwrap();
    let chapter_uri = Url::from_file_path(&chapter).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    assert_eq!(
        workspace.documents[&chapter_uri].syntax.labels[0].name,
        "disk"
    );
    workspace.open(chapter_uri.clone(), r"\label{unsaved}".into(), 2);
    std::fs::write(&chapter, r"\label{changed}").unwrap();
    workspace.reload(&chapter_uri);
    assert_eq!(
        workspace.documents[&chapter_uri].syntax.labels[0].name,
        "unsaved"
    );
    workspace.documents.remove(&chapter_uri);
    workspace.reload(&chapter_uri);
    assert_eq!(
        workspace.documents[&chapter_uri].syntax.labels[0].name,
        "changed"
    );
}

#[test]
fn follows_external_includes_and_bibliography() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(directory.path().join("chapter.tex"), r"\label{external}").unwrap();
    std::fs::write(
        directory.path().join("refs.bib"),
        "@book{external2026, title={Example}}",
    )
    .unwrap();
    let main = Url::from_file_path(project.join("main.tex")).unwrap();
    let source =
        "\\input{../chapter}\n\\addbibresource{../refs.bib}\n\\ref{external}\n\\cite{external2026}";
    let mut workspace = Workspace::default();
    workspace.open(main.clone(), source.into(), 1);
    assert_eq!(workspace.documents.len(), 3);
    assert_eq!(
        features::definitions(&workspace, &main, cursor(source, "\\ref{exter")).len(),
        1
    );
    assert_eq!(
        features::definitions(&workspace, &main, cursor(source, "\\cite{exter")).len(),
        1
    );
    let file = features::definitions(&workspace, &main, cursor(source, "\\input{../chap"));
    assert_eq!(file.len(), 1);
    assert!(file[0].uri.path().ends_with("chapter.tex"));
}

#[test]
fn follows_preindexed_dependencies_and_handles_cycles() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("chapter.tex"), r"\input{../external}").unwrap();
    std::fs::write(
        directory.path().join("external.tex"),
        r"\label{external}\input{project/chapter}",
    )
    .unwrap();
    let main = Url::from_file_path(project.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(project);
    workspace.open(main.clone(), r"\input{chapter}\ref{external}".into(), 1);
    assert_eq!(workspace.documents.len(), 3);
    assert_eq!(
        features::definitions(&workspace, &main, Position::new(0, 22)).len(),
        1
    );
}
