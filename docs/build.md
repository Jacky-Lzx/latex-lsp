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

rerunfilecheck 的请求格式、文件名折行及重编译标记清除规则见[文件变更与重编译](build-analysis.md#文件变更与重编译)。

biblatex 明确要求先运行 BibTeX 或 Biber 时，默认参数的 latexmk 也提供该操作，由 latexmk 调度文献处理与后续 LaTeX 编译。直接 TeX 引擎及自定义 `arguments` / `rebuildArguments` 的流程保留提示和所需工具摘要，但不为这类提示提供重编译操作；请先完成对应的文献处理。切换为无法处理文献的配置后，旧菜单操作会失效。

该操作调用 `latex.rebuild`，同一主文档的多条警告合并为一个入口。共享文件分别列出产生诊断的主文档，避免从共享来源猜测编译根；仅匹配当前保存的编译诊断与请求范围。诊断清除后不再提供入口，根文件缺失、根配置已改变为其他项目或自定义工具尚未配置 `rebuildArguments` 时也不提供。

支持标准 CodeAction 和旧客户端的 Command 响应，不要求客户端支持工作区编辑。执行时沿用现有编译的保存检查、排队、取消和超时规则。

菜单操作在执行时重新核对原主文档、来源版本和当前诊断。菜单生成后修改根文件配置、编辑来源或清除对应警告，旧操作会报错并要求重新请求菜单，不会转而编译其他项目。已经接受的排队编译也保留原项目归属；后续根选择改变时取消该后续任务。手动调用 `latex.rebuild` 的 URI 或 `{uri}` 形式继续按当前配置选择根文件。

## 结果分析

最终日志的重编译请求、未解析引用、文献工具及排版、字体和字符摘要，见[编译结果分析](build-analysis.md)。

## 编译配置

编译器、引擎、根文件、输出目录、参数模板及保存后编译等设置，见[编译配置](build-configuration.md)。

## 保存、取消与输入快照

编译前请保存项目依赖。编译期间输入发生变化时，结果会标为 `stale`，服务器丢弃旧诊断；开启 `onSave` 后会合并保存事件并安排后续编译。

保存检查、取消、recorder、文献资源、文件监听以及 PDF / SyncTeX 的有效性规则见[编译输入与过期结果](build-inputs.md)。

## 日志大小与分析预算

编译管道输出超过 4 MiB、日志超过 8 MiB 时，分析缓冲区保留开头和结尾，省略中间部分；完整日志仍保留在磁盘。省略后清除文件栈，显式文件位置和新出现的文件上下文仍可定位，其他诊断回退到主文档。完成通知的 `diagnosticsTruncated` 标明 TeX 或文献诊断使用了省略后的内容，此时诊断数量仅统计已分析部分。部分 recorder 或文献依赖列表不会替换上一次完整记录。
