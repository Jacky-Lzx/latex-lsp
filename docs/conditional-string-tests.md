# 条件文件钩子与字符串测试

[返回文档目录](README.md) · [跨文件上下文](string-test-projects.md) · [项目与文件](projects.md)

## 钩子执行顺序

字符串测试的含义传播支持未重定义的直接 `InputIfFileExists` 和 `IfFileExists` 调用。文件名及两个钩子须使用完整花括号参数，文件名须能静态解析。

`InputIfFileExists` 找到文件时，先分析第二个参数的命令，再读取子文件；第三个参数不会向后传播声明。文件确认不存在时只传播第三个参数。`IfFileExists` 采用相同的分支选择，但不会读取被检查文件中的源码。

例如：

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\newcommand{\identity}[1]{#1}
\begin{document}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\InputIfFileExists{./checked}{\let\ifblank\identity}{}
\end{document}
```

`checked.tex`：

```tex
\ifblank{\booltrue{draft}}
```

子文件执行前，`ifblank` 已被改成单参数宏，因此 `booltrue` 的参数可导航和重命名。主文件较早调用中的 `boolfalse` 仍是字符串数据，改名保留原文。也支持钩子内的嵌套文件检查及静态输入。

## 本地条件与更新

查找遵守项目既有的主文档目录、当前文件目录和后缀规则。已找到的本地文件可确定成功分支；带目录的显式路径（如 `./missing`）在本地候选均不存在时可确定失败分支。无法在项目中找到的裸名称仍可能来自 TeX 发行版，不能仅据本地缺失选择失败分支。

未选中钩子的源码继续参与静态符号索引，但其中的声明和输入不作为这次文件条件的执行副作用传播。动态文件名、被重定义的检查命令、未完成的参数及无法确认的条件保留不确定性，相关字符串参数不提供猜测性的自动改名。不执行任意宏展开，也不模拟其他 TeX 条件。

存在性检查也可以引用 CSV、图片等非源码文件。创建、删除或文件监听通知会重新分析相关上下文，主文档的文本和版本号保持不变，非源码文件不会因此进入 LaTeX 符号索引。共享源码的文件条件按各根文档分别解析；同一参数出现不同含义时拒绝修改该参数，其他符号仍可改名。

## 验证

```sh
cargo test --locked --test conditional_string_tests
cargo test --locked --test protocol conditional_string_hooks
cargo test --locked --test conditional_string_tests -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox。在三个引擎中分别检查两种命令的成功与失败分支，比较重命名前后的钩子、子文件执行顺序、boolean 状态和 PDF 页数。
