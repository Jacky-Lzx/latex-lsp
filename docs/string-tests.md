# 字符串测试

[返回文档目录](README.md) · [符号与重命名](symbols.md) · [布尔表达式诊断](boolean-expressions.md)

etoolbox 的下列测试将参数作为未展开的 token 数据处理，再选择条件分支：

| 命令 | 数据参数 | 判断 |
| --- | --- | --- |
| `ifstrequal` | 两个字符串 | 未展开内容的字符形式相同 |
| `ifstrempty` | 一个字符串 | 空字符串；普通空格仍属于内容 |
| `ifblank` | 一个字符串 | 空字符串或仅含空格 |
| `notblank` | 一个字符串 | 含非空格 token |

每个命令随后接受 `{then}{else}`。提供命令补全片段和参数提示；字符串内显示外层测试命令的提示，分支内可继续显示嵌套命令的提示。

## 数据与引用

```tex
\newbool{draft}
\ifstrequal{\ifbool{draft}}{\ifbool{release}}
  {Same}
  {Different.\booltrue{draft}}
```

此处两个字符串按原样比较。将 boolean `draft` 重命名为 `release` 时，声明和分支中的 `booltrue` 参数会更新，两个用于比较的字符串保留原文，结果仍为 `Different`。

字符串中的宏、`newbool` 等声明、标签、引用、环境和 `input` 等文件命令均为数据，不建立对应的符号、引用或项目依赖。数据区域不提供定义、重命名、悬停和名称补全，并按字符串进行语义高亮。两个分支仍按源码建立索引，支持普通导航、补全及版本化跨文件重命名；服务器不模拟条件求值。

## 读取边界

支持完整的花括号参数和可确认边界的单 token 参数，例如 `\ifstrempty\macro{Empty}{Nonempty}`。参数之间的注释、CRLF、转义字符和嵌套花括号保留原文。数据中的 `verb` 等命令也只作为 token 读取，花括号仍遵循普通 TeX 参数分组规则。

未闭合的数据参数保持字面状态，并保留缺失花括号诊断。若文件末尾是参数内的 `%` 注释，插入缺失 `}` 的修复会先换行，确保括号生效。

未被赋值的直接调用按内置命令语法解析；[静态副本调用](string-test-copy-calls.md)使用复制时保存的含义和数据参数数量。当前文件和先前加载的本地依赖中的显式重定义会停止使用该命令的内置数据边界；加载顺序、未保存内容、共享文件的编辑限制与静态支持边界见[跨文件上下文](string-test-projects.md)。

## 布尔表达式诊断

`test {\ifstrequal{\input{ghost}}{A}}` 中的 `input` 仍是字符串数据。已确认的这些测试参数包含命令 token 时，外层 `bool` / `togl` 操作数仍可获得[精确诊断与拼写修复](boolean-expressions.md)。表达式定位要求测试命令的字符串参数使用完整花括号，并检查项目内的测试命令重定义；任意用户测试宏继续保留行级诊断。

## 验证

默认回归覆盖索引、依赖、名称编辑、补全、参数提示、单 token、注释、未闭合参数和本文件重定义。跨文件重定义的回归命令见[项目上下文验证](string-test-projects.md#验证)。下列协议回归检查字面数据、版本化跨文件编辑和增量修改：

```sh
cargo test --locked --test string_predicates
cargo test --locked --test protocol string_predicates
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox。分别重命名 boolean 与 toggle 后，核对字符串、条件分支及 PDF；再引入表达式拼写错误，检查精确修复和重新编译后的相同结果：

```sh
cargo test --locked --test string_predicates -- --include-ignored
```
