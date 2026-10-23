# 术语、缩略语与索引符号

术语和缩略语支持 `newglossaryentry`、`longnewglossaryentry`、`newacronym`、`newabbreviation`、`newacro`、`acrodef`、`DeclareAcronym` 的静态键。常见 `gls`、`glspl`、`glsentry...`、`acrshort`、`acrlong`、`ac` 系列调用提供键补全、声明跳转、引用、悬停、重命名、文档符号及语义高亮。`loadglsentries` 加载的源文件参与项目依赖和作用域。

术语键与标签、文献键分别处理。同名键不会互相重命名。未定义术语和重复声明产生即时诊断；可通过 `diagnostics.undefinedGlossary` 关闭未定义诊断。共享声明的重命名会覆盖各个使用项目，遇到声明歧义则拒绝。

`index` / `sindex` 的静态索引项支持相同的导航和编辑功能。名称取 `@` 或 `|` 前的排序文本，允许空格与 `!` 层级；重命名保留打印形式和封装命令。例如 `\index{Ada Lovelace@Lovelace|textbf}` 改名只替换 `Ada Lovelace`。每个索引项本身也是声明，跳转会列出该项的所有位置。

`makeindex[name=people]` 与 `newindex{people}...` 声明命名索引；`index[people]` 和 `printindex[people]` 提供独立的索引名补全、导航、重命名与未定义诊断。可通过 `diagnostics.undefinedIndexes` 关闭该诊断。

仅分析静态名称。注释、逐字代码、延迟执行的宏正文、动态键以及 MakeIndex 引号转义项不参与符号改写。不模拟自定义宏展开或运行术语表处理程序。
