# 显式文献处理流程

[返回文档目录](README.md) · [编译配置](build-configuration.md) · [文献编译诊断](bibliography-diagnostics.md)

默认 latexmk 自行调度文献工具。直接运行 pdfLaTeX、XeLaTeX 或 LuaLaTeX 时，可以配置 `build.bibliographyTool`，让一个后台编译任务负责生成辅助输入、执行文献工具及后续 TeX 遍次。

普通 BibTeX 项目：

```json
{
  "build": {
    "executable": "pdflatex",
    "bibliographyTool": { "backend": "bibtex" }
  }
}
```

biblatex / Biber 项目：

```json
{
  "build": {
    "executable": "xelatex",
    "bibliographyTool": { "backend": "biber" }
  }
}
```

`bibliographyTool` 默认是 `null`，不增加额外阶段。每个根文档可选择一个显式后端，可与 `indexTools` 同时使用。只有确实生成文献辅助输入的项目才应开启；缺少 `.aux` / `.bcf` 会报错，不从日志自动猜测后端。

| 字段 | 默认值与含义 |
| --- | --- |
| `backend` | `bibtex`，也可选 `biber` |
| `executable` | `null`，使用后端名称；也可指定可执行文件或包装程序 |
| `arguments` | `[]`，添加在服务器管理的输入／输出参数之前的字面参数，最多 128 项 |
| `dependencies` | `[]`，额外样式、配置或数据文件，相对主文档目录或绝对路径，最多 128 项 |
| `timeoutSeconds` | 每次后端调用 60 秒，可设置 1–3600 秒 |

参数不经过 shell，也不展开 `%f` 等模板。BibTeX 最后追加私有辅助文件名；Biber 最后追加受控的 `--input-directory`、`--output-directory` 和私有任务名称。需要调整数据模型或后端选项时可使用 `arguments`；调用方式不同的工具应通过包装程序适配，包装程序仍须生成对应 `.bbl` 和 `.blg`。

## 输出、搜索路径与诊断

文献工具在实际产物目录下的独立暂存目录生成输出。BibTeX 会复制主 `.aux` 及通过 `\@input` 连接的辅助文件树，并重写暂存副本中的辅助文件路径；原始 `.aux` 保持原样。Biber 使用主 `.bcf` 的副本。暂存任务使用普通文件名，兼容 BibTeX 的 `openout_any=p` 策略；不修改 TeX 的文件访问设置。

BibTeX 的工作目录为暂存目录；Biber 的工作目录为主文档目录。服务器在 `BIBINPUTS` / `BSTINPUTS` 中前置主文档目录和实际产物目录，保留原有搜索项和默认搜索路径，并把原有相对目录绑定到主文档目录。数据库和本地样式继续从项目中读取，TeX 在产物目录生成的数据库也可被找到，搜索路径同时用于处理日志的来源定位。

退出成功、生成新的普通 `.bbl` / `.blg`、辅助输入和显式依赖未变化，且处理日志没有错误诊断后，才原子替换正式 `.bbl`。后端失败、取消、超时、输出缺失或受监控输入变化时保留该阶段原有 `.bbl`。不把符号链接、目录或 FIFO 当成新输出；正式 `.bbl` 为符号链接时拒绝处理。

本次产生的有效 `.blg` 会保存在实际产物目录，包括工具失败时的处理日志。日志复用现有 BibTeX / Biber 诊断解析、数据库定位和依赖记录；处理日志中的错误即使伴随退出码 0，也会使阶段失败并保留旧 `.bbl`。失败结果不返回成功 PDF。

## 后续编译与生命周期

每轮先执行文献后端，再执行已配置的索引任务，最后强制运行两遍 TeX，让文献条目及辅助引用进入最终 PDF。若文献相关的辅助输入或索引输入仍变化，再运行下一轮；最多四轮，仍不稳定时报错。BibTeX 只比较行首的 `\citation`、`\bibdata`、`\bibstyle`、`\@input` 指令及辅助文件绑定，标签、`\bibcite` 记录与注释的变化不会重复触发后端；Biber 比较完整 `.bcf`；工具日志确认的数据库和样式也参与后续收敛检查，因此 TeX 后续遍次改写生成数据库时会重新处理。工具执行期间仍对完整辅助输入做变化检查。未配置文献阶段的索引流程仍只在工具后运行一遍 TeX。

最终诊断使用最后一遍 TeX 日志，以及本次流程实际更新的 `.blg`；首遍的临时文献重运行提示不会覆盖最终结果。`latex/buildFinished` 与 `latex.buildStatus` 的分析包含：

```json
{
  "processedBibliographyTool": {
    "backend": "bibtex",
    "executable": "bibtex",
    "inputUri": "file:///project/build/latex-lsp/main.aux",
    "outputUri": "file:///project/build/latex-lsp/main.bbl",
    "transcriptUri": "file:///project/build/latex-lsp/main.blg"
  },
  "bibliographyProcessingRounds": 1
}
```

此阶段与 `latex.build`、`latex.rebuild`、保存后编译共用任务 ID、取消、超时、进度、保存检查及过期结果校验。已知来源的未保存编辑会阻止启动，执行期间改变来源会使结果过期；显式额外依赖也加入保存检查和文件事件监听。共享同一 PDF 产物的根文档不能并发编译。

biblatex 明确要求文献后端时，仅在当前配置后端匹配、且警告只请求当前根的 jobName 时提供 `Rebuild …`。多任务请求或其他名称继续保留工具摘要。菜单执行时再次检查当前配置与诊断；默认 latexmk 的既有重编译行为保持其完整调度流程。

自定义编译 `arguments` 还必须配置 `rebuildArguments`。配置和显式依赖在第一次调用编译器前校验。每份辅助输入最多 8 MiB，辅助文件树最多 1024 份、64 层、合计 16 MiB；`.bcf` / `.bbl` 最多 16 MiB，处理日志最多 8 MiB。超限时报错，不使用不完整输入继续生成文献。

## 验证

```sh
cargo test --locked --test bibliography_workflow
cargo test --locked --test protocol bibliography_workflow
cargo test --locked --test rebuild_actions
cargo test --locked --test bibliography_workflow real_bibtex -- --ignored
cargo test --locked --test bibliography_workflow real_biber -- --ignored
```

真实 BibTeX 回归需要 pdfLaTeX、BibTeX 和 plain.bst，覆盖子章节、隐藏输出目录、含空格的 jobName、数据库更新和最终 PDF。真实 Biber 回归另外需要 Biber 与 biblatex；未安装时默认跳过。默认测试验证两类调用方式、最终遍次、失败日志、旧 `.bbl` 保留、辅助输入变化、依赖和符号链接、取消／超时、非普通产物、四轮上限，以及文献与索引共用管线。
