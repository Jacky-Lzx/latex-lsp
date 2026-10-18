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
fn named_alias_signatures_accept_grouped_and_bare_control_sequence_tokens() {
    for (marked, label, active) in [
        (r"\cslet{mir¦ror}{\hello}", r"\cslet{alias}{command}", 0),
        (r"\cslet{mirror}{\hel¦lo}", r"\cslet{alias}{command}", 1),
        (r"\cslet{mirror}\hel¦lo", r"\cslet{alias}{command}", 1),
        (r"\letcs{\mir¦ror}{hello}", r"\letcs{alias}{command}", 0),
        (r"\letcs{\mirror}{hel¦lo}", r"\letcs{alias}{command}", 1),
        (r"\letcs\mirror{hel¦lo}", r"\letcs{alias}{command}", 1),
        (r"\csletcs{mir¦ror}{hello}", r"\csletcs{alias}{command}", 0),
        (r"\csletcs{mirror}{hel¦lo}", r"\csletcs{alias}{command}", 1),
        (
            r"\expandafter\letcs\csname mirror\endcsname{hel¦lo}",
            r"\letcs{alias}{command}",
            1,
        ),
    ] {
        check(r"\csdef{hello}#1#2{#1#2}", marked, label, active);
    }
}

#[test]
fn named_macro_mutations_offer_their_own_signature_instead_of_the_modified_macro_signature() {
    for command in [
        "csappto", "cseappto", "csgappto", "csxappto", "cspreto", "csepreto", "csgpreto",
        "csxpreto",
    ] {
        for (marked, active) in [
            (format!(r"\{command}{{hel¦lo}}{{Code}}"), 0),
            (format!(r"\{command}{{hello}}{{Co¦de}}"), 1),
        ] {
            check(
                r"\csdef{hello}#1#2{#1#2}",
                &marked,
                &format!(r"\{command}{{command}}{{code}}"),
                active,
            );
        }
    }
    for command in ["csundef", "csgundef"] {
        check(
            r"\csdef{hello}#1#2{#1#2}",
            &format!(r"\{command}{{hel¦lo}}"),
            &format!(r"\{command}{{command}}"),
            0,
        );
    }
}

