# 编辑器配置

[返回 README](../README.md)

先按[安装与启动](install.md)安装服务器，再配置编辑器。启动命令为 `latex-lsp --stdio`，通常由编辑器执行。

- [Neovim 0.11+ 完整配置](#neovim-011-完整配置)
- [VS Code](#vs-code)
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

新建未命名文档或打开非标准扩展名时，可先设置 `:setfiletype tex` 或 `:setfiletype bib`。客户端连接并发送相应语言类型后，服务器即可提供编辑功能，无需先保存为 `.tex` / `.bib`；编译仍需可用的已保存主文档。

调用 `vim.lsp.buf.format()` 可使用默认 tex-fmt 全文格式化；切换 latexindent 及自定义参数见[格式化](formatting.md)。请将所选工具安装到服务器的 `PATH`。

输入命令参数时，可通过 `vim.lsp.buf.signature_help()` 查看签名；支持自动参数提示的插件也可使用标准 `textDocument/signatureHelp`。

自动提示的触发字符包括 `{`、`[`、`<`、`(`、`|`、`*`、`+` 和逗号，并在常见结束分隔符处更新当前参数。其他自定义分隔符可手动请求提示。

在语法错误处调用 `vim.lsp.buf.code_action()` 可选择快速修复；`vim.lsp.buf.document_highlight()` 高亮当前符号的定义和引用。Neovim 0.12 中可用 `vim.lsp.buf.selection_range(1)` 扩大语法选择范围。

## VS Code

仓库内的 [`editors/vscode`](../editors/vscode/README.md) 提供 VS Code 客户端扩展。服务器需单独安装；在仓库根目录执行：

```sh
cargo install --path . --locked
cd editors/vscode
npm ci
npm run package
code --install-extension latex-lsp.vsix
```

扩展默认从 PATH 启动 `latex-lsp --stdio`；也可以在用户设置中配置 `latex-lsp.serverPath` 为可执行文件绝对路径。打开 LaTeX 项目文件夹和 `.tex` / `.bib` 后自动连接。

```json
{
  "latex-lsp.serverPath": "/absolute/path/to/latex-lsp",
  "latex-lsp.settings": {
    "build": { "engine": "xelatex", "onSave": true }
  }
}
```

`latex-lsp.settings` 使用服务端原有配置字段，修改后实时传递；默认值不变。项目差异可放入 `.latex-lsp.json`。命令面板提供编译、重编译、取消编译、状态、打开 PDF、重启服务器及输出面板入口；文档 CodeLens 也可编译。编译前需保存项目文件。

扩展暂未提供 PDF 内嵌预览和 SyncTeX 双向跳转。完整安装、远端环境、工具 PATH、兼容性及开发说明见[扩展 README](../editors/vscode/README.md)。

## 其他编辑器

其他编辑器可将 `latex-lsp --stdio` 配置为 `tex` / `latex` / `bib` / `bst` 文件的语言服务器。建议将根目录设为当前 LaTeX 项目，避免把多个无关项目混在同一个工作区。

## 示例项目

打开 [examples/demo/main.tex](../examples/demo/main.tex)，尝试：

1. 在 `\ref{sec:intro}` 上跳转到另一个文件里的标签。
2. 在 `\cite{knuth1984}` 上跳转到 BibTeX 条目。
3. 输入 `\sec`、`\ref{` 或 `\cite{` 触发补全。
4. 删除 `\end{equation}` 或一个 `}`，查看诊断。
5. 在标签或文献键上调用重命名，预览跨文件修改。
