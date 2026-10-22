# 文件操作

[返回文档目录](README.md) · [项目与路径解析](projects.md)

支持文件及目录的创建、删除、重命名和移动。文件路径由编辑器修改，服务器负责维护源码引用与索引。

## 重命名与移动

`workspace/willRenameFiles` 更新输入文件、文献库、BibTeX 样式、本地宏包与文档类、图片、文件 URL 及 `% !TeX root` 路径。目录移动会重算内部相对路径，尽量保留省略扩展名的写法。图片搜索目录与同名文件遮蔽规则见[图片查找与移动](projects.md#图片查找与移动)。

`includeonly` 的章节列表也会同步更新，保留注释、列表分隔符、CRLF 及文件名内部空格；选择列表本身不会加载章节。缺失章节的 PDF 过期检查与拼写修复使用相同解析，遇到动态或不连续名称时保守处理。

输入源码改为其他扩展名时保留新后缀，例如 `\input{chapter}` 在 `.tex` 改为 `.ltx` 后更新为 `\input{chapter.ltx}`，根指令同步更新。以下加载方式要求目标保留对应后缀：

| 引用方式 | 必须保留的后缀 |
| --- | --- |
| `include` / `includeonly` | `.tex` |
| 宏包加载 | `.sty` |
| 文档类加载 | `.cls` |
| `bibliographystyle` | `.bst` |
| `bibliography` | `.bib` |

文件名的首部空格使用 `./` 保留，主干末尾空格通过显式后缀保留；会被 TeX 合并或忽略的空白不生成引用编辑，具体规则见[源码文件名与移动](source-filenames.md)。

共享源码中的引用要求各项目需要的编辑一致；含宏展开、无法确定的目标或无法安全表达的新路径会拒绝改写。显式配置的 `build.rootFile` 需在主文件改名后同步修改。

## 文件 URL

`\url`、`\href` 和四参数 `\hyperref` 中的 `file:` 链接会更新文件路径，同时保留源码中的查询参数与片段，包括其中的百分号编码、TeX 转义以及空的 `?` / `#` 分隔符。例如将 `manual.pdf` 改为 `guide.pdf` 时：

```tex
\href{file:///project/manual.pdf?download=1#page=3}{Manual}
```

更新为：

```tex
\href{file:///project/guide.pdf?download=1#page=3}{Manual}
```

四参数 `\hyperref{file:///project/manual.pdf?download=1}{chapter}{intro}{Chapter}` 只修改第一参数中的路径，保留类别、名称和文字；导航使用的 `#chapter.intro` 不会额外写入第一参数。注释、逐字内容、普通正文和 HTTP 链接不参与文件改名。

## 客户端调用顺序

编辑器或文件管理插件需要按以下顺序执行：

1. 发送 `workspace/willRenameFiles`，应用返回的编辑。编辑指向移动前的文档 URI，并保留已打开文档的版本约束。
2. 移动磁盘上的文件或目录。
3. 发送 `workspace/didRenameFiles`，使服务器更新文档 URI、刷新磁盘索引，并停止使用旧输入路径的编译。

## 创建与删除

标准 `workspace/didCreateFiles` / `workspace/didDeleteFiles` 通知会递归更新来源索引，刷新导航、补全和重复定义诊断，保留未保存的缓冲区，取消受影响的编译并清除旧编译诊断。无需文件监听也能同步编辑器中的文件操作；服务器继续保留 recorder 输入，以检查旧 PDF 是否过期。
