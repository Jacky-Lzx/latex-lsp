# 文献

[文档目录](README.md) · [编辑功能](editing.md) · [编译诊断](diagnostics.md)

文献键支持补全、悬停、定义跳转、引用查找、高亮和跨文件重命名。直接调用与静态命名调用遵循相同的项目命令意义；被重定义引用的普通参数保留，参数数据或共享来源歧义会阻止部分重命名。编辑只使用可确认的静态源码范围，保留同名正文、说明文字、注释和逐字文本。

- [文献键与列表](#文献键与列表)
- [文献大纲](#文献大纲)
- [条目类型与字段补全](completion.md#bibtex-条目与字段)
- [引用命令](#引用命令)
- [biblatex 文献资源](biblatex-resources.md)
- [biblatex 引用](biblatex.md)
- [biblatex 文献分类](bibliography-categories.md)
- [手写文献](#手写文献)
- [条目关系与重命名](#条目关系与重命名)
- [字符串缩写](#字符串缩写)
- [URL 与 DOI 链接](#url-与-doi-链接)

## 文献键与列表

文献悬停、引用补全和条目列表显示 BibTeX 条目的标题、作者与年份。预览折叠空白，并按字符限制长度；保留 TeX 源文字，不展开字符串宏、拼接表达式或继承字段，重复字段不作推断。

文献键支持普通圆括号，包括不配对括号，例如 `book(old)`、`book(old` 和 `book)old`。`\bibitem`、引用命令及两种条目定界形式的 BibTeX 数据库统一支持补全、跳转、语义标记和重命名；跨条目引用与元数据使用完整键，编辑保留注释及正文。

文献引用的键列表也支持跨行补全，包括普通 `cite`、natbib 和 biblatex 的已支持命令；只替换当前键，保留其他文献键、说明参数、换行和列表空白。

键名前后的 TeX 注释会保留，注释里的逗号和括号不参与键名解析。宏展开、嵌套分组和由注释拼接的非连续键名不作静态推断。

BibTeX 条目键后的逗号可以另起一行；支持 LF、CRLF 和缩进，导航与重命名保留键后的原始空白及元数据。

BibTeX 的 `@` 与条目类型之间也允许普通空白、换行和缩进，例如 `@ book{key, ...}`；特殊条目和字段内容仍按完整条目边界处理。

BibTeX 自定义条目类型支持数字后缀、下划线、连字符等合法标识符，例如 `@custom1` 和 `@custom_type`，参与索引、元数据和重命名。编译器关于未知样式类型的标准两行警告会定位到对应 `.bib` 条目的类型名称；使用明确报告的数据库、键名与行号，类型与键分行也能定位，详见[条目类型诊断](bibliography-diagnostics.md#未定义条目类型)。

## 文献大纲

文献条目的大纲范围覆盖从 `@` 到正文结束的整个构造，选择范围仍是文献键；`@string` 缩写的范围覆盖完整赋值，选择范围是缩写名。条目继续显示标题、作者、年份预览，保留自定义类型及键中的 Unicode、引号和圆括号。

支持层级符号的客户端在条目下显示字段，字段名保留原始大小写，选择后可直接导航到该字段。字段范围覆盖名称、赋值和原始值表达式，支持多行字面量、嵌套分组、宏及 `#` 拼接；重复字段分别列出，注释和字面值内部的字段样式文本不会成为额外符号。`@comment` 和 `@preamble` 的内容不提供条目或字段符号。

编辑未完成的正文、赋值或字段名时，大纲使用解析器当前确认的范围；尚未闭合的构造仍可显示大纲，折叠继续要求完整构造。未命名文档和通过 `bib` / `bibtex` 语言类型识别的其他扩展名文件同样支持大纲，增量编辑立即更新条目及字段。

静态 `ids` 别名作为键符号列在所属字段下，显示所属主键，选择范围精确覆盖别名本身。重复别名分别保留；宏、拼接、重复字段及其他无法确认的声明不生成推测符号。

旧式客户端得到平铺符号，字段的 `containerName` 指向所属文献键，别名的容器是原始 `ids` 字段名；跳转范围保留实际名称。工作区符号搜索支持文献键、静态别名和字符串缩写，字段名本身不参与搜索。增量编辑即时更新别名与范围，关闭缓冲区后恢复磁盘内容。

无字段的花括号条目 `@misc{key}` 也参与大纲、文献键导航及重命名，键后允许换行再闭合。BibTeX 的圆括号条目需要逗号，空字段形式应写作 `@misc(key,)`。

## 引用命令

普通 `cite`、`nocite`、natbib 和已支持的 biblatex 命令共用文献键索引。

传统 LaTeX／BibTeX 的平面 `bibcite` 编译记录可在输出引用旁显示文献标签，支持独立开关、请求范围与辅助文件更新；结构化 natbib 和 biblatex 专用记录暂不推断。配置和边界见[辅助文件引用](auxiliary-references.md)。

natbib 的 `citealt` / `citealp`、`citenum`、`citeyearpar`、`citefullauthor` 及 `Citet` / `Citep` / `Citealt` / `Citealp` / `Citeauthor` 支持文献键补全、导航、引用、语义高亮、安全重命名、编译警告定位及拼写修复，并提供命令片段与参数提示。识别支持的星号形式及星号前后的空白和注释，保留引用列表、注释参数和正文。

biblatex 的单次、多组和卷号引用及其星号形式见 [biblatex 引用指南](biblatex.md)，包括标题、日期、URL、脚注和仅记录文献键的说明引用。

`\entrydata{key}{code}` 及其星号形式支持单个静态文献键的导航、补全和重命名，保留执行代码中的同名文字；详见[条目数据上下文](biblatex.md#条目数据上下文)。

`\ifentryexists{key}{true}{false}` 和 `\ifentryinbib{key}{true}{false}` 的单键检查同样参与文献编辑，保留两个分支中的同名文字；详见[条目条件检查](biblatex.md#条目条件检查)。

`bibentry` 宏包的 `\bibentry{key}` 正文内文献引用支持导航、悬停、引用查找、补全、参数提示、语义高亮和跨文件重命名；实际编译的缺失文献警告可定位到键名并提供拼写修复。按单个文献键处理，保留注释、条目正文和同名标签；不把键列表、动态名称或不支持的星号、可选说明当作可编辑键。

接受可选说明的普通引用命令最多识别两个说明参数；出现多余说明时，后面的正文不会参与文献键补全、导航、重命名或精确编译警告定位。说明参数按 TeX 规则在首个未受花括号保护的 `]` 处结束；文字中的 `[`、受花括号保护的方括号、空白和注释仍会保留，参数提示也遵循相同边界。

natbib 的 `defcitealias{键}{显示文字}` 使用单个静态文献键，`citetalias` / `citepalias` 引用相同的键。重命名同步维护别名设置与引用，保留键前后的注释及显示文字；逗号列表、含语义空白、宏或由注释拼接的动态键不作推断。别名设置本身不产生缺失文献键警告，不会领取同一行其他引用的编译诊断。

## 手写文献

手写文献 `\bibitem[label]{key}` 提示可选显示标签与必选文献键。最多读取一个可选标签，在首个未受花括号保护的 `]` 处结束，保留其中的字面量 `[`；键后的正文、多余或未闭合标签及不支持的星号不会触发文献键编辑或提示。

声明识别键前后的注释，引用可导航到实际键名并参与重命名、语义高亮、重复定义检查和拼写修复。编辑保留可选文献标签和条目正文，跳过需要宏展开或含语义空白的声明。

## 条目关系与重命名

`\defbibentryset{set key}{member keys}` 支持动态集合键声明及成员引用的导航、补全和独立重命名；详见[文献集合](bibliography-sets.md)。集合键与数据库条目及别名共用文献命名空间，存在冲突时拒绝歧义编辑。

biblatex 的静态 `ids` 字段可为条目声明多个文献别名，例如 `@book{main,ids={old,legacy},title={Example}}`。在 `\cite{old}` 或关系字段中可补全、跳转到别名声明、查看所属主键、查找引用、高亮并重命名。别名与主键独立：重命名 `old` 只更新该别名及其引用，重命名 `main` 保留 `ids` 和别名引用。别名声明也支持语义高亮和选择范围。

`ids` 字段名按 ASCII 忽略大小写，接受完整条目中唯一的花括号或双引号字面列表，保留逗号两侧空白。列表使用默认逗号分隔，不读取 Biber 自定义 `xsvsep` 配置。字符串宏、拼接、嵌套分组、TeX 命令、重复字段及未闭合条目不生成推测别名。重复别名或别名与实际主键冲突时可查看全部定义，重命名会拒绝歧义；含双引号的新别名会把双引号列表转为花括号。编译诊断的主键定位继续使用数据库实际条目。

字段名补全和悬停说明包含 `ids`，无需先在项目中使用该字段。也提供 `ismn`、`label`、`options`、`presort`、`shorthand`、`shorteditor`、`sortname` 和 `sorttitle` 的内置提示；这些提示不作为数据模型校验规则。

后记、批注、评论、前言、引言作者、专利持有人，原版语言、出版地、出版社，以及标准 `namea`–`namec` / `lista`–`listf` 槽位也提供说明；这些描述与 biblatex 数据访问命令的对应名称补全和悬停共用。

文献条目间引用支持 `crossref`、`xref`、`xdata`、`related` 和 `entryset`，包含花括号或双引号形式，以及后三类字段的逗号分隔列表；可补全、跳转、查找引用、高亮和重命名父条目。字段名不区分大小写，补全也支持尚未闭合的值。字符串宏、`#` 拼接或含 TeX 命令的值需要求值，暂不自动修改；标题和注释中的同名文本会保留。文献键不能重命名为 `*`，以免改变 `\nocite{*}` 的全选含义。

项目按 `refsection` 选择资源时，数据库中的条目主键及静态 `ids` 声明跳转到自身，悬停显示该声明的元数据。关系字段只在所有包含当前库的分区及共享项目中指向同一声明时提供目标；同名父条目分属默认库与选中库时分别导航，不把两库合并。有效资源中的真实重复声明保留多个目标且不显示任意预览；未知执行路径、未闭合条目、重复关系字段和索引不完整时不猜测目标。引用查找与高亮使用完整静态身份图，覆盖 TeX 引用、五类关系字段及所有共享消费者；主键和 IDS 各自保持独立身份，重复输入的同一源码范围只返回一次。尚未保存的新根同样参与核验，同一源码绑定冲突、延迟代码或分析不完整时不返回部分结果。可认证的声明支持准备改名和完整、版本化的工作区编辑；只更改该主键或 IDS 的完整引用，保留其他分区中的同名声明。改名拒绝同区主键或别名冲突，也检查新名称是否捕获已有 TeX 引用及关系字段；带引号的新键会合并整值编辑并转换为花括号。

关系字段补全也按这些分区和共享消费者过滤：只建议在每个相关资源集合中都对应同一声明的键，排除默认库独有的键和有冲突的候选。支持未闭合条目及字面值，替换完整当前列表项并保留相邻成员、空白和换行；选中带引号的键时保留原有的整值花括号转换。`xdata` 仅建议 `@xdata` 条目及其静态 IDS 别名，别名的类型来自所属条目，所属主键也必须唯一并在各分区保持一致；普通书籍、手写文献和源码集合不会被当作 XDATA 数据源。类型过滤与分区身份核验同时生效，未保存的条目类型修改会立即刷新候选。普通项目中的静态命名集合覆盖也按执行顺序核验；执行、消费者或完整索引无法确认时，不回退到数据库中的旧类型。含实际源码集合的 XDATA 补全继续沿用 IDS、动态执行等源码命名空间保护。

动态集合可能替换数据库中同名条目。含资源选择的分区项目中，完整静态 `defbibentryset` 的声明、成员和 TeX 引用支持按数字分区及执行顺序跳转和悬停。相同资源列表的新分区仍有独立集合；前言定义属于分区 0，分组内的定义在该分区持续生效。先显式引用再定义集合时，继续指向原先的数据库条目；单独的 `nocite{*}` 不锁定该条目。普通及静态命名调用遵循相同规则，重复输入和全部共享消费者必须给出一致目标。

关系字段也支持跳转和悬停到有效的源码集合。五类关系均先核验完整条目、唯一字面字段及数据库父键，再比较该数据库在全部数字分区与共享消费者中的目标；同一数据库在两个分区分别指向集合和书籍时，不返回任意目标。显式早引用锁定书籍时，关系继续显示书籍元数据。

关系字段补全比较全部数字分区及消费者中的候选声明；源码集合覆盖同名书籍时显示集合元数据，早显式请求锁定书籍时保留书籍信息。静态命名集合也提供自身的候选元数据，同一物理声明的重复执行合并为一个候选。`crossref`／`xref` 的源码集合候选还须有一致的底层数据库父条目；仅有源码声明的集合不能供 Biber 数据库关系使用。活动源码集合不作为 `xdata`、默认 `related` 或嵌套 `entryset` 的候选，避免类型错误、缺失集合克隆或成员关系损坏。不同分区的声明身份冲突时排除该候选，公共条目继续可用；未完成关系值、完整后缀替换及列表和引号编辑规则保持可用。

完整引用和文档高亮按源码 URI、范围及数字分区区分集合、成员和同名数据库条目，覆盖全部已保存与未保存消费者及数据库关系。重复执行同一物理声明只返回一处声明；先引用再定义时，未使用的集合只返回自身，书籍引用保持独立。共享物理引用在不同分区或消费者中绑定冲突时，不返回部分图；仅有源码集合而缺少数据库父键的关系也不认证为完整图。

完整身份得到认证后，独立源码集合和数据库成员支持准备改名与版本化工作区编辑，只更新该身份的声明和引用。新名称须在全部相关数字分区中没有主键、别名或源码声明冲突，也不能捕获已有请求、集合成员或数据库关系；互不相交的数字分区可保留同名声明。静态命名、重复输入、未保存消费者及引号值整组编辑沿用相同校验。

涉及 `crossref`／`xref` 的源码集合在双重依赖得到认证后支持完整改名：同名数据库父键必须唯一，原始库的全部消费者及每个相关数字分区都必须由同一集合覆盖，编辑同时更新集合、引用、关系值与隐含父条目主键。独立书籍消费者、其他集合身份、未知消费者及 `xdata`／`related`／`entryset` 关系继续拒绝。早请求锁定书籍后未使用的源码声明也不改名，避免使 Biber 新增集合及成员；`nocite{*}` 下覆盖同名数据库条目的集合不改名，避免暴露原先被覆盖的书籍。被活动集合覆盖的数据库主键同样不进行部分编辑。

TeX 引用和集合成员补全按实际输入位置及全部消费者认证候选，保留元数据搜索、完整后缀替换、Unicode、静态命名及当前未闭合字面值。接受较早的引用候选可能把该名称锁定为书籍，补全显示的是接受后实际使用的书籍或集合元数据；同一物理引用的重复执行也采用首次请求身份。仅有源码声明的集合可作为 TeX 引用候选，集合成员不建议其他源码集合，避免形成未支持的嵌套关系。候选在不同数字分区或共享消费者中绑定冲突时排除该名称，仍保留身份一致的公共条目；未保存提供文件和新消费者立即刷新结果，达到配置上限时报告未完整列表。

可认证源码集合项目的 TeX 引用和活动集合成员支持缺失引用警告与拼写修复；全部数字执行及已保存、未保存消费者必须一致。早期源码-only 请求即使随后定义同名集合仍缺失，已有数据库书籍则保留书籍身份。未被早期请求压制的集合会处理成员，即使尚未显式引用；被压制声明的成员不误报警告。修复按接受编辑后的请求位置认证源码集合及数据库候选，不受补全数量上限影响，编译器确认的引用警告沿用相同过滤并避免重复报告。

可认证的源码集合项目支持互不相交的静态 IDS：别名及其所属主键均须唯一，且不与任一已跟踪的源码集合名称相交。完整字面 IDS 列表通过全部已加载资源核验后，集合、成员、别名和数据库关系支持精确导航、独立改名、补全及缺失引用修复；别名预览附带所属主键的元数据，XDATA 别名继续按所属类型过滤。未保存的提供文件和新消费者参与认证，关闭后恢复磁盘上下文。非字面、重复、不完整或有冲突的 IDS、`@set` 所属别名以及别名名称与集合名称交叉的情况继续保护；不会因未生成静态别名就把 `ids=宏名` 当成安全命名空间。

唯一静态 IDS 的所属主键被源码集合覆盖时，普通完整项目与显式资源分区均支持只读定义跳转及悬停。引用别名仍跳转到 IDS 声明，预览显示所属关系，避免展示已被集合覆盖的原书籍信息；早期别名请求不会像早期主键请求那样锁定书籍身份。完整引用查找、高亮、别名自身改名及缺失键诊断共用全部消费者的完整身份图；引用键候选预览所属关系；候选字段另外验证实际加载后的类型，避免把别名间接用作动态集合成员。源码所属主键改名同时移动唯一数据库主键，即使没有关系字段直接引用该主键；全部原始库消费者和数字分区必须一致绑定当前集合。早期主键请求或独立书籍消费者继续拒绝这类改名。把别名间接用作另一个动态集合的成员、别名名称本身与集合相交，以及相交的 XDATA／`@set` 所属键继续保护。

完整普通项目也按执行顺序认证源码集合，无须使用 refsection 或切换资源。早期请求锁定数据库书籍或缺失身份，晚期请求绑定已声明集合；导航、补全、引用图、缺失诊断和改名使用同一规则，输入文件及未保存消费者参与认证。只有前言、尚无文档正文的片段保留原有编辑行为。移走早期书籍键可能激活被压制的同名集合并新增成员，因此这种准备改名和改名会拒绝。无法解析的命名调用也视为潜在源码声明，不回退同名数据库并集；字面示例和不相关的静态调用保持既有行为。

动态或延迟执行、同一数字分区中不同位置的同名集合声明、嵌套集合、含未认证 IDS、可执行 `@preamble` 或条目 `execute` 字段的资源、手写 `bibitem`、未认证的数据查询以及分析不完整时不猜测目标；显式早引用与通配加载混用的情况也暂不认证。数据库声明自身仍可查看，资源标签及字面示例保持独立处理。数据库 `@preamble` 及条目 `execute` 字段可通过生成的 BBL 注入集合。包含这些记录的 biblatex 项目会撤下数据库模型的引用身份认证、键候选和完整键改名，即使没有显式引用分区或源码集合声明；静态命名、纯转发包加载及传统 `bibliography` 资源声明同样纳入认证。同时抑制无法证明的缺失文献诊断。数据库声明自身仍可查看，传统 BibTeX、独立数据库草稿和无关项目保持原有行为。未保存数据库的新增、移除及关闭均重新认证。

文献键允许字面双引号，例如 `@misc{O"Connor}` 与 `\cite{O"Connor}`。普通引用、natbib 别名、biblatex 多组/卷号/数据引用、分类分配及手写文献共用这一规则，支持完整键的导航、补全、语义高亮、重命名和缺失引用修复。补全或重命名把带双引号的键写入双引号字段时，会把整个值转为花括号，例如 `crossref="old"` 改为 `crossref={O"Connor}`；保留列表其他成员、空白和字段外注释，同一列表的多处重命名合并为一个编辑。多行字段补全使用请求行上的主编辑和互不重叠的附加编辑，保留原有换行。含宏、嵌套分组、语义空白或由注释拼接的键仍不作推断。

BibTeX 编译报告无效 `crossref` 时，错误定位到子条目的字面字段值，并链接实际读取数据库中的父条目。相关位置会提示前置且未显式引用的父条目可能被 BibTeX 跳过，详见 [crossref 编译诊断](bibliography-diagnostics.md#无效的-crossref)。

编译器报告嵌套 `crossref` 时，警告同样定位到字段值，并链接引用链中的父条目。自引用和循环保持编译器的警告级别，缺失上级条目的错误单独显示；详见[嵌套引用诊断](bibliography-diagnostics.md#嵌套的-crossref)。

Biber 报告普通或 XDATA 循环继承时，诊断关联所报告的两个条目，并在可确认时标记具体 `crossref` 目标；详见[循环继承诊断](bibliography-diagnostics.md#biber-循环继承)。

Biber 报告集合中多余的 `crossref`、成员中多余的 `entryset` 或集合缺少 `entryset` 时，诊断分别定位到原始字段名或条目键；详见[文献集合诊断](bibliography-diagnostics.md#biber-文献集合)。

共享文献库的重命名覆盖使用它的各个主文档；其他项目中的同名条目保持独立。存在定义歧义、名称冲突或共享引用含义不一致时拒绝编辑。支持版本化工作区编辑；具体规则见[符号与重命名](symbols.md)。

## 字符串缩写

`@string` 声明及字段值、其他字符串声明和 `@preamble` 中的缩写引用支持补全、定义跳转、悬停、引用查找、高亮与跨文件重命名。名称按 BibTeX 规则比较，ASCII 大小写不敏感；文献键继续区分大小写，两个命名空间互不影响。

```bibtex
@string{Journal = {Journal of Examples}}
@string{FullTitle = Journal # {, volume } # 2026}
@book{example,
  title = FullTitle,
  publisher = JOURNAL,
  year = 2026,
  month = jan
}
```

在 `publisher = JO` 中选择 `Journal` 会替换完整缩写名，也支持 `=` 后和 `#` 后的空值位置。候选采用纯文本，保留拼接符号、字面值、注释和换行。常见的 `jan` 至 `dec` 提供补全及月份说明；实际展开由文献样式决定。

声明显示为常量文档符号，声明和裸引用均提供变量语义高亮。悬停预览原始赋值表达式并限制长度，不执行字符串展开。数字、花括号和双引号内的文本、注释及 `@comment` 正文不会被当作缩写引用；两种条目定界符、UTF-16、CRLF、未完成表达式和未保存编辑均使用同一索引。

重命名复用项目隔离和共享文献库的检查，支持版本化编辑及只改变名称大小写。多个大小写等价声明、共享引用在其他论文中指向不同或缺失定义，以及新名称会捕获已有引用时，拒绝修改。月份名作为常用缩写保留，不提供重命名。编辑导航不解析 `.bst` 中的宏定义或推断跨数据库执行顺序；无法解析的多 token 表达式不生成缩写引用。

编译发现未定义缩写时，提供精确警告及最多 5 个拼写建议；修复按日志中的数据库读取顺序，只建议此前已生效的有效声明。输入变化或定位有歧义时不提供编辑，详见[未定义字符串缩写诊断](bibliography-diagnostics.md#未定义字符串缩写)。

验证：

```sh
cargo test --locked --test bibliography_strings --test semantic
cargo test --locked --test protocol bibliography_abbreviations_
cargo test --locked --test bibliography_strings real_bibtex -- --include-ignored
```

真实 BibTeX 回归比较重命名前后 `.bbl` 的完整内容，覆盖跨数据库拼接、数字、大小写和 preamble；另以自定义样式验证自引用的重命名保留原有结果。

## URL 与 DOI 链接

BibTeX 的 `url` 和 `doi` 字段提供标准文档链接，编辑器可直接打开目标；已确认的链接值同时提供字符串语义高亮。支持大小写不敏感的字段名、花括号或双引号值，以及两种条目定界符。

```bibtex
@article{example,
  title = {An example paper},
  url = {https://example.org/paper?q=one%20two#abstract},
  doi = {10.1234/example}
}
```

`url` 支持 `http`、`https`、`mailto`、`ftp` 和 `file` 的绝对 URI。`doi` 支持裸标识符、可选的 `doi:` 前缀，以及 `doi.org` / `dx.doi.org` 的 HTTP(S) 解析网址。裸 DOI 转为 HTTPS 解析链接，只检查名称结构，不联网查询注册状态；数字注册代码可包含分段。

裸 DOI 中的 `#`、`?`、`%` 和 Unicode 按标识符内容编码，例如 `10.1234/a?b#c` 打开 `https://doi.org/10.1234/a%3Fb%23c`，不会把后半段解释为查询或页面片段。包含独立 `.` / `..` 路径段的 DOI 会编码斜线，避免浏览器归一化改变标识符。显式解析网址则保留其 URI 编码、查询参数及片段。

链接范围只覆盖值的实际内容，保留分组、引号和外围空白；兼容中文、emoji 和 CRLF。支持 `\_`、`\%`、`\#`、`\&`、`\$` 五种字面转义。动态宏、包装命令、字符串宏、`#` 拼接表达式、嵌套分组和内部空白不生成链接；标题、注释及 `@comment` / `@string` / `@preamble` 中的示例文字不参与链接索引。同一条目中的重复字段不生成该字段的链接，其他字段和条目继续正常工作。

只有闭合的字段值生成链接；编辑尚未闭合的条目时，已完成的字段仍可打开。独立或共享文献库均无需选定根文档，也无需编译。未保存的增量编辑立即更新链接和高亮；关闭缓冲区及磁盘文件变化后使用当前内容。

验证命令：

```sh
cargo test --locked --test bibliography_links --test semantic --test navigation
cargo test --locked --test protocol bibliography_links_
```

XDATA 候选类型及原生继承回归：

```sh
cargo test --locked --test bibliography_xdata_completion -- --include-ignored
cargo test --locked --test protocol xdata_relation_completions_
```

源码集合中的独立 IDS 回归：

```sh
cargo test --locked --test biblatex_source_aliases -- --include-ignored
cargo test --locked --test protocol source_set_disjoint_aliases_
```

普通完整项目的执行身份回归：

```sh
cargo test --locked --test biblatex_source_projects -- --include-ignored
cargo test --locked --test protocol source_set_default_
```

源码覆盖 IDS 所属主键的只读身份回归：

```sh
cargo test --locked --test biblatex_source_alias_owners
cargo test --locked --test biblatex_source_alias_owners -- --include-ignored
cargo test --locked --test protocol source_overridden_ids_owners_
```

相交 IDS 的完整别名引用与改名回归：

```sh
cargo test --locked --test biblatex_source_alias_references -- --include-ignored
cargo test --locked --test protocol intersecting_ids_alias_edits_
```

源码 IDS 所属主键的耦合编辑及候选认证回归：

```sh
cargo test --locked --test biblatex_source_alias_owner_edits --test biblatex_source_alias_candidates -- --include-ignored
cargo test --locked --test protocol intersecting_ids_source_owner_
cargo test --locked --test protocol intersecting_ids_owner_candidates_
```

Biber 跨数字分区的全局 IDS 缓存回归：

```sh
cargo test --locked --test biblatex_alias_loading
cargo test --locked --test biblatex_alias_loading -- --include-ignored
cargo test --locked --test protocol global_biber_alias_loading_
```

原生测试既核验缓存导入，也验证新别名捕获以及新主键删除旧别名映射的反例；需安装 pdfLaTeX、biblatex 和 Biber。


数字分区首次加载及 IDS 条目缓存回归：

```sh
cargo test --locked --test biblatex_alias_owner_cache -- --include-ignored
cargo test --locked --test protocol cached_alias_owner_proofs_
```

同名 IDS 的数字分区只读身份回归：

```sh
cargo test --locked --test biblatex_numeric_alias_owners -- --include-ignored
cargo test --locked --test protocol numeric_ids_source_owners_
```

原生测试对照默认库书籍和选中库源码集合，并核验接受候选后的 BBL；所属主键编辑继续保护；下一阶段的独立别名编辑另有原生对照。

跨分区别名自身安全编辑回归：

```sh
cargo test --locked --test biblatex_numeric_alias_edits -- --include-ignored
cargo test --locked --test protocol numeric_ids_alias_edits_
```

原生测试比较两个分区改名前后的完整 BBL，归一化改变的别名后要求一致；协议核验两种编辑格式、全部受影响文件的版本、未保存缓存导入及关闭恢复。

主键改名后的 IDS 全局条目缓存回归：

```sh
cargo test --locked --test biblatex_primary_cache_rename -- --include-ignored
cargo test --locked --test protocol primary_cache_capture_renames_
```

原生测试核验两种强制错误改名都会在没有 Biber 警告时改变后续条目数据，也核验接受的安全同名改名保持 BBL；协议覆盖版本化编辑、未保存数据库、关闭恢复和耦合源码主键。

非加载型文献键的分区交互回归：

```sh
cargo test --locked --test biblatex_nonloading_section_keys -- --include-ignored
cargo test --locked --test protocol numeric_nonloading_keys_
```

覆盖普通与静态命名调用的完整引用及版本化编辑、未定义名称捕获、被覆盖的缓存所属关系和条件查询保护。原生验证非加载调用不会预读资源，并比较应用改名后的完整 BBL。

IDS 名称与源码集合同名的只读身份回归：

```sh
cargo test --locked --test biblatex_source_named_aliases -- --include-ignored
cargo test --locked --test protocol source_named_ids_reads_
```

原生覆盖默认／所选分区的早晚请求、同时存在所属主键与别名名称的两个集合、另一个别名及独立集合成员。协议验证精确引用、拒绝相交改名、未保存静态／字符串／拼接关系撤销认证和关闭恢复。原生反例确认关系宏展开也会在无警告时抑制集合。

共享 IDS 所属主键的耦合改名回归：

```sh
cargo test --locked --test biblatex_shared_alias_owner_rename -- --include-ignored
cargo test --locked --test protocol coupled_raw_ids_owners_
```

测试核验两个分区的完整 BBL 在改名归一化后一致、三个库经多个别名关联时的完整编辑、独立项目隔离及未保存消费者。协议精确验证两种工作区编辑格式、六处编辑、三个文件的版本与关闭恢复；普通引用结果仍只包含各自声明身份。

手写文献与 biblatex 后台命名空间回归：

```sh
cargo test --locked --test biblatex_manual_namespace -- --include-ignored
cargo test --locked --test protocol handwritten_items_keep_biblatex_
```

覆盖普通项目、数字分区、静态命名条目、完整数据库改名、候选及缺失诊断、未保存输入与关闭恢复。原生对照核验改名前后完整 BBL 一致，手写条目的 `bibcite` 保持不动；协议精确验证两种编辑格式、版本、UTF-16 位置及传统辅助记录不会误用于后台引用提示。

源码集合中的纯文本谓词查询支持 `ifentryinbib`／`ifentrycategory`（含静态命名调用）：按每个数字分区最终真实请求和声明解析身份，保留导航、完整引用和改名，但不预读数据库或发送 Biber 请求。集合声明自身可使集合加载，因此不要求存在显式引用。查询缺失键不产生引用警告；真正缺失的引用与集合成员仍检查。

共享查询的所有执行上下文必须绑定同一身份；重定义、可执行／动态分支、IDS 交叉语义和未经认证的内部命令执行上下文继续保护。谓词键补全按最终分区命名空间认证候选，普通引用和集合成员仍按实际请求位置计算。

```sh
cargo test --locked --test biblatex_source_textual_queries -- --include-ignored
cargo test --locked --test protocol source_textual_predicates_
```

直接且未重定义的 `mkbibparens`、`mkbibbrackets`、`parentext` 和 `brackettext` 会初始化 biblatex 内部命令，也支持以静态 `\csuse{名称}`／`\csname 名称\endcsname` 调用这些包装。在这些包装的完整分组参数内，纯文本代码体的 `entrydata`／`entrydata*` 支持源码集合的导航、悬停、补全、引用及完整改名，包括静态命名的 `entrydata` 调用。键按最终分区身份认证，查询不提前加载数据库书籍；早期真实引用仍保留书籍身份。包装自身也提供 `{text}` 签名，内部普通标签引用可以交互。

普通正文直接调用、包装之外的代码、复制／自定义转发包装、动态名称、命名接口或包装的重定义、不完整参数和可执行代码体继续保护。`parentext`／`brackettext` 转发的底层 `mkbibparens`／`mkbibbrackets` 被重定义时，同样撤下原生参数执行及初始化认证，保留另一种独立包装。数据库 `execute`／`@preamble`、共享消费者身份和新名称捕获沿用完整认证；未保存更新及关闭后重新计算。

```sh
cargo test --locked --test biblatex_source_entrydata -- --include-ignored
cargo test --locked --test protocol initialized_entrydata_
```

原生测试实际应用 LSP 返回的改名和补全编辑，比较四种包装的普通／星号调用在集合声明前查询时的完整 BBL，确认不会额外加载原数据库书籍。协议核验两种编辑格式、UTF-16／CRLF、主文件及共享查询的范围和版本、未保存重定义与执行代码、共享新根冲突和关闭恢复。

项目包含独立源码集合时，分属不同数字分区且共享 IDS 的真实数据库所属主键可以耦合改名：按实际资源分配检查冲突，完整收集经别名相连的所有所属条目及其引用。集合成员、通配加载和跨库关系继续参与检查；源码集合遮蔽所属主键、相交的别名、不同所属主键、未认证消费者或新名称捕获仍拒绝编辑。同一物理源码集合的多个跨库先决条目认证见下文。

```sh
cargo test --locked --test biblatex_source_shared_owners -- --include-ignored
cargo test --locked --test protocol source_shared_raw_owners_
```

同一物理源码集合声明在多个分区执行时，支持同时移动各分区的真实先决主键，条目可以部分或全部没有 IDS。每个原始条目的全部消费者都必须采用这同一集合身份，所有关联 IDS 的所属条目都须纳入编辑；先引用 IDS 不会虚构早期主键请求。只读引用仍保留源码身份，改名额外包含必需的数据库头部。最多 64 个先决条目；数据库引用只认证原有的 `crossref/xref` 父条目关系，实际编辑原子地移动所有必需头部及其关系键，保留 IDS 名称。独立书籍消费者、缺失主键、不同物理集合、早期主键请求、通配遮蔽、未认证的缓存返回、跨库缓存新名称及全项目名称捕获继续保护。单个先决条目沿用原有认证。

后续分区回到已解析的库时，最终源码集合可以替换从全局缓存导入的原始先决条目。显示身份按每个分区的实际集合声明认证；先引用 IDS 本身不会阻止最终集合，但先直接请求真实主键仍保留原始书籍。通配加载、同一分区中的不同物理声明、已被真实键删除的别名和含糊提供者继续保护。完整改名仍要求全部原始消费者采用同一物理集合身份；普通数据库图与新名称缓存模拟沿用原有归属检查，分析仍受 32 MiB 工作预算约束。

```sh
cargo test --locked --test biblatex_source_prerequisites -- --include-ignored
cargo test --locked --test protocol source_prerequisite_edits_
```

本地数字引用上下文的编译标签支持纯字面 `labelprefix`／`sorting` 和全局 deferred 编号。按 AUX 为键记录的默认上下文联结 BBL 真实前缀；同一键跨打印上下文仍遵循 biblatex 实际默认选择。共享物理引用的所有执行必须得到相同完整标签；无法认证的上下文分配、命名上下文声明、重定义、动态选项和自定义格式继续保护。详见[辅助引用与编号提示](auxiliary-references.md)。

源码集合项目中的 `ifentryinbib`／`ifentrycategory` 键补全支持直接及静态命名调用：候选采用分区最终的首次真实请求或集合声明，查询本身不会成为提前加载请求。尚未发生真实请求时，可以补全后续声明的集合；确有早期 `cite` 时仍显示原书籍的元数据。普通 `cite` 的候选继续使用光标处请求语义。

支持空键、完整后缀替换、既有注释及 UTF-16／CRLF 编辑。需要完整、纯文本的分支和静态分类名；未完成分支、可执行代码、重定义、动态键、内部文献执行上下文和 IDS 相交继续保护。共享物理查询的所有根及数字分区必须取得相同物理候选身份，无法确认的单个候选排除；预算不足不返回未经认证的候选。

```sh
cargo test --locked --test biblatex_source_predicate_completion -- --include-ignored
cargo test --locked --test protocol source_predicate_completions_
```

`\assignrefcontextentries[options]{keys}` 及其星号形式支持条目键列表的跳转、预览、补全、引用、高亮和完整改名，也识别静态 `csuse`／`csname` 调用及可认证的纯转发命令。方括号是必需参数，即使选项为空也需写作 `[]{keys}`；签名提示区分选项与键列表。改名只修改真实条目键，不改 `labelprefix` 等选项、普通文字和逐字内容。

此命令分配上下文而不加载或引用条目，因此缺失键及通配 `*` 不触发缺失引用诊断，也不产生引用编号提示。普通资源的数字分区复用非加载型键身份认证；源码集合中的上下文分配、捕获的命令副本和不明确的操作数调用继续拒绝部分改名。普通资源的直接、静态命名和已执行纯转发强／弱分配支持按执行顺序显示 deferred 数字标签，选项及编译记录边界见[辅助引用与编号提示](auxiliary-references.md)；弱分配按原生分组恢复上下文开关，分类分配另按当前源码与 AUX 的成员一致性认证数字提示；关键词分配另按实际加载 BBL 的成员认证数字提示。分类列表的独立命名空间和完整编辑详见[文献分类](bibliography-categories.md)。

`\assignrefcontextkeyws[options]{keywords}` 提供必需方括号、关键词列表及星号形式的参数提示和命令补全片段，也识别静态命名调用与可认证的纯转发。关键词与条目键、分类名分别处理，不提供跨命名空间的键导航或改名。数字提示的成员、空白及覆盖顺序见[辅助引用与编号提示](auxiliary-references.md)。
