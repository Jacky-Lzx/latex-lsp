# 外部文档和编译后的引用信息

`xr` / `xr-hyper` 的 `\externaldocument[prefix]{document}` 会读取外部 `.aux` 中的标签，支持前缀补全、引用诊断、悬停、源码跳转及项目内引用查找。外部源文件存在时跳转到对应标签；仅有辅助文件时仍可补全和显示编号。外部文档不加入当前项目的源码依赖图，也不会将其中的宏声明引入当前作用域；外部标签不能通过当前项目直接重命名。

本地 `.aux` 优先从实际编译输出目录和 `jobName` 对应的产物路径读取，未找到时查找主文档旁的辅助文件。支持 `\newlabel` 的编号、页码和可选标题，以及 `\@input` 的辅助文件递归；不执行 TeX 命令。悬停显示已有源码信息及这些编译值。

`textDocument/inlayHint` 可在标签引用旁显示编号和页码，在文献引用旁显示已编译的平面文献标签：

```json
{ "latex": { "inlayHints": { "enabled": true, "numbers": true, "pages": true, "citations": true } } }
```

提示只返回请求范围内的引用，每次至多 1024 项。支持相应刷新能力的客户端在编译完成后会收到 `workspace/inlayHint/refresh` 请求。辅助文件修改后缓存按文件元数据更新。

例如根辅助文件的 `\bibcite{paper}{7}` 会在 `\cite{paper}` 的键旁显示 `→ 7`，`\bibcite{paper}{Doe26}` 则显示 `→ Doe26`。`citations` 默认开启，可独立于标签的 `numbers`／`pages` 关闭。只读取当前根及其 `@input` 的文献记录，外部 `externaldocument` 的文献不导入。`nocite`、文献数据查询和分类分配不显示文献标签；静态命名和纯转发的输出引用遵循相同规则。

平面标签最多 256 字符，不展开 TeX，不显示含 TeX 语法、控制字符或分组的标签；同一键出现冲突记录时跳过。natbib 的数字和作者年份模式可以写出相同结构化 `bibcite` 记录，当前不从其第一个字段猜测显示格式。biblatex 的数字提示读取 `\abx@aux@number` 的实际第三字段分区及 `\abx@aux@defaultrefcontext`，不会使用第一字段的引用计数器代替分区。初版支持显式 `style`／`citestyle` 为 `numeric`、`numeric-comp`、`numeric-verb` 的项目；需要真实编译记录，常见的 `defernumbers=true` 会产生这些记录。没有记录时不从数据库顺序猜测编号。

同一物理引用在重复输入或共享根的所有执行分区中必须取得相同编号。作者年份、未认证样式或包选项、自定义文献格式、未认证的上下文分配、动态分区和不完整辅助文件均跳过数字提示；标签提示独立保留。后台项目不会回退到旧 `bibcite`。数字元数据每文件各保留 4096 个编号／上下文，跨文件各最多 8192 项；AUX 整次读取最多 16 MiB，分析工作预算为 32 MiB。

提示与编辑时引用诊断共享项目参数语义：跳过重新定义的引用命令、宏参数数据、声明正文、字面内容和 token 操作数；普通文本样式参数中的引用仍有提示。跨文件未保存定义参与检查，分析不完整或超出预算时跳过提示。

静态 `\csuse{ref}{key}` 和 `\csname ref\endcsname{key}` 等命名引用也显示编号，位置在实际键参数之后。接口或目标命令被重定义时，相应提示随当前项目语义更新。

这些值来自已有辅助文件，编辑后或编译失败时可能过期。过期的本地辅助标签不会掩盖源码中已删除声明的静态警告。读取每个文件最多 2 MiB、各保留 4096 个标签和文献键，每个根辅助文件递归最多 16 个文件，汇总各最多 8192 项；忽略 cleveref 的内部 `@cref` 元数据。动态文件名、宏生成的标签声明和任意辅助文件中的代码不作展开。

已知 biblatex 项目中的手写 `\bibitem` 即使写出 `\bibcite`，也不能提供其后台引用标签；这些传统记录不会用于 biblatex 引用提示。

普通数字样式未启用 `defernumbers` 时，提示从真实加载的 BBL 文献列表认证编号，支持 `defernumbers=false` 及其默认值；此模式忽略旧 AUX 数字记录。启用 deferred 时编号只取 AUX，不回退到列表顺序；前缀可通过同一上下文的 BBL 字段补齐。两个模式沿用相同的分区、共享输入一致性、请求范围和设置检查。

非 deferred 的 BBL 编号支持签名格式 3.3、每分区唯一的标准 entry 列表；完整读取后才返回编号。集合、简写、条目选项、别名、逐字数据、多个列表、前缀及未知／损坏记录在此模式下暂不认证。读取最多 2 MiB，保留最多 4096 个条目和分区；此时也不会回退到传统 `bibcite`，独立标签提示保持可用。

本地 `refcontext` 支持直接原生 `begin/end` 环境及 `refcontext`／`newrefcontext`／`endrefcontext`，选项限于纯字面的 `labelprefix` 与 `sorting`，且需要全局 `defernumbers=true`。引用采用 AUX 为每个键记录的默认上下文；单独进入打印上下文不会直接选择该处引用的标签。因此同一键在 A、B 两个打印上下文中的引用都可能显示 `B1`，而不是分别显示 A1、B1。

非空前缀通过 AUX 的真实分区、默认上下文及编号，联结 BBL 同一分区／上下文／键的 `labelprefix` 字段。上下文中的哈希只用于联结，不反推前缀，不从列表顺序猜测编号。前缀路径允许多个不同上下文的完整标准列表；重复上下文、重复条目、缺损字段、TeX 格式、集合及未知记录会撤销整个前缀证明，独立的空前缀 AUX 标签与标签引用仍保留。最多 4096 个列表和总条目、2 MiB BBL；前缀及最终标签各最多 256 字符。

命名上下文声明、显式 `assignrefcontext*`、`localrefcontext`、动态选项、转发／命名上下文调用、重定义、钩子和自定义格式继续跳过数字提示。

```sh
cargo test --locked --lib bib_context_tests:: -- --include-ignored
cargo test --locked --test protocol local_context_citation_hints_
```
