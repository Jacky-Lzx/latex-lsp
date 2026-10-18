<img src="assets/icon.svg" width="64" height="64" alt="">

# latex-lsp

用 Rust 实现的轻量 LaTeX Language Server，通过 stdio 提供 LSP 服务。无需安装 TeX 发行版即可使用编辑功能。

## 功能

- 文档同步：完整或增量更新，使用 UTF-16 位置，支持中文、emoji 和 CRLF；拒绝无效编辑和过期版本。
- 实时诊断：未闭合/多余花括号、环境不匹配、同一文件内重复的标签或文献键。
- 快速修复：补全缺失花括号、闭合环境、删除多余的闭合符号；支持版本化编辑和旧客户端的命令式操作，拒绝过期修复。
- 补全：常用 LaTeX 命令、环境、用户命令、跨文件标签和 BibTeX 文献键。支持词中替换，根据客户端能力提供 snippet，并保留已有参数。
- 悬浮提示：内置命令说明、标签/文献/自定义命令的定义位置。
- 定义跳转：`\ref` / `\eqref` / `\cref` 等引用、常见 `\cite` 变体、自定义命令、输入文件、文献文件及本地包、类、图片。
- 查找引用：标签、文献键、自定义命令；支持包含或排除定义。
- 文档高亮：区分当前文件中的定义和引用；选择范围可逐级扩展到词、参数、命令、章节和整篇文档。
- 跨文件重命名：标签、BibTeX 文献键、自定义命令；支持准备重命名、定义歧义/重名检查及版本化工作区编辑，跳过注释和逐字文本。
- 文件/目录重命名：通过 `workspace/willRenameFiles` 更新输入文件、文献库、本地包/类、图片及 `% !TeX root` 路径；目录移动会重算内部相对路径，保留省略扩展名的写法。
- 文档符号：章节标题、标签、自定义命令和 BibTeX 条目；根据客户端能力返回层级结构或平铺列表。
- 工作区符号搜索：跨文件搜索章节、标签、宏和文献键，支持中文、不区分大小写及子序列匹配，按相关性返回最多 256 项。
- 文档折叠：章节、嵌套环境、多行花括号、连续注释和逐字环境，遵守客户端折叠数量限制。
- 文档链接：输入文件、文献库、本地包/文档类、图片及 `\url` / `\href`；文件先在当前目录查找，再尝试主文档目录。
- 工作区索引：扫描 `.tex`、`.bib`、`.sty`、`.cls`；跟踪文件变更，优先使用编辑器中未保存的内容。
- 后台编译：支持 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、Tectonic 和自定义工具；根文件识别、取消、超时、状态查询、编辑器原生进度显示和可选的保存后编译。
- 编译结果分析：错误、LaTeX/包警告、Overfull/Underfull 提示，以及 BibTeX/Biber 文献错误和警告，定位到子文件、文献条目和行号；结果提供 PDF、日志路径和输出摘要。只采用 latexmk 最后一遍 TeX 日志，避免保留已解决的引用警告。

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

在语法错误处调用 `vim.lsp.buf.code_action()` 可选择快速修复；`vim.lsp.buf.document_highlight()` 高亮当前符号的定义和引用。Neovim 0.12 中可用 `vim.lsp.buf.selection_range(1)` 扩大语法选择范围。

其他编辑器可将 `latex-lsp --stdio` 配置为 `tex` / `latex` / `bib` 文件的语言服务器。建议将根目录设为当前 LaTeX 项目，避免把多个无关项目混在同一个工作区。

## 试用

打开 [examples/demo/main.tex](examples/demo/main.tex)，尝试：

1. 在 `\ref{sec:intro}` 上跳转到另一个文件里的标签。
2. 在 `\cite{knuth1984}` 上跳转到 BibTeX 条目。
3. 输入 `\sec`、`\ref{` 或 `\cite{` 触发补全。
4. 删除 `\end{equation}` 或一个 `}`，查看诊断。
5. 在标签或文献键上调用重命名，预览跨文件修改。

