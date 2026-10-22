# 字符串命令的保存与恢复

[返回文档目录](README.md) · [跨文件上下文](string-test-projects.md) · [命令名操作数](declaration-operands.md)

副本也可直接调用，保留复制时的数据参数数量。示例、参数提示、共享文件歧义及编译诊断见[副本的直接调用](string-test-copy-calls.md)。

## 显式恢复

`ifstrequal`、`ifstrempty`、`ifblank` 和 `notblank` 支持通过静态命令复制保存原始含义，再显式恢复。复制捕获执行到该位置时的含义；之后重定义源命令不会改变已经保存的副本。

例如主文件：

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\let\saved\ifblank
\begin{document}
\input{override}
\ifblank{\booltrue{draft}}
\input{restore}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\end{document}
```

`override.tex`：

```tex
\renewcommand{\ifblank}[1]{#1}
```

`restore.tex`：

```tex
\let\ifblank\saved
```

将 boolean `draft` 改名时，声明和自定义调用中的 `booltrue` 会更新，恢复后的 `boolfalse{draft}` 保留为字符串数据。保存副本的名称 `saved` 也可通过普通宏重命名更新，恢复效果随之保持。

## 复制与参数边界

支持未重定义的 `let`（含可选 `=`）、`cslet`、`letcs`、`csletcs`、`NewCommandCopy`、`RenewCommandCopy` 和 `DeclareCommandCopy`，源和目标名称须静态可解析。链式复制、嵌套文件加载以及保存自定义含义后再复制回来，均按实际分析顺序传播。被覆盖的副本不再被当作原始字符串命令；字符串数据和尚未调用的宏正文中的复制不会在声明处执行。

已确认的自定义参数形式也会传给调用文件，包括普通参数、可选参数、xparse 声明及静态 TeX 分隔参数。因此单参数的自定义 `ifblank` 不会误吞紧随其后的 `input`，恢复文件仍参与项目分析。参数边界用于确定读取顺序，不执行任意宏展开。

依赖缓冲区的打开、增量编辑、关闭恢复和磁盘通知会更新后续字符串语义，主文档版本号保持不变。重复加载或多个根文档使同一参数兼具字面和可执行含义时，拒绝会修改该参数的编辑，其他明确的符号仍可重命名。

通过 `undef` / `gundef` / `csundef` / `csgundef` 撤销含义时，较早的副本仍保留捕获内容，较晚制作的副本不再继承已删除的字符串语法，见[撤销定义](string-test-undefinitions.md)。

## 支持边界

识别静态副本的直接调用以及恢复到上述四个测试命令后的含义，不推断用户宏调用的副作用或分组结束时的自动恢复。复制目标或来源名称需要宏展开时，不推断其效果。来源含义无法确认或复制原语被重定义时保留不确定性；多上下文中的数据参数数量冲突按[共同字面前缀与歧义规则](string-test-copy-calls.md#跨文件更新与歧义)处理。

## 验证

```sh
cargo test --locked --test string_test_projects
cargo test --locked --test protocol restored_string_predicates
cargo test --locked --test string_test_projects -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox。三个引擎分别覆盖四种字符串测试的原生、自定义和恢复后调用，比较重命名前后的字符串原文、boolean 状态和 PDF 页数。
