<img src="assets/icon.svg" width="64" height="64" alt="">

# latex-lsp

用 Rust 实现的轻量 LaTeX Language Server，通过 stdio 提供 LSP 服务。无需安装 TeX 发行版即可使用编辑功能。

## 功能

- 文档同步：完整或增量更新，使用 UTF-16 位置，支持中文、emoji 和 CRLF；拒绝无效编辑和过期版本。
- 实时诊断：未闭合/多余花括号、环境不匹配、同一文件内重复的标签或文献键，以及同一项目中跨文件重复的标签和文献键。
- 快速修复：补全缺失花括号、闭合环境、删除多余的闭合符号，以及编译警告中标签/文献键的拼写建议；支持版本化编辑和旧客户端的命令式操作，拒绝过期修复。
- 补全：常用 LaTeX 命令、环境、用户命令、跨文件标签、BibTeX 文献键及输入文件/图片/本地包等路径。支持词中替换，根据客户端能力提供 snippet，并保留已有参数。
- 悬浮提示：内置命令说明、标签/文献/自定义命令的定义位置。
- 参数提示：常用命令、环境及跨文件用户宏/环境的签名，区分必选/可选参数，跟踪嵌套调用中的当前参数。
- 定义跳转：`\ref` / `\eqref` / `\cref` 等引用、常见 `\cite` 变体、自定义命令、输入文件、文献文件及本地包、类、图片。
- 查找引用：标签、文献键、自定义命令；支持包含或排除定义。
- 文档高亮：区分当前文件中的定义和引用；选择范围可逐级扩展到词、参数、命令、章节和整篇文档。
- 跨文件重命名：标签、BibTeX 文献键、自定义命令和环境；环境同步修改声明与 `\begin` / `\end`，文献键同步更新条目间引用。支持准备重命名、定义歧义/重名检查及版本化工作区编辑，跳过注释和逐字文本。
- 定理重命名：维护 `\newtheorem` 的共享编号与编号重置关系、计数器操作、派生命令及 cleveref 名称；区分自有计数器、共享计数器和无编号定理，可从关联命令跳转或发起重命名。
- 文件/目录重命名：通过 `workspace/willRenameFiles` 更新输入文件、文献库、BibTeX 样式、本地包/类、图片及 `% !TeX root` 路径；目录移动会重算内部相对路径，保留省略扩展名的写法。
- 文档符号：章节标题、标签、自定义命令和 BibTeX 条目；根据客户端能力返回层级结构或平铺列表。
- 工作区符号搜索：跨文件搜索章节、标签、宏和文献键，支持中文、不区分大小写及子序列匹配，按相关性返回最多 256 项。
- 文档折叠：章节、嵌套环境、多行花括号、连续注释和逐字环境，遵守客户端折叠数量限制。
- 文档链接：输入文件、文献库、本地包/文档类、图片及 `\url` / `\href`；文件先在编译主文档目录查找，再尝试当前文档目录。`% !TeX root` 始终相对于声明所在文档。
- 工作区索引：扫描 `.tex`、`.ltx`、`.bib`、`.bst`、`.sty`、`.cls`、`.def`、`.clo`；跟踪文件变更，优先使用编辑器中未保存的内容。
- 后台编译：支持 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、Tectonic 和自定义工具；根文件识别、取消、超时、状态查询、编辑器原生进度显示和可选的保存后编译。
- 编译结果分析：错误、LaTeX/包/文档类/字体警告、缺字、引擎警告、Overfull/Underfull 提示，以及 BibTeX/Biber 文献错误和警告，定位到子文件、文献条目和行号；结果提供 PDF、日志路径和输出摘要。只采用 latexmk 最后一遍 TeX 日志，避免保留已解决的引用警告。
- SyncTeX 查询：源码行定位 PDF 页码/坐标，以及 PDF 坐标反查源码行；返回数据供编辑器或 PDF 预览插件使用，检查未保存来源和过期产物。

解析时跳过 `%` 注释、`\verb`、`\lstinline` 和常见逐字环境（`verbatim`、`lstlisting`、`minted`、`comment`、`Verbatim`）。

