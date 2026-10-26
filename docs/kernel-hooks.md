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

名称匹配保留内核差别：声明／注册接口规范化名称首尾的普通 TeX 空白，`UseHook`／`UseHookWithArguments` 和四个条件接口保留原始名称。`NewHook{ custom }` 可被 `UseHook{custom}` 命中，而 `UseHook{ custom }` 不与其关联。普通名称中的点保持字面含义。含内部空白、依赖默认标签的 `.`／`./...` 名称、动态展开和跨注释拼接的名称暂不推断；没有源码声明的内置或自动生成钩子也没有虚构的定义位置。

自定义字面名称支持准备改名与完整跨文件改名。要求一个明确的源码声明，全部共享消费者均认定同一声明和接口含义；会同时更新声明、注册、使用、条件和排序等名称位置，镜像声明的另一名称保持独立。同名的引用标签、宏、代码标签和普通文本保持原义。声明／注册的首尾空白及原始注释保留；裸单 token 名称改成多字符时补上花括号，避免把新名称拆成多个参数。未保存提供文件和消费者的变化即时刷新，支持两种 WorkspaceEdit 格式及打开文档的版本号。

动态或相对名称、不完整字段、未知参数数量、不同消费者的身份分歧、捕获复制及先消费自身参数的接口转发，都不能认证完整改名。目标名称不能捕获既有声明或使用，也不能进入内核或自动触发的命名空间，例如 `begindocument`、`label`、`env/...`、`cmd/...`、`file/...`、`shipout/...`，以及保存全局默认规则的保留名称 `??`。来源索引不完整、未知文件加载及可执行文献数据库代码同样拒绝编辑。

初版名称编辑只认证上述静态公开接口。底层 expl3 展开／复制、内部 hook 存储访问（含旧式 `@名称hook` 代码块）、`DisableHook`／`ProvideHook` 等已弃用接口、`UseName` 等未认证的命名执行、动态控制序列声明、相关宏补丁及未知取消定义目标，可能隐藏或改变实际调用，因此保守拒绝。显式字符串执行、字符大小写转换及 catcode 修改也不参与名称编辑认证：例如 `\lowercase{\UseHook{CUSTOM}}` 会调用 `custom`，不能按源码大小写直接改名。字面读取器（包括 `\detokenize`、verbatim、URL）的源码重定义或相关补丁同样拒绝认证，避免把被解析器隐藏的可执行调用漏出编辑集。拒绝名称编辑不撤销已有只读导航；这些限制不证明任何钩子已经执行。代码标签使用下述独立导航；代码标签改名仍待认证。

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
cargo test --locked --test hook_rename
cargo test --locked --test hook_rename -- --ignored
cargo test --locked --test protocol hook_name_renames_keep_exact_namespaces
```

## 显式代码标签

`AddToHook`／`AddToHookWithArguments` 的显式可选标签支持定义跳转、引用查找、悬浮、高亮和补全。`RemoveFromHook` 的可选标签、`DeclareHookRule` 的两个标签和 `ClearHookRule` 的两个标签使用同一索引。身份由 `(hook 名称, 代码标签)` 确定；同一标签的多次代码注册都作为定义返回，不与其他 hook、普通引用键或宏合并。

`AtBeginDocument[label]{code}` 和 `AtEndDocument[label]{code}` 的显式标签分别归入 `begindocument` 与 `enddocument`，与相应 `AddToHook`、移除及排序规则关联；同名的开始／结束标签保持独立。读取项目内可确认的接口含义时，也检查底层 `AddToHook` 是否被源码替换。入口被改成另一已确认纯转发时，以当前内核叶接口及其参数形状解析，例如转发到 `AtEndDocument` 的开始入口归入结束 hook。未知接口或数据操作数不制造注册位置。

支持直接、静态命名、传统／xparse 纯转发及环境生成的零参数接口。边界注释和首尾 TeX 空白规范化，跨注释拼接、动态及相对名称暂不推断。必选字段允许裸单 token；相邻参数的首字符优先归属于后一个字段，补全成多字符标签时自动加组。补全替换完整后缀，按 hook 和前缀筛选，支持空字段、未闭合字段及 UTF-16／CRLF 范围。未保存提供文件和共享消费者的含义变化即时刷新。

`DeclareDefaultHookRule` 的两个显式标签使用全局规则作用域：定义跳转返回当前项目各 hook 的同名显式注册，引用查找包含这些注册、局部规则及移除位置；补全从各 hook 的注册标签中取并集，并替换当前字段的完整后缀。全局规则字段的悬停说明其影响所有 hook。局部字段的定义跳转和补全仍限于自身 hook，但引用查找会包含同名全局规则，不把其他 hook 的注册合并进局部定义。

内核把全局默认规则存放在保留名称 `??` 下；已确认的 `DeclareHookRule{??}` 和 `ClearHookRule{??}` 标签也关联同一全局作用域，名称首尾 TeX 空白正常规范化。局部规则可覆盖全局排序，清除全局规则会改变各 hook 的顺序；索引保留全部源码位置，不据此模拟最终排序或删除状态。接口若被纯转发到另一已确认入口，则按当前叶接口的实际参数形状和作用域读取。

这仍是结构导航，不模拟注册时间、移除或排序后的执行状态。省略标签、默认／相对标签、`*` 和 `top-level` 的身份尚未认证，不生成相应导航。代码标签改名尚未实现：全局默认规则可能连接多个 hook，局部字符串替换会改变排序，不能直接复用只读定义位置作为完整编辑集。原生回归覆盖代码块顺序、移除、实际事件、独立引用编辑前后的完整 AUX，以及四种全局规则编辑对照。把普通自定义 hook 改成 `??` 会与内核规则存储冲突，因此名称改名明确拒绝这个目标。

生命周期专项还用原生事件区分开始和结束阶段，检查接口被替换或调用成为 hook 数据时不注册代码。同名普通引用键的实际两处编辑不会改变代码标签，编辑后保留全部代码块的执行顺序和完整 AUX。未保存提供文件的底层接口替换、转发目标切换、共享消费者分歧与恢复均有精确 UTF-16／CRLF stdio 回归，普通标签编辑保留当前文档版本。

全局规则专项同时覆盖直接、静态命名、传统／xparse 及环境生成控制序列转发、必选裸字段、动态关系参数、边界注释、空与未闭合字段。未保存的接口替换、共享消费者分歧、关闭恢复、同一行非 BMP 字符及两种编辑格式均有精确 stdio 回归；全局标签查询不触发同名普通引用键的改名。

```sh
cargo test --locked --test hook_code_labels
cargo test --locked --test hook_code_labels -- --ignored
cargo test --locked --test protocol global_hook_rules_refresh
cargo test --locked --test protocol hook_name_renames_keep_exact
```
