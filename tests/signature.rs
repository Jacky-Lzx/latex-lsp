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
fn xparse_delimited_parameters_track_defaults_nesting_and_omitted_arguments() {
    for declaration in [
        "NewDocumentCommand",
        "RenewDocumentCommand",
        "ProvideDocumentCommand",
        "DeclareDocumentCommand",
        "NewExpandableDocumentCommand",
        "RenewExpandableDocumentCommand",
        "ProvideExpandableDocumentCommand",
        "DeclareExpandableDocumentCommand",
    ] {
        let definitions =
            format!("\\{declaration}\\probe{{d<> D(){{DEF}} +r|| R[]{{FALL}} m}}{{#5}}");
        let label = r"\probe<#1>(#2=DEF)|#3|[#4=FALL]{#5}";
        for (call, active) in [
            (r"\probe<a<b¦>c>(d(e)f)|same|[req]{BODY}", 0),
            (r"\probe<a<b>c>(d(e¦)f)|same|[req]{BODY}", 1),
            (r"\probe<a<b>c>(d(e)f)|sa¦me|[req]{BODY}", 2),
            (r"\probe<a<b>c>(d(e)f)|same|[r¦eq]{BODY}", 3),
            (r"\probe<a<b>c>(d(e)f)|same|[req]{BO¦DY}", 4),
            (r"\probe|sa¦me|[req]{BODY}", 2),
            (r"\probe|same|[req]{BO¦DY}", 4),
            (r"\probe¦|same|[req]{BODY}", 2),
            (r"\probe¦", 0),
        ] {
            check(&definitions, call, label, active);
        }
        for command in [r"\csuse{probe}", r"\csname probe\endcsname"] {
            check(
                &definitions,
                &format!("{command}|same|[req]{{BO¦DY}}"),
                &label.replacen(r"\probe", command, 1),
                4,
            );
        }
    }
}

#[test]
fn xparse_delimiters_respect_protected_escaped_and_commented_tokens() {
    let definition = r"\NewDocumentCommand\probe{d<>m}{#2}";
    for call in [
        r"\probe<{a>b}>{BO¦DY}",
        r"\probe<a\>b>{BO¦DY}",
        r"\probe<a\<b>{BO¦DY}",
        r"\probe<a{<}b>{BO¦DY}",
        "\\probe<a % ignored >\r\n b>{BO¦DY}",
        "😀 \\probe % skipped <\r\n<a<b>c> % ignored <\r\n{BO¦DY}",
    ] {
        check(definition, call, r"\probe<#1>{#2}", 1);
    }
    for call in [r"\probe<a<b>c¦>", r"\probe<{a>¦b}>", r"\probe<a\>b¦>"] {
        check(definition, call, r"\probe<#1>{#2}", 0);
    }
    for call in [r"\probe<a>{BODY}¦", r"\probe<a>{BODY} body¦"] {
        let (workspace, uri, position) = setup(definition, call);
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{call}"
        );
    }
}

#[test]
fn xparse_control_sequence_and_unicode_delimiters_match_whole_tokens() {
    for (spec, call, active, label) in [
        (
            r"d\OPEN\CLOSE m",
            r"\probe\OPEN a\OPEN b\CLOSE c\CLOSE{BO¦DY}",
            1,
            r"\probe\OPEN#1\CLOSE{#2}",
        ),
        (
            r"d\OPEN\CLOSE m",
            r"\probe\OPEN x\CLOSEsuffix y¦\CLOSE{BODY}",
            0,
            r"\probe\OPEN#1\CLOSE{#2}",
        ),
        (
            r"d\OPEN\CLOSE m",
            r"\probe\OPEN x\CLOSEsuffix y\CLOSE{BO¦DY}",
            1,
            r"\probe\OPEN#1\CLOSE{#2}",
        ),
        (
            r"d\OPEN\CLOSE m",
            r"\probe\OPEN {\CLOSE} y\CLOSE{BO¦DY}",
            1,
            r"\probe\OPEN#1\CLOSE{#2}",
        ),
        (
            r"r\{\}m",
            r"\probe\{a\{b\}c\}{BO¦DY}",
            1,
            r"\probe\{#1\}{#2}",
        ),
        ("d«»m", "😀 \\probe«a«b»c»{BO¦DY}", 1, "\\probe«#1»{#2}"),
        ("d«»m", "\\probe«a«b¦»c»{BODY}", 0, "\\probe«#1»{#2}"),
        ("d{<}{>}m", r"\probe<a<b>c>{BO¦DY}", 1, r"\probe<#1>{#2}"),
        ("r||m", r"\probe|one|{BO¦DY}", 1, r"\probe|#1|{#2}"),
        ("r a b m", r"\probe aoneb{BO¦DY}", 1, r"\probe a#1b{#2}"),
    ] {
        check(
            &format!("\\NewDocumentCommand\\probe{{{spec}}}{{#2}}"),
            call,
            label,
            active,
        );
    }
    for (spec, call) in [
        (r"d\OPEN\CLOSE m", r"\probe\OPENsuffix a\CLOSE{BO¦DY}"),
        (r"r<>m", r"\probe{BO¦DY}"),
        (r"R<>{FALL}m", r"\probe{BO¦DY}"),
    ] {
        let definitions = format!("\\NewDocumentCommand\\probe{{{spec}}}{{#2}}");
        let (workspace, uri, position) = setup(&definitions, call);
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{call}"
        );
    }
}

#[test]
fn xparse_delimited_environment_signatures_work_for_begins_and_direct_calls() {
    for declaration in [
        "NewDocumentEnvironment",
        "RenewDocumentEnvironment",
        "ProvideDocumentEnvironment",
        "DeclareDocumentEnvironment",
    ] {
        let definitions = format!("\\{declaration}{{panel}}{{d<>R(){{DEF}}m}}{{}}{{}}");
        check(
            &definitions,
            r"\begin{panel}<a<b¦>c>(req){BODY}",
            r"\begin{panel}<#1>(#2=DEF){#3}",
            1,
        );
        check(
            &definitions,
            r"\begin{panel}(req){BO¦DY}",
            r"\begin{panel}<#1>(#2=DEF){#3}",
            3,
        );
        check(
            &definitions,
            r"\panel<a<b>c>(req){BO¦DY}",
            r"\panel<#1>(#2=DEF){#3}",
            2,
        );
        check(
            &definitions,
            r"\csuse{panel}(req){BO¦DY}",
            r"\csuse{panel}<#1>(#2=DEF){#3}",
            2,
        );
    }
}

