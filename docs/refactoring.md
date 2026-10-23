# 结构重构与批量修复

`textDocument/codeAction` 提供以下操作：

- `refactor.rewrite`：将选区包裹为 `\emph{...}`、`\textbf{...}` 或 `quote` 环境。
- `refactor.extract`：将主文档中的正文选区提取为无参数 `\newcommand`，在 `\begin{document}` 前插入声明，并替换原文。自动选择项目内未使用的命令名，之后可用符号重命名改为所需名称。
- `source.fixAll`：合并当前文件中没有歧义且编辑范围不冲突的静态语法修复。仅在客户端显式请求该类操作时返回，可用于保存时修复；不自动选择拼写建议或编译器错误的候选修复。

重构只处理顶层的普通正文及完整的常用文本样式命令。部分控制序列、命令参数内部、公式、特殊环境、注释、逐字代码、原始宏参数和不平衡分组不提供重构，避免改变参数边界。命令提取要求当前文件有 `document` 环境；不模拟 TeX 宏展开和任意 catcode 设置。

参数边界复用当前项目的静态签名，包含无花括号的单 token 参数、可选参数、TeX / xparse 分隔参数、标志和静态 `csuse` / `csname` 调用。纯转发别名、未保存的跨文件定义、歧义和未知调用使用相同的保守检查。例如，`\take Hello` 会读取首个 `H` 时，不将 `Hello` 包成新命令或分组；选择已消费的 `H` 之后的正文仍可重构。`noexpand` 等单 token 操作数也保留原边界，完整选择顶层样式调用继续可用。

项目索引不完整，或分析范围超过 16 MiB 源码、100000 个命令或 4096 个定义时，不提供结构重构。此限制不改变已有快速修复与显式 `source.fixAll` 的入口。

编辑使用客户端协商的文档版本。旧客户端通过 `latex.applyCodeAction` 应用操作时，服务器会重新计算并核对文档版本与操作类型；过期或伪造操作不会执行。

服务器支持标准 `codeAction/resolve`。客户端同时声明 `codeActionLiteralSupport`、`dataSupport: true`，且 `resolveSupport.properties` 包含 `edit` 时，重构、快速修复和 `source.fixAll` 返回带 `data` 的操作，编辑在解析请求中返回。标题、类型、诊断、首选标记及其他原有字段保持不变；其余客户端继续收到内联编辑或既有应用命令。重编译操作保留原来的命令和警告校验流程。

解析会核对项目文件的内容与版本、作用域、设置和操作身份，再按原始范围及诊断上下文重新计算编辑。文档更新、相关项目定义变化、诊断修复消失或操作被改写时返回 `-32801`，编辑器应重新请求操作；结构不合法的 `data` 返回 `InvalidParams`。其他独立项目的缓冲区更新不会使当前项目的操作失效。无解析数据的操作原样返回。

验证：

```sh
cargo test --locked --lib code_action_resolve
cargo test --locked --test protocol code_action_resolution
cargo test --locked --test refactor --test rebuild_actions
cargo test --locked --test refactor real_refactors_after_consumed -- --include-ignored
```
