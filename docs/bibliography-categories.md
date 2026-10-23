# biblatex 文献分类

[文档目录](README.md) · [文献](bibliography.md) · [biblatex 引用](biblatex.md)

使用 `\DeclareBibliographyCategory{category}` 声明分类，再通过 `\addtocategory{category}{keys}` 分配文献键。例如：

```tex
\documentclass{article}
\usepackage[backend=bibtex,style=numeric]{biblatex}
\addbibresource{refs.bib}
\DeclareBibliographyCategory{selected}
\addtocategory{selected}{alpha,beta}
\begin{document}
\nocite{alpha,beta}
\printbibliography[category=selected]
\end{document}
```

`refs.bib` 中需要有 `alpha` 和 `beta` 两个条目。分类分配本身不会引用条目；仍需通过 `\cite` 或 `\nocite` 将它们纳入参考文献。编译工具和配置见[编译指南](build.md)。

两个分类命令提供命令片段和参数提示。`\addtocategory` 的第二个参数支持文献键补全、悬停、定义跳转、引用查找、文档高亮、语义高亮及跨文件重命名。键列表支持跨行和注释，未闭合列表也可补全；编辑只替换当前键，保留其他键、注释和原始换行。

重命名 `.bib` 条目时，会同步更新使用该数据库的分类分配和引用。第一个参数是分类名，即使与文献键同名也会保留；分类名称使用独立符号索引，支持补全、悬浮、定义跳转、引用查找、文档高亮、准备重命名及跨文件重命名。共享数据库和歧义处理遵循[文献重命名规则](bibliography.md#条目关系与重命名)。

文献键编辑只识别第二个花括号参数中的静态键列表；不推断宏展开、嵌套键或由注释拼接的非连续键名。该命令不接受星号和可选说明，也不将后续第三个分组或正文当作文献键。

分类分配不会产生缺失文献键的引用警告，因此同一行真实引用的警告和拼写修复只定位到引用命令。

`\ifentrycategory{key}{category}{true}{false}` 的第一个参数支持单个文献键的补全、导航、悬停、引用、高亮和重命名；第二个参数使用分类名称。条目改名只更新键，分类改名只更新分类，保留分支中的普通文字。查询不产生普通缺失文献引用警告，不接受星号、可选说明或逗号键列表。


分类名称可用于 `DeclareBibliographyCategory` 声明、`addtocategory` 的第一个参数、`ifentrycategory` 的第二个参数和 `ifcategory` 的第一个参数，以及 `printbibliography` 的 `category` / `notcategory` 选项。静态 `defbibfilter` 布尔表达式中的 `category=name` 也参与编辑；保留过滤器名、其他筛选值、成立／不成立代码和标题中的同名文字。无需安装 biblatex 或运行编译即可使用这些源码功能。

普通调用、静态 `csuse` / `csname` 和唯一零参数纯转发别名使用当前项目的命令意义。完整宏正文中的潜在引用参与编辑；已知参数数据和 token 操作数中的潜在名称会阻止部分重命名，已确认的字面示例保留原文。命令重定义、未保存定义和独立项目分别处理；共享来源须在全部消费者中保持一致，且同一分类必须关联唯一声明。复制命令与有歧义的别名仅用于拒绝不完整的重命名，不据此提供可编辑调用。

选项中的单层花括号保护、名称周围的注释和跨行空白保持原样。补全会替换名称的整个剩余后缀，支持尚未闭合的参数，并停在随后的 TeX token 或空白之前。名称内部的空白、动态名称、由注释拼接的名称及不受支持的过滤器表达式不作展开；项目中存在这些分类用法时，拒绝分类重命名。新名称须为不含方括号的静态标识符，不能与同命名空间中的其他名称冲突。

单次索引仅覆盖相互关联的项目，限制为累计 32 MiB 源码、200000 个命令、8192 个声明、每个来源 20000 项及累计 100000 个名称解释。单个过滤器表达式最多读取 100000 个词法 token；超限、项目索引截断或消费者含义不一致时，不返回部分编辑。

验证：

```sh
cargo test --locked --test bibliography_categories
cargo test --locked --test protocol bibliography_category_names
cargo test --locked --test bibliography_categories real_biblatex_category -- --include-ignored
```

最后一项需要 pdfLaTeX、BibTeX 和 biblatex，验证改名后分类成员与筛选输出仍然有效。
