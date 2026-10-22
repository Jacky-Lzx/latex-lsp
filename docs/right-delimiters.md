# 多余的右定界符

[返回文档目录](README.md) · [编译诊断](diagnostics.md)

TeX 报告 `Extra \right.` 时，当前数学分组中没有与该命令配对的 `\left`。服务器核对日志源码回显，将诊断定位到实际的 `\right`，并提供 `Remove unmatched '\right'` 快速修复。

## 使用

修复移除所选的 `\right`，保留其后的普通定界符。例如：

```latex
$a\right)$                    % 修复后：$a)$
$a\left(b\right)\right]$      % 修复后：$a\left(b\right)]$
$a\right\rangle$              % 修复后：$a\rangle$
```

这会取消该处的自动尺寸配对。若原本希望整个表达式使用自动尺寸，应按公式含义补上相应的 `\left`；服务器不会猜测开定界符的位置。

不可见定界符 `\right.` 单独处理：将 `\right` 替换为 `{}`，同时移除这个点，避免生成可见句点或拼接相邻控制词。例如 `$\alpha\right.b$` 变为 `$\alpha{}b$`。

支持 `\right` 与定界符之间的空白、跨行和注释。修复只编辑命令及必要的不可见点，保留注释和换行；同一行的多个错误分别定位。已配对的 `\left...\right` 不会因为旁边存在错误而被修改。

## 已确认的定界符

支持字面 `(`、`)`、`[`、`]`、`/`、`|`、`.`。对于标准命名定界符，还核对该定界符自己的 `\delimiter` 展开帧与源码中的名称一致：

- `langle` / `rangle`、`lbrace` / `rbrace`。
- `lfloor` / `rfloor`、`lceil` / `rceil`。
- `vert` / `Vert`、`backslash`。
- `uparrow` / `downarrow` / `updownarrow` 及其首字母大写形式。
- `lgroup` / `rgroup`、`lmoustache` / `rmoustache`。

普通希腊字母、常见数学符号、大型算符及 `sin`、`lim` 等无参数算符不会吞掉后续的 `\right`；项目内的用户重定义仍按其实际参数处理。

## 来源与旧操作

诊断代码为 `unmatched-right-delimiter`。`data.delimiter` 保留源码定界符，`sourceContext` 保存源码回显，`sourceLine` 是从零开始的回显行号；跨行调用中，该行可与被标出的命令行不同。命名定界符的 `delimiterFrame` 保存已确认的展开名称。

支持传统 `!` 日志、文件行号格式、子文件、CRLF 和 UTF-16 范围。完整回显须匹配源码前缀；缩短回显须在整行中唯一匹配。不完整或不匹配的回显、未知来源、用户宏展开栈、定义正文、注释、逐字文本及未支持的动态定界符保留原诊断，不提供编辑。命名定界符缺少对应标准展开帧时也不作推断。

修复沿用编译时已加载来源中的参数语法，排除被宏消费的命令；`right` 或相应命名定界符被重定义时不提供此修复。请求操作及执行旧客户端命令时重新校验源码、文档版本和宏来源；依赖中的参数定义变化后，旧修复失效。来源分析规则见[宏来源说明](literal-macro-scopes.md)。

## 验证

```sh
cargo test --locked --test right_delimiters
cargo test --locked --test right_delimiters -- --include-ignored
cargo test --locked --test protocol right_delimiter
```

真实回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX，在 60 / 79 列及两种错误格式中修复跨文件正文，核对字面定界符、全部上述命名定界符、不可见点和注释，再实际生成一页 PDF 并确认错误清除。