未定义标签或文献的编译警告会尽量标出具体键名，包括列表中的单个键、同一行的多处引用，以及使用单引号的包警告；无法从源码明确匹配时保留整行位置。日志文件上下文支持 `.def`、`.clo` 等来源、带空格/括号的路径，以及 TeX 自动换行的长文件名。

对能够精确定位的未定义引用，快速修复会从当前项目的已知标签或文献中提供最多 5 个相近键名，支持相邻字符颠倒和中文。建议需要手动选择，只修改当前引用；跳过重复定义、宏生成的键名和已经存在的键，诊断过期后失效。

跨文件重复定义会标出每处键名，并提供可跳转的关联定义；编辑、关闭缓冲区、磁盘文件变化或项目依赖调整时同步刷新其他打开文件的诊断。独立论文之间的同名键互不冲突，共享定义会汇总实际使用项目中的冲突。该检查无需编译，按静态依赖分析；不尝试展开宏或模拟 TeX 条件分支。

跳转、悬浮、补全、参数提示和重命名会使用已保存主文档的依赖关系区分独立论文，避免其他项目的同名标签、文献、宏或环境干扰。共享定义的引用查询和重命名会覆盖所有使用它的项目，重命名也会检查这些项目中的冲突；共享源码中的引用如果在不同项目里指向不同定义，或缺少对应定义，则拒绝重命名。尚未关联到已保存主文档的缓冲区继续使用工作区范围。

共享源码中的输入文件和图片路径也按各主文档分别解析，编译输入检查使用当前编译的上下文。根不唯一且同一参数指向不同文件时，不提供单一跳转；路径补全可搜索这些主文档目录。文件移动要求所有使用者的路径编辑一致，一致时合并为一次编辑；存在冲突或某个项目无法解析该路径时拒绝改写。

文件路径补全会同时查找主文档目录和当前文档目录，优先选择编译主文档目录，与 TeX 读取同名文件时保持一致，支持相对父目录和子目录；保留显式扩展名、文献列表中的其他项，以及目录后已有的斜杠。`\url` / `\href` 的 URL 参数独立解析，允许 `%20` 等编码，不把 URL 中的文本当成 LaTeX 命令。

`\includegraphics` 还会搜索当前项目中 `\graphicspath{{figures/}{plots/}}` 声明的目录，支持图片跳转、子目录补全和文件重命名。移动图片目录或主文档时会同步更新目录声明，并尽量保留图片的简短名称和省略的扩展名。路径以当前编译主文档目录为基准，共享包里的声明也遵守这一规则。允许空格和中文，识别以 `/` 结尾的静态目录；移动主文档时也会重算尚不存在的目录。如果共享声明在不同项目中需要不同的替换路径，会拒绝该次文件移动编辑。不展开宏，也不模拟条件、局部作用域或多次声明的覆盖顺序。

`\includeonly{chapter-a, chapter-b}` 支持列表路径补全、跳转和文件/目录重命名，保留其他列表项与空白；重命名选中章节时同步维护编译选择。该命令只选择 `\include` 已声明的章节，不会单独建立输入依赖。

