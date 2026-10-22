# 特殊字符诊断与修复

[返回文档目录](README.md) · [编译诊断](diagnostics.md)

编译器报告正文中的裸 `&`、`#`、`_`、`^` 或多余的对齐分隔符时，服务器会核对日志的源码回显，精确标出出错字符，并提供转义快速修复。

多余 `}` 的删除操作见[多余右花括号](extra-braces.md)。

数学模式中的 `Double subscript.` / `Double superscript.` 使用独立的[重复上下标](math-scripts.md)诊断和修复。

## 使用

保存文件并执行 `latex.build`，在错误位置请求编辑器的代码操作：

| 编译错误 | 快速修复 | 修改结果 |
| --- | --- | --- |
| `Misplaced alignment tab character &.` | `Escape '&' as '\&'` | `&` → `\&` |
| `Extra alignment tab has been changed to \cr.` | `Escape '&' as '\&'` | `&` → `\&` |
| `You can't use` 后接 `macro parameter character #` 的模式错误 | `Escape '#' as '\#'` | `#` → `\#` |
| `Missing $ inserted.`，回显确认 `_` | `Escape '_' as '\_'` | `_` → `\_` |
| `Missing $ inserted.`，回显确认 `^` | `Escape '^' as '\textasciicircum{}'` | `^` → `\textasciicircum{}` |

支持 `#` 在水平、垂直、数学、受限水平和内部垂直模式中的上述错误。例如：

```tex
Before & and & after.
Before # after.
```

每个出错字符分别产生诊断和修复。逐项应用后变为：

```tex
Before \& and \& after.
Before \# after.
```

再次保存并编译，已解决的诊断会清除。修复只替换选中的一个字符，保留同一行的其他内容；表格中的合法 `&` 不会产生这类修复。

## 多余的对齐分隔符

表格或数组中的 `&` 多于列格式所允许的数量时，TeX 报告 `Extra alignment tab has been changed to \cr.`。服务器核对内部 `\endtemplate` 读取帧或以该命令结尾的 `<template>` 帧，再将错误定位到实际分隔符；支持普通与展开的错误上下文、`tabular`、数学 `array` 和原始 `\halign`。

精确诊断使用 `extra-alignment-tab` 代码，仍通过 `data.character` 和 `data.sourceContext` 保存字符与回显。只有源码能确认时才提供转义操作。

如果这里需要显示字面 `&`，可以将：

```tex
\begin{tabular}{ll}A & B & C\\\end{tabular}
```

修复为：

```tex
\begin{tabular}{ll}A & B \& C\\\end{tabular}
```

如果这里本来就需要第三列，应调整列格式（例如改为 `{lll}`）。转义操作会把选中的分隔符改为当前单元格内的字面字符，需根据内容选择。

不匹配的内部帧、额外用户宏展开帧、重复的缩短回显，以及已加载项目来源中可识别的宏参数保持整行诊断。正常的无参数排版命令后仍可定位分隔符，例如 `\hfil&`。跨文件裸参数、分隔参数和来源边界见[宏来源说明](literal-macro-scopes.md)。

## 文本中的下划线与脱字符

`Missing $ inserted.` 的原因可能是遗漏数学定界符，也可能是在普通文本中输入了 `_` / `^`。如果目标是公式，应补上正确的数学定界符；转义操作适用于确实需要显示字面字符的情况，例如将 `file_name` 改为 `file\_name`。

脱字符使用 `\textasciicircum{}` 输出；`\^` 是重音命令，会把后面的字符作为重音对象，因此不作为本操作的替换文本。已有的数学上下标（例如 `$x_1^2$`）保持原样。

只有日志包含准确的 `<inserted text>` / `$` 恢复帧，且紧随其后的源码回显以实际出错的 `_` 或 `^` 结束时，才提供精确诊断和转义操作。TeX 恢复期间在 `\par`、段落结束或 `\end{document}` 产生的连带错误继续保留原诊断；应用修复后重新编译，会清除相应的连带错误。命令实参中的展开栈、其他缺少数学模式的原因和不明确的错误位置不提供此操作。

## 定位与编辑校验

- 支持传统 `!` 日志和“文件名:行号:错误”格式；子文件错误定位到实际来源，位置按 LSP 的 UTF-16 单位计算。
- 完整的 `l.N` 回显须与源码前缀一致；被 TeX 缩短为 `...` 的回显须能在整行中唯一匹配。
- 正文特殊字符的精确诊断使用 `unescaped-special-character` 代码，多余对齐分隔符使用 `extra-alignment-tab`；`data.character` 保存字符，`data.sourceContext` 保存用于核对的源码回显。
- 支持版本化 Code Action 编辑，以及旧客户端的 `latex.applyQuickFix` 命令。请求修复时重新核对编译诊断、当前源码和宏参数；执行旧客户端命令时还会重新核对文档版本。跨文件宏变化后的处理见[修复重校验](literal-macro-scopes.md#修复重校验)。

宏展开栈、无法确认的来源、无效行号、源码变化或不唯一的回显保留原诊断，不提供转义编辑。注释、逐字内容、宏定义体和已转义字符也不参与修复。此功能依据已发生的编译错误提供建议，不把所有出现的 `&` / `#` / `_` / `^` 都当成错误。

真实引擎的验证方式见[真实工具回归](testing.md)。