## 编译

编辑功能不依赖 TeX；编译需要相应工具在语言服务器的 `PATH` 中。默认使用 `latexmk` 和 pdfLaTeX，把输出写入根文档旁的 `build/latex-lsp/`。编译前先保存项目文件，服务器会拒绝编译含未保存内容的依赖，避免诊断与编辑器内容不一致。

通过 `workspace/executeCommand` 调用以下命令，第一个参数为当前文档 URI（也可使用 `{ "uri": "file:///..." }`）：

- `latex.build`：立即返回 `{status: "started", buildId, rootUri}`，编译在后台运行。同一根文件已有任务时返回 `running` 和原任务 ID。
- `latex.cancelBuild`：取消该文档对应的编译；省略参数时取消全部编译。Unix 上同时终止 latexmk 启动的子进程。
- `latex.buildStatus`：查询当前任务或最近一次编译结果，未编译时返回 `idle`；运行中的 `queued` 表示是否还有保存触发的后续编译。

服务器发出 `latex/buildStarted` 和 `latex/buildFinished` 通知。完成通知包含 `status`（`success` / `error` / `cancelled` / `timeout`）、`buildId`、`rootUri`、`pdfUri`、`logUri`、`exitCode`、`diagnosticsCount`、`durationMs`、`message` 和有长度限制的 `output` 摘要。若文档在编译中变化，`stale` 为 `true`，不会发布这次编译的旧诊断。失败编译不把可能不完整的 PDF 标记为成功结果。

客户端声明 `window.workDoneProgress` 时，服务器还通过标准 `$/progress` 显示编译开始和完成，可通过编辑器的取消按钮中止编译。客户端拒绝或不响应进度创建请求时，编译仍会继续。

开启 `onSave` 后，编译期间的多次保存会合并为一次后续编译，重新读取最新保存内容。取消编译或关闭服务器会同时清除排队任务。若开始后续编译时仍有未保存的依赖，会跳过该次编译并记录原因。

编译前会重新遍历已保存文件中的依赖，包含本地 `.sty`、`.cls` 及其输入文件；子文件路径也会尝试相对于主文档目录解析。这样，磁盘上的子文件新增依赖或本地包在编译期间变化时，也会参与保存检查和过期结果检查。

默认输出目录使用 `build/latex-lsp`，默认 latexmk 参数让 BibTeX 保持在主文档目录运行，以正确处理 `../library.bib` 等相对路径。显式配置隐藏目录、绝对路径或包含 `..` 的输出目录时，会保留 latexmk 切换目录运行 BibTeX 的方式，以适应 BibTeX 的文件写入限制；这类配置中，父目录文献路径可能需要额外的工具配置。

服务器读取本次编译新增或更新的 `.blg` 文件，分析 BibTeX 的重复条目、语法错误和字段警告，以及 Biber 的错误/警告；能识别数据库和文献键时定位到 `.bib` 文件。未能定位的消息显示在主文档。旧 `.blg` 不会被当成本次编译错误。

Neovim 中可这样触发编译：

```lua
vim.lsp.buf_request(0, 'workspace/executeCommand', {
  command = 'latex.build',
  arguments = { vim.uri_from_bufnr(0) },
}, function(err, result)
  if err then vim.notify(err.message, vim.log.levels.ERROR)
  else vim.notify('LaTeX: ' .. result.status) end
end)

vim.lsp.handlers['latex/buildFinished'] = function(_, result)
  local level = (result.status == 'success' or result.status == 'cancelled')
    and vim.log.levels.INFO or vim.log.levels.ERROR
  vim.notify(result.message, level)
end
```

根文件优先级为：配置的 `rootFile` → 文件前 20 行的 `% !TeX root = ../main.tex` 指令（支持链式解析并检查循环）→ 包含当前子文件的唯一主文档 → 当前 `.tex` 文件。有多个主文档时需要显式指定根文件。