#[test]
fn xparse_delimited_completions_insert_required_tokens_and_preserve_existing_arguments() {
    for (definition, following, expected) in [
        (
            r"\NewDocumentCommand\probe{d<>D(){DEF}r||R[]{FALL}m}{}",
            "",
            "probe|${1:arg3}|[${2:arg4}]{${3:arg5}}",
        ),
        (
            r"\NewDocumentCommand\probe{r\OPEN\CLOSE m}{}",
            "",
            r"probe\\OPEN ${1:arg1}\\CLOSE {${2:arg2}}",
        ),
        (
            r"\NewDocumentCommand\probe{r\{\}m}{}",
            "",
            r"probe\\{${1:arg1}\\\}{${2:arg2}}",
        ),
        (
            r"\NewDocumentCommand\probe{r$$m}{}",
            "",
            r"probe\$${1:arg1}\${${2:arg2}}",
        ),
        (
            r"\NewDocumentCommand\probe{r a b m}{}",
            "",
            "probe a${1:arg1}b{${2:arg2}}",
        ),
        (
            "\\NewDocumentCommand\\probe{r«»m}{}",
            "",
            "probe«${1:arg1}»{${2:arg2}}",
        ),
        (
            r"\NewDocumentEnvironment{probe}{d<>r()m}{}{}",
            "",
            "probe(${1:arg2}){${2:arg3}}",
        ),
        (
            r"\NewDocumentCommand\probe{d<>r()m}{}",
            "<old>(req){BODY}",
            "probe",
        ),
        (
            r"\NewDocumentCommand\probe{d<>r()m}{}",
            "(req){BODY}",
            "probe",
        ),
        (
            r"\NewDocumentCommand\probe{r\OPEN\CLOSE m}{}",
            " % comment\n\\OPEN old\\CLOSE{BODY}",
            "probe",
        ),
    ] {
        let (mut workspace, uri, position) = setup(definition, &format!("\\pro¦{following}"));
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == "probe").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.new_text, expected, "{definition}; {following}");
        assert_eq!(
            item.insert_text_format,
            Some(if expected == "probe" {
                InsertTextFormat::PLAIN_TEXT
            } else {
                InsertTextFormat::SNIPPET
            })
        );
    }
}

#[test]
fn xparse_malformed_or_multi_token_delimiters_remain_conservative() {
    for spec in [
        "d<",
        "D<>",
        "R()",
        "d{ab}{>}m",
        "r{}{>}m",
        "d{#}{>}m",
        "d<>v",
        "d<>mmmmmmmmm",
    ] {
        let definition = format!("\\NewDocumentCommand\\probe{{{spec}}}{{}}");
        let (workspace, uri, position) = setup(&definition, r"\probe<value>{BO¦DY}");
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{spec}"
        );
        let scope = workspace.documents.keys().cloned().collect();
        assert!(
            signature::completion(&workspace, "probe", &scope).is_none(),
            "{spec}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn xparse_delimited_signature_boundaries_match_real_tex_argument_values() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for engine in ["pdflatex", "xelatex"] {
        for declaration in ["NewDocumentCommand", "NewExpandableDocumentCommand"] {
            let definition = format!("\\{declaration}\\probe{{d<>D(){{DEF}}r||R[]{{FALL}}m}}{{\\typeout{{A=\\detokenize{{#1}};B=\\detokenize{{#2}};C=#3;D=#4;E=#5}}}}");
            let calls = [
                r"\probe<a<b>c>(d(e)f)|same|[req]{BO¦DY}",
                r"\probe|same|[req]{BO¦DY}",
                r"\probe<{a>b}>|same|[req]{BO¦DY}",
            ];
            for call in &calls {
                check(&definition, call, r"\probe<#1>(#2=DEF)|#3|[#4=FALL]{#5}", 4);
            }
            let source = format!("\\documentclass{{article}}\n{definition}\n\\begin{{document}}\nBody.\n{}\n\\end{{document}}\n", calls.join("\n").replace('¦', ""));
            std::fs::write(&main, source).unwrap();
            let config = BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let result = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(result.status, "success", "{}", result.output);
            for marker in [
                "A=a<b>c;B=d(e)f;C=same;D=req;E=BODY",
                "A=-NoValue-;B=DEF;C=same;D=req;E=BODY",
                "A=a>b;B=DEF;C=same;D=req;E=BODY",
            ] {
                assert!(result.output.contains(marker), "{}", result.output);
            }
        }
    }
}

#[test]
fn text_font_commands_skip_encodings_and_retain_optional_defaults_in_call_signatures() {
    for name in [
        "DeclareTextCommand",
        "ProvideTextCommand",
        "DeclareTextCommandDefault",
        "ProvideTextCommandDefault",
    ] {
        let encoding = if name.ends_with("Default") {
            ""
        } else {
            "{OT1}"
        };
        let definition =
            format!("\\{name} % target\r\n\\custom % encoding\r\n{encoding}[2][blue]{{#1:#2}}");
        check(&definition, r"\custom{Val¦ue}", r"\custom[#1=blue]{#2}", 1);
        check(
            &definition,
            r"\custom[re¦d]{Value}",
            r"\custom[#1=blue]{#2}",
            0,
        );
        let (mut workspace, uri, position) = setup(&definition, r"\cust¦");
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == "custom").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing call snippet")
        };
        assert_eq!(edit.new_text, "custom{${1:arg2}}");
        let label = format!(
            "\\{name}{{command}}{}[arguments][default]{{definition}}",
            if encoding.is_empty() {
                ""
            } else {
                "{encoding}"
            }
        );
        let optional_index = if encoding.is_empty() { 1 } else { 2 };
        check(
            "",
            &format!("\\{name}\\custom{encoding}[¦2][blue]{{#1:#2}}"),
            &label,
            optional_index,
        );
        check(
            "",
            &format!("\\{name}{{\\cust¦om}}{encoding}[2][blue]{{#1:#2}}"),
            &label,
            0,
        );
    }
    for name in ["DeclareTextSymbol", "DeclareTextAccent"] {
        check(
            "",
            &format!("\\{name}\\custom{{OT1}}{{6¦5}}"),
            &format!("\\{name}{{command}}{{encoding}}{{slot}}"),
            2,
        );
    }
}

