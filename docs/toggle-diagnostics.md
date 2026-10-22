# toggle 诊断与修复

[返回文档索引](README.md) · [布尔开关与 toggle](symbols.md#布尔开关与-toggle)

etoolbox 报告 `Toggle 'darft' undefined` 时，服务器核对源码与日志回显，精确标记直接操作的名称，并提供当前项目内的拼写修复：

```tex
\newtoggle{draft}
\settoggle{darft}{true}
\iftoggle{darft}{Yes}{No}
```

选择 ``Replace with toggle `draft` `` 仅修改相应名称，保留赋值、分支正文与 `\global` 等前缀。支持 `settoggle`、`toggletrue`、`togglefalse`、`iftoggle` 和 `nottoggle`，包括跨行参数和同一行的重复调用。

## 候选与边界

建议来自当前项目的静态 `newtoggle` / `providetoggle` 声明。boolean、普通命令和其他项目中的同名符号不作为候选；重复声明或内部 `etb@tgl@名称` 控制序列与宏、寄存器、环境等冲突时不提供该候选。

动态或不完整实参、被重定义的操作、宏展开、操作被宏当作参数消费以及无法唯一匹配的源码回显保留整行诊断。布尔表达式中的 `togl {...}` 错误由 etoolbox 以 `Invalid boolean expression` 报告，目前保留编译器的行级错误。

## 协议与验证

精确诊断代码为 `undefined-toggle`，名称数据例如 `{"toggle":"darft"}`，同时记录错误发生时的项目宏来源。修复前重新检查参数边界、候选声明、来源和文档版本；兼容版本化编辑与旧客户端，拒绝过期修复。通用规则见[编译诊断](diagnostics.md#分析范围与来源确认)，验证命令见[测试说明](testing.md#toggle-诊断)。
