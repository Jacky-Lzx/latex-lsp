# 索引与术语表处理

[返回文档目录](README.md) · [编译结果分析](build-analysis.md) · [编译配置](build-configuration.md)

`analysis.requiredIndexTools` 汇总最终 TeX 日志中明确要求的索引工具。目前识别 rerunfilecheck 的两种完整警告正文：

- `Rerun LaTeX/makeindex to get index right.`
- `Rerun LaTeX/makeindex to get glossary right.`

两者均报告小写工具名 `makeindex`。列表去重并排序；索引和术语表同时变化时仍只有一项。请求还设置 `rerunRequired: true`，表示完成工具处理后需要继续运行 LaTeX：

```json
{
  "rerunRequired": true,
  "requiredIndexTools": ["makeindex"],
  "requiredBibliographyTools": []
}
```

索引工具与 BibTeX / Biber 独立汇总，可以同时出现在各自列表中。没有明确索引请求时，`requiredIndexTools` 为 `[]`。

## 显式编译管线

`build.indexTools` 默认是空列表。配置后，`latex.build`、`latex.rebuild` 和保存后编译均在同一个后台任务中执行：先运行 LaTeX 生成输入，再按配置顺序处理索引，最后强制运行 LaTeX 读取结果。无需从日志猜测工具参数，也不启动独立于编译任务的并发流程。

普通索引可使用默认 makeindex 设置：

```json
{
  "build": {
    "executable": "pdflatex",
    "indexTools": [{}]
  }
}
```

术语表与普通索引可以一起配置。例如主文档旁有 `glossary.ist`，其中声明 `keyword "\\glossaryentry"`：

```json
{
  "build": {
    "executable": "pdflatex",
    "indexTools": [
      {},
      {
        "inputExtension": "glo",
        "outputExtension": "gls",
        "arguments": ["-s", "%r/glossary.ist", "-o", "%t", "-t", "%l", "%i"],
        "dependencies": ["glossary.ist"]
      }
    ]
  }
}
```

每个任务的设置：

| 字段 | 默认值与含义 |
| --- | --- |
| `executable` | `makeindex`，直接调用可执行文件 |
| `arguments` | `["-o", "%t", "-t", "%l", "%i"]`，完整参数列表，不经过 shell |
| `inputExtension` / `outputExtension` | `idx` / `ind`；也支持 `glo` / `gls`、`acn` / `acr`、`nlo` / `nls` |
| `jobName` | 未设置时使用实际编译产物名称；命名索引可设为 `people`，读取 `people.idx` 并生成 `people.ind` |
| `dependencies` | 样式等额外输入文件，相对主文档目录，也可使用绝对路径；最多 128 项 |
| `timeoutSeconds` | 每次工具调用 30 秒，范围 1–3600 |

工具工作目录是实际产物目录，与 `build.outputDirectory`、latexmkrc 和 `build.jobName` 保持一致。`%i` 是当前索引输入的相对文件名，`%t` 是本次暂存输出的相对文件名，`%l` 是暂存处理日志的相对文件名；`%r` 是主文档目录，`%f` 是主文件绝对路径，`%o` 是产物目录，`%j` 是该索引任务的产物名称。占位符只替换一次，路径中的空格或 `%j` 等字面文字不会被再次展开。参数必须使用 `%i` 和 `%t`；makeindex 还必须用 `-t %l` 指定可核验的处理日志。

每次调用在确认退出成功、输入和额外依赖未变化、输出确实产生后，才原子替换正式索引。取消、超时、工具失败或没有产生输出时保留该任务原有索引。makeindex 会在拒绝条目后仍返回退出码 0，因此服务器另行检查新处理日志；存在拒绝条目或缺少完整摘要时按失败处理，详情进入编译输出。成功日志保存为 `.ilg` / `.glg` / `.alg` / `.nlg`，完成分析中的 `processedIndexTools` 提供输入、输出及处理日志 URI。

最后一遍 LaTeX 若改变了索引输入，继续运行工具和 LaTeX，最多四轮；仍未稳定时返回错误，不把含旧索引的 PDF 报告为成功。`analysis.indexProcessingRounds` 返回实际处理轮数。空索引输入／输出是合法的；只有确认最终 PDF 有效后才返回编译成功。日志和诊断以最后一遍 TeX 及本次管线实际更新的文献处理日志为准，先前遍次的临时警告不会覆盖最终结果。

