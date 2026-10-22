# 编译

[返回 README](../README.md)

后台编译命令、配置、状态和产物一致性检查。错误与警告的具体定位规则见[编译诊断](diagnostics.md)，源码与 PDF 查询见[SyncTeX](synctex.md)。

- [工具与编译入口](#工具与编译入口)
- [命令、状态与通知](#命令状态与通知)
- [从诊断重新编译](#从诊断重新编译)
- [结果分析](#结果分析)
- [编译配置](#编译配置)
- [保存、取消与输入快照](#保存取消与输入快照)
- [日志大小与分析预算](#日志大小与分析预算)

## 工具与编译入口

- 后台编译：支持 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、Tectonic 和自定义工具；根文件识别、取消、超时、状态查询、编辑器原生进度显示和可选的保存后编译。

- 文档内编译入口：支持标准 CodeLens，在主文件或已关联的章节、包等 TeX 来源中直接编译项目。遵守根文件配置和 `% !TeX root` 指令；共享来源可分别选择各主文档，多工作区文件夹中的同名主文件用完整路径区分。每次请求重新核对根文件，最多返回 32 个入口；未保存的独立文档、无所属项目的片段和不可用的显式根不提供入口。

编辑功能不依赖 TeX；编译需要相应工具在语言服务器的 `PATH` 中。默认使用 `latexmk` 和 pdfLaTeX，把输出写入根文档旁的 `build/latex-lsp/`。编译前先保存项目文件，服务器会拒绝编译含未保存内容的依赖，避免诊断与编辑器内容不一致。

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

## 命令、状态与通知

通过 `workspace/executeCommand` 调用以下命令，第一个参数为当前文档 URI（也可使用 `{ "uri": "file:///..." }`）：

- `latex.build`：立即返回 `{status: "started", buildId, rootUri}`，编译在后台运行。同一根文件已有任务时返回 `running` 和原任务 ID。

- `latex.rebuild`：强制重新编译；默认 latexmk 增加 `-g`，直接 TeX 引擎和 Tectonic 每次直接运行。用于恢复被修改的产物或需要跳过 latexmk 增量判断的情况。同一根文件正在编译时，将重复请求合并为一次后续强制编译，即使 `onSave` 为 `false` 也会执行；保存事件保留已排队的强制请求，后续任务使用最新配置和已保存内容。取消、相关文件事件和关闭服务器会清除队列。

- `latex.cancelBuild`：取消该文档对应的编译；省略参数时取消全部编译。Unix 上同时终止 latexmk 启动的子进程。

- `latex.buildStatus`：查询当前任务或最近一次编译结果，未编译时返回 `idle`；运行中的 `queued` 表示是否还有后续编译，`queuedRebuild` 表示后续任务是否为强制编译。启动响应、运行状态及开始/完成通知的 `rebuild` 表示当前任务是否为强制编译。

任务在编译工作线程启动前已被取消时，不会调用编译器、创建输出目录或替换旧日志，已有 PDF、日志和辅助文件会保留。取消结果不把这些旧文件作为本次编译产物返回。

服务器发出 `latex/buildStarted` 和 `latex/buildFinished` 通知，两者都包含实际 `executable` 和可推断的 `engine`（使用自定义参数或无法识别的工具时为 `null`）。完成通知还包含 `status`（`success` / `error` / `cancelled` / `timeout`）、`buildId`、`rootUri`、`pdfUri`、`logUri`、`exitCode`、`diagnosticsCount`、`errorsCount`、`warningsCount`、`dependenciesCount`（包含根文件的来源数量）、`durationMs`、`message` 和有长度限制的 `output` 摘要。成功结果的 `pages` 为最终 TeX 日志报告的页数（未知时为 `null`），`pdfSize` 为实际 PDF 的字节数；失败结果两者为 `null`。`latex.buildStatus` 保留相同摘要。若文档或已知输入选择在编译中变化，`stale` 为 `true`，不会发布这次编译的旧诊断，并清除该项目上一次编译的诊断，避免旧错误继续留在编辑器中。失败编译不把可能不完整的 PDF 标记为成功结果。

## 从诊断重新编译

编译器明确要求再次编译时，可在对应警告处请求快速修复，选择 `Rebuild …`。支持 LaTeX 交叉引用、natbib 文献、biblatex 的 LaTeX 重运行提示、longtable 表格列宽、hyperref 页标签和 rerunfilecheck 文件变更警告；未定义引用本身不会产生这个操作。

biblatex 明确要求先运行 BibTeX 或 Biber 时，默认参数的 latexmk 也提供该操作，由 latexmk 调度文献处理与后续 LaTeX 编译。直接 TeX 引擎及自定义 `arguments` / `rebuildArguments` 的流程保留提示和所需工具摘要，但不为这类提示提供重编译操作；请先完成对应的文献处理。切换为无法处理文献的配置后，旧菜单操作会失效。

该操作调用 `latex.rebuild`，同一主文档的多条警告合并为一个入口。共享文件分别列出产生诊断的主文档，避免从共享来源猜测编译根；仅匹配当前保存的编译诊断与请求范围。诊断清除后不再提供入口，根文件缺失、根配置已改变为其他项目或自定义工具尚未配置 `rebuildArguments` 时也不提供。

支持标准 CodeAction 和旧客户端的 Command 响应，不要求客户端支持工作区编辑。执行时沿用现有编译的保存检查、排队、取消和超时规则。

菜单操作在执行时重新核对原主文档、来源版本和当前诊断。菜单生成后修改根文件配置、编辑来源或清除对应警告，旧操作会报错并要求重新请求菜单，不会转而编译其他项目。已经接受的排队编译也保留原项目归属；后续根选择改变时取消该后续任务。手动调用 `latex.rebuild` 的 URI 或 `{uri}` 形式继续按当前配置选择根文件。

## 结果分析

结果的 `analysis` 汇总本次最终 TeX 日志中的已确认重复编译请求（`rerunRequired`）、未解析交叉引用键（`unresolvedReferences`）、文献引用键（`unresolvedCitations`）、缺失 PDF 目标（`unresolvedDestinations`）、重复 PDF 目标（`duplicateDestinations`）和重复标签键（`duplicateLabels`）。仅从标准 LaTeX、包或类告警正文开头识别未解析引用；普通告警示例或文件名中的同类文字仍保留为告警，不计入引用键。键去重并排序；PDF 目标名解码引擎输出的 PDF 字符串转义，引用、文献和重复标签键保留原文。日志折行不会在引用键的括号前添加空格或把括号误读为文件边界。缺少定义本身不视为重复编译请求。表格列宽需要更新时，识别 `longtable` 的明确重编译提示；列宽收敛后的最终日志不再保留该请求。也识别 hyperref 关于 PDF 页标签（`/PageLabels`）的明确重编译提示；例如 XeLaTeX 首次构建后需要补全页标签，后续日志不再请求时清除该标记。每类最多 128 个键，每个键最多 128 个 Unicode 字符；超出预算的键不截短身份，而是省略并设置 `truncated`。诊断输入被省略时该标记也为 `true`。取消、超时、无法观测最终日志或因输入变化而丢弃的结果返回 `analysis: null`；早期编译轮次已解决的警告不会进入分析。

### 文献处理与重编译

`analysis.requiredBibliographyTools` 汇总 biblatex 明确请求重新运行的文献工具，使用小写名称 `biber` / `bibtex`，去重并排序。完整的文献处理请求也会设置 `rerunRequired: true`，因为文献处理后还需要运行 LaTeX；仅出现未定义引用不会推断所需工具。

例如，BibTeX 后端的首次编译可能返回：

```json
{
  "rerunRequired": true,
  "requiredBibliographyTools": ["bibtex"]
}
```

运行 BibTeX 后，若 biblatex 只要求再运行 LaTeX，工具列表变为 `[]`，`rerunRequired` 仍为 `true`。最终日志不再请求时，两者分别为 `[]` 和 `false`。分页变化、语言加载的明确 LaTeX 重运行提示也参与分析；普通警告示例、不完整请求及其他宏包中的相似文字不会设置这些字段。

摘要依据本次最终日志，不保留前一轮已满足的请求。`latex/buildFinished` 和 `latex.buildStatus` 返回相同字段；取消、超时和过期结果仍按原规则返回 `analysis: null`。重编译操作的工具适用范围见[从诊断重新编译](#从诊断重新编译)。

### 排版警告摘要

`analysis.boxWarnings` 汇总水平、垂直方向的 Overfull / Underfull 诊断数量，以及最大溢出量或 badness。字段、计数和预算规则见[排版警告分析](typesetting.md)。

### 字体替换摘要

`analysis.fontSubstitutions` 和 `analysis.fontSizeSubstitutions` 分别汇总最终日志中的字体与字号替换。字段、去重及预算规则见[字体诊断与结果分析](fonts.md#字体与字号替换)。

### 缺字摘要

`analysis.missingCharacters` 按字符与字体汇总缺字，并保留最高严重级别。字段示例、预算与定位规则见[缺字摘要](fonts.md#缺字摘要)。

## 编译配置

使用 `initializationOptions`（Neovim 的 `init_options`）设置，运行中也可发送 `workspace/didChangeConfiguration`，其 `settings` 可直接使用下列对象或包在 `latex` 字段内。无效配置会被拒绝并保留原配置。

```json
{
  "build": {
    "executable": "latexmk",
    "arguments": null,
    "rebuildArguments": null,
    "engine": null,
    "outputDirectory": "build/latex-lsp",
    "rootFile": null,
    "onSave": false,
    "timeoutSeconds": 120
  }
}
```

`rootFile` 相对于当前工作区根目录，`outputDirectory` 相对于主文档所在目录。`executable` 可改为 `pdflatex`、`xelatex`、`lualatex` 或 `tectonic`，省略 `arguments` 时自动选择对应的默认参数。自定义参数中的 `%f` 替换为根文件绝对路径，`%o` 替换为输出目录，`%j` 替换为产物名称（未配置时为主文件名去掉扩展名）；参数直接传给进程，不经过 shell。默认 TeX/latexmk 参数关闭 shell escape。超时范围为 1–3600 秒。

`build.rebuildArguments` 单独指定强制编译的完整参数列表，也支持 `%f`、`%o`、`%j`；普通 `latex.build` 仍使用 `arguments`。自定义了 `arguments` 或使用未知编译工具时，必须显式设置 `rebuildArguments` 才能调用 `latex.rebuild`，否则在修改旧日志前报错。例如自定义 latexmk 参数时，可设 `"rebuildArguments": ["-g", "-pdf", "-interaction=nonstopmode", "-file-line-error", "-synctex=1", "-no-shell-escape", "-outdir=%o", "%f"]`；参数应包含工具所需的完整编译选项，设置 `jobName` 时也需自行传入 `%j`。使用这份自定义参数的重编译通知中，`engine` 为 `null`。

例如 Neovim 配置中可加入 `init_options = { build = { executable = 'xelatex', onSave = true } }`。手动使用 XeLaTeX/pdfLaTeX 时只运行一遍，需要多遍处理文献和交叉引用的项目建议使用 latexmk，并自行配置对应引擎参数。

`build.jobName` 可设置 PDF、日志、recorder 和 SyncTeX 的共同文件名，例如 `"jobName": "论文 draft.v2"` 生成 `论文 draft.v2.pdf`，主文件仍可以叫 `main.tex`。latexmk 和三个直接 TeX 引擎会自动添加 `-jobname`；自定义 `arguments` 需自行传递 `%j`。名称支持字母、数字、空格、点、下划线和短横线，不能包含目录或前后空格。Tectonic 默认调用不支持该选项。开始和完成通知的 `jobName` 返回实际产物名称；两个项目若将写入同一个 PDF，会拒绝并行编译，待前一次结束后可再次编译。

使用 latexmk 时，`engine` 可设为 `pdflatex`、`xelatex` 或 `lualatex`，服务器会选择相应的多遍编译模式。未指定时，优先读取主文件前 20 行的 `% !TeX program = xelatex` 或 `% !TeX TS-program = lualatex`，否则使用 pdfLaTeX；指令忽略大小写和逐字环境，只接受这三个引擎名称。子文件中的 program 指令不会覆盖主文件。显式 `engine` 优先于指令，显式选择的直接引擎 `executable` 和自定义 `arguments` 保持其调用方式。例如 `init_options = { build = { engine = 'xelatex', onSave = true } }` 可用 latexmk 自动处理 XeLaTeX 的多遍编译及文献。

## 保存、取消与输入快照

编译前请保存项目依赖。编译期间输入发生变化时，结果会标为 `stale`，服务器丢弃旧诊断；开启 `onSave` 后会合并保存事件并安排后续编译。

保存检查、取消、recorder、文献资源、文件监听以及 PDF / SyncTeX 的有效性规则见[编译输入与过期结果](build-inputs.md)。

## 日志大小与分析预算

编译管道输出超过 4 MiB、日志超过 8 MiB 时，分析缓冲区保留开头和结尾，省略中间部分；完整日志仍保留在磁盘。省略后清除文件栈，显式文件位置和新出现的文件上下文仍可定位，其他诊断回退到主文档。完成通知的 `diagnosticsTruncated` 标明 TeX 或文献诊断使用了省略后的内容，此时诊断数量仅统计已分析部分。部分 recorder 或文献依赖列表不会替换上一次完整记录。