#[test]
fn environment_copy_declarations_hint_the_target_and_source_names() {
    for name in [
        "NewEnvironmentCopy",
        "RenewEnvironmentCopy",
        "DeclareEnvironmentCopy",
    ] {
        let label = format!("\\{name}{{environment}}{{source environment}}");
        check("", &format!("\\{name}{{cop¦y}}{{original}}"), &label, 0);
        check(
            "",
            &format!("\\{name}{{copy}} % keep\r\n{{orig¦inal}}"),
            &label,
            1,
        );
    }
}

#[test]
fn command_copy_declarations_hint_both_command_tokens_with_or_without_groups() {
    for name in ["NewCommandCopy", "RenewCommandCopy", "DeclareCommandCopy"] {
        let label = format!("\\{name}{{command}}{{source command}}");
        for (arguments, active) in [
            (r"{\ali¦as}{\original}", 0),
            (r"\ali¦as\original", 0),
            (r"{\alias}{\orig¦inal}", 1),
            ("\\alias % target\r\n\\orig¦inal", 1),
        ] {
            check("", &format!("\\{name}{arguments}"), &label, active);
        }
        let (mut workspace, uri, position) = setup("", &format!("\\{name}¦"));
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing snippet: {name}")
        };
        assert_eq!(
            edit.new_text,
            format!("{name}{{\\\\${{1:name}}}}{{\\\\${{2:source}}}}")
        );
    }
}

#[test]
fn document_command_declarations_hint_grouped_and_bare_targets_and_preserve_snippet_backslashes() {
    for name in [
        "NewDocumentCommand",
        "RenewDocumentCommand",
        "ProvideDocumentCommand",
        "DeclareDocumentCommand",
        "NewExpandableDocumentCommand",
        "RenewExpandableDocumentCommand",
        "ProvideExpandableDocumentCommand",
        "DeclareExpandableDocumentCommand",
    ] {
        let label = format!("\\{name}{{command}}{{argument specification}}{{definition}}");
        for (arguments, active) in [
            (r"{\cust¦om}{O{blue}m}{#1:#2}", 0),
            (r"\cust¦om{O{blue}m}{#1:#2}", 0),
            (r"{\custom}{O{blue}¦m}{#1:#2}", 1),
            (r"\custom{O{blue}m}{#1:¦#2}", 2),
        ] {
            check("", &format!("\\{name}{arguments}"), &label, active);
        }
        let (mut workspace, uri, position) = setup("", &format!("\\{name}¦"));
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing snippet: {name}")
        };
        assert_eq!(
            edit.new_text,
            format!("{name}{{\\\\${{1:name}}}}{{${{2:m}}}}{{${{3:definition}}}}")
        );
    }
}

#[test]
fn fontspec_declarations_accept_bare_and_grouped_switches_and_both_feature_lists() {
    for name in [
        "newfontfamily",
        "renewfontfamily",
        "providefontfamily",
        "setfontfamily",
        "newfontface",
        "renewfontface",
        "providefontface",
        "setfontface",
    ] {
        let label = format!("\\{name}{{command}}[features]{{font}}[features]");
        for (arguments, active) in [
            (
                r"{\fontsw¦itch}[Scale=1]{Latin Modern Roman}[Ligatures=TeX]",
                0,
            ),
            (
                r"\fontsw¦itch[Scale=1]{Latin Modern Roman}[Ligatures=TeX]",
                0,
            ),
            (
                r"\fontswitch[Sca¦le=1]{Latin Modern Roman}[Ligatures=TeX]",
                1,
            ),
            (r"{\fontswitch}{Latin Mo¦dern Roman}", 2),
            (r"\fontswitch{Latin Mo¦dern Roman}[Ligatures=TeX]", 2),
            (
                r"\fontswitch[Scale=1]{Latin Mo¦dern Roman}[Ligatures=TeX]",
                2,
            ),
            (
                r"{\fontswitch}[Scale=1]{Latin Modern Roman}[Ligatures=¦TeX]",
                3,
            ),
            (r"\fontswitch{Latin Modern Roman}[Ligatures=¦TeX]", 3),
        ] {
            check("", &format!("\\{name}{arguments}"), &label, active);
        }
        let (mut workspace, uri, position) = setup("", &format!("\\{name}¦"));
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing snippet: {name}")
        };
        assert_eq!(
            edit.new_text,
            format!("{name}{{\\\\${{1:command}}}}{{${{2:font}}}}")
        );
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    }
    check("", "\\newfontfamily % target\r\n\\fontswitch % features\r\n[Scale=1] % font\r\n{Latin Mo¦dern Roman}[Ligatures=TeX]", r"\newfontfamily{command}[features]{font}[features]", 2);
}

