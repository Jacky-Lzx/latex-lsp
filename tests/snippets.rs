use latex_lsp::{
    features,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

#[test]
#[ignore = "requires Neovim 0.10+ with vim.snippet"]
fn snippets_expand_in_a_real_editor_without_consuming_tex_backslashes() {
    let uri = Url::parse("file:///snippets/main.tex").unwrap();
    let mut workspace = Workspace {
        snippets: true,
        ..Default::default()
    };
    let mut cases = Vec::new();
    for (name, expected) in [
        ("newcommand", r"newcommand{\name}[1]{definition}"),
        ("renewcommand", r"renewcommand{\name}{definition}"),
        (
            "NewDocumentCommand",
            r"NewDocumentCommand{\name}{m}{definition}",
        ),
        (
            "includegraphics",
            r"includegraphics[width=\linewidth]{file}",
        ),
        ("date", r"date{\today}"),
        ("def", r"def\name{definition}"),
        ("let", r"let\alias=\command"),
        ("lim", r"lim_{x \to \infty} "),
        ("left", r"left(  \right)"),
    ] {
        let source = format!("\\{name}");
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(1)));
        let items = features::completion(&workspace, &uri, Position::new(0, source.len() as u32));
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!();
        };
        cases.push(serde_json::json!({"snippet":edit.new_text,"expected":expected}));
    }
    let macros = Url::parse("file:///snippets/macros.sty").unwrap();
    workspace.documents.insert(macros.clone(),Document::new(&macros,r"\newcommand{\pair}[2][\linewidth]{#2}\NewDocumentCommand{\render}{s O{blue} m m}{#3#4}\def\texpair#1#2{#1#2}".into(),None));
    for (name, expected) in [
        ("pair", "pair{arg2}"),
        ("render", "render{arg3}{arg4}"),
        ("texpair", "texpair{arg1}{arg2}"),
    ] {
        let source = format!("\\{name}");
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(1)));
        let items = features::completion(&workspace, &uri, Position::new(0, source.len() as u32));
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit");
        };
        cases.push(serde_json::json!({"snippet":edit.new_text,"expected":expected}));
    }
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("snippets.lua");
    let source = format!(
        r#"
local cases = vim.json.decode([==[{}]==])
for _, case in ipairs(cases) do
  vim.cmd('enew!')
  vim.snippet.expand(case.snippet)
  local actual = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n')
  assert(actual == case.expected, vim.inspect({{ snippet = case.snippet, expected = case.expected, actual = actual }}))
  vim.snippet.stop()
end
vim.cmd('qa!')
"#,
        serde_json::to_string(&cases).unwrap()
    );
    std::fs::write(&script, source).unwrap();
    let output = std::process::Command::new("nvim")
        .args(["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l"])
        .arg(script)
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .env("XDG_STATE_HOME", directory.path().join("state"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
