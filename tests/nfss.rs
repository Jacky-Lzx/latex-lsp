use latex_lsp::{
    features, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

const DECLARATIONS: &[(&str, &[&str])] = &[
    ("DeclareFontEncoding", &["LSP", "", ""]),
    ("DeclareFontEncodingDefaults", &["", ""]),
    ("DeclareFontFamily", &["LSP", "lsptest", ""]),
    (
        "DeclareFontShape",
        &["LSP", "lsptest", "m", "n", "<-> cmr10", ""],
    ),
    ("DeclareFontSubstitution", &["LSP", "lsptest", "m", "n"]),
    ("DeclareMathSizes", &["10", "10", "7", "5"]),
];

fn setup(source: &str, snippets: bool) -> (Workspace, Url) {
    let uri = Url::parse("file:///nfss-tests/lsplsptest.fd").unwrap();
    let mut workspace = Workspace {
        snippets,
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(5)));
    (workspace, uri)
}

fn command_item(workspace: &Workspace, uri: &Url, name: &str, cursor: usize) -> CompletionItem {
    features::completion(
        workspace,
        uri,
        workspace.documents[uri].text.position(cursor),
    )
    .into_iter()
    .find(|item| item.label == name)
    .unwrap()
}

fn edit(item: &CompletionItem) -> &TextEdit {
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    edit
}

fn fill(template: &str, arguments: &[&str]) -> String {
    let mut result = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find("${") {
        result.push_str(&remaining[..start]);
        let end = remaining[start..].find('}').unwrap() + start;
        let index = remaining[start + 2..end]
            .split_once(':')
            .unwrap()
            .0
            .parse::<usize>()
            .unwrap();
        result.push_str(arguments[index - 1]);
        remaining = &remaining[end + 1..];
    }
    result.push_str(remaining);
    result
}

#[test]
fn nfss_parameter_hints_track_every_field_comments_nested_settings_and_math_stars() {
    for (name, arguments) in DECLARATIONS {
        for star in if *name == "DeclareMathSizes" {
            &["", "*"][..]
        } else {
            &[""][..]
        } {
            let mut source = format!("% 😀\r\n\\{name} % declaration\r\n{star}");
            let mut positions = Vec::new();
            for argument in *arguments {
                source.push_str("{% before\r\n");
                positions.push(source.len() + argument.len() / 2);
                source.push_str(argument);
                source.push_str("% after\r\n}");
            }
            let (workspace, uri) = setup(&source, true);
            assert!(workspace.documents[&uri].syntax.definitions.is_empty());
            for (index, cursor) in positions.into_iter().enumerate() {
                let help =
                    signature::help(&workspace, &uri, Text::new(source.clone()).position(cursor))
                        .unwrap();
                assert_eq!(help.active_parameter, Some(index as u32), "{source}");
                assert_eq!(
                    help.signatures[0].parameters.as_ref().unwrap().len(),
                    arguments.len()
                );
                assert!(help.signatures[0].label.starts_with(&format!("\\{name}{{")));
            }
        }
    }
    let source =
        "\\DeclareFontShape{OT1}{family}{m}{n}{<-> ssub * cmr/m/{n}}{\\typeout{{nested}} setting}";
    let (workspace, uri) = setup(source, false);
    let cursor = source.find("setting").unwrap() + 3;
    assert_eq!(
        signature::help(&workspace, &uri, Text::new(source.into()).position(cursor))
            .unwrap()
            .active_parameter,
        Some(5)
    );
}

#[test]
fn nfss_completions_negotiate_snippets_preserve_arguments_and_honor_user_signatures() {
    for (name, arguments) in DECLARATIONS {
        let source = format!("\\{name}");
        for snippets in [false, true] {
            let (workspace, uri) = setup(&source, snippets);
            let item = command_item(&workspace, &uri, name, source.len());
            assert!(item.documentation.is_some());
            assert_eq!(
                item.insert_text_format,
                Some(if snippets {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                })
            );
            if snippets {
                assert_eq!(
                    fill(&edit(&item).new_text, arguments),
                    format!(
                        "{name}{}",
                        arguments
                            .iter()
                            .map(|argument| format!("{{{argument}}}"))
                            .collect::<String>()
                    )
                );
            } else {
                assert_eq!(edit(&item).new_text, *name);
            }
        }
        let source = format!("\\{name}{{existing}}{{arguments}}");
        let (workspace, uri) = setup(&source, true);
        let item = command_item(&workspace, &uri, name, name.len() + 1);
        assert_eq!(edit(&item).new_text, *name);
        let changed = workspace.documents[&uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit(&item).range),
                range_length: None,
                text: edit(&item).new_text.clone(),
            }])
            .unwrap();
        assert_eq!(changed.source, source);
        let source = format!("\\newcommand\\{name}[1]{{#1}}\n\\{name}");
        let (workspace, uri) = setup(&source, true);
        let item = command_item(&workspace, &uri, name, source.len());
        assert_eq!(edit(&item).new_text, format!("{name}{{${{1:arg1}}}}"));
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_completed_nfss_declarations_load_the_declared_font_in_three_engines() {
    let mut declarations = String::new();
    for (name, arguments) in DECLARATIONS {
        let source = format!("\\{name}");
        let (workspace, uri) = setup(&source, true);
        let item = command_item(&workspace, &uri, name, source.len());
        declarations.push('\\');
        declarations.push_str(&fill(&edit(&item).new_text, arguments));
        declarations.push('\n');
    }
    declarations.push_str("\\DeclareMathSizes*{12}{12}{8}{6}\n");
    let source = format!("\\documentclass{{article}}\n\\makeatletter\n{declarations}\\begin{{document}}\n{{\\fontencoding{{LSP}}\\fontfamily{{lsptest}}\\fontseries{{m}}\\fontshape{{n}}\\selectfont\\typeout{{NFSS-FONT:\\fontname\\font}}Native font.}}\n$x^2$\\typeout{{NFSS-MATH:\\tf@size/\\sf@size/\\ssf@size}}\n\\end{{document}}\n");
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let cache = root.join("cache");
        std::fs::create_dir(&cache).unwrap();
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let output = std::process::Command::new(engine)
            .args([
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-no-shell-escape",
                "main.tex",
            ])
            .env("TEXMFVAR", &cache)
            .env("TEXMFCACHE", &cache)
            .current_dir(root)
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{engine}: {log}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains("NFSS-FONT:cmr10"), "{engine}: {log}");
        assert!(log.contains("NFSS-MATH:10/7/5"), "{engine}: {log}");
        assert!(std::fs::read(root.join("main.pdf"))
            .unwrap()
            .starts_with(b"%PDF"));
    }
}
