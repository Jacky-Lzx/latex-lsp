# 布尔表达式诊断与修复

[返回文档索引](README.md) · [boolean 诊断](boolean-diagnostics.md) · [toggle 诊断](toggle-diagnostics.md)

etoolbox 的 `ifboolexpr`、`ifboolexpe`、`whileboolexpr` 和 `unlessboolexpr` 会将未定义操作数报告为 `Invalid boolean expression`，具体名称位于日志的帮助正文。服务器保留该说明，并在核对完整表达式和源码回显后精确标记 `bool {...}` 或 `togl {...}` 中的名称。

```tex
\newbool{draft}
\newtoggle{draft}
\ifboolexpr{not (bool {darft} and togl {darft})}{Yes}{No}
```

两个拼写错误分别提供 ``Replace with boolean `draft` `` 和 ``Replace with toggle `draft` ``，保留运算符、分组、另一命名空间的操作数及分支。boolean 候选和 toggle 候选各自遵循对应的[声明规则](boolean-diagnostics.md#候选与边界)与[冲突检查](toggle-diagnostics.md#候选与边界)。`bool` 操作数也支持未被覆盖的内置 `true` / `false` 建议。

## 表达式边界

支持静态名称、`not` / `and` / `or`、圆括号、注释和跨行参数，包括 `nottogl{...}` 等紧接的关键字。`test {...}` 支持未被重定义的 `ifstrequal`、`ifstrempty`、`ifblank` 和 `notblank`，要求字符串参数为字面内容；字符串中的类似操作数文本不会被当作引用。

动态名称、任意宏生成的表达式或测试、重定义、残缺或无法确认的语法，以及同一表达式内同一命名空间的重复名称保留行级诊断。宏生成的名称和嵌套测试可能产生与外层操作数相同的回显，因此不能仅按错误名称搜索源码。分析超出预算时也保留原错误。

循环命令在报错前已读取表达式和循环体，日志必须确认完整读取边界；条件命令在报错前只读取表达式。修复不修改循环体、条件分支或赋值。

缩短的源码回显必须在对应源码行中唯一，并与调用的读取终点一致。帮助正文提供完整名称时，回显可只覆盖名称的后半部分；如果只剩结束花括号，或长循环体使操作数完全不可见，则保留行级诊断。

## 注释与源码编辑

关键词内部的注释续行遵循 TeX 的读取规则。例如，下面的 `bo` 和 `ol` 组成 `bool`：

```tex
\newbool{draft}
\ifboolexpr{bo% 保留这条注释
  ol{draft}}{Yes}{No}
```

`bool`、`togl`、`not`、`and`、`or` 和 `test` 均支持这种写法，包括 CRLF、下一行缩进和连续注释。导航、引用、悬停、名称补全与跨文件重命名能识别相应的操作数；编译诊断及拼写修复使用名称本身的源码范围。编辑保留关键词、注释和缩进，同名 boolean 与 toggle 各自更新。

普通空格、空行和转义的百分号不会拼接成关键词。`test` 中字面字符串里的类似操作数文本不作为 boolean / toggle 引用；名称本身仍须为静态字面内容。

## 日志与协议

诊断消息追加编译器给出的 `Boolean '名称' undefined.` 或 `Toggle '名称' undefined.`，保留两个命名空间的独立错误。标准帮助正文缺失、残缺或与当前错误无法确认关联时，不猜测名称；折行说明在确认完整结束后恢复，不跨越其他诊断。

精确诊断复用 `undefined-boolean` / `undefined-toggle` 代码、名称数据与宏来源校验。版本化编辑、旧客户端以及过期修复遵循[通用规则](diagnostics.md#分析范围与来源确认)。即使主文件版本未变，依赖文件重定义测试命令后也会拒绝旧修复；同一命名空间新增重复操作数后也须重新确认位置。

## 验证

默认回归检查两个命名空间、表达式与循环的读取边界、字面测试字符串、动态或歧义操作数、帮助正文的关联与折行、重复及缩短回显、跨行和 UTF-16 范围，以及版本化和旧客户端修复：

```sh
cargo test --locked --test boolean_expression_diagnostics
cargo test --locked --test protocol expression_fixes_recheck_cross_file_tests_without_a_main_version_change
```

注释续行另有编辑、诊断和版本化跨文件重命名回归：

```sh
cargo test --locked --test expression_keywords
cargo test --locked --test protocol commented_expression_keywords_return_versioned_cross_file_renames_over_stdio
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox：

```sh
cargo test --locked --test boolean_expression_diagnostics -- --include-ignored
cargo test --locked --test expression_keywords -- --include-ignored
```

三个引擎分别使用 60 / 79 列和传统 / 文件行号日志。每组验证 11 处操作数修复，再重新编译生成 PDF，并核对条件分支、循环次数、内置条件、跨行表达式与长名称的实际结果；另确认动态名称及用户测试宏保留行级诊断。其他外部工具测试见[真实工具回归](testing.md)。

注释续行的真实回归在三个引擎中分别使用传统和文件行号日志。先独立重命名跨文件的 boolean 与 toggle，核对分支、字面字符串和两次循环的结果；再引入两个命名空间的拼写错误，验证精确范围、修复及重新编译后的相同结果。
