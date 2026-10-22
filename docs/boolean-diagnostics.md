# boolean 诊断与修复

[返回文档索引](README.md) · [布尔开关与 toggle](symbols.md#布尔开关与-toggle)

etoolbox 报告 `Boolean '\ifdarft' undefined` 时，服务器将日志中的条件命令名对应到源码的基础名称 `darft`，精确标记名称并提供项目内拼写修复：

```tex
\newbool{draft}
\setbool{darft}{true}
\ifbool{darft}{Yes}{No}
```

选择 ``Replace with boolean `draft` `` 仅修改相应名称。支持 `setbool`、`booltrue`、`boolfalse`、`ifbool` 和 `notbool`，保留赋值、分支、注释及 `\global` 等前缀；支持跨行参数和重复调用。

## 候选与边界

建议来自当前项目静态的 `newif`、`newbool` / `providebool`、`newboolean` / `provideboolean` 声明，包括展开字面 `csname` 的 `newif`。toggle 和普通命令使用独立命名空间，其他项目的声明不参与修复。重复声明、条件命令或任一派生赋值命令与其他定义冲突时不提供该候选。

内置 `true` / `false` 仅用于 `ifbool` / `notbool` 的只读条件建议；它们没有对应赋值命令，不能用于赋值修复。被项目定义覆盖的内置条件不作为候选。

动态、不完整或含语义空白的名称、宏展开、被宏消费的操作、重定义以及无法唯一核对的日志回显保留整行错误。ifthen 自身的错误与 etoolbox 的 `Invalid boolean expression` 使用不同格式，目前保留原有行级错误。

## 协议与验证

精确诊断代码为 `undefined-boolean`，名称数据例如 `{"boolean":"darft"}`，不含日志中的 `\if` 前缀。诊断记录宏来源；提供修复前重新检查实参、候选、来源与版本，支持版本化编辑和旧客户端，拒绝过期修复。通用规则见[编译诊断](diagnostics.md#分析范围与来源确认)，验证命令见[测试说明](testing.md#boolean-诊断)。