#[test]
fn classic_font_declarations_hint_font_fields_and_offer_valid_command_snippets() {
    for (marked, label, active) in [
        (
            r"\DeclareTextFontCommand\cust¦om{\bfseries}",
            r"\DeclareTextFontCommand{command}{font commands}",
            0,
        ),
        (
            r"\DeclareTextFontCommand{\custom}{\bfse¦ries}",
            r"\DeclareTextFontCommand{command}{font commands}",
            1,
        ),
        (
            r"\DeclareFixedFont\custom{OT1}{cmr}{m}{n}{1¦0}",
            r"\DeclareFixedFont{command}{encoding}{family}{series}{shape}{size}",
            5,
        ),
        (
            r"\DeclareMathAlphabet{\custom}{OT1}{cmr}{m}{i¦t}",
            r"\DeclareMathAlphabet{command}{encoding}{family}{series}{shape}",
            4,
        ),
        (
            r"\DeclareSymbolFontAlphabet\custom{oper¦ators}",
            r"\DeclareSymbolFontAlphabet{command}{symbol font}",
            1,
        ),
        (
            r#"\DeclareMathSymbol\custom{\mathord}{letters}{"0¦B}"#,
            r"\DeclareMathSymbol{command}{class}{symbol font}{slot}",
            3,
        ),
        (
            r"\DeclareMathAccent\custom{\mathalpha}{operators}{9¦4}",
            r"\DeclareMathAccent{command}{class}{symbol font}{slot}",
            3,
        ),
        (
            r"\DeclareMathRadical{\custom}{symbols}{112}{largesymbols}{11¦2}",
            r"\DeclareMathRadical{command}{small symbol font}{small slot}{large symbol font}{large slot}",
            4,
        ),
        (
            r"\DeclareMathDelimiter\custom{\mathopen}{symbols}{104}{largesymbols}{1¦0}",
            r"\DeclareMathDelimiter{command}{class}{small symbol font}{small slot}{large symbol font}{large slot}",
            5,
        ),
    ] {
        check("", marked, label, active);
    }
    for (name, expected) in [
        (
            "DeclareTextFontCommand",
            r"DeclareTextFontCommand{\\${1:command}}{\\${2:bfseries}}",
        ),
        (
            "DeclareFixedFont",
            r"DeclareFixedFont{\\${1:command}}{${2:OT1}}{${3:cmr}}{${4:m}}{${5:n}}{${6:10}}",
        ),
        (
            "DeclareMathAlphabet",
            r"DeclareMathAlphabet{\\${1:command}}{${2:OT1}}{${3:cmr}}{${4:m}}{${5:it}}",
        ),
        (
            "DeclareSymbolFontAlphabet",
            r"DeclareSymbolFontAlphabet{\\${1:command}}{${2:operators}}",
        ),
        (
            "DeclareMathSymbol",
            r"DeclareMathSymbol{\\${1:command}}{\\${2:mathord}}{${3:letters}}{${4:0}}",
        ),
    ] {
        let (mut workspace, uri, position) = setup("", &format!("\\{name}¦"));
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, expected);
    }
}

#[test]
fn paired_delimiter_signatures_follow_body_counts_and_exclusive_sizing_modes() {
    for definition in [
        r"\DeclarePairedDelimiter{\norm}{\lVert}{\rVert}",
        r"\DeclarePairedDelimiter\norm{(}{)}",
        r"\DeclarePairedDelimiterX{\norm}[1]{(}{)}{#1}",
        r"\DeclarePairedDelimiterXPP\norm[1]{\mathopen{}}{(}{)}{^2}{#1}",
        r"\expandafter\DeclarePairedDelimiterX\csname norm\endcsname[1]{(}{)}{#1}",
    ] {
        for (call, active) in [
            (r"\norm{Te¦xt}", 2),
            (r"\norm*{Te¦xt}", 2),
            (r"\norm[\Bi¦g]{Text}", 1),
            (r"\norm[\Big]{Te¦xt}", 2),
            ("\\norm % star\r\n* % body\r\n{Te¦xt}", 2),
        ] {
            check(definition, call, r"\norm*[size]{#1}", active);
        }
        for call in [r"\norm*[\Big]{Te¦xt}", r"\norm{Text}Body¦"] {
            let (workspace, uri, position) = setup(definition, call);
            assert!(
                signature::help(&workspace, &uri, position).is_none(),
                "{definition}: {call}"
            );
        }
        let (mut workspace, uri, position) = setup(definition, r"\no¦");
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == "norm").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, "norm{${1:arg1}}");
    }
    for definition in [
        r"\DeclarePairedDelimiterX{\pair}[2]{\langle}{\rangle}{#1,#2}",
        r"\DeclarePairedDelimiterXPP{\pair}[2]{}{(}{)}{}{#1,#2}",
    ] {
        check(definition, r"\pair{one}{tw¦o}", r"\pair*[size]{#1}{#2}", 3);
        check(definition, r"\pair*{one}{tw¦o}", r"\pair*[size]{#1}{#2}", 3);
        check(
            definition,
            r"\pair[\big]{one}{tw¦o}",
            r"\pair*[size]{#1}{#2}",
            3,
        );
        let (mut workspace, uri, position) = setup(definition, r"\pa¦");
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == "pair").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, "pair{${1:arg1}}{${2:arg2}}");
    }
    for count in [
        "",
        "0",
        "10",
        "-1",
        "9999999999999999999999999",
        r"\countmacro",
    ] {
        let definition = format!("\\DeclarePairedDelimiterX{{\\pair}}[{count}]{{(}}{{)}}{{#1}}");
        let (workspace, uri, position) = setup(&definition, r"\pair{Te¦xt}");
        assert!(
            signature::help(&workspace, &uri, position).is_none(),
            "{count}"
        );
    }
    let (workspace, uri, position) = setup(
        r"\DeclarePairedDelimiter{\pair}{(}{)}\DeclarePairedDelimiterX{\pair}[2]{(}{)}{#1,#2}",
        r"\pair{Te¦xt}",
    );
    assert!(signature::help(&workspace, &uri, position).is_none());
}