所有步骤共用现有任务 ID、状态、进度、`latex.cancelBuild`、关闭服务器和保存检查。额外依赖的未保存缓冲区会阻止启动；执行期间修改来源或文件绑定会使结果过期，样式文件事件会取消受影响任务。两个项目即使 PDF 名称不同，也不能同时写入同一个命名索引或处理日志。最多配置 32 个任务；重复输出会被拒绝。

`build.timeoutSeconds` 限制每次 LaTeX 调用，各索引任务使用自己的超时。使用自定义编译 `arguments` 时，还需显式配置 `rebuildArguments`，用于处理后的强制 LaTeX 遍次；无效配置和缺少样式依赖会在首次编译前报错。仅在确有索引输入的项目中开启对应任务；缺少输入文件会报错。默认 latexmk 可能已处理普通索引，显式任务适合需要指定额外步骤或参数的项目。

## 手动处理

普通索引通常先由 LaTeX 的 `\makeindex` 声明和 `\index` 条目生成 `.idx`，再调用工具生成 `.ind`，最后使用 `makeidx` 宏包的 `\printindex` 将其排入 PDF。例如在产物目录中运行：

```sh
makeindex main.idx
```

工具完成后，在编辑器中执行 `latex.build`。产物目录和文件名前缀取决于项目的 `outputDirectory` 与 `jobName` 配置。

术语表所需的样式、输入与输出参数依项目约定。例如内核 `makeglossary` 写入的 `glossaryentry` 数据可以使用相应样式文件 `glossary.ist`：

```text
keyword "\\glossaryentry"
```

然后运行：

```sh
makeindex -s glossary.ist -o main.gls main.glo
```

LaTeX 文档还需加载生成的文件。其他术语表宏包可能需要不同工具、样式或处理步骤，应遵循对应宏包的配置。

当当前根文档的 `build.indexTools` 能处理警告中的实际输入时，快速修复菜单提供 `Rebuild …`。普通索引请求必须匹配 `idx` 任务；术语表请求必须匹配相应的 `glo`、`acn` 或 `nlo` 任务。文件名必须符合任务的 jobName：单独的文件名按实际产物目录解析，含目录的相对路径按主文档目录解析，绝对路径必须对应同一产物。配置任务的工具及参数由用户决定，菜单不会推断样式参数。

没有匹配任务、文件类型或名称不一致时，仅保留工具摘要；仍可手动运行独立工具。菜单执行时重新校验当前配置、原始诊断和文档版本，移除任务或修改名称／类型后的旧操作会被拒绝。共享来源按产生诊断的各根文档分别核对，不借用其他项目的任务。

## 日志与更新边界

识别完整的 rerunfilecheck 警告头、文件名和请求正文，支持宏包续行、60 / 79 列折行及满行后的额外空行。文件名中的同类文字不会被当作真正的请求，括号也不会改变相邻诊断的来源。示例、不完整请求、其他工具名及错误级别的同类文字不进入工具列表；单纯写入 `.idx` / `.glo` 或出现缺失文件也不推断所需工具。

列表反映本次最终日志的明确请求，不作为持久待办队列。rerunfilecheck 在校验文件不再变化时可以停止警告，即使用户尚未运行 makeindex；列表清空本身不证明索引已处理。较早轮次的控制台输出不会覆盖最终日志。`latex/buildFinished` 和 `latex.buildStatus` 使用相同摘要；取消、超时或过期结果继续返回 `analysis: null`。

诊断输入被省略时，`truncated` 为 `true`，工具列表代表已观测到的请求。

## 验证

```sh
cargo test --locked --test index_workflow
cargo test --locked --test protocol index_workflow
cargo test --locked --test index_workflow -- --include-ignored
cargo test --locked --test index_tools --test rebuild_actions
cargo test --locked --test protocol index_rebuild_actions
cargo test --locked --test protocol index_tool_analysis
cargo test --locked --test index_tools -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX、makeindex、makeidx 和 rerunfilecheck。在三个引擎的 60 / 79 列日志中，先产生索引与术语表请求，执行工具生成 `.ind` / `.gls` 并编译 PDF，再修改条目并重复处理，核对工具列表、实际条目和 PDF 页数。

显式管线回归另外使用真实 pdfLaTeX / makeindex 验证只有索引排版的首遍、术语表样式、条目更新、最终 PDF，以及普通／quiet 模式下退出码 0 的拒绝条目。默认测试覆盖原子替换、四轮上限、缺少输入、额外依赖、符号链接、输入变更、取消／超时和子进程清理；stdio 覆盖状态一致性、保存检查、共享命名索引冲突及过期结果。
