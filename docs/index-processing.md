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

## 处理流程

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

此类警告提供工具摘要，不生成现有的 `Rebuild …` 菜单操作。单纯重跑 TeX 无法完成索引处理；默认 latexmk 或自定义流程也无法从一条警告确认所有索引、术语表任务的参数。手动构建与项目已有的外部工具流程继续可用。

## 日志与更新边界

识别完整的 rerunfilecheck 警告头、文件名和请求正文，支持宏包续行、60 / 79 列折行及满行后的额外空行。文件名中的同类文字不会被当作真正的请求，括号也不会改变相邻诊断的来源。示例、不完整请求、其他工具名及错误级别的同类文字不进入工具列表；单纯写入 `.idx` / `.glo` 或出现缺失文件也不推断所需工具。

列表反映本次最终日志的明确请求，不作为持久待办队列。rerunfilecheck 在校验文件不再变化时可以停止警告，即使用户尚未运行 makeindex；列表清空本身不证明索引已处理。较早轮次的控制台输出不会覆盖最终日志。`latex/buildFinished` 和 `latex.buildStatus` 使用相同摘要；取消、超时或过期结果继续返回 `analysis: null`。

诊断输入被省略时，`truncated` 为 `true`，工具列表代表已观测到的请求。

## 验证

```sh
cargo test --locked --test index_tools --test rebuild_actions
cargo test --locked --test protocol index_tool_analysis
cargo test --locked --test index_tools -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX、makeindex、makeidx 和 rerunfilecheck。在三个引擎的 60 / 79 列日志中，先产生索引与术语表请求，执行工具生成 `.ind` / `.gls` 并编译 PDF，再修改条目并重复处理，核对工具列表、实际条目和 PDF 页数。
