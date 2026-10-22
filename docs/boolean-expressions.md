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

## 日志与协议

诊断消息追加编译器给出的 `Boolean '名称' undefined.` 或 `Toggle '名称' undefined.`，保留两个命名空间的独立错误。标准帮助正文缺失、残缺或与当前错误无法确认关联时，不猜测名称；折行说明在确认完整结束后恢复，不跨越其他诊断。

精确诊断复用 `undefined-boolean` / `undefined-toggle` 代码、名称数据与宏来源校验。版本化编辑、旧客户端以及过期修复遵循[通用规则](diagnostics.md#分析范围与来源确认)。验证命令见[测试说明](testing.md#布尔表达式诊断)。
