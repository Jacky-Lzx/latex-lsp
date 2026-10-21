# biblatex 引用

[文档目录](README.md) · [文献键与重命名](bibliography.md) · [编译诊断](diagnostics.md)

以下命令共用文献键补全、悬停、导航、引用查找、高亮和跨文件重命名，并提供命令片段与参数提示。编译中的缺失文献警告定位到键名，支持拼写修复；说明文字、同名标签、正文、注释和逐字文本会保留。

- [单次引用](#单次引用)
- [多组引用](#多组引用)
- [卷号引用](#卷号引用)

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

