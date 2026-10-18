use latex_lsp::{
    signature,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn setup(definitions: &str, marked: &str) -> (Workspace, Url, Position) {
    let mut workspace = Workspace::default();
    let definitions_uri = Url::parse("file:///signature/macros.sty").unwrap();
    workspace.documents.insert(
        definitions_uri.clone(),
        Document::new(&definitions_uri, definitions.into(), None),
    );
    let uri = Url::parse("file:///signature/main.tex").unwrap();
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let position = Text::new(source.clone()).position(offset);
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source, Some(1)));
    (workspace, uri, position)
}

fn check(definitions: &str, marked: &str, label: &str, active: u32) {
    let (workspace, uri, position) = setup(definitions, marked);
    let help = signature::help(&workspace, &uri, position)
        .unwrap_or_else(|| panic!("missing signature: {marked}"));
    assert_eq!(help.signatures[0].label, label, "{marked}");
    assert_eq!(help.active_parameter, Some(active), "{marked}");
    assert_eq!(help.signatures[0].active_parameter, Some(active));
    assert!(
        matches!(&help.signatures[0].documentation, Some(Documentation::String(value)) if !value.is_empty())
    );
}

#[test]
fn builtin_signatures_select_nested_arguments_and_skip_omitted_optional_parameters() {
    for (source, label, active) in [
        (r"\frac{a}{b¦}", r"\frac{numerator}{denominator}", 1),
        (r"😀 \frac{😀}{😀¦}", r"\frac{numerator}{denominator}", 1),
        (r"\frac{a}{\sqrt[3]{b¦}}", r"\sqrt[degree]{expression}", 1),
        (r"\sqrt{b¦}", r"\sqrt[degree]{expression}", 1),
        (r"\sqrt¦{b}", r"\sqrt[degree]{expression}", 1),
        (r"\sqrt[3¦]{b}", r"\sqrt[degree]{expression}", 0),
        (r"\section*{Title¦}", r"\section[short title]{title}", 1),
        (
            r"\includegraphics[width={a]b},¦]{fig}",
            r"\includegraphics[options]{file}",
            0,
        ),
        (
            "\\frac{a} % ignored }\r\n {b¦}",
            r"\frac{numerator}{denominator}",
            1,
        ),
        (r"\frac{\{text\}}{b¦}", r"\frac{numerator}{denominator}", 1),
        (r"\frac{a}¦", r"\frac{numerator}{denominator}", 1),
        (
            r"\href{https://example.test/a%20b¦}{Text}",
            r"\href{url}{text}",
            0,
        ),
        (
            r"\href{https://example.test/a%20b}{Text¦}",
            r"\href{url}{text}",
            1,
        ),
    ] {
        check("", source, label, active);
    }
}

#[test]
fn custom_macro_completions_insert_required_tabstops_and_preserve_existing_arguments() {
    use latex_lsp::features;
    let definitions = r"\newcommand{\pair}[2][\linewidth]{#2}\NewDocumentCommand{\render}{s O{blue} m m}{#3#4}\def\texpair#1#2{#1#2}\NewDocumentCommand{\optional}{O{blue}}{#1}";
    for (source, name, snippets, expected, format) in [
        (
            r"\pa¦",
            "pair",
            true,
            "pair{${1:arg2}}",
            InsertTextFormat::SNIPPET,
        ),
        (
            r"\ren¦",
            "render",
            true,
            "render{${1:arg3}}{${2:arg4}}",
            InsertTextFormat::SNIPPET,
        ),
        (
            r"\texp¦",
            "texpair",
            true,
            "texpair{${1:arg1}}{${2:arg2}}",
            InsertTextFormat::SNIPPET,
        ),
        (r"\pa¦", "pair", false, "pair", InsertTextFormat::PLAIN_TEXT),
        (
            r"\pa¦ir[red]{value}",
            "pair",
            true,
            "pair",
            InsertTextFormat::PLAIN_TEXT,
        ),
        (
            "\\pa¦ir % comment\n{value}",
            "pair",
            true,
            "pair",
            InsertTextFormat::PLAIN_TEXT,
        ),
        (
            r"\opt¦",
            "optional",
            true,
            "optional",
            InsertTextFormat::PLAIN_TEXT,
        ),
    ] {
        let (mut workspace, uri, position) = setup(definitions, source);
        workspace.snippets = snippets;
        let items = features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit");
        };
        assert_eq!(edit.new_text, expected, "{source}");
        assert_eq!(item.insert_text_format, Some(format));
        assert!(item
            .detail
            .as_ref()
            .unwrap()
            .starts_with(&format!("\\{name}")));
    }
    let (mut workspace, uri, position) = setup(
        r"\newcommand{\pair}[2]{#1#2}\newcommand{\pair}[1]{#1}",
        r"\pa¦",
    );
    workspace.snippets = true;
    let items = features::completion(&workspace, &uri, position);
    let item = items.iter().find(|item| item.label == "pair").unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
}

