# LaTeX 内核钩子

[返回文档目录](README.md)

标准内核钩子提供命令补全、片段、悬浮说明及参数提示，包括直接调用、静态 `csuse`／`csname` 和纯转发别名。项目内重定义使用用户签名，未保存依赖更新即时生效。

| 接口 | 参数 |
| --- | --- |
| `AddToHook`、`AddToHookWithArguments` | `{hook}[label]{code}` |
| `AddToHookNext`、`AddToHookNextWithArguments` | `{hook}{code}` |
| `AtBeginDocument`、`AtEndDocument` | `[label]{code}` |
| `RemoveFromHook` | `{hook}[label]` |
| `NewHook`、`NewReversedHook`、`UseHook`、`UseOneTimeHook` | `{hook}` |
| `NewHookWithArguments`、`NewReversedHookWithArguments` | `{hook}{argument count}` |
| `NewMirroredHookPair` | `{before hook}{after hook}` |
| `NewMirroredHookPairWithArguments` | `{before hook}{after hook}{argument count}` |
| `DeclareHookRule` | `{hook}{first label}{relation}{second label}` |
| `DeclareDefaultHookRule` | `{first label}{relation}{second label}` |
| `ClearHookRule` | `{hook}{first label}{second label}` |
| `ClearHookNext`、`ActivateGenericHook`、`DisableGenericHook`、`ShowHook`、`LogHook` | `{hook}` |
| `SetDefaultHookLabel`、`PushDefaultHookLabel` | `{label}` |
| `PopDefaultHookLabel`、`DebugHooksOn`、`DebugHooksOff` | 无参数 |

片段省略可选标签并保留已有实参。完整代码参数中的标签引用保留结构导航和重命名；名称、标签、参数数量及排序规则按数据保护。未闭合的参数继续保护内部 token，完整调用后的独立源码正常参与分析。

代码参数按延迟代码处理，不据此推断当前文献分区、分类或引用上下文，也不与正文的环境及数学状态直接合并。代码中的消息或字面读取器不认证其中标签的执行身份；保持与延迟宏正文一致的结构索引。

已登记代码可能重新定义字符串判断命令时，撤销相关命令后续的已知含义，避免在钩子执行后仍把真实宏调用当作字面数据而漏改。此保护不假定钩子一直未使用；显式恢复后仍可能再次触发，因此恢复命令不会清除尚未确定的钩子影响。无法确认的文件加载和超限分析同样保守处理，独立标签保持可编辑。

目前不模拟钩子触发、删除、排序或一次性执行，也不提供钩子名称／代码标签自身的导航与改名。`UseHookWithArguments`／`UseOneTimeHookWithArguments` 的可变参数数量，以及钩子条件接口继续待实现。