#[test]
fn paired_delimiter_declarations_offer_target_count_and_body_hints_and_snippets() {
    for (marked, label, active) in [
        (
            r"\DeclarePairedDelimiter\norm{le¦ft}{right}",
            r"\DeclarePairedDelimiter{command}{left}{right}",
            1,
        ),
        (
            r"\DeclarePairedDelimiter\no¦rm{(}{)}",
            r"\DeclarePairedDelimiter{command}{left}{right}",
            0,
        ),
        (
            r"\DeclarePairedDelimiter{\norm}{\lv¦ert}{\rvert}",
            r"\DeclarePairedDelimiter{command}{left}{right}",
            1,
        ),
        (
            r"\DeclarePairedDelimiterX\pair[¦2]{(}{)}{#1,#2}",
            r"\DeclarePairedDelimiterX{command}[arguments]{left}{right}{body}",
            1,
        ),
        (
            r"\DeclarePairedDelimiterX{\pair}[2]{(}{)}{#1,¦#2}",
            r"\DeclarePairedDelimiterX{command}[arguments]{left}{right}{body}",
            4,
        ),
        (
            r"\DeclarePairedDelimiterXPP\norm[1]{pre¦}{(}{)}{post}{#1}",
            r"\DeclarePairedDelimiterXPP{command}[arguments]{before}{left}{right}{after}{body}",
            2,
        ),
        (
            r"\DeclarePairedDelimiterXPP{\norm}[1]{pre}{(}{)}{post¦}{#1}",
            r"\DeclarePairedDelimiterXPP{command}[arguments]{before}{left}{right}{after}{body}",
            5,
        ),
        (
            r"\DeclarePairedDelimiterXPP{\norm}[1]{pre}{(}{)}{post}{#¦1}",
            r"\DeclarePairedDelimiterXPP{command}[arguments]{before}{left}{right}{after}{body}",
            6,
        ),
    ] {
        check("", marked, label, active);
    }
    for (name, expected) in [
        (
            "DeclarePairedDelimiter",
            r"DeclarePairedDelimiter{\\${1:command}}{${2:left}}{${3:right}}",
        ),
        (
            "DeclarePairedDelimiterX",
            r"DeclarePairedDelimiterX{\\${1:command}}[${2:1}]{${3:left}}{${4:right}}{${5:body}}",
        ),
        (
            "DeclarePairedDelimiterXPP",
            r"DeclarePairedDelimiterXPP{\\${1:command}}[${2:1}]{${3:before}}{${4:left}}{${5:right}}{${6:after}}{${7:body}}",
        ),
    ] {
        let (mut workspace, uri, position) = setup("", &format!("\\{name}¦"));
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, expected);
    }
    let (workspace, uri, position) = setup("", r"\DeclarePairedDelimiterX{\pair}{(}{)}{Te¦xt}");
    assert!(
        signature::help(&workspace, &uri, position).is_none(),
        "X requires an explicit bracketed count"
    );
}

#[test]
fn pdfbookmark_hints_distinguish_level_title_and_destination_base_and_offer_a_snippet() {
    for (marked, active) in [
        (r"\pdfbookmark[¦1]{Title}{name}", 0),
        (r"\pdfbookmark{Ti¦tle}{name}", 1),
        (r"\pdfbookmark[01]{Title}{na¦me}", 2),
    ] {
        check("", marked, r"\pdfbookmark[level]{title}{name}", active);
    }
    let (mut workspace, uri, position) = setup("", r"\pdfbook¦");
    workspace.snippets = true;
    let items = latex_lsp::features::completion(&workspace, &uri, position);
    let item = items
        .iter()
        .find(|item| item.label == "pdfbookmark")
        .unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.new_text, "pdfbookmark{${1:title}}{${2:name}}");
}

#[test]
fn hyperref_signatures_distinguish_label_links_from_four_argument_urls() {
    for (marked, label, active) in [
        (r"\hyperref[sec:¦intro]{Link}", r"\hyperref[key]{text}", 0),
        (r"\hyperref[sec:intro]{Li¦nk}", r"\hyperref[key]{text}", 1),
        (
            r"\hyperref{https://example.com/a%20¦b}{category}{name}{Link}",
            r"\hyperref{url}{category}{name}{text}",
            0,
        ),
        (
            r"\hyperref{url}{cat¦egory}{name}{Link}",
            r"\hyperref{url}{category}{name}{text}",
            1,
        ),
        (
            r"\hyperref{url}{category}{na¦me}{Link}",
            r"\hyperref{url}{category}{name}{text}",
            2,
        ),
        (
            r"\hyperref{url}{category}{name}{Li¦nk}",
            r"\hyperref{url}{category}{name}{text}",
            3,
        ),
    ] {
        check("", marked, label, active);
    }
    check(
        r"\newcommand{\hyperref}[2]{#1 #2}",
        r"\hyperref{one}{tw¦o}",
        r"\hyperref{#1}{#2}",
        1,
    );
}

#[test]
fn citation_note_signatures_stop_at_unprotected_closing_brackets() {
    for command in ["citep", "fullcite", "footfullcite"] {
        let label = format!("\\{command}[note / prenote][postnote]{{keys}}");
        check(
            "",
            &format!("\\{command}[see [li¦teral][p.~2]{{parent}}"),
            &label,
            0,
        );
        check(
            "",
            &format!("\\{command}[see [literal][p.¦~2]{{parent}}"),
            &label,
            1,
        );
        check(
            "",
            &format!("\\{command}[see [literal][p.~2]{{pa¦rent}}"),
            &label,
            2,
        );
        check(
            "",
            &format!("\\{command}[see {{nested [note]}}][p.~2]{{pa¦rent}}"),
            &label,
            2,
        );
        for notes in ["[see [nested]]", "[see [nested]][p.~2]", "[see][p.~[2]]"] {
            let marked = format!("\\{command}{notes}{{pa¦rent}}");
            let (workspace, uri, at) = setup("", &marked);
            assert!(signature::help(&workspace, &uri, at).is_none(), "{marked}");
        }
    }
}

