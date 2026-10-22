# 静态花括号诊断

[返回文档目录](README.md) · [命令名操作数与声明](declaration-operands.md) · [编译器的多余右花括号诊断](extra-braces.md)

编辑时检查源码中的花括号配对，无需启动编译。诊断的来源为 `latex-lsp`，代码为 `syntax`。

| 诊断 | 快速修复 |
| --- | --- |
| `Unclosed brace` | 在文档末尾插入缺少的 `}`；末尾为注释时先换行 |
| `Unexpected closing brace` | 删除选中的多余 `}` |

## let 的单 token 来源

原始 `let` 读取一个来源 token。下面的两个花括号用于赋值，不开启或关闭分组：

```tex
\let\opening={
\let\closing=}
{Ordinary text.}
```

这两处赋值不会触发花括号诊断或插入、删除修复，后面的普通分组仍正常检查。支持省略 `=`、操作数之间的空白和注释，以及显式 `expandafter` 展开的静态 `csname` 目标：

```tex
\expandafter\let\csname closing\endcsname=}
```

复制之后实际出现的宏声明仍进入索引。`futurelet` 会继续执行前瞻 token，其后花括号仍参与配对，详见[前瞻目标](futurelet.md)。

## 宏正文与参数

宏定义正文和分组实参在读取时按字面花括号配对。正文中出现 `let` 不会跳过这些花括号；未闭合正文和多余闭合字符仍报告错误。宏参数分隔文本中的 `let` 也不会消费定义正文的起始花括号。

检查覆盖传统声明、xparse、环境正文、静态 `csname` 目标和内置命令的分组参数。未知参数结构采取保守的字面配对，不执行任意宏展开、类别码变更或别名调用。本文件前文已显式改写 `let` 时，不再按原语规则确认后续来源；不模拟分组结束后的含义恢复。

编译器错误可能来自执行、宏展开或其他定界符，相关日志定位与删除建议见[多余右花括号](extra-braces.md)。