### 编译配置

使用 `initializationOptions`（Neovim 的 `init_options`）设置，运行中也可发送 `workspace/didChangeConfiguration`，其 `settings` 可直接使用下列对象或包在 `latex` 字段内。无效配置会被拒绝并保留原配置。

```json
{
  "build": {
    "executable": "latexmk",
    "arguments": null,
    "outputDirectory": "build/latex-lsp",
    "rootFile": null,
    "onSave": false,
    "timeoutSeconds": 120
  }
}
```

`rootFile` 相对于当前工作区根目录，`outputDirectory` 相对于主文档所在目录。`executable` 可改为 `pdflatex`、`xelatex`、`lualatex` 或 `tectonic`，省略 `arguments` 时自动选择对应的默认参数。自定义参数中的 `%f` 替换为根文件绝对路径，`%o` 替换为输出目录；参数直接传给进程，不经过 shell。默认 TeX/latexmk 参数关闭 shell escape。超时范围为 1–3600 秒。

例如 Neovim 配置中可加入 `init_options = { build = { executable = 'xelatex', onSave = true } }`。手动使用 XeLaTeX/pdfLaTeX 时只运行一遍，需要多遍处理文献和交叉引用的项目建议使用 latexmk，并自行配置对应引擎参数。

## 开发

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
# 已安装 latexmk、pdflatex 和 TeX 发行版时，也执行真实编译测试：
cargo test --locked -- --include-ignored
```

测试包含 UTF-16/增量编辑、语法容错、跨文件导航、文献补全、磁盘/未保存内容同步，以及启动真实进程验证的 stdio 协议流程。

代码按职责拆分：`text.rs` 处理位置与编辑，`syntax.rs` 解析 LaTeX/BibTeX，`workspace.rs` 管理文件索引与根文档，`features.rs` 实现语言功能，`navigation.rs` 处理工作区搜索、折叠和链接，`editing.rs` 处理快速修复、高亮和选择范围，`file_operations.rs` 维护文件移动后的引用，`build.rs` 管理后台编译，`build_log.rs` 分析日志，`config.rs` 校验配置，`lib.rs` 对接 LSP，`main.rs` 启动服务。

## 当前范围

这是可运行的基础版本，使用静态解析，不执行 TeX 宏展开或读取 TeX 发行版的包数据库。自定义命令支持 `\newcommand`、`\renewcommand`、`\providecommand` 和 `\DeclareMathOperator` 的花括号形式；BibTeX 支持常见 `{...}` / `(...)` 条目和嵌套字段。

目前不提供 PDF 预览、SyncTeX 跳转、格式化或数学语义检查；默认编译会生成 SyncTeX 数据。未定义引用的即时静态检查暂不报错，编译器提供的相关警告会显示在诊断中。定义和引用在整个工作区索引中查找；重命名要求该名字在索引中只有一个定义，存在歧义时会拒绝操作。内置命令不支持重命名。

文件/目录重命名需要编辑器或文件管理插件发送 `workspace/willRenameFiles` 并应用返回的编辑，然后移动文件并发送 `workspace/didRenameFiles`。服务器会保留打开文档的版本、刷新磁盘索引，并停止使用旧输入路径的编译；不会自行移动文件。含 TeX 宏的路径无法静态解析，新路径含无法安全表达的特殊字符时会拒绝引用更新；显式配置的 `build.rootFile` 需在主文件改名后同步修改。

磁盘索引最多 1024 个文件、单文件 2 MiB、目录深度 16，跳过隐藏目录、构建目录和目录符号链接；编辑器主动打开的文件不受磁盘文件大小限制。相对路径的文件引用还会按最多 16 层递归载入根目录外的依赖。支持动态文件监听的客户端会自动注册监听；其他客户端需发送 `workspace/didChangeWatchedFiles` 才能更新未打开文件的磁盘变化。

## 许可证

[MIT](LICENSE)