#[test]
fn full_citation_signatures_and_snippets_preserve_optional_notes() {
    for command in ["fullcite", "footfullcite"] {
        let label = format!("\\{command}[note / prenote][postnote]{{keys}}");
        check("", &format!("\\{command}[se¦e][p.~2]{{parent}}"), &label, 0);
        check("", &format!("\\{command}[see][p.¦~2]{{parent}}"), &label, 1);
        check("", &format!("\\{command}[see][p.~2]{{pa¦rent}}"), &label, 2);
        let marked = format!("\\{command}¦");
        let (mut workspace, uri, at) = setup("", &marked);
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, at);
        let item = items.iter().find(|item| item.label == command).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing snippet")
        };
        assert_eq!(edit.new_text, format!("{command}{{${{1:key}}}}"));
    }
}

#[test]
fn natbib_signatures_distinguish_numeric_citations_alias_text_and_optional_notes() {
    check(
        "",
        "\\Citealp % before\r\n* % after\r\n[see][p.~2]{pa¦rent}",
        r"\Citealp[note / prenote][postnote]{keys}",
        2,
    );
    check(
        "",
        r"\defcitealias{pa¦rent}{Display}",
        r"\defcitealias{key}{text}",
        0,
    );
    check(
        "",
        r"\defcitealias{parent}{Dis¦play}",
        r"\defcitealias{key}{text}",
        1,
    );
    check("", r"\citenum{pa¦rent}", r"\citenum{keys}", 0);
    for command in [
        "citealt",
        "citealp",
        "citeyearpar",
        "citefullauthor",
        "Citet",
        "Citep",
        "Citealt",
        "Citealp",
        "Citeauthor",
        "citetalias",
        "citepalias",
    ] {
        let label = format!("\\{command}[note / prenote][postnote]{{keys}}");
        check("", &format!("\\{command}[se¦e][p.~2]{{parent}}"), &label, 0);
        check("", &format!("\\{command}[see][p.¦~2]{{parent}}"), &label, 1);
        check("", &format!("\\{command}[see][p.~2]{{pa¦rent}}"), &label, 2);
    }
}

#[test]
fn hyperdef_signatures_offer_optional_labels_and_separate_destination_fields() {
    for (marked, active) in [
        (r"\hyperdef[sec:¦intro]{section}{1}{Text}", 0),
        (r"\hyperdef[sec:intro]{sec¦tion}{1}{Text}", 1),
        (r"\hyperdef{section}{na¦me}{Text}", 2),
        (r"\hyperdef[{sec:intro]}]{section}{1}{Te¦xt}", 3),
    ] {
        check("", marked, r"\hyperdef[key]{category}{name}{text}", active);
    }
    let (mut workspace, uri, position) = setup("", r"\hyperd¦");
    workspace.snippets = true;
    let items = latex_lsp::features::completion(&workspace, &uri, position);
    let item = items.iter().find(|item| item.label == "hyperdef").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    assert_eq!(
        edit.new_text,
        "hyperdef[${1:key}]{${2:category}}{${3:name}}{${4:text}}"
    );
}

#[test]
fn pdf_destination_commands_hint_target_and_text_parameters() {
    for name in ["hypertarget", "hyperlink"] {
        check(
            "",
            &format!("\\{name}{{dest¦}}{{Text}}"),
            &format!("\\{name}{{target}}{{text}}"),
            0,
        );
        check(
            "",
            &format!("\\{name}{{dest}}{{Te¦xt}}"),
            &format!("\\{name}{{target}}{{text}}"),
            1,
        );
    }
}

#[test]
fn refcount_queries_offer_label_field_default_and_branch_parameters() {
    for (marked, label, active) in [
        (r"\getrefnumber{key¦}", r"\getrefnumber{key}", 0),
        (r"\getpagerefnumber{key¦}", r"\getpagerefnumber{key}", 0),
        (r"\refused{key¦}", r"\refused{key}", 0),
        (
            r"\getrefbykeydefault{key}{pa¦ge}{0}",
            r"\getrefbykeydefault{key}{field}{default}",
            1,
        ),
        (
            r"\getrefbykeydefault{key}{page}{0¦}",
            r"\getrefbykeydefault{key}{field}{default}",
            2,
        ),
        (
            r"\IfRefUndefinedExpandable{key}{missing¦}{found}",
            r"\IfRefUndefinedExpandable{key}{undefined}{defined}",
            1,
        ),
        (
            r"\IfRefUndefinedBabel{key}{missing}{found¦}",
            r"\IfRefUndefinedBabel{key}{undefined}{defined}",
            2,
        ),
    ] {
        check("", marked, label, active);
    }
    let (mut workspace, uri, position) = setup("", r"\getrefb¦");
    workspace.snippets = true;
    let items = latex_lsp::features::completion(&workspace, &uri, position);
    let item = items
        .iter()
        .find(|item| item.label == "getrefbykeydefault")
        .unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    assert_eq!(
        edit.new_text,
        "getrefbykeydefault{${1:key}}{${2:page}}{${3:0}}"
    );
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    for name in [
        "ref",
        "Ref",
        "pageref",
        "autopageref",
        "autoref",
        "nameref",
        "cref",
        "Cref",
    ] {
        check(
            "",
            &format!("\\{name}*{{key¦}}"),
            &format!("\\{name}{{keys}}"),
            0,
        );
        check(
            "",
            &format!("\\{name} % star\n * {{key¦}}"),
            &format!("\\{name}{{keys}}"),
            0,
        );
    }
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
fn classic_macro_defaults_and_calls_end_at_the_first_unbraced_closing_bracket() {
    for declaration in [
        r"\newcommand{\pair}[2][x[y]{#1:#2}",
        r"\newcommand\pair[2][x[y]{#1:#2}",
        r"\renewcommand{\pair}[2][x[y]{#1:#2}",
        r"\providecommand{\pair}[2][x[y]{#1:#2}",
        r"\DeclareRobustCommand{\pair}[2][x[y]{#1:#2}",
        r"\newrobustcmd{\pair}[2][x[y]{#1:#2}",
        r"\renewrobustcmd*\pair[2][x[y]{#1:#2}",
        r"\providerobustcmd\pair[2][x[y]{#1:#2}",
        r"\DeclareTextCommand{\pair}{OT1}[2][x[y]{#1:#2}",
        r"\DeclareTextCommandDefault{\pair}[2][x[y]{#1:#2}",
    ] {
        for call in [
            r"\pair{val¦ue}",
            r"\pair[x[y]{val¦ue}",
            r"\pair[{x]y}]{val¦ue}",
            "😀 \\pair[x[y] % ] ignored\r\n{val¦ue}",
        ] {
            check(declaration, call, r"\pair[#1=x[y]{#2}", 1);
        }
        check(
            declaration,
            r"\csuse{pair}[x[y]{val¦ue}",
            r"\csuse{pair}[#1=x[y]{#2}",
            1,
        );
        check(
            declaration,
            r"\csname pair\endcsname[x[y]{val¦ue}",
            r"\csname pair\endcsname[#1=x[y]{#2}",
            1,
        );
        for call in [r"\pair[x[y]{value}Body¦", r"\pair[x[y]]{val¦ue}"] {
            let (workspace, uri, at) = setup(declaration, call);
            assert!(signature::help(&workspace, &uri, at).is_none(), "{call}");
        }
    }
    let (mut workspace, uri, at) = setup(r"\newcommand{\pair}[2][x[y]{#1:#2}", r"\pa¦");
    workspace.snippets = true;
    let items = latex_lsp::features::completion(&workspace, &uri, at);
    let item = items.iter().find(|item| item.label == "pair").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing macro snippet")
    };
    assert_eq!(edit.new_text, "pair{${1:arg2}}");
}

