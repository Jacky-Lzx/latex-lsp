# Editor configuration

[Back to README](../README.md)

Install the server as described in [installation and startup](install.md), then configure your editor. The editor normally launches `texquill --stdio`.

- [Neovim 0.11+ full configuration](#neovim-011-full-configuration)
- [VS Code](#vs-code)
- [Other Editors](#other-editors)
- [Sample Project](#sample-project)

## Neovim 0.11+ full configuration

Ensure `texquill` is on your `PATH`, then add:

```lua
vim.lsp.config('texquill', {
  cmd = { 'texquill', '--stdio' },
  filetypes = { 'tex', 'plaintex', 'bib', 'bst' },
  root_markers = { '.latexmkrc', 'latexmkrc', '.git' },
  capabilities = { workspace = { diagnostic = { refreshSupport = true } } },
  handlers = {
    ['workspace/diagnostic/refresh'] = function(err, result, ctx, config)
      local response = vim.lsp.handlers['workspace/diagnostic/refresh'](err, result, ctx, config)
      local client = vim.lsp.get_client_by_id(ctx.client_id)
      if not err and client and client:supports_method('workspace/diagnostic') then
        for bufnr in pairs(client.attached_buffers) do
          client:request('textDocument/diagnostic', {
            textDocument = { uri = vim.uri_from_bufnr(bufnr) },
            identifier = client.server_capabilities.diagnosticProvider.identifier,
          }, nil, bufnr)
        end
      end
      return response
    end,
  },
})
vim.lsp.enable('texquill')
```

The capability above enables server-initiated diagnostic refreshes; the handler updates both workspace diagnostics and open documents. Neovim 0.12.6 does not advertise `refreshSupport` by default, and its default handler skips buffers already using document pull diagnostics once workspace pulls are enabled. This configuration keeps the current file up to date after builds and cross-file edits.

After opening a `.tex` file, use `vim.lsp.buf.hover()` for hover information, `vim.lsp.buf.definition()` for definitions, `vim.lsp.buf.references()` for references, and `vim.lsp.buf.rename()` for cross-file rename. Trigger completion with `<C-x><C-o>` or an LSP completion plugin. Snippets require client support.

For untitled documents or nonstandard extensions, set `:setfiletype tex` or `:setfiletype bib`. Once the client connects with that language ID, editing features work without saving as `.tex` or `.bib`. Builds still require a usable, saved root document.

Use `vim.lsp.buf.format()` for document formatting with tex-fmt by default. See [formatting](formatting.md) to select latexindent or customize arguments. Install the selected tool on the server's `PATH`.

Use `vim.lsp.buf.signature_help()` while entering command arguments. Plugins with automatic signature help can use standard `textDocument/signatureHelp` requests.

Automatic signature help triggers on `{`, `[`, `<`, `(`, `|`, `*`, `+`, and commas, and updates the active argument at common closing delimiters. Request help manually for other custom delimiters.

Use `vim.lsp.buf.code_action()` at syntax errors for quick fixes, and `vim.lsp.buf.document_highlight()` to highlight the current symbol's definitions and references. Neovim 0.12 also provides `vim.lsp.buf.selection_range(1)` to expand the syntax selection.

## VS Code

The [`editors/vscode`](../editors/vscode/README.md) directory contains the VS Code client extension. Install the server separately, then run from the repository root:

```sh
cargo install --path . --locked
cd editors/vscode
npm ci
npm run package
code --install-extension texquill.vsix
```

By default, the extension launches `texquill --stdio` from PATH. Alternatively, set `texquill.serverPath` to the executable's absolute path in user settings. It connects automatically when you open a LaTeX project folder and a `.tex` or `.bib` file.

```json
{
  "texquill.serverPath": "/absolute/path/to/texquill",
  "texquill.settings": {
    "build": { "engine": "xelatex", "onSave": true }
  }
}
```


`texquill.settings` uses the server's existing configuration fields and sends updates live, preserving server defaults. Use `.texquill.json` for project overrides. The command palette provides build, rebuild, cancellation, status, Open PDF, restart, and output actions; document code lenses also trigger builds. Save project files before building.

Open PDF uses the built-in PDF.js preview beside the source without moving keyboard focus. Successful builds, including build on save, refresh the preview while preserving the reading position. Use Ctrl+Alt+J (Cmd+Alt+J on macOS) for source-to-PDF SyncTeX search, and Ctrl+click (Cmd+click on macOS) in the PDF to return to its source line. See the [extension README](../editors/vscode/README.md) for installation, remote environments, tool PATH configuration, compatibility, and development.

## Other editors

Other editors can configure `texquill --stdio` as the language server for `tex` / `latex` / `bib` / `bst` documents. Use the current LaTeX project as the root to avoid combining unrelated projects.

## Sample project

Open [examples/demo/main.tex](../examples/demo/main.tex) and try:

1. Follow `\ref{sec:intro}` to a label in another file.
2. Follow `\cite{knuth1984}` to its BibTeX entry.
3. Type `\sec`, `\ref{`, or `\cite{` to trigger completion.
4. Delete `\end{equation}` or a `}` to inspect diagnostics.
5. Rename a label or citation key and preview the cross-file edits.
