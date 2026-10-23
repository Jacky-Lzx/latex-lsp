# minted 代码

[文档目录](README.md) · [编辑功能](editing.md)

`\mintinline[options]{language}{code}` 与 `\mint[options]{language}{code}` 支持命令补全、片段及头部参数提示。也可使用成对的字符定界符，例如 `\mintinline{python}|print("Hello")|`。代码作为逐字文本高亮，其中的标签、命令、引用和文件路径不会进入 LaTeX 索引，也不会被重命名。代码结束后的正文继续正常解析。

普通源码中的花括号形式按原始字符平衡嵌套花括号，代码中的反斜杠不会转义花括号，百分号不会开始注释。字符形式支持 Unicode、`%` 和 `#` 定界符；两种形式均允许跨行，支持 UTF-16 位置、CRLF 和增量编辑。尚未输入结束定界符时，后续内容保持逐字文本，直到补齐边界。

当前源文件中参数语法已知的宏参数和宏定义正文会预先读取代码 token；此时花括号按普通 TeX 规则配对，反斜杠保护控制符，百分号开始注释，嵌套分组保护其中的同名定界符。支持稳定的复制宏和纯静态转发。代码前的注释也正常续行；普通作用域、零参数包装及未知参数语法继续使用普通源码规则。

可选设置和语言参数之前的空白、TeX 注释按普通参数规则读取；普通源码中语言参数之后已切换为逐字读取规则，`%` 是代码定界符。参数提示覆盖设置和语言名称；代码内部不提供 LaTeX 命令提示。已有本地命令重定义后，解析恢复普通参数规则。不展开自定义 minted 命令或推断 `escapeinside` 等选项执行的 LaTeX 内容。

`minted` 环境按 FancyVerb 的逐行规则读取。先读取普通 TeX 选项和语言参数，再从下一行扫描代码；头部参数中的 `\end{minted}` 不会结束环境。每行只检查第一个字面的 `\end{…}`，名称必须恰好为 `minted`；如果首先出现其他名称，该行的后续标记仍是代码。真实结束行的剩余内容不会进入符号、文件依赖或重命名索引；原生工具对此报告的额外内容错误仍保留为编译诊断。折叠和环境名联动编辑使用真实结束边界，未闭合代码可补齐结束行。支持 CRLF、UTF-16 位置及增量编辑。静态分析不调用 Pygments 或启用 shell escape。

`\begin{minted}[options]{language}` 还提供头部参数提示，覆盖未完成的选项和语言分组。选项在第一个不受花括号保护的 `]` 处结束，读取语言时跳过普通 TeX 注释和续行。头部注释、完成语言参数后的内容、代码正文及关闭行尾部不显示提示。当前项目中重定义或取消定义 `begin` / `minted` 时不套用内置签名；其他项目的同名定义不影响当前文档，未保存设置变化会立即更新提示。

`\inputminted[options]{language}{file}` 提供路径补全、文件链接、定义跳转、参数提示，以及版本化文件和目录重命名。文件名可以使用任意后缀或不带后缀；按主文档目录及 TeX 的 `.tex` 优先规则查找。文件中的 LaTeX 示例作为构建数据读取，不把标签、命令或嵌套输入加入主文档的源码索引；同一文件同时被 `\input` 读取时保留其源码用途。

补全和文件移动保留花括号内的空格文件名，Unicode 名称使用不带引号的 `\detokenize{文件名}`，已有包装保持不变。minted 的默认高亮模式把文件名直接传给 Python，双引号不能用作路径包装；静态编辑跳过这类引号参数。选项、语言、外围注释和换行均保留。静态文件在首次编译之前便进入内容快照，文件内容、别名或路径变化会使构建结果、旧 PDF 和 SyncTeX 失效。

显式的 `verbatim` 宏包选项会绕过 Python，改用 TeX 直接读取文件。此时静态路径仅支持不含空格的名称，不建议补全为空格路径，文件移动也会拒绝引入空格。对旧版 `draft` 设置采用同样的保守限制。Unicode 名称仍使用 `\detokenize` 保护。

路径推断支持默认查找和常见字面格式选项，包括 `linenos`、`breaklines`、`firstline` 与 `lastline`；检查项目内 `\setminted` 和 FancyVerb 设置。动态语言或文件名、未知/宏值选项、执行 TeX 的 `escapeinside` / `texcomments` / `mathescape`、命令重定义及 import 路径前缀会跳过静态推断。`mathescape=false` 与 `texcomments=false` 可继续使用。

编译器明确报告文件缺失时，minted v3 的原生错误和 `verbatim` 模式的 FancyVerb 错误会定位到文件名，使用 `missing-minted-input` 诊断代码并提供有界拼写修复。修复重新核对源码、项目设置、文件查找及文档版本，保留语言和选项。其他 Pygments、样式或 shell escape 错误保持原始编译诊断。

```sh
cargo test --locked --test minted
cargo test --locked --test minted_environments
cargo test --locked --test minted_inputs --test missing_minted_inputs
cargo test --locked --test protocol minted_inline_
cargo test --locked --test protocol static_code_
```

安装 pdfLaTeX 与 minted v3 后，可运行下面的命令。内联边界使用关闭 shell escape 的 `verbatim` 模式，预分词代码还需 XeLaTeX；文件名、环境边界及双模式缺失文件修复回归还需要 PATH 上的 `latexminted`。高亮回归使用受限 shell escape 和 `highlightmode=immediate`，检查实际生成的代码内容，核对空格及 Unicode 路径，避免首次快速构建的占位输出掩盖问题。环境回归同时检查合法代码和结束行多余内容的原生错误，核对标签重命名前后的真实输出：

```sh
cargo test --locked --test minted --test minted_environments --test minted_inputs --test missing_minted_inputs -- --include-ignored
```
