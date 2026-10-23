# 补全

[返回文档目录](README.md) · [编辑功能](editing.md) · [编辑器配置](editors.md)

支持常用 LaTeX 命令、环境、项目内用户命令、跨文件标签、BibTeX 条目类型、字段名、字符串缩写与文献键，以及输入文件、图片和本地包等路径。支持词中替换；引用和路径的可选参数含有嵌套命令时，仍可找到当前的键名或文件参数。

biblatex 的 `citefield` / `printfield`、`citelist` / `printlist`、`citename` / `printnames` 还按参数类型补全常用输出字段、字面列表或姓名列表名称，并提供悬停说明；详见[biblatex 数据访问](biblatex.md#字段列表和姓名引用)。

字段读取、undef 检查和字段/列表比较共用名称候选。`iffieldsequal` 的两个字段参数都可补全；比较字符串、命令内容和执行分支中的普通文字保持独立，详见[字段与列表条件](biblatex.md#字段与列表条件)。

`ifentrytype` 的首参数建议常用 biblatex 输出条目类型，使用普通名称编辑和类型说明；它的两个分支保持独立，详见[条目条件检查](biblatex.md#条目条件检查)。

## BibTeX 条目与字段

在 `@` 后补全常用 BibTeX / biblatex 条目类型，也提供当前项目文献库中已出现的自定义类型。输入 `@art` 可选择 `article`，大小写不敏感，词中补全会替换完整类型名并保留 `@`。已有花括号或圆括号正文时，只替换名称，保留条目键、字段和注释。

支持片段的客户端可以直接插入新条目的字段模板，例如选择 `article` 后填入文献键、作者、标题、期刊和年份。模板采用当前文档的 LF / CRLF；`comment`、`string` 和 `preamble` 使用各自的语法。模板提供常用字段，不用于判断样式要求或校验必填项；自定义类型生成通用条目骨架。

在条目键后的逗号、字段之间的空白或未完成字段名处，提供常用字段补全。候选包含字段说明，优先显示当前类型常用的字段，排除同一条目中已经填写的字段名；比较不区分大小写。编辑已有字段名时保留其赋值、字段值及注释。例如，把 `AUTHbad = {Writer}` 中的 `AUTHbad` 补全为 `author`，不会修改 `{Writer}`。

候选也包含当前项目文献库中已赋值的自定义字段。保留原始名称，按 ASCII 大小写去重；同名字段优先采用当前文件中的拼写，内置字段使用标准小写名称。当前类型常用字段排在最前，其余内置字段之后显示自定义字段。尚未赋值的名称不会成为项目候选，已输入 `=` 的未完成赋值仍可贡献字段名。其他项目、注释和特殊条目中的示例不参与收集。

字段名采用 BibTeX 标识符规则，支持 `custom:field`、`field$` 和非 ASCII 字符等合法名称；它们的后续字段继续参与大纲、导航、补全和语义高亮。在 `AUTH:bad` 中输入 `AUTH` 后选择 `author`，会替换整个 `AUTH:bad` 名称。数字开头的字段名不作为有效赋值解析。

支持片段时，新字段插入值占位符，如 `author = {…},`，并保留已有逗号；已有 `=` 时只改字段名。自定义字段名中的 `$` 和反斜杠会按片段语法转义，展开后保留原始字符。没有片段能力的客户端只插入类型名或字段名。内置条目类型和字段名也提供悬停说明；条目键和条目间引用继续使用文献导航与补全。

上下文复用条目及字段解析，不在条目键、标题、URL、其他字段值、注释和特殊条目正文中建议类型或字段名。缺少键后的逗号或遇到无法解析的字段时，不猜测后续字段。自定义类型和字段的候选按项目隔离，不引入独立论文中的名称；未保存的增量编辑立即更新候选，关闭文件后恢复当前磁盘内容。

验证：

```sh
cargo test --locked --test bibliography_completion --test bibliography
cargo test --locked --test protocol bibliography_type_and_field_
cargo test --locked --test protocol custom_bibliography_field_completions
cargo test --locked --test bibliography_completion real_bibtex_reads_completed -- --include-ignored
cargo test --locked --test bibliography_completion bibliography_snippets -- --include-ignored
```

最后一项通过真实 Neovim 0.10+ 的 `vim.snippet` 检查片段展开；原生 BibTeX 回归使用自定义样式读取补全后的字段，验证已有赋值和后续标题保留。

BibTeX 字段值、字符串赋值和 `@preamble` 的表达式还提供项目内[字符串缩写](bibliography.md#字符串缩写)及常见月份补全。候选可替换完整裸标识符，或填入 `=`、`#` 后的空值位置，保留字面文本和拼接结构。

## 调用片段与已有参数

客户端声明 snippet 支持时，命令补全可插入参数占位符；否则只插入命令名。用户命令根据项目内的静态签名生成片段，省略可选参数与标志，保留默认行为。

跨文件声明及参数数量中的注释规则见[参数提示](signatures.md)。

已有参数时只替换名称，不重复添加参数。内置命令也保留已有的无花括号 ASCII 实参，例如 `\frac12` 和 `\textbf\payload`；在数学、对齐或环境结束符前仍插入参数占位符。用户宏使用相同规则，显式声明的标志和分隔符优先。

## 命令名操作数

声明目标、命令复制来源和 `meaning` 查看位置只读取命令名，不调用该命令。输入部分命令名并接受补全时，仅替换名称，不插入被命名命令的参数；只输入反斜杠、尚未输入字母时也遵循这一规则，兼容尚未闭合的名称分组。例如，项目内有 `\newcommand{\ordinary}[1]{#1}` 时：

| 输入 | 选择 `ordinary` 后 |
| --- | --- |
| `\meaning\` | `\meaning\ordinary` |
| `\meaning\ord` | `\meaning\ordinary` |
| `\let\saved=\ord` | `\let\saved=\ordinary` |
| `\newcommand{\ord}` | `\newcommand{\ordinary}` |
| `\futurelet\ord\relax x` | `\futurelet\ordinary\relax x` |

内置命令与跨文件用户命令适用相同规则。词中补全替换剩余命令字母，保留反斜杠、注释、换行和后续正文；位置采用 UTF-16，兼容中文与 emoji。

实际调用仍提供参数片段，包括 `futurelet` 的后续执行命令，以及被查看或复制的操作数之后再次出现的真实命令。操作数判断沿用静态解析边界；本文件前文已显式重定义相关关键词时，按普通调用处理。不执行任意宏展开，也不完整推断跨文件改写或分组恢复。

相关语义见[命令名操作数与声明](declaration-operands.md)、[meaning](meaning.md) 和 [futurelet](futurelet.md)。

`detokenize` 的分组正文、`string` 转成文字的 token 不提供普通命令补全；文件名参数中的这些包装仍可用于路径补全。被 `meaning` 查看或被复制的转换命令不启动文字转换，后续真实命令继续提供补全。

## 名称替换

选择单标签引用、环境或计数器名称补全时，以候选的完整名称替换当前名称参数，包括旧名称内部的首尾空白，避免补全后仍引用另一个名称。保留参数分隔符、数值、格式和参数外的排版。

跨行名称使用当前行的主编辑与互不重叠的附加删除编辑，符合 LSP 对补全编辑的单行要求。动态名称与无法安全安放主编辑的空行不提供建议。
