use latex_lsp::{features, signature, text::Text, workspace::Workspace};
use lsp_types::*;

fn open(workspace: &mut Workspace, source: &str) -> Url {
    let uri = Url::parse("file:///color-commands/main.tex").unwrap();
    workspace.open(uri.clone(), source.into(), 7);
    uri
}

fn apply(source: &str, item: CompletionItem) -> String {
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
        panic!("missing edit")
    };
    assert!(item.additional_text_edits.unwrap_or_default().is_empty());
    Text::new(source.into())
        .changed(vec![TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        }])
        .unwrap()
        .source
}

#[test]
fn color_command_snippets_and_hover_describe_their_model_and_required_arguments() {
    let mut workspace = Workspace::default();
    for (command, snippet) in [
        (
            "definecolor",
            "definecolor{${1:name}}{${2:rgb}}{${3:0,0,0}}",
        ),
        ("color", "color{${1:color}}"),
        ("textcolor", "textcolor{${1:color}}{${2:text}}"),
        ("colorbox", "colorbox{${1:color}}{${2:text}}"),
        ("pagecolor", "pagecolor{${1:color}}"),
    ] {
        for snippets in [false, true] {
            workspace.snippets = snippets;
            let source = format!("😀 \\{}", &command[..command.len() - 1]);
            let uri = open(&mut workspace, &source);
            let position = Text::new(source.clone()).position(source.len());
            let item = features::completion(&workspace, &uri, position)
                .into_iter()
                .find(|item| item.label == command)
                .unwrap();
            assert_eq!(
                item.insert_text_format,
                Some(if snippets {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                })
            );
            assert_eq!(
                apply(&source, item),
                format!("😀 \\{}", if snippets { snippet } else { command })
            );
        }
        let source = format!("\\{command}");
        let uri = open(&mut workspace, &source);
        let hover = features::hover(&workspace, &uri, Position::new(0, 3)).unwrap();
        let HoverContents::Markup(contents) = hover.contents else {
            panic!("missing hover documentation")
        };
        assert!(contents.value.contains("color"));
        assert!(contents.value.contains("Requires"));
    }
}

#[test]
fn middle_of_word_completion_preserves_existing_color_arguments_and_comment_continuations() {
    let mut workspace = Workspace {
        snippets: true,
        ..Workspace::default()
    };
    for (command, arguments) in [
        ("definecolor", "[named]{paint}[]{rgb}{1,0,0}"),
        ("color", "[rgb]{1,0,0}"),
        ("textcolor", "[rgb]{1,0,0}{Body}"),
        ("colorbox", "[gray]{.5}{Body}"),
        ("pagecolor", "{white}"),
    ] {
        for separator in ["", "% keep😀\r\n"] {
            let prefix = &command[..command.len() - 1];
            let source = format!("😀 \\{prefix}BAD{separator}{arguments}");
            let uri = open(&mut workspace, &source);
            let position = Text::new(source.clone()).position("😀 \\".len() + prefix.len());
            let item = features::completion(&workspace, &uri, position)
                .into_iter()
                .find(|item| item.label == command)
                .unwrap();
            assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
            assert_eq!(
                apply(&source, item),
                format!("😀 \\{command}{separator}{arguments}")
            );
        }
    }
}

