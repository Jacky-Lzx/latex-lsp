# 编辑器配置

[返回 README](../README.md)

先按[安装与启动](install.md)安装服务器，再配置编辑器。启动命令为 `latex-lsp --stdio`，通常由编辑器执行。

- [Neovim 0.11+ 完整配置](#neovim-011-完整配置)
- [其他编辑器](#其他编辑器)
- [示例项目](#示例项目)

## Neovim 0.11+ 完整配置

确保 `latex-lsp` 在 `PATH` 中，在配置里添加：

```lua
vim.lsp.config('latex_lsp', {
  cmd = { 'latex-lsp', '--stdio' },
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
vim.lsp.enable('latex_lsp')
```

上面的能力声明启用服务器主动刷新诊断，处理器同时更新工作区和已打开的文档。Neovim 0.12.6 默认未声明 `refreshSupport`，其默认处理器在启用工作区拉取后还会跳过已按文档拉取的缓冲区；这段配置保证编译完成和跨文件修改后，当前文件也显示最新诊断。

打开 `.tex` 后，可用 `vim.lsp.buf.hover()` 查看提示，`vim.lsp.buf.definition()` 跳转，`vim.lsp.buf.references()` 查找引用，`vim.lsp.buf.rename()` 跨文件重命名。补全可通过 `<C-x><C-o>` 或支持 Neovim LSP 的补全插件触发；snippet 需要客户端声明支持。

输入命令参数时，可通过 `vim.lsp.buf.signature_help()` 查看签名；支持自动参数提示的插件也可使用标准 `textDocument/signatureHelp`。

自动提示的触发字符包括 `{`、`[`、`<`、`(`、`|`、`*`、`+` 和逗号，并在常见结束分隔符处更新当前参数。其他自定义分隔符可手动请求提示。

在语法错误处调用 `vim.lsp.buf.code_action()` 可选择快速修复；`vim.lsp.buf.document_highlight()` 高亮当前符号的定义和引用。Neovim 0.12 中可用 `vim.lsp.buf.selection_range(1)` 扩大语法选择范围。

## 其他编辑器

其他编辑器可将 `latex-lsp --stdio` 配置为 `tex` / `latex` / `bib` / `bst` 文件的语言服务器。建议将根目录设为当前 LaTeX 项目，避免把多个无关项目混在同一个工作区。

## 示例项目

打开 [examples/demo/main.tex](../examples/demo/main.tex)，尝试：

1. 在 `\ref{sec:intro}` 上跳转到另一个文件里的标签。
2. 在 `\cite{knuth1984}` 上跳转到 BibTeX 条目。
3. 输入 `\sec`、`\ref{` 或 `\cite{` 触发补全。
4. 删除 `\end{equation}` 或一个 `}`，查看诊断。
5. 在标签或文献键上调用重命名，预览跨文件修改。
