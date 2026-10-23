# 用户声明文档注释

[返回文档目录](README.md) · [符号与重命名](symbols.md)

用户宏的悬浮提示显示声明前连续的说明注释，同时保留原有的名称、声明类型、定义文件与行号。例如：

```tex
% Format a highlighted term.
%
% #1 is the term text.
\newcommand{\term}[1]{\textbf{#1}}
```

悬停 `\term` 时可看到这段说明。支持当前项目内跨文件定义、未保存的缓冲区、中文、emoji、CRLF、跨行声明参数，以及 `long` / `global` / `protected` 等声明前缀。修改或关闭定义缓冲区后，说明随现有文档生命周期更新。

只使用语法解析器确认的真实注释。每行移除行首缩进、连续 `%` 和之后的一个空格，保留段落与内部布局。物理空行、其他代码、尾随代码的行内注释，以及 TeX / Bib 指令、arara 和 Vim modeline 会结束说明块；声明所在行前有其他代码时不附加说明。宏参数头中的注释不会代替声明前的说明。

说明按纯文本发送，Markdown、HTML、URL 和反斜杠均保留字面内容。当前项目必须只有一个同名用户声明，歧义时不附加任何一个声明的注释；其他独立项目的同名宏互不影响。内置命令原有帮助保持原样。

每块最多 32 行、8192 字节，超限时不附加注释。此功能提供源码中的说明，不执行 TeX、不推断参数含义，也不把宏正文当成文档。

[参数提示](signatures.md) 同样显示唯一用户宏或环境声明前的说明，保留签名、当前参数和定义位置；直接调用与静态命名调用共用此规则。纯转发别名在签名可推断时显示自身声明的说明，环境使用独立于宏的声明命名空间。

用户环境的悬浮提示、`begin` / `end` 名称补全、环境复制来源补全和直接控制词补全也使用相同的注释规则。传统、xparse、Environ 和环境复制声明均可提供说明；只出现过但未声明的环境保留原有简要信息。宏和环境的唯一性分别判断，同名宏不会取代环境说明，重复环境声明不会选择某一份注释。环境补全文档遵循[补全](completion.md)中的客户端能力协商与过期校验，保留跨行名称的所有编辑。

```sh
cargo test --locked --lib command_docs
cargo test --locked --lib environment_documentation
cargo test --locked --test protocol user_command_documentation
cargo test --locked --test protocol user_environment_documentation
```
