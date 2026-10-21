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

重命名 `.bib` 条目时，会同步更新使用该数据库的分类分配和引用。第一个参数是分类名，即使与文献键同名也会保留；分类名本身暂不支持分类符号导航或重命名。共享数据库和歧义处理遵循[文献重命名规则](bibliography.md#条目关系与重命名)。

文献键编辑只识别第二个花括号参数中的静态键列表；不推断宏展开、嵌套键或由注释拼接的非连续键名。该命令不接受星号和可选说明，也不将后续第三个分组或正文当作文献键。

分类分配不会产生缺失文献键的引用警告，因此同一行真实引用的警告和拼写修复只定位到引用命令。
