# 排版警告分析

[返回文档目录](README.md) · [编译](build.md) · [编译诊断](diagnostics.md)

`analysis.boxWarnings` 按警告类型和方向汇总最终 TeX 日志中的 Overfull / Underfull 诊断，帮助客户端展示排版问题的数量和最严重程度。

图表超出页面高度的独立统计见[浮动体高度警告](floats.md)。

## 字段

每项包含 `kind`（`overfull` / `underfull`）、`axis`（`horizontal` / `vertical`）和 `count`。Overfull 额外提供字符串 `maxExcessPt`，表示最大的溢出量，单位为 TeX pt；Underfull 提供整数 `maxBadness`，表示最大的 badness。

例如，`analysis` 中的相关字段：

```json
{
  "boxWarnings": [
    {"kind": "overfull", "axis": "horizontal", "count": 2, "maxExcessPt": "19.0"},
    {"kind": "underfull", "axis": "vertical", "count": 1, "maxBadness": 10000}
  ],
  "truncated": false
}
```

四类分别统计，按 `kind`、`axis` 排序，只返回实际出现的类别；没有排版警告时为 `[]`。最大溢出量使用精确十进制比较，保留编译器的数值文本，避免浮点舍入；相等数值选择稳定的原始写法。

## 计数与来源

`count` 统计最终日志解析后保留的警告诊断。不同文件或位置的警告分别计数；日志中相同位置、相同内容的重复诊断已经合并，不重复增加数量。支持段落、alignment、单行 detected 和输出例程警告，以及引擎按字节折行或带显式文件名、行号的格式。

只采用完整标准警告头，不从宏包警告的示例文字、盒子内容转储或不完整消息中推断指标。即使源码位置无法确认，日志中明确报告的排版指标仍可统计；位置和来源是否可靠继续由逐条诊断表达。服务器附加的生成文件行号说明不会影响指标识别。

最多返回四个类别。单个溢出量超过 128 个 ASCII 字符时，整条记录不参与计数和最大值，并设置 `analysis.truncated: true`。诊断输入本身被省略时也设置该标记，此时计数和最大值只代表已观测部分。

## 编译状态

摘要本身不改变构建成功状态、警告数或 `rerunRequired`。`latex/buildFinished` 和 `latex.buildStatus` 返回相同摘要；较早编译轮次中的警告不保留，排版修复后相应类别消失。取消、超时或过期等结果继续按[结果分析](build.md#结果分析)返回 `analysis: null`。
