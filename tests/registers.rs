use latex_lsp::{build, features, signature, text::Text, workspace::Workspace};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

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
