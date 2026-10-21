# biblatex 文献资源

[文档目录](README.md) · [引用命令](biblatex.md) · [文献键与重命名](bibliography.md) · [编译输入检查](build-inputs.md)

## 加载与编辑

```tex
\usepackage{biblatex}
\addbibresource{references.bib}
\addglobalbib[bibencoding=utf8,label={Shared references}]{shared.bib}
```

`addbibresource` 添加默认文献资源；`addglobalbib` 添加 biblatex 各个 `refsection` 都使用的全局资源。两者支持路径补全、文件链接、定义跳转、文件或目录移动后的路径更新，以及所加载文献键的导航和跨文件重命名。命令片段保留完整 `.bib` 文件名，参数提示使用 `[options]{file}`。

每次调用只读取一个完整文件名，包括扩展名。`{refs}` 只查找名为 `refs` 的文件，不会选择 `refs.bib`；`{refs,part.bib}` 是一个带逗号的文件名。直接调用的资源参数中，`%` 按字面文件名字符处理，不会吞掉后面的引用；路径补全和文件移动保留这些字符及完整扩展名。

共享文献库的键重命名覆盖关联主文档，保留独立项目中的同名条目，并遵循现有的歧义和版本检查。资源作为 BibTeX 数据索引，不导入其中看似 TeX 宏的文字。

## 静态支持范围

默认按本地 BibTeX 数据处理，也识别以下可选参数：

- `type=file`、`datatype=bibtex`、`location=local`、`glob=false`。
- 字面的 `bibencoding`，以及资源标签 `label`；标签按原文保存，可包含 `\emph{…}` 等宏文字。

允许选项值外的一层花括号及受分组保护的逗号。远程资源、其他数据格式、通配符展开、未知选项或决定文件选择的动态值不推断为本地依赖。含注释或参数占位符的选项、宏展开文件名，以及前后空白、连续空格、换行等无法确认的路径也不作推断。

静态 `\csuse{addglobalbib}`、`\csname addbibresource\endcsname` 和可确认的纯别名共用资源分析；不执行任意宏展开，也不完整模拟自定义命令中的字符类别变化。被其他宏作为数据消费、逐字展示或已重定义的调用不会误提供文件编辑；未执行宏体中的资源不作为确定的缺失文件条件。

`addsectionbib` 只注册供之后选择的资源，单独出现时不会加入依赖。当前文献索引仍按主文档和项目建立，尚未实现 `refsection` 的资源选择及独立文献键作用域。

## 编译保护

静态本地资源在首次编译前就参与未保存内容检查。编译期间补回缺失文献库会使本次结果过期，避免继续发布旧的缺失资源诊断；详细规则见[编译输入与过期结果](build-inputs.md)。