#[test]
fn color_signatures_track_models_names_prefixes_specs_and_text_at_tex_option_boundaries() {
    let mut workspace = Workspace::default();
    for (marked, active, label) in [
        (
            r"\definecolor[na¦med]{paint}[prefix]{rgb}{1,0,0}",
            0,
            r"\definecolor[type]{name}[prefix]{model}{specification}",
        ),
        (
            r"\definecolor[named]{pa¦int}[prefix]{rgb}{1,0,0}",
            1,
            r"\definecolor[type]{name}[prefix]{model}{specification}",
        ),
        (
            r"\definecolor[named]{paint}[pre¦fix]{rgb}{1,0,0}",
            2,
            r"\definecolor[type]{name}[prefix]{model}{specification}",
        ),
        (
            r"\definecolor{paint}{r¦gb}{1,0,0}",
            3,
            r"\definecolor[type]{name}[prefix]{model}{specification}",
        ),
        (
            r"\definecolor{paint}{rgb}{1,0,¦0}",
            4,
            r"\definecolor[type]{name}[prefix]{model}{specification}",
        ),
        (
            r"\definecolor[named [literal]{paint}[prefix [literal]{rgb}{1,0,¦0}",
            4,
            r"\definecolor[type]{name}[prefix]{model}{specification}",
        ),
        (
            r"\definecolor[{named]}]{paint}[{prefix]}]{rgb}{1,0,¦0}",
            4,
            r"\definecolor[type]{name}[prefix]{model}{specification}",
        ),
        (r"\color[r¦gb]{1,0,0}", 0, r"\color[model]{color}"),
        (r"\color[rgb]{1,0,¦0}", 1, r"\color[model]{color}"),
        (r"\color{re¦d}", 1, r"\color[model]{color}"),
        (r"\color[rgb [literal]{1,0,¦0}", 1, r"\color[model]{color}"),
        (
            r"\textcolor[r¦gb]{1,0,0}{Body}",
            0,
            r"\textcolor[model]{color}{text}",
        ),
        (
            r"\textcolor[rgb]{1,0,¦0}{Body}",
            1,
            r"\textcolor[model]{color}{text}",
        ),
        (
            r"\textcolor[rgb]{1,0,0}{Bo¦dy}",
            2,
            r"\textcolor[model]{color}{text}",
        ),
        (
            r"\textcolor[rgb [literal]{1,0,0}{Bo¦dy}",
            2,
            r"\textcolor[model]{color}{text}",
        ),
        (r"\textcolor{red}¦X", 2, r"\textcolor[model]{color}{text}"),
        (
            r"\colorbox[gr¦ay]{.5}{Body}",
            0,
            r"\colorbox[model]{color}{text}",
        ),
        (
            r"\colorbox[gray]{.¦5}{Body}",
            1,
            r"\colorbox[model]{color}{text}",
        ),
        (
            r"\colorbox[gray]{.5}{Bo¦dy}",
            2,
            r"\colorbox[model]{color}{text}",
        ),
        (
            r"\colorbox[gray [literal]{.5}{Bo¦dy}",
            2,
            r"\colorbox[model]{color}{text}",
        ),
        (r"\pagecolor[r¦gb]{1,0,0}", 0, r"\pagecolor[model]{color}"),
        (r"\pagecolor[rgb]{1,0,¦0}", 1, r"\pagecolor[model]{color}"),
        (
            r"\pagecolor[rgb [literal]{1,0,¦0}",
            1,
            r"\pagecolor[model]{color}",
        ),
        (
            "\\textcolor[rgb% fake ]\r\n]{1,0,0}% between\r\n{Bo¦dy}",
            2,
            r"\textcolor[model]{color}{text}",
        ),
    ] {
        let marked = format!("😀 {marked}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let uri = open(&mut workspace, &source);
        let help = signature::help(&workspace, &uri, Text::new(source).position(offset))
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.active_parameter, Some(active), "{marked}");
        assert_eq!(help.signatures[0].label, label, "{marked}");
    }
}

#[test]
fn completed_calls_unsupported_stars_extra_notes_and_ignored_text_do_not_capture_prose() {
    let mut workspace = Workspace::default();
    for marked in [
        r"\definecolor{paint}{rgb}{1,0,0}Body¦",
        r"\color[rgb]{1,0,0}Body¦",
        r"\textcolor[rgb]{1,0,0}{Body}After¦",
        r"\colorbox[gray]{.5}{Body}After¦",
        r"\pagecolor{white}Body¦",
        r"\definecolor*{paint}{rgb}{1,0,¦0}",
        r"\definecolor[named][extra]{paint}{rgb}{1,0,¦0}",
        r"\textcolor*{red}{Bo¦dy}",
        r"\textcolor[rgb][extra]{1,0,0}{Bo¦dy}",
        r"\colorbox*{red}{Bo¦dy}",
        r"\colorbox[rgb][extra]{1,0,0}{Bo¦dy}",
        r"% \textcolor[rgb]{1,0,¦0}{Body}",
        r"\verb|\color[rgb]{1,0,¦0}|",
        "\\begin{verbatim}\n\\textcolor[rgb]{1,0,0}{Bo¦dy}\n\\end{verbatim}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let uri = open(&mut workspace, &source);
        assert!(
            signature::help(&workspace, &uri, Text::new(source).position(offset)).is_none(),
            "{marked}"
        );
    }
}
