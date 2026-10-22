# 编译结果分析

[返回文档目录](README.md) · [编译指南](build.md)

结果的 `analysis` 汇总本次最终 TeX 日志中的已确认重复编译请求（`rerunRequired`）、未解析交叉引用键（`unresolvedReferences`）、文献引用键（`unresolvedCitations`）、缺失 PDF 目标（`unresolvedDestinations`）、重复 PDF 目标（`duplicateDestinations`）和重复标签键（`duplicateLabels`）。

仅从标准 LaTeX、包或类告警正文开头识别未解析引用；普通告警示例或文件名中的同类文字仍保留为告警，不计入引用键。键去重并排序；PDF 目标名解码引擎输出的 PDF 字符串转义，引用、文献和重复标签键保留原文。日志折行不会在引用键的括号前添加空格或把括号误读为文件边界。

缺少定义本身不视为重复编译请求。表格列宽需要更新时，识别 `longtable` 的明确重编译提示；列宽收敛后的最终日志不再保留该请求。也识别 hyperref 关于 PDF 页标签（`/PageLabels`）的明确重编译提示；例如 XeLaTeX 首次构建后需要补全页标签，后续日志不再请求时清除该标记。

使用末页钩子后，总页数变化可能造成临时额外页或钩子执行在旧页。两类内核的明确请求也设置重编译标记并提供操作；具体警告、实际 PDF 页数及收敛行为见[末页变更与重编译](last-page.md)。

每类最多 128 个键，每个键最多 128 个 Unicode 字符；超出预算的键不截短身份，而是省略并设置 `truncated`。诊断输入被省略时该标记也为 `true`。

取消、超时、无法观测最终日志或因输入变化而丢弃的结果返回 `analysis: null`；早期编译轮次已解决的警告不会进入分析。

## 折行的重编译警告

LaTeX 交叉引用与末页、natbib 文献、longtable 列宽和 hyperref 页标签的已知请求兼容逐字节折行。恢复时保留单词中的连接符和原有空格，例如 60 列日志中的 `cross` 换行 `-references` 仍为 `cross-references`；行尾已有的空格也不会被裁掉而拼接两个单词。

第一句以句号结束后，显式换行的请求仍属于同一条警告。识别同一宏包的续行前缀和 LaTeX 的缩进续行；独立的 natbib 简短提示继续有效，但若存在续行，使用完整正文判断。示例、额外说明和不完整请求不会借用前面的有效短句触发重编译。文件切换、相邻错误或已省略的日志片段不用于补全未完成的请求。

结果摘要与重编译操作使用相同规则。natbib 的最终检查可能在回读生成的 `.aux` 时发出；在该诊断处发起的操作仍编译原主文档。标签或文献键跨文件改名后，后续构建会重新分析引用；日志稳定后，标记和操作随之清除。

## 文件变更与重编译