`\input` 可补全上述 TeX 来源类型，省略 `.tex` 时保留常见简写，其他扩展名会显式写入；跳转也遵守仅自动推断 `.tex` 的规则。主文档可以使用 `.tex` 或 `.ltx` 扩展名。 输入路径遵循 TeX 的默认扩展名搜索顺序，例如 `chapter.ltx` 和 `chapter.ltx.tex` 并存时优先后者。 `\include` 和选章列表按 LaTeX 规则补 `.tex`，不会误指向同名无扩展文件或 `.ltx` 文件；例如 `\include{chapter.ltx}` 对应 `chapter.ltx.tex`。

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
  filetypes = { 'tex', 'plaintex', 'bib', 'bst' },
  root_markers = { '.latexmkrc', 'latexmkrc', '.git' },
})
vim.lsp.enable('latex_lsp')
```

打开 `.tex` 后，可用 `vim.lsp.buf.hover()` 查看提示，`vim.lsp.buf.definition()` 跳转，`vim.lsp.buf.references()` 查找引用，`vim.lsp.buf.rename()` 跨文件重命名。补全可通过 `<C-x><C-o>` 或支持 Neovim LSP 的补全插件触发；snippet 需要客户端声明支持。

输入命令参数时，可通过 `vim.lsp.buf.signature_help()` 查看签名；支持自动参数提示的插件也可使用标准 `textDocument/signatureHelp`。

环境参数提示支持 `\newenvironment`、`\NewEnviron` 和 xparse 的 `\NewDocumentEnvironment` 系列定义，以及 `tabular`、`minipage`、`figure` 等常用环境；`\newtheorem` 定理提示可选标题 `[note]`。通过 `\begin{panel}` 或直接调用 `\panel` 均可查看参数，自定义环境的直接命令补全会插入必选参数的占位符并保留可选参数默认值。自定义命令与环境识别常规参数数量/默认值，以及 xparse 的 `m`、`o`、`O`、`s` 参数；复杂分隔参数和存在多个定义的名称不推断签名。

在语法错误处调用 `vim.lsp.buf.code_action()` 可选择快速修复；`vim.lsp.buf.document_highlight()` 高亮当前符号的定义和引用。Neovim 0.12 中可用 `vim.lsp.buf.selection_range(1)` 扩大语法选择范围。

其他编辑器可将 `latex-lsp --stdio` 配置为 `tex` / `latex` / `bib` / `bst` 文件的语言服务器。建议将根目录设为当前 LaTeX 项目，避免把多个无关项目混在同一个工作区。

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

服务器发出 `latex/buildStarted` 和 `latex/buildFinished` 通知，两者都包含实际 `executable` 和可推断的 `engine`（使用自定义参数或无法识别的工具时为 `null`）。完成通知还包含 `status`（`success` / `error` / `cancelled` / `timeout`）、`buildId`、`rootUri`、`pdfUri`、`logUri`、`exitCode`、`diagnosticsCount`、`errorsCount`、`warningsCount`、`dependenciesCount`（包含根文件的来源数量）、`durationMs`、`message` 和有长度限制的 `output` 摘要。成功结果的 `pages` 为最终 TeX 日志报告的页数（未知时为 `null`），`pdfSize` 为实际 PDF 的字节数；失败结果两者为 `null`。`latex.buildStatus` 保留相同摘要。若文档在编译中变化，`stale` 为 `true`，不会发布这次编译的旧诊断。失败编译不把可能不完整的 PDF 标记为成功结果。

编译管道输出超过 4 MiB、日志超过 8 MiB 时，分析缓冲区保留开头和结尾，省略中间部分；完整日志仍保留在磁盘。省略后清除文件栈，显式文件位置和新出现的文件上下文仍可定位，其他诊断回退到主文档。完成通知的 `diagnosticsTruncated` 标明 TeX 或文献诊断使用了省略后的内容，此时诊断数量仅统计已分析部分。部分 recorder 或文献依赖列表不会替换上一次完整记录。

工具返回成功但没有更新 PDF 时，服务器会检查已知输入的修改时间：来源比旧 PDF 新或已被删除则报告产物过期，并保留磁盘上的旧 PDF；产物仍有效的无操作编译可以成功。有 recorder 记录时优先按记录检查，并保留文献来源检查；未被记录的选章外文件不会导致过期产物误报。部分 TeX 版本也记录选章外文件的存在性检查，这些文件仍会保守参与检查。图片也参与编译期间变更、过期 PDF 和 SyncTeX 检查；二进制内容不进入文本索引。不生成 recorder 的工具使用当前能静态解析的图片路径。

客户端声明 `window.workDoneProgress` 时，服务器还通过标准 `$/progress` 显示编译开始和完成，可通过编辑器的取消按钮中止编译。客户端拒绝或不响应进度创建请求时，编译仍会继续。

开启 `onSave` 后，编译期间的多次保存会合并为一次后续编译，重新读取最新保存内容。取消编译或关闭服务器会同时清除排队任务。若开始后续编译时仍有未保存的依赖，会跳过该次编译并记录原因。

latexmk 和直接 TeX 引擎编译前会为静态 `\include{chapters/name}` 预建输出目录中的 `.aux` 子目录，包含由 `\input` 间接载入的章节声明。目录创建仅处理输出目录内的相对静态路径，跳过宏、父目录路径、注释和逐字文本。

编译前会重新遍历已保存文件中的依赖，包含本地 `.sty`、`.cls` 及其输入文件；子文件路径也会尝试相对于主文档目录解析。这样，磁盘上的子文件新增依赖或本地包在编译期间变化时，也会参与保存检查和过期结果检查。

编译后读取本次更新的 `.fls` recorder，发现 TeX 宏展开后实际读取的 `.tex`、`.ltx`、`.sty`、`.cls`、`.def`、`.clo` 和 `.bib` 文件，以及 PDF、PNG、JPEG、EPS、SVG 图片。项目目录内的来源会进入索引，参与之后的根文件识别、保存编译和未保存检查；系统 TeX 包不会批量进入索引。首次发现的来源已有未保存内容或在编译期间发生磁盘变化时，本次诊断也会被丢弃，无关的未保存缓冲区不受影响。上次记录的依赖会保守保留到新的 recorder 替换，因此刚移除的依赖可能仍需先保存。默认 latexmk 和 TeX 引擎参数启用 recorder；自定义参数需自行启用 `-recorder`，不生成 `.fls` 的工具仍使用静态依赖。

默认输出目录使用 `build/latex-lsp`，默认 latexmk 参数让 BibTeX 保持在主文档目录运行，以正确处理 `../library.bib` 等相对路径。显式配置隐藏目录、绝对路径或包含 `..` 的输出目录时，会保留 latexmk 切换目录运行 BibTeX 的方式，以适应 BibTeX 的文件写入限制；这类配置中，父目录文献路径可能需要额外的工具配置。

本地 `.bst` 样式通过 `bibliographystyle` 参与路径补全、跳转、文件移动及编译输入检查；修改时需要先保存，编译期间变化会使结果过期。BibTeX 报告的样式错误会定位到 `.bst` 对应行；编辑时不把它作为 LaTeX 语法解析。

`\input` 支持花括号、裸文件名和双引号文件名，例如 `\input chapter.tex` 与 `\input "my file"`；这些形式都参与补全、跳转、项目依赖和文件重命名。裸文件名以空白结束；改名或补全引入空格时自动添加花括号，已有双引号则保留。含宏展开、注释拼接或混合引号的文件名留给编译后的 recorder 发现。

服务器读取本次编译新增或更新的 `.blg` 文件，分析 BibTeX 的重复条目、语法错误和字段警告，以及 Biber 的错误/警告；能识别数据库和文献键时定位到 `.bib` 文件。未能定位的消息显示在主文档。旧 `.blg` 不会被当成本次编译错误。日志中实际读取的 `.bib` 和 `.bst` 也会加入项目依赖，支持宏展开后的文献路径、文献键重命名、根文件识别、未保存检查和过期 PDF 检查；系统目录中的样式不会批量索引。文献记录独立于 `.fls` 保留，直到新的文献日志更新它，避免只运行 TeX 时丢失文献来源。

文献依赖和日志定位共用搜索规则，识别 BibTeX 的工作目录与 latexmk 报告的数据库/样式搜索路径，避免主目录和输出目录中的同名文件导致定位错误。Biber 日志中的空格、中文及含单引号的数据库名也会保留。

TeX 可能在缺少 `\include` 章节时仍生成 PDF 并返回成功。服务器会把 `No file chapter.tex.` 转为 `missing-include` 警告，识别并标出调用文件中的静态章节路径；支持长路径自动折行，补回文件并重新编译后清除。正常首次编译时缺少 `.aux` / `.bbl` 的消息，以及无法关联到章节声明的可选文件消息不会作为章节警告。日志和编译输出中被 TeX 按字节换行拆开的 UTF-8 字符会先恢复，保留中文等路径和对应文件的诊断位置。

Overfull/Underfull 诊断会覆盖日志中 `at lines N--M` 指定的完整源码范围，并分别标记 `overfull-box` / `underfull-box` 诊断代码；支持 UTF-16 和 CRLF，异常范围会限制在文档内。

字体替换、文档类和引擎警告，以及 `Missing character` 缺字消息也会显示为警告，即使编译成功生成了 PDF。支持换行消息，避免把相邻错误并入上一条警告；没有来源行号的消息显示在当前文件第一行，重复的同位置消息合并显示。

`.aux`、`.bbl`、`.toc`、`.lof`、`.lot`、`.out`、`.nav`、`.snm` 和 `.vrb` 中的编译错误或警告定位到实际生成文件，支持传统错误、显式文件行号、空格/中文路径和长路径折行；这些文件仍不进入项目来源索引。若 LaTeX 报错后重写文件，导致原行号已不存在，诊断保持有效位置，并在消息及 `data.reportedLine` 中保留原始行号。

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
    "engine": null,
    "outputDirectory": "build/latex-lsp",
    "rootFile": null,
    "onSave": false,
    "timeoutSeconds": 120
  }
}
```

