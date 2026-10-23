# biblatex 引用

[文档目录](README.md) · [文献资源加载](biblatex-resources.md) · [文献键与重命名](bibliography.md) · [编译诊断](diagnostics.md)

文献工具与 LaTeX 的重运行请求也会进入[编译结果分析](build-analysis.md#文献处理与重编译)。

动态集合声明 `\defbibentryset{集合键}{成员列表}` 的独立键编辑见[文献集合](bibliography-sets.md)。

文献键共用补全、悬停、导航、引用查找、高亮和跨文件重命名；字段、列表和条目类型提供各自的名称补全与悬停。以下命令还提供命令片段与参数提示。引用命令的普通缺失文献警告定位到键名，支持拼写修复；说明文字、同名标签、正文、注释和逐字文本会保留。

- [字段、列表和姓名引用](#字段列表和姓名引用)
- [条目数据上下文](#条目数据上下文)
- [字段与列表访问](#字段与列表访问)
- [字段与列表条件](#字段与列表条件)
- [条目条件检查](#条目条件检查)
- [单次引用](#单次引用)
- [多组引用](#多组引用)
- [卷号引用](#卷号引用)

## 字段、列表和姓名引用

`citefield`、`citelist`、`citename` 分别打印文献条目的字段、字面列表和姓名列表。它们接受最多两个前置说明，之后是文献键列表、可选格式名和必选的字段或列表名：

```tex
\citefield[参见][12]{alpha,beta}[citefield]{title}
\citelist{alpha}{location}
\citename{alpha}[citename]{author}
```

文献键列表支持补全、悬停、导航、引用查找、高亮和跨文件重命名，包括跨行和未闭合列表；文献重命名保留格式名、字段名、列表名及说明中的同名文字。命令片段只插入两个必选参数，参数提示分别跟踪说明、键、格式和字段，也保留用户定义的命令签名。

最后一个参数还提供字段或列表名称补全与悬停说明：`citefield` 建议 `title`、`year`、`doi` 等输出字段，`citelist` 建议 `publisher`、`location` 等字面列表，`citename` 建议 `author`、`editor`、`translator` 等姓名列表。候选采用默认数据模型中的常用输出名称，不列出 `date`、`sortname` 等仅供输入处理的名称，也不校验自定义模型。

补全按 ASCII 忽略大小写筛选并插入标准小写名称，替换整个当前名称，保留说明、格式、注释及调用外文字；支持跨行和未完成参数。这些名称独立于文献键，即使数据库存在同名条目，也不会触发文献跳转或重命名。动态名称、多余说明、逐字文本和已被用户重定义的命令不提供内置选择器提示。

编译器可能在读完格式和字段后，甚至读取下一条命令时才报告缺失文献键。诊断会核对完整调用范围，将警告和拼写修复定位到实际键名；结果分析继续按键去重。三个命令均不推断星号或首字母大写形式，不把多余说明或动态键猜测为静态文献键。

## 条目数据上下文

`\entrydata{key}{code}` 和 `\entrydata*{key}{code}` 在指定条目的数据上下文中执行代码。首参数按单个静态文献键处理，支持补全、定义跳转、悬停、引用查找、高亮、语义 token、选择范围及跨文件重命名；第二参数中的普通文字保留，嵌套的已支持引用命令仍独立参与文献编辑。

```tex
\entrydata{alpha}{\printfield{title}}
\entrydata*{beta}{\printnames{author}}
```

参数提示区分 `{key}` 和 `{code}`，命令片段提供两个必选参数。支持星号前后的空白、注释及跨行键名，补全保留注释；已有参数时只补命令名。键允许 Unicode、引号及普通圆括号，不把逗号列表、语义空白、嵌套分组或宏展开猜测为单个键。此处不推断数据上下文的字段展开、引用跟踪或执行顺序，也不领取普通 `Citation … undefined` 警告的位置。

## 字段与列表访问

`printfield`、`printlist` 和 `printnames` 提供命令片段与参数提示：字段输出接受可选格式，字面列表和姓名列表还接受可选的 `[start-stop]` 范围。可选参数在首个未受花括号保护的 `]` 处结束，字面 `[` 不增加嵌套。片段只插入必选字段或列表名，已有参数保持原样；这些名称不作为文献键处理。

三种打印命令的最后一个参数共用上述字段、字面列表和姓名列表补全及悬停说明，格式与范围参数保持独立。

`\thefield{field}` 和 `\strfield{field}` 的字段读取也提供片段、参数提示、字段补全及悬停，不接受可选格式或星号。

## 字段与列表条件

`\iffieldundef{field}{true}{false}`、`\iflistundef{literal list}{true}{false}` 和 `\ifnameundef{name list}{true}{false}` 的首参数分别建议对应类型的名称，两个分支的普通文字保持原样；参数提示分别跟踪名称与两个分支。这些命令不接受可选格式或星号。

字段比较也支持片段与参数提示：`\iffieldequalstr{field}{string}{true}{false}` 比较字段和字符串，`\iffieldequals{field}{command}{true}{false}` 比较字段和命令内容，`\iffieldsequal{field 1}{field 2}{true}{false}` 比较两个字段。前两者只在首参数提供字段名称补全，后者在两个字段参数均提供；比较字符串及两个分支中的普通文字不作为字段名。`\iflistequals{literal list}{command}{true}{false}` 和 `\ifnameequals{name list}{command}{true}{false}` 则分别建议字面列表和姓名列表。这些命令不接受星号或可选格式，也不执行比较或预测分支结果。

三个 `equals` 命令的第二参数按宏名操作数处理：补全只插入命令名，不附加调用片段；被比较的 `\cite`、`\newcommand` 或逐字命令也不作为调用解析。分支中的真实命令保持独立。与其他命令名操作数相同，当前文件前文已显式重新定义比较命令时按普通调用处理；不会完整模拟跨文件重定义或分组恢复。

用户声明及其派生命令会撤下有冲突的内置选择器与参数提示，例如 `\newcounter{field}` 生成的 `\thefield` 或 `\newif\ifnameundef` 声明的条件命令；命令名补全也不再插入文献字段占位符，移除覆盖后恢复。其他独立项目中的声明不影响当前文档。

## 条目条件检查

`\ifentryexists{key}{true}{false}` 与 `\ifentryinbib{key}{true}{false}` 分别检查条目数据是否存在及条目是否列入文献列表。首参数支持单个静态文献键的导航、补全、悬停、引用查找、高亮、语义 token、选择范围和重命名；两个分支中的普通文字保留，嵌套引用独立索引。

`\ifentrycategory{key}{category}{true}{false}` 检查指定条目的分类，首参数使用相同的单键编辑规则；第二参数属于独立的[分类名称](bibliography-categories.md)命名空间，保留两个代码分支中的普通文字。参数提示分别标明全部四个参数。

参数提示区分键、成立代码和不成立代码，命令片段提供三个必选参数。键采用与 `entrydata` 相同的注释边界和静态校验，支持 Unicode、引号及圆括号，逗号列表和动态键不作推断。两个命令不接受星号和可选说明，不领取普通缺失文献引用警告；实际分支结果与执行顺序由 biblatex 决定。

`\ifentrytype{entry type}{true}{false}` 检查当前条目的输出类型。首参数提供常用 biblatex 输出类型的补全与悬停，例如 `article`、`book`、`online`、`report` 和 `thesis`，说明与 `.bib` 条目类型共用；补全只替换名称，保留注释、换行和两个分支，也支持未完成参数。片段插入三个必选参数，参数提示跟踪类型与两个分支。名称不作为文献键或字段名处理；星号、可选参数、动态名称和用户覆盖不提供内置类型提示。

类型候选采用默认数据模型中的常用输出名称，不列出 `comment`、`string`、`preamble`、仅供继承的 `xdata` 或 `conference`、`phdthesis` 等输入别名；不推断自定义数据模型及条目类型转换，也不预测条件结果。

## 单次引用

| 命令 | 首字母大写形式 | 用途 |
| --- | --- | --- |
| `cite` | `Cite` | 普通引用 |
| `parencite` | `Parencite` | 括号引用 |
| `footcite` | `Footcite` | 脚注引用 |
| `footcitetext` | `Footcitetext` | 无脚注标记的脚注文字 |
| `smartcite` | `Smartcite` | 按上下文使用正文或脚注引用 |
| `supercite` | 无 | 上标引用，需要支持它的样式 |
| `textcite` | `Textcite` | 叙述式引用 |
| `autocite` | `Autocite` | 按配置选择引用形式 |
| `citeauthor` | `Citeauthor` | 作者 |
| `citeyear` | 无 | 年份 |
| `citetitle` | `Citetitle` | 标题 |
| `citedate` | 无 | 日期 |
| `citeurl` | 无 | URL |
| `notecite` | `Notecite` | 输出说明并记录文献键 |
| `pnotecite` | `Pnotecite` | 括号内的说明引用 |
| `fnotecite` | `Fnotecite` | 脚注内的说明引用 |
| `fullcite` | 无 | 完整文献条目 |
| `footfullcite` | 无 | 脚注内的完整条目 |

表中命令均识别普通和星号形式；星号前后允许空白和注释。星号的排版含义由命令与样式决定；没有独立星号实现时，biblatex 沿用普通实现，例如 `fullcite*`、`footfullcite*`。`citetitle*` 使用完整标题，`citedate*` 包含日期消歧信息。大小写按实际命令识别，不推断 `Supercite`、`Citeurl` 或 `Citedate`。

单次引用接受最多两个方括号说明和一个花括号文献键列表：

```tex
\Parencite[参见][12]{alpha,beta}
\citetitle*{alpha}
\citeurl{alpha}
\notecite[另见][34]{beta}
\fullcite*{alpha}
```

说明与键即使同名，也只编辑键列表。补全支持跨行或尚未闭合的列表，只替换当前键；命令补全保留已有参数。参数提示区分前注、后注与键，用户定义的命令签名优先。

`notecite` 系列不打印引用正文，但仍读取文献键、参与引用跟踪并报告缺失键，因此也参与导航和重命名。`supercite` 的具体行为依赖引用样式；`numeric` 等样式忽略可选说明并产生警告，这些警告会保留。

## 多组引用

支持以下标准命令及星号形式：

| 命令 | 首字母大写形式 |
| --- | --- |
| `cites` | `Cites` |
| `parencites` | `Parencites` |
| `footcites` | `Footcites` |
| `footcitetexts` | `Footcitetexts` |
| `smartcites` | `Smartcites` |
| `textcites` | `Textcites` |
| `autocites` | `Autocites` |
| `supercites` | 无 |

调用可先给出最多两个圆括号整体说明，再重复“最多两个方括号说明 + 花括号文献键列表”：

```tex
\parencites(整体前注)(整体后注)
  [参见][12]{alpha,beta}
  [34]{gamma}\relax
```

每一组的键都参与导航、引用查找、补全、语义高亮和重命名。从后续组或 `.bib` 条目发起改名会更新所有确认的引用，保留整体说明、各组说明及其他键。补全也支持后续组中尚未闭合的键列表；参数提示分别标明整体说明和各组的当前参数。

圆括号和方括号说明在首个未受花括号保护的右括号处结束，同类左括号不增加嵌套。组间允许空白和注释；空段落、普通正文或 `\relax` 会结束引用序列。多余说明、未闭合说明和动态键不会被猜测为静态文献键。卷号形式见下节。

编译器可能在向前读取下一条命令后才报告缺失文献警告。诊断会核对整个调用及其向前读取的位置，将警告定位到匹配键；多个匹配位置分别提供诊断和拼写修复。

## 卷号引用

以下 biblatex 命令区分卷号、页码和文献键，支持表中的大小写形式及星号：

| 单次引用 | 重复引用 |
| --- | --- |
| `volcite` / `Volcite` | `volcites` / `Volcites` |
| `pvolcite` / `Pvolcite` | `pvolcites` / `Pvolcites` |
| `fvolcite` / `Fvolcite` | `fvolcites` / `Fvolcites` |
| `ftvolcite` / `Ftvolcite` | `ftvolcites` / `Ftvolcites` |
| `svolcite` / `Svolcite` | `svolcites` / `Svolcites` |
| `tvolcite` / `Tvolcite` | `tvolcites` / `Tvolcites` |
| `avolcite` / `Avolcite` | `avolcites` / `Avolcites` |

每组按“可选前注、必选卷号、可选页码、必选文献键列表”读取：

```tex
\volcite[参见]{2}[12--16]{alpha}
\volcites(整体前注)(整体后注)
  [参见]{1}[12]{alpha,beta}
  [另见]{2}[34]{gamma}\relax
```

卷号、页码和前注即使与键同名，也不会被文献重命名或拼写修复修改。各组的键支持导航、悬停、引用查找、补全、高亮和跨文件重命名；缺失文献警告精确定位到键，允许编译器报告下一条命令的行号。

单次命令只读取一组；重复命令允许最多两个圆括号整体说明，后续组须以方括号前注或花括号卷号开始。也识别已开始组中的裸 ASCII 卷号或单个宏 token，例如 `\volcite2{alpha}` 和 `\volcites{1}{alpha}[另见]2{beta}`；不展开卷号宏。裸 Unicode、`^^` 转义及无法确认的 token 边界不作推断，文献键使用花括号分组。

补全片段只插入卷号和文献键两个必选参数，保留已有调用和可选说明。参数提示分别跟踪卷号、页码和各组键；填写说明或卷号后尚未输入必选参数时，继续提示下一个必选参数。普通多组引用也支持说明之后的等待提示。



## 回调代码

常见的 `AtEveryBibitem`、`AtBeginBibliography`、`AtEveryCite`、`AtEveryCitekey`、`AtEveryMultiCite` 及下一次／当前引用回调提供代码片段和参数提示；代码中的静态标签、文献键和分类名称可正常补全、导航与改名，回调之后的调用保持独立。也支持参考文献分区、简写列表、驱动和卷号引用回调。

`AtDataInput[entry type]{code}` 的可选条目类型与 `AtBeginBiblist{bibliography list}{code}`／`AtEveryBiblistitem` 的首参数按选择器数据处理，其余参数作为代码。`AtEveryEntrykey{code}{success}{failure}` 的三个参数都保留代码语义；`AtUsedriver` 和 `AtVolcite` 的星号形式保留替换回调的原生行为。省略可选参数的补全不会改变默认值，已有调用的代码会原样保留。

`AtDataInput` 的可选类型提供与 `ifentrytype` 相同的常用输出类型补全和悬浮说明。补全替换完整名称，支持未完成头部、注释、CRLF 和 UTF-16，保留后续代码；静态命名和纯转发调用同样适用。参数数据、字面内容、未知捕获副本、用户覆盖和共享消费者含义不一致时不提供内置类型候选。项目索引不完整或累计分析超过 32 MiB、200000 个命令、8192 个声明时跳过。

静态命名调用与唯一纯转发别名使用相同的签名；用户重定义、传给其他宏的参数数据和共享消费者歧义继续受项目编辑保护。不模拟回调的执行次数、顺序或动态展开，不通过源码预测当前条目的数据。
