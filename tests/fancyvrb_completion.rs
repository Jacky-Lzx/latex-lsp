use lsp_types::*;
use texquill::{
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};

fn fixture(marked: &str) -> (Workspace, Url, Position) {
    let at = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let main = Url::parse("file:///fancy-completion/main.tex").unwrap();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            snippets: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.open(main.clone(), source.clone(), 7);
    (workspace, main, Text::new(source).position(at))
}

#[test]
fn inline_command_templates_preserve_existing_code_names_and_control_operands() {
    let (workspace, main, at) = fixture("\\Ve¦");
    let items = features::completion(&workspace, &main, at);
    let item = items.iter().find(|item| item.label == "Verb").unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.new_text, "Verb|${1:code}|");
    for (marked, name) in [
        (r"\Ve¦rb|\label{phantom}|", "Verb"),
        (r"\Ve¦rb{|}text|", "Verb"),
        (r"\Ve¦rb *[showspaces=true]|code|", "Verb"),
        (r"\usepackage{fvextra}\Ve¦rb%\label{phantom}%", "Verb"),
        (r"\usepackage{fvextra}\Ve¦rb\label{phantom}\", "Verb"),
        (r"\let\local\Ve¦", "Verb"),
        (r"\Sa¦veVerb x|code|", "SaveVerb"),
        (r"\Sa¦veGVerb\slot|code|", "SaveGVerb"),
        (r"\UseV¦erb{saved}", "UseVerb"),
        (r"\UseV¦erbatim x", "UseVerbatim"),
        (r"\UseV¦erb 中", "UseVerb"),
    ] {
        let (workspace, main, at) = fixture(marked);
        let items = features::completion(&workspace, &main, at);
        let item = items
            .iter()
            .find(|item| item.label == name)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        assert_eq!(
            item.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT),
            "{marked}"
        );
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, name, "{marked}");
    }
    let (workspace, main, at) = fixture(r"\textbf{\Ve¦rb}");
    let items = features::completion(&workspace, &main, at);
    let item = items.iter().find(|item| item.label == "Verb").unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    let (mut workspace, main, at) = fixture("\\Ve¦");
    workspace.client.snippets = false;
    let items = features::completion(&workspace, &main, at);
    let item = items.iter().find(|item| item.label == "Verb").unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
    let (workspace, main, at) = fixture(r"\newcommand{\Verb}[1]{#1}\Ve¦");
    let items = features::completion(&workspace, &main, at);
    let item = items.iter().find(|item| item.label == "Verb").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert!(edit.new_text.starts_with("Verb{"), "{edit:?}");
    assert!(!edit.new_text.contains('|'));
}

#[test]
fn saved_name_hints_follow_ordinary_tokens_and_the_first_unbraced_option_end() {
    for command in [
        "SaveVerb",
        "SaveGVerb",
        "UseVerb",
        "UseVerbatim",
        "LUseVerbatim",
        "BUseVerbatim",
        "UseMVerb",
        "pUseMVerb",
    ] {
        for (tail, parameter) in [
            ("[frame=si¦ngle]", 0),
            ("*[label={A]B},frame=si¦", 0),
            ("{na¦me}", 1),
            ("[label=[literal]{na¦me}", 1),
            (" ¦x", 1),
            (" \\slot¦", 1),
            ("% header\r\n{na% hidden }\r\nme¦}", 1),
            ("{na¦", 1),
        ] {
            let marked = format!("% 😀\r\n\\{command}{tail}");
            let (workspace, main, at) = fixture(&marked);
            let help = signature::help(&workspace, &main, at).unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(
                help.signatures[0].label,
                format!("\\{command}[options]{{name}}"),
                "{marked}"
            );
            assert_eq!(help.active_parameter, Some(parameter), "{marked}");
        }
    }
    for marked in [
        r"\Verb|code¦|",
        r"\SaveVerb{saved}|code¦|",
        r"\SaveGVerb{saved}|code¦|",
        "\\UseVerb{na% comment¦\r\nme}",
        "\\SaveVerb% header¦\r\n{saved}|code|",
        r"\UseVerb[frame=single]{saved} prose¦",
        r"\UseVerb x¦",
    ] {
        let (workspace, main, at) = fixture(marked);
        assert!(signature::help(&workspace, &main, at).is_none(), "{marked}");
    }
    let (workspace, main, at) = fixture(r"\Verb[label=[literal]¦|code|");
    assert!(signature::help(&workspace, &main, at).is_none());
}

#[test]
#[ignore = "requires pdflatex, xelatex, and fvextra"]
fn real_completed_inline_templates_preserve_saved_text_and_literal_commands() {
    use std::sync::atomic::AtomicBool;
    use texquill::{compilation::build, config::BuildConfig};
    let mut calls = Vec::new();
    for name in [
        "fvset",
        "fvinlineset",
        "Verb",
        "SaveVerb",
        "SaveGVerb",
        "UseVerb",
        "UseMVerb",
        "pUseMVerb",
        "UseVerbatim",
        "LUseVerbatim",
        "BUseVerbatim",
    ] {
        let (workspace, main, at) = fixture(&format!("\\{name}¦"));
        let items = features::completion(&workspace, &main, at);
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert_eq!(
            item.insert_text_format,
            Some(InsertTextFormat::SNIPPET),
            "{name}"
        );
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        let saved = match name {
            "SaveGVerb" | "UseMVerb" | "pUseMVerb" => "moving",
            "UseVerbatim" | "LUseVerbatim" | "BUseVerbatim" => "block",
            _ => "saved",
        };
        let code = r"Text \label{phantom}\input{fake}";
        let snippet = edit
            .new_text
            .replace("${1:name}", saved)
            .replace("${1:code}", code)
            .replace("${2:code}", code)
            .replace("${1:options}", "showspaces=true");
        assert!(!snippet.contains("${"), "{snippet}");
        calls.push(format!("\\{snippet}\n"));
    }
    let source = format!("\\documentclass{{article}}\n\\usepackage{{fvextra}}\n\\begin{{document}}\n\\begin{{SaveVerbatim}}{{block}}\nText \\label{{phantom}}\\input{{fake}}\n\\end{{SaveVerbatim}}\n{}\\label{{real}}\\end{{document}}\n", calls.concat());
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let outcome = build::run(
            &root.join("main.tex"),
            &BuildConfig {
                executable: engine.into(),
                ..Default::default()
            },
            &AtomicBool::new(false),
        );
        assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
        let aux = std::fs::read_to_string(root.join("build/texquill/main.aux")).unwrap();
        assert!(aux.contains("newlabel{real}"), "{aux}");
        assert!(!aux.contains("newlabel{phantom}"), "{aux}");
    }
}
