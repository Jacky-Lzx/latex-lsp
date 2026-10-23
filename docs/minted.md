# minted 代码

[文档目录](README.md) · [编辑功能](editing.md)

`\mintinline[options]{language}{code}` 与 `\mint[options]{language}{code}` 支持命令补全、片段及头部参数提示。也可使用成对的字符定界符，例如 `\mintinline{python}|print("Hello")|`。代码作为逐字文本高亮，其中的标签、命令、引用和文件路径不会进入 LaTeX 索引，也不会被重命名。代码结束后的正文继续正常解析。

花括号形式按原始字符平衡嵌套花括号，代码中的反斜杠不会转义花括号，百分号不会开始注释。字符形式支持 Unicode、`%` 和 `#` 定界符；两种形式均允许跨行，支持 UTF-16 位置、CRLF 和增量编辑。尚未输入结束定界符时，后续内容保持逐字文本，直到补齐边界。

可选设置和语言参数之前的空白、TeX 注释按普通参数规则读取；语言参数之后已切换为逐字读取规则，`%` 是代码定界符。参数提示覆盖设置和语言名称；代码内部不提供 LaTeX 命令提示。已有本地命令重定义后，解析恢复普通参数规则。不展开自定义 minted 命令或推断 `escapeinside` 等选项执行的 LaTeX 内容。

`minted` 环境继续作为逐字环境处理。静态分析不调用 Pygments 或启用 shell escape。

```sh
cargo test --locked --test minted
cargo test --locked --test protocol minted_inline_
```

安装 pdfLaTeX 与 minted v3 后，可运行 `cargo test --locked --test minted real_minted_ -- --include-ignored`，用 `verbatim` 模式和关闭 shell escape 的真实编译核对代码边界及正文标签。
