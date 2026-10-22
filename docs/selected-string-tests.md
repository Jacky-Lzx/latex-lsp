# 章节选择与字符串语义

[返回文档目录](README.md) · [跨文件上下文](string-test-projects.md) · [条件文件钩子](conditional-string-tests.md)

## 加载顺序

字符串测试的跨文件语义传播遵守未重定义的直接 `\includeonly` 和 `\include` 调用。每个根文档分别维护章节选择：未声明选择时包含所有章节，空列表排除所有章节；后续选择替换先前选择，已加载设置文件中的选择也按加载顺序生效。普通 `\input` 不受章节选择过滤。

例如主文件：

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\includeonly{}
\begin{document}
\include{definitions}
\ifblank{\booltrue{draft}}{}{}
\boolfalse{draft}
\end{document}
```

`definitions.tex`：

```tex
\renewcommand{\ifblank}[1]{#1}
```

章节被排除时，其中的重定义不会改变后续 `ifblank` 的含义。重命名 boolean `draft` 会更新声明和 `boolfalse`，保留字符串数据中的 `booltrue{draft}`。将选择改为 `\includeonly{definitions}` 后，章节中的重定义生效，后续调用中的 `booltrue` 也会参与重命名。

选择也可来自[条件文件钩子](conditional-string-tests.md)：只传播选中的钩子；`InputIfFileExists` 先执行成功钩子，再读取文件，被读取文件中的后续选择可以覆盖钩子中的选择。原生字符串数据、尚未调用的宏正文和复制命令的操作数不会在声明处执行章节选择。

## 名称匹配

- 列表和 `include` 参数均可省略一个末尾的 `.tex`；`chapter.tex.tex` 与 `chapter.tex` 仍是不同章节名称。
- 保留路径拼写，`./chapter` 与 `chapter` 不匹配，即使它们指向同一磁盘文件。
- 忽略列表条目外围空白及有效的 TeX 注释，保留静态文件名内部的空格。
- 文件名须能静态解析，列表和文件参数须完整。

## 共享文件、更新与边界

被排除的章节及其依赖继续供编辑器索引，但不会作为额外入口向实际执行的共享文件传播上下文。同一共享文件在其他根文档或其他实际加载位置仍会按各自的选择分析。字符串参数存在数据与可执行源码两种含义时，拒绝会修改该参数的重命名，同文件中语义明确的其他符号仍可编辑。

依赖缓冲区的未保存内容、增量编辑、关闭恢复及磁盘变更通知都会重新计算字符串语义，主文档的文本版本保持不变。

动态选择、被重定义的选择命令或包含命令无法确定实际章节时，在遇到相关 `include` 后为后续字符串参数保留歧义；仅声明了未知选择、尚未遇到包含调用时，不因此改变已有字符串含义。此处不执行任意宏展开，也不推断章节选择的调用别名或分组恢复。

编译输入和过期结果检查的支持范围见[编译输入规则](build-inputs.md)；无法一致确认的多次选择仍按该文档的保守规则处理。

## 验证

```sh
cargo test --locked --test selected_string_tests
cargo test --locked --test protocol selected_string_predicates
cargo test --locked --test selected_string_tests -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox，覆盖默认全选、空列表、显式后缀、不同路径拼写及设置文件中的选择覆盖，比较重命名前后的章节执行、boolean 状态和 PDF 页数。
