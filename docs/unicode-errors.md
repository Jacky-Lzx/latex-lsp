# Unicode 输入错误

[返回文档目录](README.md) · [编译诊断](diagnostics.md) · [字体与缺字](fonts.md)

pdfLaTeX 遇到未定义映射的 Unicode 字符时，会报告 `LaTeX Error: Unicode character … not set up for use with LaTeX.`。服务器保留编译错误级别，尽量标出具体字符，并在编译结果中汇总这些字符及码点。

## 定位

来源和行号已确认时，诊断带有 `unsupported-unicode-character` 代码，`data.character` 和 `data.codepoint` 分别保存字符与 `U+…` 码点。只有日志中的单个 Unicode 标量与合法码点一致时才提供这些信息；消息残缺、码点不匹配或其他包的相似消息不作推断。

直接源码回显须能匹配完整前缀，或在该行唯一匹配被缩短的后缀。支持子文件、UTF-16、CRLF、同一行的重复字符，以及 60 / 79 列日志的消息折行。pdfTeX 恰好到达打印宽度时插入的一个空行，仅在后续内容能组成完整、一致的错误消息时合并；不会越过其他诊断或文件切换。

宏展开、命令参数中的隐藏展开、注释、逐字内容、宏定义体及不明确的回显保留整行位置。来源不可确认或行号无效时，不借用父文件或夹到首尾行后的字符；仍可根据完整错误消息汇总字符。

组合 emoji 可能包含多个 Unicode 标量，编译器分别报错时分别诊断，例如 `👩‍💻` 可包含 `U+1F469`、`U+200D`、`U+1F4BB` 三项。

## 编译结果摘要

`latex/buildFinished` 与 `latex.buildStatus` 的 `analysis.unsupportedUnicodeCharacters` 按码点排序，同一字符在不同位置和文件中只保留一项：

```json
{
  "unsupportedUnicodeCharacters": [
    {"character": "中", "codepoint": "U+4E2D"},
    {"character": "😀", "codepoint": "U+1F600"}
  ],
  "truncated": false
}
```

摘要只收录错误级别的完整标准消息，每次根据本次最终日志重新生成，不保留前一轮编译或修复前的记录；错误解决后返回 `[]`。最多保留 128 个不同字符，超出预算或诊断被截断时设置 `analysis.truncated: true`。逐位置诊断和错误数保持独立：同一行三个字符位置可以产生三项错误，摘要只保留其中不同的字符。

字体缺少字形时的 `Missing character: There is no … in font …` 继续进入 `analysis.missingCharacters`，保留字体与严重级别。两类问题可以同时存在，详见[缺字摘要](fonts.md#缺字摘要)。取消、超时和过期结果仍按[编译指南](build.md#结果分析)返回 `analysis: null`。

## 处理方式

如果需要保留 pdfLaTeX，可以在导言区为少量符号定义语义明确的映射。例如将 Unicode 减号排为数学减号：

```tex
\DeclareUnicodeCharacter{2212}{\ensuremath{-}}
```

映射的替代内容需根据字符含义选择。中文和多语种正文通常需要选择合适的 Unicode 引擎、语言宏包和字体；仅切换到 XeLaTeX 或 LuaLaTeX 仍可能产生字体缺字，编译配置见[编译指南](build.md#编译配置)。服务器提供诊断信息，保留用户对字符表示和编译引擎的选择。

真实工具回归命令见[测试文档](testing.md)。
