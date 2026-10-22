# 参数提示

[返回文档目录](README.md) · [编辑功能](editing.md) · [补全](completion.md)

常用命令、环境及跨文件用户宏/环境提供参数签名，区分必选和可选参数，跟踪嵌套调用中的当前参数。编辑器可通过标准 `textDocument/signatureHelp` 请求提示，Neovim 可调用 `vim.lsp.buf.signature_help()`。

## 声明与可选参数

传统 `newcommand` / `renewcommand` / `providecommand`、`DeclareRobustCommand` 和 etoolbox 的 robust 变体，以及 `newenvironment` / `renewenvironment` 的声明本身也提示名称、参数数量、默认值及定义正文，识别星号和两种宏名称分组形式；补全保留已有的控制词名称。

章节及图表标题的短标题、`sqrt` 的根式次数遵循首个未受花括号保护的 `]` 边界；其中的字面量 `[` 不会吞掉后续标题或表达式的参数提示，调用结束后停止提示。星号章节命令直接提示标题，不接受短标题参数，也不会将后续正文当作标题。

## 参数数量中的注释

参数数量按 TeX 注释规则读取，支持注释续行、CRLF 和被注释拆开的数字。例如：

```tex
\newcommand{\pair}[2% 两个参数
][DEF]{#1:#2}
\pair{Body}
```

调用签名为 `\pair[#1=DEF]{#2}`；命令补全只为必选的第二个参数插入占位符，保留第一个参数的默认值。宏在项目的其他文件中声明时也适用。

传统宏和环境的参数数量限制为 0–9；`DeclarePairedDelimiterX` 与 `DeclarePairedDelimiterXPP` 限制为 1–9。普通数字间空格不会被注释处理删除，例如 `1 2` 不会变成 `12`。动态计数、转义 token、非法内容、溢出和越界数量不推断签名，不执行宏展开。

宏参数消费分析使用相同的数量规则；颜色重命名等功能继续排除作为其他宏实参传入的命令，避免把数据中的名称当作实际调用修改。符号与重命名的其余规则见[符号与重命名](symbols.md)。
