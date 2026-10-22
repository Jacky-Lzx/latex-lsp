# 字体诊断与结果分析

[返回文档目录](README.md) · [编译](build.md) · [编译诊断](diagnostics.md)

编译成功生成 PDF 时，字体替换和缺字警告仍可能表示排版内容与预期不同。结果摘要帮助客户端集中显示这些问题，逐位置诊断保留原有级别和计数。

## 字体与字号替换

最终日志中的完整标准 `LaTeX Font Warning` 分为两类：

- `analysis.fontSubstitutions`：请求字体与实际替代字体，字段为 `requested` 和 `replacement`。
- `analysis.fontSizeSubstitutions`：字号替换，字段为 `font`、`requestedSize` 和 `replacementSize`。

字体使用 NFSS 的 `encoding/family/series/shape` 四段身份；字号保留编译器输出的十进制字符串，不转换为浮点数。例如，`analysis` 中的相关字段：

```json
{
  "fontSubstitutions": [
    {"requested": "OT1/missing/m/n", "replacement": "OT1/cmr/m/n"}
  ],
  "fontSizeSubstitutions": [
    {"font": "OT1/limited/m/n", "requestedSize": "14.7", "replacementSize": "10"}
  ],
  "truncated": false
}
```

同一替换在不同位置或文件中出现时合并，按字段元组排序；两类各最多 128 条。每个字体身份最多 256 个 Unicode 字符，字号最多 128 个 ASCII 字符。超出预算时省略完整记录，并设置 `analysis.truncated: true`，不截短名称。

恢复引擎按字节折行的长字体名和字号，保留名称内部原有的空白、Unicode 和括号；无法确认完整警告时不推测替换关系。仅有 “Some font shapes…” 或 “Size substitutions…” 等汇总警告不产生具体记录。低于 LaTeX `fontsubfuzz` 阈值而未报警的字号替换也不会被推断。

两类替换本身不设置 `rerunRequired`，不改变构建成功状态或警告数。`latex/buildFinished` 和 `latex.buildStatus` 返回相同摘要，只采用本次最终日志；修复后两数组恢复为 `[]`，早期编译轮次中的替换不保留。取消、超时或过期等结果继续按[结果分析](build.md#结果分析)返回 `analysis: null`。

## 缺字摘要

`analysis.missingCharacters` 汇总最终日志中的缺字错误和警告，按字符码点排序。兼容警告和错误的字节折行，不在字体名称中插入空格。同一字符在不同位置、文件或字体中出现时合并为一项；`fonts` 保留去重并排序后的引擎字体描述，去掉引擎添加的一枚结尾标点，保留字体本身的标点；服务器附加的辅助文件行号说明不进入字体描述。`severity` 使用 LSP 级别：`1` 为错误，`2` 为警告；同一字符兼有两种级别时保留错误。摘要不改变逐位置诊断及构建错误数。

例如 `analysis` 中的相关字段：

```json
{
  "missingCharacters": [
    {
      "character": "中",
      "codepoint": "U+4E2D",
      "severity": 1,
      "fonts": ["[lmroman10-regular]:mapping=tex-text;"]
    }
  ],
  "truncated": false
}
```

仅收录字面字符与合法 Unicode 码点一致的缺字消息；无法确认源码位置时仍可汇总日志确认的字符和字体，定位是否可靠继续由诊断表达。每次编译重新生成摘要，缺字解决后返回空数组；较早编译轮次的输出不会保留到最终日志的摘要中。

最多保留 128 个字符，每个字符最多 8 个字体描述，每个描述最多 256 个 Unicode 字符。超出限制的字符或字体描述会被省略，并设置 `analysis.truncated`；不会截短名称。某个字符的字体描述全部超长时，仍保留字符、级别及空的 `fonts` 数组。已收录字符的后续错误仍会提升其级别，即使新字体描述超出限制。

LaTeX 未设置 Unicode 字符映射的错误单独记录到 `analysis.unsupportedUnicodeCharacters`；与字体缺字的区别和处理方式见[Unicode 输入错误](unicode-errors.md)。

## 缺字定位

来源和行号已确认，且缺字消息中的字面字符与合法 Unicode 码点一致时，诊断附带 `missing-character` 代码及 `character`、`codepoint` 数据。源码回显能唯一核对到连续的直接字面文本时，精确标出其中缺字的每个位置，支持子文件、UTF-16、CRLF 和缩短回显。例如 `中😀中` 的三个缺字即使共用相同回显，也保留三个范围并计入三个错误；重复日志不会重复增加同一位置的诊断。

定位不跨实际空白借用前一个词的字符。控制词后会被 TeX 忽略的空白不能确认文本片段边界；此类情况及宏展开、生成字符、忽略区域、不明确的回显和无效行号保留行级位置，不根据附近的同名字符推测来源。同次日志解析复用相同缺字上下文的定位结果，保留不同文件、行号、回显和字体的诊断。