`rootFile` 相对于当前工作区根目录，`outputDirectory` 相对于主文档所在目录。`executable` 可改为 `pdflatex`、`xelatex`、`lualatex` 或 `tectonic`，省略 `arguments` 时自动选择对应的默认参数。自定义参数中的 `%f` 替换为根文件绝对路径，`%o` 替换为输出目录，`%j` 替换为产物名称（未配置时为主文件名去掉扩展名）；参数直接传给进程，不经过 shell。默认 TeX/latexmk 参数关闭 shell escape。超时范围为 1–3600 秒。

例如 Neovim 配置中可加入 `init_options = { build = { executable = 'xelatex', onSave = true } }`。手动使用 XeLaTeX/pdfLaTeX 时只运行一遍，需要多遍处理文献和交叉引用的项目建议使用 latexmk，并自行配置对应引擎参数。

`build.jobName` 可设置 PDF、日志、recorder 和 SyncTeX 的共同文件名，例如 `"jobName": "论文 draft.v2"` 生成 `论文 draft.v2.pdf`，主文件仍可以叫 `main.tex`。latexmk 和三个直接 TeX 引擎会自动添加 `-jobname`；自定义 `arguments` 需自行传递 `%j`。名称支持字母、数字、空格、点、下划线和短横线，不能包含目录或前后空格。Tectonic 默认调用不支持该选项。开始和完成通知的 `jobName` 返回实际产物名称；两个项目若将写入同一个 PDF，会拒绝并行编译，待前一次结束后可再次编译。

