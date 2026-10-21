<img src="assets/icon.svg" width="64" height="64" alt="">

# latex-lsp

用 Rust 实现的轻量 LaTeX Language Server，通过 stdio 提供 LSP 服务。编辑功能无需安装 TeX 发行版；编译需要相应的 TeX 工具。

## 功能

- 补全、悬停、参数提示、跨文件跳转与引用查找。
- 标签、文献键、宏、环境等符号的安全重命名，以及文件移动后的路径更新。
- 实时诊断与快速修复、语义高亮、折叠和文档符号。
- 后台编译、取消与保存后编译，解析 TeX、BibTeX 和 Biber 的错误与警告。
- 编译结果分析与 SyncTeX 双向位置查询，供编辑器或 PDF 插件集成。

## 快速开始

安装当前稳定版 Rust，在仓库目录执行：

```sh
cargo install --path . --locked
latex-lsp --version
```

将 `latex-lsp --stdio` 配置为编辑器的语言服务器。进程通过标准输入输出通信，直接运行后等待输入属于正常行为。

Neovim 0.11+ 最小配置（确保 `latex-lsp` 在 `PATH` 中）：

```lua
vim.lsp.config('latex_lsp', {
  cmd = { 'latex-lsp', '--stdio' },
  filetypes = { 'tex', 'plaintex', 'bib', 'bst' },
  root_markers = { '.latexmkrc', 'latexmkrc', '.git' },
})
vim.lsp.enable('latex_lsp')
```

诊断刷新配置及常用操作见[完整编辑器配置](docs/editors.md#neovim-011-完整配置)。打开 [examples/demo/main.tex](examples/demo/main.tex) 可试用跨文件跳转、补全与重命名。

编译默认使用 `latexmk` 和 pdfLaTeX，输出到主文档旁的 `build/latex-lsp/`。先保存项目文件，再调用 `latex.build`；切换引擎和保存后编译见[编译配置](docs/build.md#编译配置)。

## 文档

- [安装与编辑器配置](docs/editors.md)：构建、Neovim 配置及示例。
- [编辑功能](docs/editing.md)：补全、导航、高亮、颜色和文献支持。
- [符号与重命名](docs/symbols.md)：宏、环境、计数器等符号的支持范围。
- [项目与文件](docs/projects.md)：根文件、依赖、路径和文件操作。
- [编译](docs/build.md)：命令、配置、通知、结果分析和产物检查。
- [编译诊断](docs/diagnostics.md)：错误定位、警告和拼写修复规则。
- [SyncTeX 双向查询](docs/synctex.md)：源码与 PDF 的查询协议。
- [开发与测试](docs/development.md)：检查命令和代码结构。

## 范围

采用静态解析，不执行任意 TeX 宏展开，不读取 TeX 发行版的包数据库。不内置 PDF 预览、格式化或数学语义检查；重命名和修复遇到歧义时会拒绝编辑。

## 许可证

[MIT](LICENSE)
