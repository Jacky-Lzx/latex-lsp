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
| `UseHookWithArguments`、`UseOneTimeHookWithArguments` | `{hook}{argument count}`，后接 hook 的数据参数 |
| `IfHookEmptyTF`、`IfHookExistsTF` | `{hook}{true code}{false code}` |
| `IfHookEmptyT` | `{hook}{true code}` |
| `IfHookEmptyF` | `{hook}{false code}` |
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

`UseHook`／`UseOneTimeHook` 也会读取声明的参数，并非只有带 `WithArguments` 的接口才读取。参数可能被钩子作为字面文本使用：若宏或引用的改名会涉及这些数据 token，服务器拒绝整个改名，避免改变捕获的控制序列名称或显示的引用键。保护涵盖静态命名、纯转发及复制调用、跨文件未保存声明和共享源码的不同消费者。动态数量、声明歧义及一次性调用使用保守参数上界；这项保护不证明参数中的命令会执行。独立标签仍可改名，分析工作量有上限。

参数保护也沿包装宏传递：例如 `\newcommand\invoke[1]{\UseHook{custom}{#1}}` 的实际参数可能成为字面数据。分析覆盖传统／xparse 参数、可选默认值、嵌套包装和可能保留旧正文的复制；没有进入数据的独立形参继续参与完整改名。裸 `#1` 可以替换为空，因此还会检查 hook 是否继续读取包装调用后的 token；TeX 固定前缀和分隔符属于调用语法，不计作实际数据。无法确认参数语法或工作量超限时保守拒绝改名。

传统及 xparse 环境包装同样适用。开始正文中的形参、xparse 结束正文引用的开始参数，以及可选默认值，都映射到 `begin` 的实际参数；支持命名／纯转发的环境接口、命令与环境的嵌套和可能保留旧参数语法的环境复制。共享文件须兼顾所有消费者，未保存的正文与默认值更新即时生效；独立形参和标签保持完整编辑。

先消耗自身参数、再只转发 `UseHook` 或环境接口的包装不等同于零参数别名。例如 `\def\invoke#1{\UseHook}` 会先消耗 `#1`，hook 名称在其后读取；当前尚不认证这类额外语法的完整边界，遇到相关调用时拒绝改名。动态环境名称及不完整参数同样使用保守边界，不据此证明环境或 hook 实际执行。

环境声明还生成可直接调用的开始／结束控制序列。`\newenvironment{invoke}[1]{\UseHook}{}` 的 `\invoke{Ignored}{custom}\word` 也会先消费自己的参数；安全图覆盖这些控制序列、静态命名、嵌套转发及命令／环境复制。零参数开始或结束控制序列按可能的接口读取保护数据，带额外参数的转发保持拒绝，未保存与共享消费者的含义变化即时生效。这些安全边不用于认证只读交互或即时执行身份。

已登记代码可能重新定义字符串判断命令时，撤销相关命令后续的已知含义，避免在钩子执行后仍把真实宏调用当作字面数据而漏改。此保护不假定钩子一直未使用；显式恢复后仍可能再次触发，因此恢复命令不会清除尚未确定的钩子影响。无法确认的文件加载和超限分析同样保守处理，独立标签保持可编辑。

条件接口的完整分支保留结构引用与标签改名，但不认证实际选中的分支，也不用于证明代码即时执行。任一分支中的潜在赋值、宏补丁回调及其静态命名／转发／复制调用，都纳入字符串含义保护，避免只改写部分实际宏调用。未选择分支中的恢复不会抵消另一分支的潜在修改；无关修改及未调用定义中的条件不影响原有字符串判断。

## 钩子名称

字面钩子名称支持定义跳转、引用查找、悬浮、高亮及名称补全。定义由 `NewHook`、`NewReversedHook` 和对应带参数／镜像声明提供；注册、使用、条件、移除、排序和查看接口的名称按独立命名空间关联，不与同名标签、宏或代码标签混合。补全替换名称的完整后缀，支持空名称、未闭合分组和 UTF-16／CRLF 坐标；候选来自当前项目的完整字面声明。

直接调用、静态 `csuse`／`csname` 和唯一纯转发别名使用同一索引。跨文件未保存定义即时生效；共享源码只在所有消费者对调用含义达成一致时提供交互。完整延迟正文保留源码导航，已确认的数据实参和被替换的内核接口不制造钩子引用。这是源码结构导航，不证明钩子已经声明或触发。

名称匹配保留内核差别：声明／注册接口规范化名称首尾的普通 TeX 空白，`UseHook` 和四个条件接口保留原始名称。`NewHook{ custom }` 可被 `UseHook{custom}` 命中，而 `UseHook{ custom }` 不与其关联。普通名称中的点保持字面含义。含内部空白、依赖默认标签的 `.`／`./...` 名称、动态展开和跨注释拼接的名称暂不推断；没有源码声明的内置或自动生成钩子也没有虚构的定义位置。名称与代码标签的改名继续待独立认证。

使用接口根据项目内唯一、可确认的字面声明提供 `{argument 1}` 等参数提示，并将数据参数排除出引用导航。已声明的 hook 使用声明数量，覆盖调用处数量；未声明的字面 hook 使用 `WithArguments` 接口给出的 0–9 数量。直接、静态命名和纯转发调用均适用，未保存的提供文件变化刷新提示与导航边界。已声明零参数 hook 后的独立引用保留正常跳转。

这仍是静态源码分析，不模拟钩子触发、删除、排序或一次性执行。检测到一次性调用时不提供精确的附加参数提示，数据边界采用声明与调用数量的保守上界；动态名称、动态声明和歧义来源不认证精确数量。改名检查另行使用更宽的安全边界，因此部分原生可执行引用仍可能被拒绝改名；不能把参数提示当作执行证明。

验证名称、条件交互及原生行为：

```sh
cargo test --locked --test hook_names --test hook_conditions --test hook_arguments --test hook_variadic
cargo test --locked --test hook_names --test hook_conditions --test hook_arguments --test hook_variadic -- --ignored
cargo test --locked --test protocol hook_names_keep_exact_ranges
cargo test --locked --test protocol hook_condition_guards
cargo test --locked --test protocol hook_argument_data_guards
cargo test --locked --test protocol hook_wrapper_data_guards
cargo test --locked --test hook_environments
cargo test --locked --test hook_environments -- --ignored
cargo test --locked --test protocol hook_environment_data_guards
cargo test --locked --test protocol generated_environment_interface_guards
cargo test --locked --test protocol variadic_hook_help
```
