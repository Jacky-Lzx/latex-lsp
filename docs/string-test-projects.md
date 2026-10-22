# 字符串测试的跨文件上下文

[返回文档目录](README.md) · [字符串测试](string-tests.md) · [项目与文件](projects.md)

## 加载顺序

`ifstrequal`、`ifstrempty`、`ifblank` 和 `notblank` 的数据边界按直接调用处的上下文确定。当前文件或已加载的本地依赖显式重定义该命令后，后续调用停止使用内置字符串语法，参数中的符号按普通源码建立索引。

例如主文件：

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\begin{document}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\input{definitions}
\ifblank{\booltrue{draft}}
\end{document}
```

`definitions.tex`：

```tex
\renewcommand{\ifblank}[1]{#1}
```

将 boolean `draft` 改名时，声明和第二次调用中的 `booltrue` 会更新；第一次调用的 `boolfalse` 仍是字符串数据，保留原文。

支持直接源码包含及本地宏包、文档类的嵌套加载。[章节选择](selected-string-tests.md)按加载顺序过滤 `include` 的语义传播，被排除章节中的重定义不会影响后续字符串调用。父文件中先前的重定义传给子文件，子文件的重定义也影响返回父文件后的调用；原生字符串中的加载命令不生效。宏正文中仅声明、尚未调用的加载和重定义不会在声明处传播。

依赖使用编辑器中的未保存内容。打开、增量编辑、关闭依赖缓冲区及磁盘变更通知都会更新相关解析；关闭时恢复磁盘内容，主文档的版本号保持不变。自定义调用暴露的新静态输入也会载入，包括工作区目录外的文件，遵守[文件索引限制](projects.md#工作区索引与项目隔离)。

[保存与显式恢复](string-test-copies.md)说明命令副本捕获的含义、跨文件参数边界，以及恢复前后的重命名规则。

## 共享文件与支持边界

同一源码在多个主文档中，或在一个主文档中被多次加载时，可能同时存在字符串数据与可执行参数两种解释。这种参数不提供单一导航或准备重命名结果；从声明发起的重命名若会修改它，也会被拒绝。同文件中语义一致的其他符号仍可重命名。

采用静态分析，此处的上下文传播限于直接文件加载和显式命令赋值。`InputIfFileExists` / `IfFileExists` 的本地条件选择、前置钩子和文件事件更新见[条件文件钩子](conditional-string-tests.md)。不推断其他 TeX 条件、加载别名、动态命令名、用户宏调用后的副作用或分组结束后的命令恢复，也不依赖 TeX 发行版内部的宏包实现。条件分支仍按源码索引；循环、递归深度或分析次数超限时不选择任意的字符串解释，相关编辑会受到保守限制。

## 验证

默认回归覆盖加载顺序、嵌套源码与宏包、未保存内容、外部依赖、共享文件、重复加载，以及字面命令和延迟宏正文的边界。stdio 回归检查增量编辑、关闭恢复、磁盘通知和主文档版本化编辑：

```sh
cargo test --locked --test string_test_projects
cargo test --locked --test protocol imported_string_predicates
```

三引擎回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox，核对导入重定义前后的字符串原文、实际 boolean 状态、重命名结果及 PDF 页数：

```sh
cargo test --locked --test string_test_projects -- --include-ignored
```
