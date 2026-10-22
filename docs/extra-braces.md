# 多余右花括号

[返回文档目录](README.md) · [编译诊断](diagnostics.md) · [特殊字符转义](special-characters.md)

编译器报告多余的 `}` 时，服务器会核对日志中的源码回显，精确标出该字符，并提供 `Remove extra '}'` 快速修复。操作只删除选中的一个字符，随后重新编译以确认错误是否解决。

## 支持的错误

| 编译器消息 | 常见位置 |
| --- | --- |
| `Too many }'s.` | 普通文本中的多余右花括号 |
| `Extra }, or forgotten $.` | 数学模式 |
| `Extra }, or forgotten \endgroup.` | `\begingroup ... \endgroup` 分组 |
| `Extra }, or forgotten \right.` | `\left ... \right` 定界符 |

例如，选择下面错误位置的快速修复：

```tex
{Valid} Before } after.
```

会得到：

```tex
{Valid} Before  after.
```

删除操作保留其他分组和空白。同一行有多个错误时，每个字符分别定位和修复。

如果错误实际源于遗漏 `$`、`\endgroup` 或 `\right`，应补齐对应的定界符。删除建议适用于确实多写了 `}` 的情况；服务器保留编译器的完整原始消息，供选择修复方式。

## 来源确认与客户端支持

- 精确诊断使用 `extra-closing-brace` 代码；`data.character` 为 `}`，`data.sourceContext` 保存核对用的回显。
- 支持传统 `!` 日志与文件行号格式、子文件、CRLF 和 UTF-16 位置。完整回显须匹配源码前缀；以 `...` 缩短的回显须能唯一匹配。
- 宏展开、无法确认的来源、无效行号、不匹配或不唯一的回显保留整行诊断。注释、逐字内容、宏定义体及 `\}` 不提供删除操作。
- 支持版本化 Code Action 和旧客户端的 `latex.applyQuickFix`。请求与执行修复时会重新核对当前诊断、源码和文档版本，拒绝过期或被改写的编辑。

该功能依据已经发生的编译错误提供修复。编辑时的配对检查、`let` 的字面来源和宏正文规则见[静态花括号诊断](brace-diagnostics.md)。