#[test]
fn custom_newcommand_and_robust_command_signatures_follow_cross_file_declarations() {
    for declaration in [
        r"\newcommand{\pair}[2][blue]{#1:#2}",
        r"\newcommand\pair[2][blue]{#1:#2}",
        r"\renewcommand{\pair}[2][blue]{#1:#2}",
        r"\providecommand{\pair}[2][blue]{#1:#2}",
        r"\DeclareRobustCommand{\pair}[2][blue]{#1:#2}",
    ] {
        check(declaration, r"\pair{value¦}", r"\pair[#1=blue]{#2}", 1);
        check(declaration, r"\pair[red¦]{value}", r"\pair[#1=blue]{#2}", 0);
    }
    check(
        r"\newcommand{\triple}[3]{#1:#2:#3}",
        r"\triple{one}{two}{three¦}",
        r"\triple{#1}{#2}{#3}",
        2,
    );
    check(
        r"\newcommand{\pair}[2][{a]b}]{#1:#2}",
        r"\pair{value¦}",
        r"\pair[#1={a]b}]{#2}",
        1,
    );
}

#[test]
fn xparse_and_undelimited_tex_macros_report_the_correct_parameter() {
    for command in [
        "NewDocumentCommand",
        "RenewDocumentCommand",
        "ProvideDocumentCommand",
        "DeclareDocumentCommand",
    ] {
        let definitions = format!("\\{command}{{\\render}}{{s O{{blue}} +m}}{{#3}}");
        check(
            &definitions,
            r"\render*{value¦}",
            r"\render*[#2=blue]{#3}",
            2,
        );
        check(
            &definitions,
            r"\render[red¦]{value}",
            r"\render*[#2=blue]{#3}",
            1,
        );
    }
    check(
        r"\NewDocumentCommand\pair{om}{#2}",
        r"\pair{value¦}",
        r"\pair[#1]{#2}",
        1,
    );
    for command in ["def", "gdef", "edef", "xdef"] {
        check(
            &format!("\\{command}\\pair#1#2{{#1:#2}}"),
            r"\pair{one}{two¦}",
            r"\pair{#1}{#2}",
            1,
        );
    }
}

#[test]
fn signatures_suppress_comments_raw_text_completed_calls_and_unsupported_or_ambiguous_macros() {
    for (definitions, source) in [
        ("", r"% \frac{a}{b¦}"),
        ("", r"\verb|\frac{a}{b¦}|"),
        ("", "\\begin{verbatim}\n\\frac{a}{b¦}\n\\end{verbatim}"),
        ("", r"\frac{a}{b}¦"),
        (r"\newcommand{\frac}{zero}", r"\frac{a}{b¦}"),
        (
            r"\newcommand{\pair}[2]{#1#2}\newcommand{\pair}[2]{#1#2}",
            r"\pair{a}{b¦}",
        ),
        (r"\def\pair#1,#2{#1#2}", r"\pair{a}{b¦}"),
        (r"\NewDocumentCommand{\pair}{d()m}{#2}", r"\pair{a}{b¦}"),
    ] {
        let (workspace, uri, position) = setup(definitions, source);
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{source}"
        );
    }
    let (workspace, uri, _) = setup("", r"\frac{😀¦}{b}");
    assert!(signature::help(&workspace, &uri, Position::new(0, 7)).is_none());
}
