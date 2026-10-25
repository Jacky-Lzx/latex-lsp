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

命名上下文声明、未认证的自动分配、`localrefcontext`、动态选项、转发／命名上下文调用、重定义、钩子和自定义格式继续跳过数字提示。

```sh
cargo test --locked --lib bib_context_tests:: -- --include-ignored
cargo test --locked --test protocol local_context_citation_hints_
```

直接且未重定义的强分配 `\assignrefcontextentries[options]{keys}` 支持普通资源的 deferred 数字提示，选项限于字面 `sorting` 和 `labelprefix`。省略 `sorting` 时，要求 biblatex 包选项明确给出字面排序；暂不推断样式的排序默认值。分类自动分配另按下述成员规则认证；关键词自动分配另按下述 BBL 成员认证；源码集合和 IDS 相交继续保护。

星号弱分配 `\assignrefcontextentries*[options]{keys}` 也支持相同的字面选项与编译记录。强分配优先于弱分配，各自先查通配再查单键；同类分配按最后一次执行更新。弱分配只在显式上下文外生效；进入显式上下文时，跳过弱分配并使用 AUX 默认记录，不直接采用该环境的前缀。直接原生 `refcontext` 环境及 `refcontext/newrefcontext/endrefcontext` 命令按实际分组认证局部开关：花括号、普通环境和原生 `begingroup/endgroup`、`bgroup/egroup` 结束时恢复进入前的状态；条目分配本身仍全局保存。分组与上下文边界可跨输入文件，文件名参数不会制造包围输入执行的分组。未知／错配作用域、命名／转发的分组控制、宏正文中的隐式分组或数学切换继续跳过弱提示。普通公式可以出现在上下文命令前后；美元及括号数学内部的上下文切换尚不认证。直接且未重定义的 `endinput` 只执行当前行，随后忽略的括号不影响调用文件的分组；命名／转发的终止调用继续保护。作用域深度最多 128，沿用 32 MiB 分析预算。

静态 `assignrefcontextcats[options]{categories}` 及其星号形式也支持 deferred 数字提示，包括静态命名调用和前言中已执行的纯转发声明。选项沿用条目分配的字面 `sorting`／`labelprefix` 认证。每个分类的分配值按数字分区保存，分类的迭代顺序则全局保存；再次分配同名分类会将它移到最后。每次引用取最后一个匹配分类的有效分配，同一分类中强分配优先于弱分配，不同分类仍按顺序覆盖。最后再应用条目分配，因此条目弱分配也可覆盖分类强分配。分类名 `*` 是字面名称，不是条目通配。

成员需由当前源码中可确认的 `DeclareBibliographyCategory`／`addtocategory` 建立，并与所选输出目录、jobName 及递归 AUX 中的全部分类成员一致。分类成员为全局集合，且原生会在正文前从 AUX 预载，所以较晚添加的成员也可影响较早的引用。未保存成员变化在 AUX 更新前、宏正文／回调／捕获／未知参数中的潜在更新、重定义、数据库生成代码、源码集合及未认证的 IDS 均继续跳过相关提示。缺失、畸形或不匹配的分类记录不借用默认编号；它们不影响独立的条目分配提示。关键词自动分配按下述 BBL 成员独立认证。

```sh
cargo test --locked --lib automatic_categories_
cargo test --locked --lib native_automatic_category_hints_ -- --ignored --test-threads=1
cargo test --locked --test protocol automatic_category_hints_
```

静态 `assignrefcontextkeyws[options]{keywords}` 及其星号形式支持普通资源的 deferred 数字提示，包括静态命名调用和前言中已执行的纯转发。关键词成员只取完整认证 BBL 的 `keyw` 记录，按数字分区累积全部加载上下文的成员；不直接推断原始数据库字段，因此 Biber 已处理的字符串及 XDATA 继承沿用真实输出。关键词使用独立的全局追加顺序和逐分区强／弱映射，先应用关键词，再应用分类，最后应用条目分配；显式上下文内仍跳过弱分配。

关键词认证限于无展开、无嵌套分组的字面列表，支持 ASCII、Unicode 字母／数字及基本组合重音（U+0300–U+036F），包括中文和希腊文。保留源码及实际 BBL 的原始拼写、大小写和组合形式，不自动做 NFC 归一化；例如 Biber 将成员输出为 `café` 时，源码分解形式 `café` 不会命中同一个分配。pdfLaTeX、XeLaTeX 和 LuaLaTeX 均使用该字面身份规则。

内部普通空格、制表符及换行按 TeX 归一化，注释去掉当前行剩余内容和换行；项目前的空格忽略，末尾空格保留，`*` 是字面关键词。空列表不改变分配。分配自身不加载条目，也不产生提示。未知代码、空行产生的段落、Unicode 空白／控制字符及未认证符号、未认证的源码集合／IDS 和不完整 BBL 继续省略相关提示。关键词索引的展开工作沿用 32 MiB 上限；复杂关键词数据或索引超限不影响独立条目分配中已经认证的编号。

```sh
cargo test --locked --lib bib_context_tests::keywords::
cargo test --locked --lib native_automatic_keywords_ -- --ignored --test-threads=1
cargo test --locked --test protocol automatic_keyword_hints_
```

条目分配按实际执行顺序更新各数字分区的全局状态，TeX 花括号分组结束不会撤销它；同一键在分配前后可显示不同标签。通配 `*` 优先于单键分配，空列表不改变状态。原生列表忽略项目前的空白，但保留末尾空白；`a ` 或 `* ` 不会被当成静态键 `a` 或通配 `*`，分类名同理。提示只显示在真正输出的引用后，分配命令自身不加载条目，也不产生提示。重复输入和共享主文档仍要求同一物理引用的所有执行取得相同完整标签。

已分配的引用使用完整 AUX `(分区, 键, 上下文)` 编号，并联合严格加载的 BBL 排序及真实前缀认证；对应记录缺失或冲突时省略该处提示，不借用默认上下文编号。未写 `labelprefix` 与显式 `labelprefix={}` 分别认证原生的空上下文分量和空字符串摘要；后者允许 Biber 省略空前缀字段，其他未知摘要仍要求真实字段。该显示分析不放宽源码集合或完整改名的认证边界。

```sh
cargo test --locked --lib bib_context_tests:: -- --include-ignored
cargo test --locked --test protocol context_assignment_hints_
```

静态 `csuse`／`csname` 条目分配及零参数纯转发命令支持同样的强弱数字提示。转发声明必须由未重定义的原生声明命令在全局前言中实际执行；支持跨前言输入与最多 64 条已执行转发边，外层声明可以早于内层声明。分组内、文档正文内、延迟执行、条件提供和捕获的命令副本继续保护；命名／转发的上下文及分组控制仍不认证。未保存的提供文件、共享根和编译记录变更会重新检查提示。