rerunfilecheck 支持文件变更后的标准 `Rerun.` 请求，以及 hyperref 的完整书签和反向文献引用重编译提示，兼容日志折行与宏包续行前缀；结果分析与[重新编译操作](build.md#从诊断重新编译)使用相同规则。含额外说明的相似文字及要求运行 makeindex 的索引、术语表提示不提供此操作。后续编译中文件不再变化时，重编译标记与操作随对应警告清除。

启用 hyperref 的 `backref=page` 等反向引用功能后，`.brf` 文件变化会产生 `Rerun to get bibliographical references right.` 请求。该请求设置 `rerunRequired: true`，并可通过直接 TeX 引擎或已有的重编译配置处理。这类请求只要求再运行 LaTeX；没有其他文献处理请求时，`requiredBibliographyTools` 为 `[]`。引用页码再次变化时会重新出现请求，后续稳定日志会清除标记和操作；编译管道较早轮次的控制台提示不会覆盖最终日志。

告警头恰好占满 TeX 输出行时，显式换行可能留下额外空行。服务器保留其后同一宏包的完整说明，包括继续折行的请求正文；要求运行 makeindex 的索引和术语表说明不会因此丢失或被误判为普通 LaTeX 重编译请求。真正的文件切换及相邻诊断仍保持各自来源。

文件名在日志折行处出现左右括号时，先尝试恢复完整的已知警告，确认后才消费对应行，避免把文件名当成源码的进入或退出事件。60 / 79 列输出中的文件名可以包含看似完整的 `has changed. Rerun.` 正文；识别使用实际的完整文件名，不提前结束警告。不完整警告仍保留真正的文件切换，后续主文件和子文件的诊断分别归属各自来源。

## 文献处理与重编译

`analysis.requiredBibliographyTools` 汇总 biblatex 明确请求重新运行的文献工具，使用小写名称 `biber` / `bibtex`，去重并排序。完整的文献处理请求也会设置 `rerunRequired: true`，因为文献处理后还需要运行 LaTeX；仅出现未定义引用不会推断所需工具。

例如，BibTeX 后端的首次编译可能返回：

```json
{
  "rerunRequired": true,
  "requiredBibliographyTools": ["bibtex"]
}
```

运行 BibTeX 后，若 biblatex 只要求再运行 LaTeX，工具列表变为 `[]`，`rerunRequired` 仍为 `true`。最终日志不再请求时，两者分别为 `[]` 和 `false`。分页变化、语言加载的明确 LaTeX 重运行提示也参与分析；普通警告示例、不完整请求及其他宏包中的相似文字不会设置这些字段。

摘要依据本次最终日志，不保留前一轮已满足的请求。`latex/buildFinished` 和 `latex.buildStatus` 返回相同字段；取消、超时和过期结果仍按原规则返回 `analysis: null`。重编译操作的工具适用范围见[从诊断重新编译](build.md#从诊断重新编译)。

## 索引与术语表工具

`analysis.requiredIndexTools` 汇总 rerunfilecheck 明确请求的 `makeindex`，去重并排序。完整的索引或术语表请求同时设置 `rerunRequired: true`；该列表与 `requiredBibliographyTools` 独立。处理命令、日志快照规则及重编译操作的适用边界见[索引与术语表处理](index-processing.md)。

## 排版警告摘要

`analysis.boxWarnings` 汇总水平、垂直方向的 Overfull / Underfull 诊断数量，以及最大溢出量或 badness。字段、计数和预算规则见[排版警告分析](typesetting.md)。

`analysis.oversizedFloats` 单独汇总浮动体过高警告的数量与最大超出量，见[浮动体高度警告](floats.md)。

## 字体替换摘要

`analysis.fontSubstitutions` 和 `analysis.fontSizeSubstitutions` 分别汇总最终日志中的字体与字号替换。字段、去重及预算规则见[字体诊断与结果分析](fonts.md#字体与字号替换)。

## 缺字摘要

`analysis.missingCharacters` 按字符与字体汇总缺字，并保留最高严重级别。字段示例、预算与定位规则见[缺字摘要](fonts.md#缺字摘要)。

## Unicode 输入错误摘要

`analysis.unsupportedUnicodeCharacters` 汇总最终日志中 LaTeX 未设置映射的 Unicode 字符，提供字符和码点。字段、预算与处理方式见[Unicode 输入错误](unicode-errors.md)。

## PDF 页数

完成通知和 `latex.buildStatus` 的 `pages` 字段来自最终 TeX 输出摘要，仅在本次编译返回有效 PDF 时提供。解析支持 PDF、DVI 和 XDV 摘要，兼容多遍编译、带引号的文件名和日志折行；DVI / XDV 用于先生成中间文件再转换 PDF 的编译流程。

输出目录或文件名中的 `(9 pages).`、`main.pdf (9 pages).` 等字面文字不会作为页数。服务器读取完整摘要后确认输出文件名与页数，不把目录名处的折行当作摘要结尾；最后一份摘要不完整或跨越已省略的日志时返回 `null`，不会沿用前一轮的页数。

最后一条明确的输出报告为“没有页面输出”时，不保留早期轮次的页数，也不把目录中旧 PDF 作为本次成功产物；详见[输入与产物校验](build-inputs.md#输入与产物校验)。最终日志优先于编译管道中早期轮次的报告，普通警告示例不作为此类报告。