使用 latexmk 时，`engine` 可设为 `pdflatex`、`xelatex` 或 `lualatex`，服务器会选择相应的多遍编译模式。未指定时，优先读取主文件前 20 行的 `% !TeX program = xelatex` 或 `% !TeX TS-program = lualatex`，否则使用 pdfLaTeX；指令忽略大小写和逐字环境，只接受这三个引擎名称。子文件中的 program 指令不会覆盖主文件。显式 `engine` 优先于指令，显式选择的直接引擎 `executable` 和自定义 `arguments` 保持其调用方式。例如 `init_options = { build = { engine = 'xelatex', onSave = true } }` 可用 latexmk 自动处理 XeLaTeX 的多遍编译及文献。

### SyncTeX 双向查询

成功编译后，安装的 `synctex` 命令行工具可提供源码与 PDF 之间的映射。通过 `workspace/executeCommand` 调用：

```json
{
  "command": "latex.forwardSearch",
  "arguments": [{
    "uri": "file:///project/chapter.tex",
    "position": { "line": 12, "character": 4 }
  }]
}
```

返回 `{rootUri, pdfUri, locations: [{page, x, y, h, v, width, height}, ...]}`。源码 `position` 遵守 LSP 的零起始 UTF-16 约定，查询采用行级映射；PDF `page` 从 1 开始，`x` / `y` 从页面左上角计量，单位为 1/72 英寸。可选的 `h` / `v` / `width` / `height` 描述所在排版框。

