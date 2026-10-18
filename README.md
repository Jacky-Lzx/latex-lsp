<img src="assets/icon.svg" width="64" height="64" alt="">

# latex-lsp

用 Rust 实现的轻量 LaTeX Language Server，通过 stdio 提供 LSP 服务。无需安装 TeX 发行版即可使用编辑功能。

## 功能

- 文档同步：完整或增量更新，使用 UTF-16 位置，支持中文、emoji 和 CRLF；拒绝无效编辑和过期版本。
- 实时诊断：未闭合/多余花括号、环境不匹配、同一文件内重复的标签或文献键。
- 补全：常用 LaTeX 命令、环境、用户命令、跨文件标签和 BibTeX 文献键。支持词中替换，根据客户端能力提供 snippet，并保留已有参数。
- 悬浮提示：内置命令说明、标签/文献/自定义命令的定义位置。
- 定义跳转：`\ref` / `\eqref` / `\cref` 等引用、常见 `\cite` 变体、自定义命令、`\input` / `\include` / `\subfile` 及文献文件。
- 查找引用：标签、文献键、自定义命令；支持包含或排除定义。
- 跨文件重命名：标签、BibTeX 文献键、自定义命令；支持准备重命名、定义歧义/重名检查及版本化工作区编辑，跳过注释和逐字文本。
- 文档符号：章节标题、标签、自定义命令和 BibTeX 条目；根据客户端能力返回层级结构或平铺列表。
- 工作区索引：扫描 `.tex`、`.bib`、`.sty`、`.cls`；跟踪文件变更，优先使用编辑器中未保存的内容。

解析时跳过 `%` 注释、`\verb`、`\lstinline` 和常见逐字环境（`verbatim`、`lstlisting`、`minted`、`comment`、`Verbatim`）。

## 构建与运行

安装当前稳定版 Rust，然后执行：

```sh
cargo build --release --locked
./target/release/latex-lsp --stdio
```

进程从标准输入读取 LSP 消息，并向标准输出写入带 `Content-Length` 的 JSON-RPC 消息；通常由编辑器启动。直接运行时等待输入属于正常行为。也可以安装到 Cargo 的可执行文件目录：

```sh
cargo install --path . --locked
latex-lsp --version
```

### Neovim 0.11+

确保 `latex-lsp` 在 `PATH` 中，在配置里添加：

```lua
vim.lsp.config('latex_lsp', {
  cmd = { 'latex-lsp', '--stdio' },
  filetypes = { 'tex', 'plaintex', 'bib' },
  root_markers = { '.latexmkrc', 'latexmkrc', '.git' },
})
vim.lsp.enable('latex_lsp')
```

打开 `.tex` 后，可用 `vim.lsp.buf.hover()` 查看提示，`vim.lsp.buf.definition()` 跳转，`vim.lsp.buf.references()` 查找引用，`vim.lsp.buf.rename()` 跨文件重命名。补全可通过 `<C-x><C-o>` 或支持 Neovim LSP 的补全插件触发；snippet 需要客户端声明支持。

其他编辑器可将 `latex-lsp --stdio` 配置为 `tex` / `latex` / `bib` 文件的语言服务器。建议将根目录设为当前 LaTeX 项目，避免把多个无关项目混在同一个工作区。

## 试用

打开 [examples/demo/main.tex](examples/demo/main.tex)，尝试：

1. 在 `\ref{sec:intro}` 上跳转到另一个文件里的标签。
2. 在 `\cite{knuth1984}` 上跳转到 BibTeX 条目。
3. 输入 `\sec`、`\ref{` 或 `\cite{` 触发补全。
4. 删除 `\end{equation}` 或一个 `}`，查看诊断。

## 开发

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

测试包含 UTF-16/增量编辑、语法容错、跨文件导航、文献补全、磁盘/未保存内容同步，以及启动真实进程验证的 stdio 协议流程。

代码按职责拆分：`text.rs` 处理位置与编辑，`syntax.rs` 解析 LaTeX/BibTeX，`workspace.rs` 管理文件索引，`features.rs` 实现语言功能，`lib.rs` 对接 LSP，`main.rs` 启动服务。

## 当前范围

这是可运行的基础版本，使用静态解析，不执行 TeX 宏展开或读取 TeX 发行版的包数据库。自定义命令支持 `\newcommand`、`\renewcommand`、`\providecommand` 和 `\DeclareMathOperator` 的花括号形式；BibTeX 支持常见 `{...}` / `(...)` 条目和嵌套字段。

目前不提供编译/PDF 预览、SyncTeX、格式化、数学语义检查或编译器日志诊断。未定义引用不会报错，以免在项目索引不完整时误报。定义和引用在整个工作区索引中查找；重命名要求该名字在索引中只有一个定义，存在歧义时会拒绝操作。内置命令不支持重命名。

磁盘索引最多 1024 个文件、单文件 2 MiB、目录深度 16，跳过隐藏目录、构建目录和目录符号链接；编辑器主动打开的文件不受磁盘文件大小限制。相对路径的文件引用还会按最多 16 层递归载入根目录外的依赖。支持动态文件监听的客户端会自动注册监听；其他客户端需发送 `workspace/didChangeWatchedFiles` 才能更新未打开文件的磁盘变化。

## 许可证

[MIT](LICENSE)