#[test]
fn builtin_signatures_select_nested_arguments_and_skip_omitted_optional_parameters() {
    for (source, label, active) in [
        (r"\newtoggle{draft¦}", r"\newtoggle{toggle}", 0),
        (r"\providetoggle{draft¦}", r"\providetoggle{toggle}", 0),
        (r"\toggletrue{draft¦}", r"\toggletrue{toggle}", 0),
        (r"\togglefalse{draft¦}", r"\togglefalse{toggle}", 0),
        (r"\settoggle{draft}{tr¦}", r"\settoggle{toggle}{value}", 1),
        (
            r"\iftoggle{draft}{Y¦}{N}",
            r"\iftoggle{toggle}{then}{else}",
            1,
        ),
        (
            r"\nottoggle{draft}{N}{Y¦}",
            r"\nottoggle{toggle}{then}{else}",
            2,
        ),
        (r"\frac{a}{b¦}", r"\frac{numerator}{denominator}", 1),
        (r"\includeonly{first, chap¦}", r"\includeonly{files}", 0),
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
        r"\newrobustcmd{\pair}[2][blue]{#1:#2}",
        r"\renewrobustcmd*\pair[2][blue]{#1:#2}",
        r"\providerobustcmd\pair[2][blue]{#1:#2}",
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
    for command in ["csdef", "csgdef", "csedef", "csxdef"] {
        let declaration = format!("\\{command}{{pair}}#1#2{{#1:#2}}");
        check(&declaration, r"\pair{one}{two¦}", r"\pair{#1}{#2}", 1);
        check(
            &declaration,
            r"\csuse{pair}{one}{two¦}",
            r"\csuse{pair}{#1}{#2}",
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
        (r"\csdef{pair}#1,#2{#1#2}", r"\csuse{pair}{a}{b¦}"),
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

#[test]
fn custom_environment_signatures_follow_latex_and_xparse_declarations() {
    for declaration in ["newenvironment", "renewenvironment", "provideenvironment"] {
        let definitions = format!("\\{declaration}  {{panel}} [2][blue]{{#1#2}}{{}}");
        check(
            &definitions,
            r"\begin{panel}{title¦}",
            r"\begin{panel}[#1=blue]{#2}",
            2,
        );
        check(
            &definitions,
            r"\begin{panel}[red¦]{title}",
            r"\begin{panel}[#1=blue]{#2}",
            1,
        );
        check(
            &definitions,
            "\\begin{panel} % comment\r\n {😀¦}",
            r"\begin{panel}[#1=blue]{#2}",
            2,
        );
        check(
            &definitions,
            r"\begin{panel}{\frac{a}{b¦}}",
            r"\frac{numerator}{denominator}",
            1,
        );
    }
    for declaration in [
        "NewDocumentEnvironment",
        "RenewDocumentEnvironment",
        "ProvideDocumentEnvironment",
        "DeclareDocumentEnvironment",
    ] {
        let definitions = format!("\\{declaration}{{panel}}{{s O{{blue}} m}}{{#3}}{{}}");
        check(
            &definitions,
            r"\begin{panel}*{title¦}",
            r"\begin{panel}*[#2=blue]{#3}",
            3,
        );
        check(
            &definitions,
            r"\begin{panel}[red¦]{title}",
            r"\begin{panel}*[#2=blue]{#3}",
            2,
        );
    }
    check(
        r"\newcommand{\panel}[1]{#1}\newenvironment{panel}[2]{#1#2}{}",
        r"\begin{panel}{a}{b¦}",
        r"\begin{panel}{#1}{#2}",
        2,
    );
    check(
        r"\newenvironment{panel}[1]{#1}{}",
        r"\begin{pan¦el}{value}",
        r"\begin{environment}",
        0,
    );
    for definitions in [
        r"\newenvironment{panel}[1]{#1}{}",
        r"\NewDocumentEnvironment{panel}{d()m}{}{}",
        r"\newenvironment{panel}[1]{#1}{}\newenvironment{panel}[1]{#1}{}",
        r"\newenvironment{ panel }[1]{#1}{}",
    ] {
        let source = if definitions == r"\newenvironment{panel}[1]{#1}{}" {
            r"\begin{panel}{value}¦ body"
        } else {
            r"\begin{panel}{value¦}"
        };
        let (workspace, uri, position) = setup(definitions, source);
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{definitions}"
        );
    }
    let (workspace, uri, position) = setup(
        r"\newenvironment{panel}[1]{#1}{}",
        r"\begin{ panel }{value¦}",
    );
    assert!(signature::help(&workspace, &uri, position).is_none());
}

#[test]
fn builtin_environment_signatures_track_required_and_optional_arguments() {
    for (source, label, active) in [
        (
            r"\begin{tabular}{l¦c}",
            r"\begin{tabular}[position]{columns}",
            2,
        ),
        (
            r"\begin{tabular}[t¦]{lc}",
            r"\begin{tabular}[position]{columns}",
            1,
        ),
        (
            r"\begin{tabular*}{\linewidth¦}[t]{lc}",
            r"\begin{tabular*}{width}[position]{columns}",
            1,
        ),
        (
            r"\begin{minipage}[t][3cm][b¦]{2cm}",
            r"\begin{minipage}[position][height][inner position]{width}",
            3,
        ),
        (
            r"\begin{minipage}{2cm¦}",
            r"\begin{minipage}[position][height][inner position]{width}",
            4,
        ),
        (
            r"\begin{figure}[ht¦]
",
            r"\begin{figure}[placement]",
            1,
        ),
        (
            r"\begin{thebibliography}{99¦}",
            r"\begin{thebibliography}{widest label}",
            1,
        ),
        (r"\begin{alignat}{2¦}", r"\begin{alignat}{column pairs}", 1),
    ] {
        check("", source, label, active);
    }
    let (workspace, uri, position) = setup(
        r"\newenvironment{tabular}{custom}{}",
        r"\begin{tabular}{lc¦}",
    );
    assert!(signature::help(&workspace, &uri, position).is_none());
}

#[test]
fn direct_environment_calls_use_declared_arguments_and_respect_macro_overrides() {
    for declaration in [
        r"\newenvironment{panel}[2][blue]{#1#2}{}",
        r"\NewEnviron{panel}[2][blue]{#1#2}",
        r"\RenewEnviron{panel}[2][blue]{#1#2}",
        r"\NewDocumentEnvironment{panel}{O{blue}m}{#1#2}{}",
    ] {
        check(
            declaration,
            r"\panel[red¦]{title}",
            r"\panel[#1=blue]{#2}",
            0,
        );
        check(declaration, r"\panel{😀¦}", r"\panel[#1=blue]{#2}", 1);
        check(
            declaration,
            r"\begin{panel}[red¦]{title}",
            r"\begin{panel}[#1=blue]{#2}",
            1,
        );
        let (workspace, uri, position) = setup(declaration, r"\panel{title} body¦");
        assert!(signature::help(&workspace, &uri, position).is_none());
    }
    check(
        r"\NewDocumentEnvironment{panel}{s O{blue} m}{}{}",
        r"\panel*[red]{title¦}",
        r"\panel*[#2=blue]{#3}",
        2,
    );
    check(
        r"\newenvironment{panel}[2]{#1#2}{}\renewcommand{\panel}[1]{#1}",
        r"\panel{title¦}",
        r"\panel{#1}",
        0,
    );
    for definitions in [
        r"\newenvironment{panel}[1]{#1}{}\newenvironment{panel}[2]{#1#2}{}",
        r"\NewDocumentEnvironment{panel}{d()m}{}{}",
        r"\newenvironment{panel}[1]{#1}{}\renewcommand{\panel}{No arguments}",
    ] {
        let (workspace, uri, position) = setup(definitions, r"\panel{title¦}");
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{definitions}"
        );
    }
}

#[test]
fn theorem_signatures_offer_optional_notes_for_numbered_shared_and_starred_declarations() {
    for declaration in [
        r"\newtheorem{claim}{Claim}",
        r"\newtheorem{claim}{Claim}[section]",
        r"\newtheorem{claim}[lemma]{Claim}",
        r"\newtheorem*{claim}{Claim}",
    ] {
        check(
            declaration,
            r"\begin{claim}[中文¦]",
            r"\begin{claim}[note]",
            1,
        );
        check(declaration, r"\begin{claim}¦", r"\begin{claim}[note]", 1);
        check(declaration, r"\claim[Title¦]", r"\claim[note]", 0);
        check(declaration, r"\claim[\textbf{Title¦}]", r"\textbf{text}", 0);
        for call in [r"\begin{claim}Body¦", r"\claim[Title]Body¦", r"\endclaim¦"] {
            let (workspace, uri, position) = setup(declaration, call);
            assert!(
                signature::help(&workspace, &uri, position).is_none(),
                "{call}"
            );
        }
    }
}

#[test]
fn direct_environment_completions_insert_required_arguments_and_omit_optional_defaults() {
    use latex_lsp::features;
    for (definitions, marked, name, snippets, expected, label) in [
        (
            r"\newenvironment{panel}[2][blue]{}{}",
            r"\pa¦",
            "panel",
            true,
            "panel{${1:arg2}}",
            r"\panel[#1=blue]{#2}",
        ),
        (
            r"\NewDocumentEnvironment{panel}{s O{blue}mm}{}{}",
            r"\pa¦",
            "panel",
            true,
            "panel{${1:arg3}}{${2:arg4}}",
            r"\panel*[#2=blue]{#3}{#4}",
        ),
        (
            r"\newtheorem{claim}{Claim}",
            r"\cla¦",
            "claim",
            true,
            "claim",
            r"\claim[note]",
        ),
        (
            r"\newenvironment{panel}[1]{}{}",
            r"\pa¦nel{Existing}",
            "panel",
            true,
            "panel",
            r"\panel{#1}",
        ),
        (
            r"\newenvironment{panel}[1]{}{}",
            r"\pa¦",
            "panel",
            false,
            "panel",
            r"\panel{#1}",
        ),
    ] {
        let (mut workspace, uri, position) = setup(definitions, marked);
        workspace.snippets = snippets;
        let items = features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert_eq!(item.detail.as_deref(), Some(label));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit");
        };
        assert_eq!(edit.new_text, expected);
        assert_eq!(
            item.insert_text_format,
            Some(if expected.contains("${") {
                InsertTextFormat::SNIPPET
            } else {
                InsertTextFormat::PLAIN_TEXT
            })
        );
    }
    let (workspace, uri, position) = setup(r"\newenvironment{panel-two}[1]{}{}", r"\pan¦");
    assert!(!features::completion(&workspace, &uri, position)
        .iter()
        .any(|item| item.label == "panel-two"));
}

#[test]
fn expanded_literal_macro_declarations_keep_optional_defaults_and_required_parameters() {
    for declaration in [
        r"\expandafter\newcommand\csname pair\endcsname[2][blue]{#1#2}",
        r"\expandafter\newrobustcmd\csname pair\endcsname[2][blue]{#1#2}",
        r"\expandafter\renewrobustcmd\csname pair\endcsname[2][blue]{#1#2}",
        r"\expandafter\providerobustcmd\csname pair\endcsname[2][blue]{#1#2}",
        r"\expandafter\NewDocumentCommand\csname pair\endcsname{O{blue}m}{#1#2}",
    ] {
        check(declaration, r"\pair{Value¦}", r"\pair[#1=blue]{#2}", 1);
        check(declaration, r"\pair[Red¦]{Value}", r"\pair[#1=blue]{#2}", 0);
        let (workspace, _, _) = setup(declaration, r"\pa¦");
        let scope = workspace.documents.keys().cloned().collect();
        let (label, snippet) = signature::completion(&workspace, "pair", &scope).unwrap();
        assert_eq!(label, r"\pair[#1=blue]{#2}");
        assert_eq!(snippet.as_deref(), Some("pair{${1:arg2}}"));
    }
}

#[test]
fn literal_control_sequence_calls_offer_builtin_macro_and_environment_signatures() {
    for (definitions, marked, label, active) in [
        (
            "",
            r"\csuse{frac}{a}{b¦}",
            r"\csuse{frac}{numerator}{denominator}",
            1,
        ),
        (
            r"\csdef{模式}#1{#1}",
            r"\csuse{模式}{Text¦}",
            r"\csuse{模式}{#1}",
            0,
        ),
        ("", r"\csuse{fr¦ac}", r"\csuse{command}", 0),
        (
            r"\newrobustcmd{\pair}[2][blue]{#1#2}",
            r"\csuse{pair}{😀¦}",
            r"\csuse{pair}[#1=blue]{#2}",
            1,
        ),
        (
            r"\newrobustcmd{\pair}[2][blue]{#1#2}",
            r"\csuse{pair}[red¦]{Text}",
            r"\csuse{pair}[#1=blue]{#2}",
            0,
        ),
        (
            r"\newenvironment{panel}[1]{}{}",
            r"\csuse{begin}{panel}{Text¦}",
            r"\csuse{begin}{panel}{#1}",
            1,
        ),
        (
            r"\newenvironment{panel}[2][blue]{}{}",
            r"\csuse{panel}{Text¦}",
            r"\csuse{panel}[#1=blue]{#2}",
            1,
        ),
        (
            "",
            r"\ifcsequal{first}{other¦}{Y}{N}",
            r"\ifcsequal{first command}{second command}{then}{else}",
            1,
        ),
        (
            "",
            r"\ifcsstring{first}{Text¦}{Y}{N}",
            r"\ifcsstring{command}{string}{then}{else}",
            1,
        ),
        (
            "",
            r"\csname frac\endcsname{a}{b¦}",
            r"\csname frac\endcsname{numerator}{denominator}",
            1,
        ),
        (
            "",
            r"\csname sqrt\endcsname[3¦]{b}",
            r"\csname sqrt\endcsname[degree]{expression}",
            0,
        ),
        (
            r"\newcommand{\pair}[2][blue]{#1#2}",
            r"\csname pair\endcsname{😀¦}",
            r"\csname pair\endcsname[#1=blue]{#2}",
            1,
        ),
        (
            r"\expandafter\def\csname pair-two\endcsname#1#2{#1#2}",
            r"\csname pair-two\endcsname{a}{b¦}",
            r"\csname pair-two\endcsname{#1}{#2}",
            1,
        ),
        (
            r"\NewDocumentEnvironment{panel}{O{blue}m}{}{}",
            r"\csname panel\endcsname[red¦]{title}",
            r"\csname panel\endcsname[#1=blue]{#2}",
            0,
        ),
        (
            r"\newtheorem{claim}{Claim}",
            r"\csname claim\endcsname[Title¦]",
            r"\csname claim\endcsname[note]",
            0,
        ),
        (
            r"\newenvironment{panel}[1]{}{}",
            r"\csname begin\endcsname{panel}{Text¦}",
            r"\csname begin\endcsname{panel}{#1}",
            1,
        ),
        (
            r"\newcommand{\pair}[2]{#1#2}",
            r"\csname pair\endcsname{a}{\sqrt{b¦}}",
            r"\sqrt[degree]{expression}",
            1,
        ),
    ] {
        check(definitions, marked, label, active);
    }
    for (definitions, marked) in [
        (r"\newcommand{\pair}[1]{#1}", r"\csuse{pair\suffix}{Text¦}"),
        (r"\newcommand{\pair}[1]{#1}", r"\csuse{pair}{Text}Body¦"),
        (
            r"\newcommand{\pair}[1]{#1}\newcommand{\pair}[2]{#1#2}",
            r"\csuse{pair}{Text¦}",
        ),
        (
            r"\newcommand{\pair}[1]{#1}",
            r"\csname pair\suffix\endcsname{Text¦}",
        ),
        (
            r"\newcommand{\pair}[1]{#1}",
            r"\csname pair\endcsname{Text}Body¦",
        ),
        (r"\newcommand{\pair}[1]{#1}", r"\csname pair{Text¦}"),
        (
            r"\newcommand{\pair}[1]{#1}\newcommand{\pair}[2]{#1#2}",
            r"\csname pair\endcsname{Text¦}",
        ),
        (
            "",
            r"\expandafter\newcommand\csname pair\endcsname[2¦][blue]{#1#2}",
        ),
    ] {
        let (workspace, uri, position) = setup(definitions, marked);
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{marked}"
        );
    }
}