```json
{
  "command": "latex.inverseSearch",
  "arguments": [{
    "uri": "file:///project/main.tex",
    "page": 1,
    "x": 150.0,
    "y": 135.0
  }]
}
```

反查也用源文件 `uri` 识别项目，返回 `{rootUri, pdfUri, locations: [Location, ...]}`，其中 `Location` 是可交给编辑器跳转的标准 LSP 位置，范围位于对应行首。查询可能返回多个候选或空列表，服务器不会自动启动 PDF 阅读器或编辑器。查询在后台执行，最多同时运行 4 个，每次最多运行 5 秒、返回 256 项；期间仍可编辑和发送其他请求。标准 `$/cancelRequest`、超时和 shutdown 会终止查询进程及其子进程，返回前会再次检查来源和产物。要求输出目录内已有 PDF 和 `.synctex.gz` 或 `.synctex`。项目存在未保存内容、来源晚于同步记录，或者当前编译运行中/最近编译未成功时，会要求保存并重新编译。

## 开发

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
# 已安装 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、BibTeX、SyncTeX 和 Neovim 0.10+ 时，执行真实编译及编辑器测试：
cargo test --locked -- --include-ignored
```

测试包含 UTF-16/增量编辑、语法容错、跨文件导航、文献补全、磁盘/未保存内容同步，以及启动真实进程验证的 stdio 协议流程。

代码按职责拆分：`text.rs` 处理位置与编辑，`syntax.rs` 解析 LaTeX/BibTeX，`workspace.rs` 管理文件索引与根文档，`features.rs` 实现语言功能，`signature.rs` 提供命令参数提示，`navigation.rs` 处理工作区搜索、折叠和链接，`editing.rs` 处理快速修复、高亮和选择范围，`file_operations.rs` 维护文件移动后的引用，`build.rs` 管理后台编译，`build_log.rs` 分析日志，`recorder.rs` 读取实际输入记录，`synctex.rs` 查询源码/PDF 映射，`config.rs` 校验配置，`dispatch.rs` 保留消息顺序并释放耗时查询的等待，`lib.rs` 对接 LSP，`main.rs` 启动服务。

## 当前范围

这是可运行的基础版本，使用静态解析，不执行 TeX 宏展开或读取 TeX 发行版的包数据库。自定义命令支持 `\newcommand`、`\renewcommand`、`\providecommand`、`\DeclareMathOperator`、`\DeclareRobustCommand` 和 xparse 的 `\NewDocumentCommand` 等声明，包含花括号与直接控制序列形式；也识别 `\def` / `\gdef` / `\edef` / `\xdef` 和 `\let`。BibTeX 支持常见 `{...}` / `(...)` 条目和嵌套字段。

自定义环境支持 `\newenvironment` / `\renewenvironment`、xparse 的 `\NewDocumentEnvironment` 等声明、`\NewEnviron` / `\RenewEnviron` 和 `\newtheorem`，提供补全、定义跳转、引用、悬浮、符号及跨文件重命名。改名时同步维护 `\begin{panel}` / `\end{panel}`、直接调用的 `\panel` / `\endpanel` 和字面 `\csname panel\endcsname`；可从这些调用处跳转或发起重命名。检查环境生成的同名宏和 `\end...` 宏，避免与已有命令冲突；存在直接控制序列调用时，新名称须只含 TeX 命令字母。

普通自定义宏也支持字面 `\csname hello\endcsname` 的定义跳转、悬浮、引用、高亮和重命名，与直接调用的 `\hello` 统一处理；保留名称前后的空白及调用参数。识别 `\expandafter\def\csname hello\endcsname#1{...}`、同样写法的 `gdef` / `edef` / `xdef`、`newcommand` / xparse 系列以及 `let` 别名定义；可从字面定义处发起重命名，参数提示与直接定义一致。字面调用可提示内置命令、自定义宏和环境参数，签名保留 `\csname ...\endcsname` 写法。名称中的宏展开、注释拼接、内部空白和未闭合的 `csname` 不作推断，也不会误记为重新定义 `csname` 原语；共享调用在不同项目里指向不同宏时拒绝重命名。

