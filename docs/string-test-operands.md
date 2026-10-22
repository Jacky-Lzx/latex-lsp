# 作为宏参数的字符串测试命令

[返回文档目录](README.md) · [字符串测试](string-tests.md) · [参数提示](signatures.md)

用户宏读取 `ifblank`、`notblank`、`ifstrempty` 或 `ifstrequal` 作为未展开的控制词参数时，该位置保留命令引用，不按字符串测试调用读取后面的参数。此规则也适用于[保存的字符串命令副本](string-test-copy-calls.md)，无需先复制、重定义或撤销原命令。

## 参数边界与跨文件加载

例如 `defs.tex`：

```tex
\newcommand{\discard}[1]{}
```

主文件：

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\input{defs}
\begin{document}
\discard\ifblank\input{after}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\end{document}
```

`discard` 读取单个 `ifblank` token，后面的 `input{after}` 仍是实际加载；独立调用中的 `boolfalse{draft}` 则保留为字符串数据。若 `after.tex` 含有 `\booltrue{draft}`，将 boolean `draft` 重命名时会更新声明和该文件中的操作，保留字符串里的原文。`discard` 的声明与调用仍可跨文件导航和重命名。

按调用处之前的加载顺序确认参数语法，支持当前文件和已加载依赖中的静态声明，包括：

| 声明示例 | 调用示例 |
| --- | --- |
| `\newcommand{\discard}[1]{}` | `\discard\ifblank` |
| `\newcommand{\discard}[2]{}` | `\discard{Unused}\ifblank` |
| `\newcommand{\discard}[2][Default]{}` | `\discard[note]\ifblank` |
| `\NewDocumentCommand{\discard}{O{Default} m}{}` | `\discard[note]\ifblank` |
| `\def\discard#1!{}` | `\discard\ifblank!` |

依赖缓冲区的增量修改、关闭后的磁盘恢复及文件变更通知都会刷新参数边界，主文档无需改动。将 `discard` 从一参数改成零参数后，后面的 `ifblank` 恢复按实际字符串调用分析；导航和版本化重命名随之更新。

## 参数提示

在 `\discard\ifblank` 的控制词参数内部请求提示时，显示外层 `\discard{#1}`。若光标位于文件末尾的控制词末端，仍视为正在输入该参数；若后面已经有 `{` 等分隔符，控制词参数已结束，外层提示停止。该操作数不显示独立 `ifblank` 调用的参数提示。

## 不确定的参数语法

未知宏的保守参数回退不作为确定消费控制词的证据。例如：

```tex
\let\saved\ifblank
\unknown\saved{\booltrue{draft}\input{ghost}}{Y}{N}
```

无法确认 `unknown` 的参数语法时，服务器继续保护 `saved` 的字符串数据，不据此加载 `ghost` 或将数据里的 boolean 当作实际引用。

共享文件在不同根中使用不同参数数量时，分别保留各根上下文；如果同一位置因此既可能是字符串数据，也可能是普通代码，会拒绝修改该歧义位置的重命名。其他明确符号仍可编辑，详见[跨文件上下文](string-test-projects.md)。这里只推断静态参数读取，不模拟任意用户宏正文的执行或参数重新展开。

## 验证

```sh
cargo test --locked --test string_test_operands
cargo test --locked --test protocol imported_string_predicate_operands
cargo test --locked --test string_test_operands -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox。覆盖四种谓词及多种参数语法，分别重命名外层宏和跨文件 boolean，核对后续文件加载、字符串原文、实际状态输出和 PDF 页数。
