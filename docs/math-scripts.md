# 重复上下标

[返回文档目录](README.md) · [编译诊断](diagnostics.md) · [正文特殊字符](special-characters.md)

TeX 报告 `Double subscript.` 或 `Double superscript.` 时，同一个数学基底已有同类上下标。服务器核对日志的源码回显，精确标出第二个 `_` 或 `^`，并提供显式添加空基底的快速修复。

## 使用

| 错误 | 诊断代码 | 快速修复 | 修改示例 |
| --- | --- | --- | --- |
| `Double subscript.` | `double-subscript` | `Insert empty base before '_'` | `x_1_2` → `x_1{}_2` |
| `Double superscript.` | `double-superscript` | `Insert empty base before '^'` | `y^1^2` → `y^1{}^2` |

操作只把选中的脚本符号替换为 `{}_` 或 `{}^`，保留已有上下标内容及后续分组。例如：

```tex
$x_{ab}_{cd}$ and $y^{ab}^{cd}$
```

分别选择修复后得到：

```tex
$x_{ab}{}_{cd}$ and $y^{ab}{}^{cd}$
```

空基底让后续上下标附着到新的数学原子上。如果希望合并为一个上下标，或表达嵌套关系，应按公式含义调整花括号，例如 `x_{12}` 或 `x_{1_2}`。正常的 `x_1^2` 不属于重复上下标；正文中的 `_` / `^` 转义仍由[特殊字符诊断](special-characters.md)处理。

支持行内与显示公式、分组上下标及注释续行。同一行的多个错误分别定位；修复后重新编译，已解决的诊断清除。

## 来源确认

精确诊断的 `data.character` 保存脚本符号，`data.sourceContext` 保存核对用的源码回显。支持传统 `!` 日志、文件行号格式、子文件、CRLF 和 UTF-16 位置。完整回显须匹配源码前缀，缩短回显须在整行中唯一匹配。

宏展开帧、不匹配或不唯一的回显、未知来源和无效行号保留整行诊断。注释、逐字内容、宏定义体及已转义符号不提供该操作；已加载项目来源中可识别的宏参数中的脚本符号也不视为直接执行的上下标。跨文件来源边界见[宏来源说明](literal-macro-scopes.md)。

支持版本化 Code Action 及旧客户端的 `latex.applyQuickFix`。请求修复时核对当前诊断、消息、字符、源码和宏参数；旧命令执行时还会检查文档版本，并拒绝被改写的替换内容。
