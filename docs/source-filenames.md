# 源码文件名与移动

[返回文档目录](README.md) · [文件操作](file-operations.md) · [项目与文件](projects.md)

文件移动后，引用必须继续选择同一目标。`workspace/willRenameFiles` 同时检查新位置的查找顺序和路径在 LaTeX 中的实际含义，保留已打开来源的版本约束；客户端仍按[文件操作顺序](file-operations.md#客户端调用顺序)应用编辑并移动磁盘文件。

## 空格与扩展名

单个内部空格可用于静态文件引用。例如把 `chapter.tex` 改为 `chapter name.tex`，服务器将 `\input{chapter}` 更新为 `\input{chapter name}`。原来没有分组的 `\input chapter` 会按需补入花括号；已有引号继续保留。

文件名以一个空格开始时，直接写在参数开头会被 LaTeX 忽略。服务器添加 `./`，把空格保留在路径内部。例如目标 ` leading.tex` 使用：

```tex
\input{./ leading}
```

文件名主干以空格结尾时，保留显式后缀，避免编辑器将有效空格裁掉。例如目标 `chapter .tex` 使用：

```tex
\input{chapter .tex}
```

这些写法适用于 `input`、`include` 及静态 `InputIfFileExists` / `IfFileExists` 文件参数；移动根文档或目录时继续重算相对路径。移动完成后的导航、依赖索引和再次重命名沿用同一目标。

## 无法直接表达的路径

源码引用中的连续空格、Tab 和换行可能被 TeX 合并或忽略；控制字符也不能作为普通源码文件名直接写入。若新路径会因此改变目标，服务器拒绝此次引用更新，不返回部分编辑。给整个参数添加普通引号不能保留已在 TeX 分词时合并的空格。此规则不展开动态文件名或插入用于拼接文件名的宏。

`% !TeX root` 注释和 `file:` 链接按各自语义处理，不套用源码参数的空格合并规则；内部连续空格可保留，文件 URL 使用百分号编码。图片和 biblatex 资源各自的路径规则见[图片文件名](graphics-filenames.md)和[biblatex 文献资源](biblatex-resources.md)。宏包、文档类及其他加载命令仍遵守[后缀要求](file-operations.md#重命名与移动)。