#[test]
fn classic_environment_defaults_follow_their_optional_argument_reader() {
    for declaration in [
        "newenvironment",
        "renewenvironment",
        "provideenvironment",
        "NewEnviron",
        "RenewEnviron",
    ] {
        let definitions = format!("\\{declaration}{{panel}}[2][x[y]{{#1:#2}}{{}}");
        for call in [
            r"\begin{panel}{val¦ue}",
            r"\begin{panel}[x[y]{val¦ue}",
            r"\begin{panel}[{x]y}]{val¦ue}",
        ] {
            check(&definitions, call, r"\begin{panel}[#1=x[y]{#2}", 2);
        }
        check(
            &definitions,
            r"\panel[x[y]{val¦ue}",
            r"\panel[#1=x[y]{#2}",
            1,
        );
    }
    for call in [
        r"\begin{result}[x[y]Body¦\end{result}",
        r"\result[x[y]Body¦\endresult",
    ] {
        let (workspace, uri, at) = setup(r"\newtheorem{result}{Result}", call);
        assert!(signature::help(&workspace, &uri, at).is_none(), "{call}");
    }
}

#[test]
fn xparse_optional_brackets_keep_nesting_even_when_overriding_a_builtin_name() {
    for name in [
        "pair",
        "hyperref",
        "hyperdef",
        "newcounter",
        "fullcite",
        "section",
        "caption",
        "sqrt",
    ] {
        let definitions = format!("\\NewDocumentCommand{{\\{name}}}{{O{{x[y}} m}}{{#1:#2}}");
        let label = format!("\\{name}[#1=x[y]{{#2}}");
        check(
            &definitions,
            &format!("\\{name}[x[y]]{{val¦ue}}"),
            &label,
            1,
        );
        check(&definitions, &format!("\\{name}[x[y]{{val¦ue}}"), &label, 0);
    }
    check(
        r"\NewDocumentEnvironment{panel}{O{x[y}m}{#1:#2}{}",
        r"\begin{panel}[x[y]]{val¦ue}",
        r"\begin{panel}[#1=x[y]{#2}",
        2,
    );
}

#[test]
fn heading_caption_and_root_signatures_end_optional_arguments_at_unbraced_closing_brackets() {
    for name in [
        "part",
        "chapter",
        "section",
        "subsection",
        "subsubsection",
        "paragraph",
        "subparagraph",
        "caption",
        "sqrt",
    ] {
        let label = if name == "sqrt" {
            r"\sqrt[degree]{expression}".into()
        } else {
            format!("\\{name}[short title]{{title}}")
        };
        check(
            "",
            &format!("\\{name}[literal [opening]{{val¦ue}}"),
            &label,
            1,
        );
        check(
            "",
            &format!("\\{name}[{{protected ] closing}}]{{val¦ue}}"),
            &label,
            1,
        );
        check(
            "",
            &format!("\\{name}[literal [ope¦ning]{{value}}"),
            &label,
            0,
        );
        check(
            "",
            &format!("😀 \\{name}[literal [opening] % ignored ]\r\n{{val¦ue}}"),
            &label,
            1,
        );
        check(
            "",
            &format!("\\csuse{{{name}}}[literal [opening]{{val¦ue}}"),
            &label.replace(&format!("\\{name}"), &format!("\\csuse{{{name}}}")),
            1,
        );
        for call in [
            format!("\\{name}[literal [opening]{{value}}Body¦"),
            format!("\\{name}[literal [opening]]{{val¦ue}}"),
        ] {
            let (workspace, uri, at) = setup("", &call);
            assert!(signature::help(&workspace, &uri, at).is_none(), "{call}");
        }
    }
}

#[test]
fn starred_headings_hint_the_title_and_do_not_accept_a_short_title() {
    for name in [
        "part",
        "chapter",
        "section",
        "subsection",
        "subsubsection",
        "paragraph",
        "subparagraph",
    ] {
        for command in [
            format!("\\{name}"),
            format!("\\csuse{{{name}}}"),
            format!("\\csname {name}\\endcsname"),
        ] {
            let label = format!("{command}[short title]{{title}}");
            check("", &format!("{command}*¦"), &label, 1);
            check("", &format!("{command}*{{val¦ue}}"), &label, 1);
            check(
                "",
                &format!("{command}* % ignored\r\n{{val¦ue}}"),
                &label,
                1,
            );
            check("", &format!("{command}[Short]{{val¦ue}}"), &label, 1);
            for call in [
                format!("{command}*[Short]{{Body¦}}"),
                format!("{command}*[Sh¦ort]{{Body}}"),
                format!("{command}* % ignored\r\n[Short]{{Body¦}}"),
                format!("{command}*{{Title}}Body¦"),
            ] {
                let (workspace, uri, at) = setup("", &call);
                assert!(signature::help(&workspace, &uri, at).is_none(), "{call}");
            }
        }
        let definitions = format!("\\NewDocumentCommand{{\\{name}}}{{s O{{x[y}} m}}{{#2:#3}}");
        check(
            &definitions,
            &format!("\\{name}*[x[y]]{{val¦ue}}"),
            &format!("\\{name}*[#2=x[y]{{#3}}"),
            2,
        );
    }
}