用户宏参数提示识别 `\newcommand` 系列和 `\DeclareRobustCommand` 的参数数量/首个可选参数默认值，xparse 的 `m` / `o` / `O{...}` / `s`，以及 `\def` 系列连续的 `#1#2...` 参数。调用按花括号或方括号分组匹配，支持星号、嵌套调用、转义和多行注释。复杂分隔参数、字符串求值或定义有歧义时暂不推断签名；不在注释和逐字文本中提示。

支持 snippet 的客户端补全用户宏时，还会按同一签名生成必选参数的 tabstop。例如 `\newcommand{\pair}[2][blue]{...}` 会补成 `\pair{...}`，保留首个可选参数的默认值；xparse 星号参数也默认省略。宏后已有分组参数（包含隔着注释的情况）时仅替换命令名。客户端不支持 snippet 或签名无法安全推断时，继续提供命令名补全。

文献条目间引用支持 `crossref`、`xref`、`xdata`、`related` 和 `entryset`，包含花括号或双引号形式，以及后三类字段的逗号分隔列表；可补全、跳转、查找引用、高亮和重命名父条目。字段名不区分大小写，补全也支持尚未闭合的值。字符串宏、`#` 拼接或含 TeX 命令的值需要求值，暂不自动修改；标题和注释中的同名文本会保留。文献键不能重命名为 `*`，以免改变 `\nocite{*}` 的全选含义。

目前不内置 PDF 预览、格式化或数学语义检查；默认编译生成 SyncTeX 数据，可由预览插件接入双向查询。未定义引用的即时静态检查暂不报错，编译器提供的相关警告会显示在诊断中。定义和引用在整个工作区索引中查找；重命名要求该名字在索引中只有一个定义，存在歧义时会拒绝操作。内置命令不支持重命名。

文件/目录重命名需要编辑器或文件管理插件发送 `workspace/willRenameFiles` 并应用返回的编辑，然后移动文件并发送 `workspace/didRenameFiles`。服务器会保留打开文档的版本、刷新磁盘索引，并停止使用旧输入路径的编译；不会自行移动文件。含 TeX 宏的路径无法静态解析，新路径含无法安全表达的特殊字符时会拒绝引用更新；显式配置的 `build.rootFile` 需在主文件改名后同步修改。 输入源码改为其他扩展名时会保留新后缀，例如 `\input{chapter}` 在 `.tex` 改为 `.ltx` 后更新为 `\input{chapter.ltx}`，根指令同步更新；`\include` / `\includeonly` 章节需要保留 `.tex`，通过包/文档类加载命令使用的文件需要保留 `.sty` / `.cls` 后缀，本地 BibTeX 样式需要保留 `.bst`。

重命名静态 `\newtheorem{claim}{Claim}` 时，同步维护 `\newtheorem{lemma}[claim]{Lemma}`、`\newtheorem{fact}{Fact}[claim]`、`\newcounter{inner}[claim]`、常见计数器操作与 `counterwithin` / `counterwithout` / `numberwithin`，以及 `\claim`、`\endclaim`、`\theclaim` 等派生命令；支持字面 `\csname theclaim\endcsname`，不展开动态宏。共享计数器的定理保留底层计数器，无编号定理不修改独立计数器。新名称与计数器或派生命令冲突、或共享引用属于其他项目中的独立计数器时拒绝改动；存在直接派生命令调用时，新名称需能组成 TeX 控制序列。

磁盘索引最多 1024 个文件、单文件 2 MiB、目录深度 16，跳过隐藏目录、构建目录和目录符号链接；编辑器主动打开的文件不受磁盘文件大小限制。相对路径的文件引用还会按最多 16 层递归载入根目录外的依赖。支持动态文件监听的客户端会自动注册监听；其他客户端需发送 `workspace/didChangeWatchedFiles` 才能更新未打开文件的磁盘变化。

## 许可证

[MIT](LICENSE)
