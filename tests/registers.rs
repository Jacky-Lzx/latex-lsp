use latex_lsp::{build, features, signature, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};

fn setup(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn renamed(
    workspace: &Workspace,
    uri: &Url,
    position: Position,
    name: &str,
) -> BTreeMap<Url, String> {
    features::rename(workspace, uri, position, name)
        .unwrap()
        .unwrap()
        .changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = workspace.documents[&uri]
                .text
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
            (uri, text.source)
        })
        .collect()
}

const FONT_DECLARATIONS: [&str; 8] = [
    "newfontfamily",
    "renewfontfamily",
    "providefontfamily",
    "setfontfamily",
    "newfontface",
    "renewfontface",
    "providefontface",
    "setfontface",
];

const CLASSIC_FONT_DECLARATIONS: [(&str, &str, bool, usize); 14] = [
    ("DeclareTextFontCommand", r"{\bfseries}", false, 1),
    ("DeclareFixedFont", "{OT1}{cmr}{m}{n}{10}", true, 0),
    ("DeclareMathAlphabet", "{OT1}{cmr}{m}{it}", false, 1),
    ("DeclareSymbolFontAlphabet", "{operators}", false, 1),
    ("DeclareMathSymbol", r#"{\mathord}{letters}{"0B}"#, true, 0),
    ("DeclareTextCommand", "{OT1}[1]{#1}", false, 1),
    ("ProvideTextCommand", "{OT1}[1]{#1}", false, 1),
    ("DeclareTextCommandDefault", "[1]{#1}", false, 1),
    ("ProvideTextCommandDefault", "[1]{#1}", false, 1),
    ("DeclareTextSymbol", "{OT1}{65}", true, 0),
    ("DeclareTextAccent", "{OT1}{19}", false, 1),
    (
        "DeclareMathAccent",
        r"{\mathalpha}{operators}{94}",
        false,
        1,
    ),
    (
        "DeclareMathRadical",
        "{symbols}{112}{largesymbols}{112}",
        false,
        1,
    ),
    (
        "DeclareMathDelimiter",
        r"{\mathopen}{symbols}{104}{largesymbols}{10}",
        true,
        0,
    ),
];

#[test]
#[ignore = "requires pdflatex"]
fn real_text_font_declaration_renames_preserve_defaults_symbols_accents_and_fallbacks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = concat!(
        "\\DeclareTextCommand{\\fonttextcmd}{OT1}[2][blue]{#1:#2}\n",
        "\\ProvideTextCommand\\fontprovidecmd{OT1}[1]{#1}\n",
        "\\DeclareTextCommandDefault{\\fontdefaultcmd}[1]{#1}\n",
        "\\ProvideTextCommandDefault\\fontfallbackcmd[1]{#1}\n",
        "\\DeclareTextSymbol{\\fontsymbolcmd}{OT1}{65}\n",
        "\\DeclareTextAccent\\fontaccentcmd{OT1}{19}\n",
    );
    let names = [
        "fonttextcmd",
        "fontprovidecmd",
        "fontdefaultcmd",
        "fontfallbackcmd",
        "fontsymbolcmd",
        "fontaccentcmd",
    ];
    let content = concat!(
        "\\setbox0=\\hbox{\\fonttextcmd{Text}\\fonttextcmd[red]{Explicit}\\fontprovidecmd{Provided}\\fontdefaultcmd{Default}\\fontfallbackcmd{Fallback}\\fontsymbolcmd\\fontaccentcmd{a}}\n",
        "\\typeout{TEXT-FONT-DIMENSIONS:\\the\\wd0/\\the\\ht0/\\the\\dp0}\\box0\n",
        "% \\fonttextcmd \\fontprovidecmd \\fontdefaultcmd \\fontfallbackcmd \\fontsymbolcmd \\fontaccentcmd\n",
        "\\verb|\\fonttextcmd \\fontprovidecmd \\fontdefaultcmd \\fontfallbackcmd \\fontsymbolcmd \\fontaccentcmd|\n",
    );
    setup(root, &[("main.tex", "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n"), ("definitions.tex", definitions), ("content.tex", content)]);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let main = root.join("main.tex");
    let before = build::run(&main, &config, &Default::default());
    assert_eq!(before.status, "success", "{}", before.output);
    let definition_uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
    for name in names {
        let workspace = setup(root, &[]);
        let doc = &workspace.documents[&definition_uri];
        let position = doc.text.position(doc.text.source.find(name).unwrap() + 2);
        let changes = renamed(
            &workspace,
            &definition_uri,
            position,
            &format!("{name}renamed"),
        );
        assert_eq!(changes.len(), 2);
        for (uri, source) in changes {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
    }
    let changed = std::fs::read_to_string(root.join("content.tex")).unwrap();
    assert_eq!(
        changed.split_once('%').unwrap().1,
        content.split_once('%').unwrap().1
    );
    let after = build::run(&main, &config, &Default::default());
    assert_eq!(after.status, "success", "{}", after.output);
    let metrics = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("TEXT-FONT-DIMENSIONS:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(metrics(&before.output).len(), 1);
    assert_eq!(metrics(&before.output), metrics(&after.output));
}

#[test]
#[ignore = "requires pdflatex"]
fn real_math_font_renames_preserve_accents_radicals_and_extensible_delimiters() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = concat!(
        "\\DeclareMathAccent{\\mathaccentcmd}{\\mathalpha}{operators}{94}\n",
        "\\expandafter\\DeclareMathRadical\\csname mathradicalcmd\\endcsname{symbols}{112}{largesymbols}{112}\n",
        "\\DeclareMathDelimiter{\\mathdelimitercmd}{\\mathopen}{symbols}{104}{largesymbols}{10}\n",
    );
    let names = ["mathaccentcmd", "mathradicalcmd", "mathdelimitercmd"];
    let content = concat!(
        "\\setbox0=\\hbox{$\\mathaccentcmd{a}+\\mathradicalcmd{b}+\\mathdelimitercmd c\\rangle+\\left\\mathdelimitercmd\\frac{1}{2}\\right\\rangle+\\csname mathaccentcmd\\endcsname{d}$}\n",
        "\\typeout{MATH-FONT-DIMENSIONS:\\the\\wd0/\\the\\ht0/\\the\\dp0}\\box0\n",
        "% \\mathaccentcmd \\mathradicalcmd \\mathdelimitercmd\n",
        "\\verb|\\mathaccentcmd \\mathradicalcmd \\mathdelimitercmd|\n",
    );
    setup(root, &[("main.tex", "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n"), ("definitions.tex", definitions), ("content.tex", content)]);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let main = root.join("main.tex");
    let before = build::run(&main, &config, &Default::default());
    assert_eq!(before.status, "success", "{}", before.output);
    let definition_uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
    for name in names {
        let workspace = setup(root, &[]);
        let doc = &workspace.documents[&definition_uri];
        let position = doc.text.position(doc.text.source.find(name).unwrap() + 2);
        let changes = renamed(
            &workspace,
            &definition_uri,
            position,
            &format!("{name}renamed"),
        );
        assert_eq!(changes.len(), 2);
        for (uri, source) in changes {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
    }
    let changed = std::fs::read_to_string(root.join("content.tex")).unwrap();
    assert_eq!(
        changed.split_once('%').unwrap().1,
        content.split_once('%').unwrap().1
    );
    let after = build::run(&main, &config, &Default::default());
    assert_eq!(after.status, "success", "{}", after.output);
    let metrics = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("MATH-FONT-DIMENSIONS:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(metrics(&before.output).len(), 1);
    assert_eq!(metrics(&before.output), metrics(&after.output));
}

#[test]
fn classic_font_commands_index_names_with_their_actual_arity_and_symbol_kind() {
    for (declaration, arguments, variable, count) in CLASSIC_FONT_DECLARATIONS {
        for target in [r"\oldfont", r"{\oldfont}", r"\csname oldfont\endcsname"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let prefix = if target.starts_with(r"\csname") {
                r"\expandafter"
            } else {
                ""
            };
            let definitions = format!("{prefix}\\{declaration}{target}{arguments}\n");
            let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n😀 Use: \\oldfont{Text} \\csname oldfont\\endcsname{Text}\n% \\oldfont\n\\verb|\\oldfont|\n\\end{document}\n\\oldf";
            let mut workspace = setup(
                root,
                &[("main.tex", source), ("definitions.tex", &definitions)],
            );
            workspace.snippets = true;
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let definition = Url::from_file_path(root.join("definitions.tex")).unwrap();
            let text = &workspace.documents[&main].text;
            let position = text.position(source.find("Use: ").unwrap() + 8);
            let locations = features::definitions(&workspace, &main, position);
            assert_eq!(locations.len(), 1, "{definitions}");
            assert_eq!(locations[0].uri, definition);
            assert!(features::hover(&workspace, &main, position).is_some());
            assert_eq!(
                features::references(&workspace, &main, position, true).len(),
                3
            );
            let items = features::completion(&workspace, &main, text.position(source.len()));
            let item = items.iter().find(|item| item.label == "oldfont").unwrap();
            assert_eq!(
                item.kind,
                Some(if variable {
                    CompletionItemKind::VARIABLE
                } else {
                    CompletionItemKind::FUNCTION
                })
            );
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(
                edit.new_text,
                if count == 0 {
                    "oldfont"
                } else {
                    "oldfont{${1:arg1}}"
                }
            );
            assert!(features::symbols(&workspace.documents[&definition])
                .iter()
                .any(|symbol| symbol.name == "oldfont"
                    && symbol.kind
                        == if variable {
                            SymbolKind::VARIABLE
                        } else {
                            SymbolKind::FUNCTION
                        }));
            let help = signature::help(
                &workspace,
                &main,
                text.position(source.find("{Text}").unwrap() + 3),
            );
            if count == 0 {
                assert!(help.is_none(), "{definitions}");
            } else {
                assert_eq!(help.unwrap().signatures[0].label, r"\oldfont{#1}");
            }
            let changed = renamed(&workspace, &main, position, "newfontcommand");
            assert_eq!(
                changed[&definition],
                definitions.replace("oldfont", "newfontcommand")
            );
            assert_eq!(
                changed[&main],
                source.replace(
                    "Use: \\oldfont{Text} \\csname oldfont",
                    "Use: \\newfontcommand{Text} \\csname newfontcommand"
                )
            );
            for name in [
                "DeclareTextFontCommand",
                "DeclareFixedFont",
                "DeclareMathAlphabet",
                "DeclareSymbolFontAlphabet",
                "DeclareMathSymbol",
                "DeclareTextCommand",
                "ProvideTextCommand",
                "DeclareTextCommandDefault",
                "ProvideTextCommandDefault",
                "DeclareTextSymbol",
                "DeclareTextAccent",
                "DeclareMathAccent",
                "DeclareMathRadical",
                "DeclareMathDelimiter",
            ] {
                assert!(features::rename(&workspace, &main, position, name).is_err());
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let workspace = setup(
        directory.path(),
        &[(
            "main.tex",
            r#"\DeclareMathSymbol{+}{\mathbin}{operators}{"2B}\DeclareTextFontCommand{not-a-command}{\bfseries}\DeclareTextSymbol{+}{OT1}{65}\DeclareTextAccent{not-a-command}{OT1}{19}\DeclareMathDelimiter{<}{\mathopen}{symbols}{104}{largesymbols}{10}\DeclareMathAccent{not-a-command}{\mathalpha}{operators}{94}\DeclareMathRadical{not-a-command}{symbols}{112}{largesymbols}{112}"#,
        )],
    );
    assert!(workspace
        .documents
        .values()
        .all(|doc| doc.syntax.definitions.is_empty()));
}

#[test]
#[ignore = "requires pdflatex"]
fn real_classic_font_renames_preserve_selected_fonts_and_text_and_math_dimensions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let names = [
        "textcustom",
        "fixedcustom",
        "mathcustom",
        "symbolcustom",
        "alphacustom",
    ];
    let definitions = CLASSIC_FONT_DECLARATIONS
        .iter()
        .zip(names)
        .map(|((command, args, _, _), name)| format!("\\{command}{{\\{name}}}{args}\n"))
        .collect::<String>();
    let content = concat!(
        "\\setbox0=\\hbox{\\textcustom{Text} {\\fixedcustom Fixed.} $\\mathcustom{A}+\\symbolcustom{B}+\\alphacustom$}\n",
        "\\typeout{CLASSIC-FONT-DIMENSIONS:\\the\\wd0/\\the\\ht0/\\the\\dp0}\\box0\n",
        "{\\fixedcustom\\typeout{CLASSIC-FONT-SELECTION:\\fontname\\font}}\n",
    );
    setup(root, &[("main.tex", "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n"), ("definitions.tex", &definitions), ("content.tex", content)]);
    let main = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "success", "{}", before.output);
    let definition = Url::from_file_path(root.join("definitions.tex")).unwrap();
    for name in names {
        let workspace = setup(root, &[]);
        let text = &workspace.documents[&definition].text;
        let position = text.position(text.source.find(&format!("\\{name}")).unwrap() + 2);
        let changed = renamed(&workspace, &definition, position, &format!("{name}renamed"));
        assert_eq!(changed.len(), 2, "{name}");
        for (uri, source) in changed {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
    }
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
    let metrics = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("CLASSIC-FONT-"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(metrics(&before.output).len(), 2);
    assert_eq!(metrics(&before.output), metrics(&after.output));
}

#[test]
fn fontspec_switches_support_project_navigation_completion_and_safe_rename() {
    for declaration in FONT_DECLARATIONS {
        for target in [r"\oldfont", r"{\oldfont}"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let definitions = format!("\\{declaration} % target\r\n{target}[Scale=1]{{Latin Modern Roman}}[Ligatures=TeX]\r\n");
            let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n😀 Use: \\oldfont \\csname oldfont\\endcsname\n% \\oldfont\n\\verb|\\oldfont|\n\\end{document}\n";
            let independent = "\\documentclass{article}\\newcommand{\\oldfont}{Other}\\begin{document}\\oldfont\\end{document}";
            let mut workspace = setup(
                root,
                &[
                    ("main.tex", source),
                    ("definitions.tex", &definitions),
                    ("other.tex", independent),
                ],
            );
            workspace.snippets = true;
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let definition = Url::from_file_path(root.join("definitions.tex")).unwrap();
            let text = &workspace.documents[&main].text;
            let position = text.position(source.find("Use: ").unwrap() + 8);
            let locations = features::definitions(&workspace, &main, position);
            assert_eq!(locations.len(), 1, "{definitions}");
            assert_eq!(locations[0].uri, definition);
            assert!(features::hover(&workspace, &main, position).is_some());
            let items = features::completion(&workspace, &main, position);
            let item = items.iter().find(|item| item.label == "oldfont").unwrap();
            assert_eq!(item.kind, Some(CompletionItemKind::FUNCTION));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.new_text, "oldfont", "font switches have no arguments");
            assert!(features::symbols(&workspace.documents[&definition])
                .iter()
                .any(|symbol| symbol.name == "oldfont" && symbol.kind == SymbolKind::FUNCTION));
            assert_eq!(
                features::references(&workspace, &main, position, true).len(),
                3
            );
            let Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) =
                features::prepare_rename(&workspace, &main, position).unwrap()
            else {
                panic!("missing rename: {definitions}")
            };
            assert_eq!(placeholder, "oldfont");
            let changed = renamed(&workspace, &main, position, "newfontswitch");
            assert_eq!(
                changed.len(),
                2,
                "independent project must remain unchanged"
            );
            assert_eq!(
                changed[&definition],
                definitions.replace("oldfont", "newfontswitch")
            );
            assert_eq!(
                changed[&main],
                source.replace(
                    "Use: \\oldfont \\csname oldfont",
                    "Use: \\newfontswitch \\csname newfontswitch"
                )
            );
            let declaration_position = workspace.documents[&definition]
                .text
                .position(definitions.find("oldfont").unwrap() + 2);
            assert_eq!(
                renamed(
                    &workspace,
                    &definition,
                    declaration_position,
                    "newfontswitch"
                ),
                changed
            );
            assert!(signature::help(
                &workspace,
                &main,
                text.position(source.find("\\oldfont ").unwrap() + 9)
            )
            .is_none());
        }
    }
}

#[test]
fn fontspec_renames_reject_macro_register_font_and_builtin_collisions() {
    let directory = tempfile::tempdir().unwrap();
    let source = "\\newfontfamily\\oldfont{Latin Modern Roman}\n\\newfontface\\occupiedfont{Latin Modern Roman}\n\\newcommand{\\occupiedmacro}{X}\n\\newlength{\\occupiedlength}\n\\newcounter{exercise}\n";
    let workspace = setup(directory.path(), &[("main.tex", source)]);
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let position = workspace.documents[&uri]
        .text
        .position(source.find("oldfont").unwrap() + 2);
    for name in [
        "occupiedfont",
        "occupiedmacro",
        "occupiedlength",
        "theexercise",
        "newfontfamily",
        "renewfontface",
        "textwidth",
    ] {
        assert!(
            features::rename(&workspace, &uri, position, name).is_err(),
            "{name}"
        );
    }
}

#[test]
fn fontspec_declarations_index_expanded_literal_names_and_skip_dynamic_or_ignored_names() {
    let directory = tempfile::tempdir().unwrap();
    let source = concat!(
        "\\expandafter\\newfontfamily\\csname literalfont\\endcsname{Latin Modern Roman}\n",
        "\\expandafter\\newfontface\\csname \\dynamic\\endcsname{Latin Modern Roman}\n",
        "\\newfontfamily{not-a-command}{Latin Modern Roman}\n",
        "% \\newfontfamily\\commentfont{Latin Modern Roman}\n",
        "\\verb|\\newfontface\\verbatimfont{Latin Modern Roman}|\n",
        "\\literalfont \\csname literalfont\\endcsname\n",
    );
    let workspace = setup(directory.path(), &[("main.tex", source)]);
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let doc = &workspace.documents[&uri];
    assert_eq!(doc.syntax.definitions.len(), 1);
    assert_eq!(doc.syntax.definitions[0].name, "literalfont");
    let position = doc.text.position(source.find("literalfont").unwrap() + 2);
    let changed = renamed(&workspace, &uri, position, "renamedfont");
    assert_eq!(changed[&uri], source.replace("literalfont", "renamedfont"));
}

#[test]
#[ignore = "requires xelatex and fontspec"]
fn real_fontspec_renames_preserve_font_selection_and_compile_across_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    // This wrapper initializes renew targets without pretending its parameter
    // is a statically known declaration. The call's command token is renamed.
    let mut definitions =
        String::from("\\newcommand{\\bootstrap}[1]{\\newfontfamily#1{Latin Modern Roman}}\n");
    let mut content = String::new();
    let names = [
        "fonta", "fontb", "fontc", "fontd", "fonte", "fontf", "fontg", "fonth",
    ];
    for (index, (declaration, name)) in FONT_DECLARATIONS.iter().zip(names).enumerate() {
        if declaration.starts_with("renew") {
            definitions.push_str(&format!("\\bootstrap\\{name}\n"));
        }
        let target = if index % 2 == 0 {
            format!("\\{name}")
        } else {
            format!("{{\\{name}}}")
        };
        definitions.push_str(&format!(
            "\\{declaration}{target}[Scale=1]{{Latin Modern Roman}}[Ligatures=TeX]\n"
        ));
        content.push_str(&format!(
            "{{\\{name}\\typeout{{FONT-AUDIT-{index}:\\fontname\\font}}Sample.}}\n"
        ));
    }
    setup(root, &[("main.tex", "\\documentclass{article}\n\\usepackage{fontspec}\n\\input{definitions}\n\\begin{document}\n\\input{content}\n\\end{document}\n"), ("definitions.tex", &definitions), ("content.tex", &content)]);
    let config = latex_lsp::config::BuildConfig {
        executable: "xelatex".into(),
        ..Default::default()
    };
    let main = root.join("main.tex");
    let before = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "success", "{}", before.output);
    let definition = Url::from_file_path(root.join("definitions.tex")).unwrap();
    for name in names {
        let workspace = setup(root, &[]);
        let text = &workspace.documents[&definition].text;
        let position = text.position(text.source.find(&format!("\\{name}")).unwrap() + 2);
        let changed = renamed(&workspace, &definition, position, &format!("{name}renamed"));
        assert_eq!(changed.len(), 2, "{name}");
        for (uri, source) in changed {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
    }
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
    let fonts = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("FONT-AUDIT-"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(fonts(&before.output).len(), 8);
    assert_eq!(fonts(&before.output), fonts(&after.output));
    let definitions = std::fs::read_to_string(root.join("definitions.tex")).unwrap();
    assert!(definitions.contains(r"\bootstrap\fontbrenamed"));
    assert!(definitions.contains(r"\bootstrap\fontfrenamed"));
}

#[test]
fn registers_lengths_fonts_and_constants_support_navigation_completion_and_rename() {
    for declaration in [
        r"\newlength{\oldreg}",
        r"\newsavebox{\oldreg}",
        r"\newfont{\oldreg}{cmr10}",
        r"\newcount\oldreg",
        r"\newdimen\oldreg",
        r"\newskip\oldreg",
        r"\newmuskip\oldreg",
        r"\newtoks\oldreg",
        r"\newbox\oldreg",
        r"\newread\oldreg",
        r"\newwrite\oldreg",
        r"\newinsert\oldreg",
        r"\newfam\oldreg",
        r"\chardef\oldreg=65",
        r"\mathchardef\oldreg=65",
        r"\countdef\oldreg=0",
        r"\dimendef\oldreg=0",
        r"\skipdef\oldreg=0",
        r"\muskipdef\oldreg=0",
        r"\toksdef\oldreg=0",
        r"\font\oldreg=cmr10",
        r"\expandafter\newlength\csname oldreg\endcsname",
        r"\expandafter\newcount\csname oldreg\endcsname",
        r"\expandafter\chardef\csname oldreg\endcsname=65",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n😀 Use: \\oldreg \\csname oldreg\\endcsname\n% \\oldreg\n\\verb|\\oldreg|\n\\end{document}\n";
        let workspace = setup(
            root,
            &[("main.tex", source), ("definitions.tex", declaration)],
        );
        let main = Url::from_file_path(root.join("main.tex")).unwrap();
        let definition = Url::from_file_path(root.join("definitions.tex")).unwrap();
        let text = &workspace.documents[&main].text;
        let position = text.position(source.find("Use: ").unwrap() + 8);
        let locations = features::definitions(&workspace, &main, position);
        assert_eq!(locations.len(), 1, "{declaration}");
        assert_eq!(locations[0].uri, definition);
        assert!(features::hover(&workspace, &main, position).is_some());
        assert!(
            features::completion(&workspace, &main, position)
                .iter()
                .any(|item| item.label == "oldreg"
                    && item.kind == Some(CompletionItemKind::VARIABLE)),
            "{declaration}"
        );
        assert!(features::symbols(&workspace.documents[&definition])
            .iter()
            .any(|symbol| symbol.name == "oldreg" && symbol.kind == SymbolKind::VARIABLE));
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            3,
            "{declaration}"
        );
        let Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) =
            features::prepare_rename(&workspace, &main, position).unwrap()
        else {
            panic!("{declaration}")
        };
        assert_eq!(placeholder, "oldreg");
        let changed = renamed(&workspace, &main, position, "newreg");
        assert_eq!(
            changed[&definition],
            declaration.replace("oldreg", "newreg")
        );
        assert_eq!(
            changed[&main],
            source.replace(
                "Use: \\oldreg \\csname oldreg",
                "Use: \\newreg \\csname newreg"
            )
        );
        assert!(signature::help(
            &workspace,
            &main,
            text.position(source.find("\\oldreg ").unwrap() + 8)
        )
        .is_none());
    }
}

#[test]
fn register_renames_reject_other_registers_macros_counters_and_builtin_lengths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newlength{\\oldreg}\n\\newcount\\occupied\n\\newcommand{\\macro}{X}\n\\newcounter{exercise}\n";
    let workspace = setup(root, &[("main.tex", source)]);
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let position = workspace.documents[&uri]
        .text
        .position(source.find("oldreg").unwrap() + 2);
    for name in [
        "occupied",
        "macro",
        "theexercise",
        "textwidth",
        "linewidth",
        "parindent",
        "baselineskip",
        "count",
        "the",
        "advance",
        "font",
        "countdef",
        "newread",
    ] {
        assert!(
            features::rename(&workspace, &uri, position, name).is_err(),
            "{name}"
        );
    }
}

#[test]
fn length_and_box_operations_offer_signatures_and_escaped_snippets() {
    for (marked, expected, active) in [
        (r"\newlength{\mylen¦}", r"\newlength{length}", 0),
        (
            r"\setlength{\mylen}{12pt¦}",
            r"\setlength{length}{value}",
            1,
        ),
        (
            r"\addtolength{\mylen}{1pt¦}",
            r"\addtolength{length}{value}",
            1,
        ),
        (
            r"\settowidth{\mylen}{Some text¦}",
            r"\settowidth{length}{text}",
            1,
        ),
        (r"\sbox{\mybox}{Text¦}", r"\sbox{box}{text}", 1),
        (
            r"\savebox{\mybox}[4cm][c]{Text¦}",
            r"\savebox{box}[width][position]{text}",
            3,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let workspace = setup(root, &[("main.tex", &source)]);
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let help = signature::help(&workspace, &uri, Text::new(source).position(offset))
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.signatures[0].label, expected);
        assert_eq!(help.active_parameter, Some(active));
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = setup(root, &[("main.tex", r"\newl")]);
    workspace.snippets = true;
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let items = features::completion(&workspace, &uri, Position::new(0, 5));
    let item = items.iter().find(|item| item.label == "newlength").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    assert_eq!(edit.new_text, r"newlength{\\${1:length}}");
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
}

#[test]
#[ignore = "requires pdflatex"]
fn real_register_renames_keep_lengths_numbers_tokens_boxes_and_constants_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newlength{\\widthreg}\\setlength{\\widthreg}{10pt}\n\\newcount\\countreg\\countreg=7\n\\newdimen\\dimreg\\dimreg=12pt\n\\newskip\\skipreg\\skipreg=2pt plus 1pt\n\\newtoks\\toksreg\\toksreg={Saved}\n\\newsavebox{\\boxreg}\\sbox{\\boxreg}{Box}\n\\chardef\\letterreg=65\n";
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\typeout{REGISTER-LENGTH:\\the\\widthreg}\n\\typeout{REGISTER-COUNT:\\the\\countreg}\n\\typeout{REGISTER-DIM:\\the\\dimreg}\n\\typeout{REGISTER-SKIP:\\the\\skipreg}\n\\typeout{REGISTER-TOKS:\\the\\toksreg}\n\\typeout{REGISTER-CHAR:\\number\\letterreg}\n\\usebox{\\boxreg}\\letterreg\n\\end{document}\n";
    setup(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "success", "{}", before.output);
    for name in [
        "widthreg",
        "countreg",
        "dimreg",
        "skipreg",
        "toksreg",
        "boxreg",
        "letterreg",
    ] {
        let workspace = setup(root, &[]);
        let uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
        let text = &workspace.documents[&uri].text;
        let position = text.position(text.source.find(name).unwrap() + 2);
        for (uri, changed) in renamed(&workspace, &uri, position, &name.replace("reg", "renamed")) {
            std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
        }
    }
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
    let values = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("REGISTER-"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&before.output), values(&after.output));
    assert_eq!(values(&after.output).len(), 6);
}