#[test]
fn named_builtin_calls_keep_the_same_optional_boundaries_as_direct_calls() {
    check(
        "",
        r"\csuse{citep}[x[y]{ke¦y}",
        r"\csuse{citep}[note / prenote][postnote]{keys}",
        2,
    );
    check(
        "",
        r"\csname vref\endcsname[x[y]{ke¦y}",
        r"\csname vref\endcsname[same-page]{key}",
        1,
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn optional_signature_boundaries_match_the_values_received_by_real_tex_macros() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;

    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    for modern in [false, true] {
        let declaration = if modern {
            r"\NewDocumentCommand{\pair}{O{x[y}m}"
        } else {
            r"\newcommand{\pair}[2][x[y]"
        };
        let definitions = format!("{declaration}{{\\typeout{{FIRST=#1;SECOND=#2}}#1:#2}}");
        let optional = if modern { "[x[y]]" } else { "[x[y]" };
        let calls = [
            r"\pair{DEFAULTBO¦DY}".to_string(),
            format!("\\pair{optional}{{EXPLICITBO¦DY}}"),
            r"\pair[{x]y}]{PROTECTEDBO¦DY}".into(),
        ];
        for call in &calls {
            check(&definitions, call, r"\pair[#1=x[y]{#2}", 1);
        }
        let source = format!(
            "\\documentclass{{article}}\n{definitions}\n\\begin{{document}}\n{}\n\\end{{document}}\n",
            calls.join("\n").replace('¦', "")
        );
        std::fs::write(&main, source).unwrap();
        let result = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(result.status, "success", "{}", result.output);
        for marker in [
            "FIRST=x[y;SECOND=DEFAULTBODY".to_string(),
            format!(
                "FIRST={};SECOND=EXPLICITBODY",
                if modern { "x[y]" } else { "x[y" }
            ),
            "FIRST=x]y;SECOND=PROTECTEDBODY".into(),
        ] {
            assert!(result.output.contains(&marker), "{}", result.output);
        }
    }
}

#[test]
fn xparse_and_undelimited_tex_macros_report_the_correct_parameter() {
    for command in [
        "NewDocumentCommand",
        "RenewDocumentCommand",
        "ProvideDocumentCommand",
        "DeclareDocumentCommand",
        "NewExpandableDocumentCommand",
        "RenewExpandableDocumentCommand",
        "ProvideExpandableDocumentCommand",
        "DeclareExpandableDocumentCommand",
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
        (r"\NewDocumentCommand{\pair}{v m}{#2}", r"\pair{a}{b¦}"),
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
        r"\NewDocumentEnvironment{panel}{v m}{}{}",
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
        r"\NewDocumentEnvironment{panel}{v m}{}{}",
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

#[test]
fn math_operator_declarations_and_inline_names_hint_starred_and_bare_control_targets() {
    for (source, active) in [
        (r"\DeclareMathOperator\loc¦alop{local}", 0),
        (r"\DeclareMathOperator*{\localop}{lo¦cal}", 1),
        (
            "\\DeclareMathOperator % star\r\n* % target\r\n\\localop % text\r\n{lo¦cal}",
            1,
        ),
    ] {
        check("", source, r"\DeclareMathOperator{command}{text}", active);
    }
    for source in [
        r"\operatorname{ar¦g\,max}",
        r"\operatorname*{ar¦g\,max}_{x}",
    ] {
        check("", source, r"\operatorname{text}", 0);
    }
    let (workspace, uri, position) = setup(
        r"\DeclareMathOperator*{\localop}{arg\,max}",
        r"\localop{x¦}",
    );
    assert!(signature::help(&workspace, &uri, position).is_none());
}

#[test]
fn package_and_class_loader_signatures_include_minimum_versions_and_forwarded_options() {
    for name in ["documentclass", "usepackage", "RequirePackage", "LoadClass"] {
        let label = format!("\\{name}[options]{{name}}[minimum version]");
        for (arguments, active) in [
            ("[dra¦ft]{local}[2020/01/01]", 0),
            ("{loc¦al}[2020/01/01]", 1),
            ("[draft]{local} % version\r\n[2020/¦01/01]", 2),
        ] {
            check("", &format!("\\{name}{arguments}"), &label, active);
        }
    }
    for name in ["RequirePackageWithOptions", "LoadClassWithOptions"] {
        let label = format!("\\{name}{{name}}[minimum version]");
        check("", &format!("\\{name}{{loc¦al}}[2020/01/01]"), &label, 0);
        check(
            "",
            &format!("\\{name}{{local}} % version\r\n[2020/¦01/01]"),
            &label,
            1,
        );
        let (workspace, uri, position) = setup("", &format!("\\{name}[draft]{{loc¦al}}"));
        assert!(signature::help(&workspace, &uri, position).is_none());
    }
    for name in [
        "RequirePackage",
        "LoadClass",
        "RequirePackageWithOptions",
        "LoadClassWithOptions",
    ] {
        let (mut workspace, uri, position) = setup("", &format!("\\{name}¦"));
        workspace.snippets = true;
        let items = latex_lsp::features::completion(&workspace, &uri, position);
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        let argument = if name.starts_with("LoadClass") {
            "class"
        } else {
            "package"
        };
        assert_eq!(edit.new_text, format!("{name}{{${{1:{argument}}}}}"));
        let Some(Documentation::String(documentation)) = &item.documentation else {
            panic!()
        };
        assert!(!documentation.is_empty());
    }
}
