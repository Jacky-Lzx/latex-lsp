# 末页变更与重编译

[返回文档目录](README.md) · [编译结果分析](build-analysis.md)

使用 `shipout/lastpage` 钩子时，LaTeX 根据上一遍的实际总页数决定在哪里执行末页内容。编辑使总页数变化后，即使本次成功生成 PDF，末页内容仍可能需要再次编译才能正确放置。

## 两类明确请求

文档缩短后，钩子预期的末页可能已不存在。LaTeX 临时附加一页来承载末页内容，并报告：

```text
LaTeX Warning: Temporary extra page added at the end. Rerun to get it removed.
```

例如从三页缩为一页，第一遍可能生成两页 PDF；再次编译后恢复一页。结果中的 `pages` 始终报告本次实际生成的页数，包括临时页面。

文档变长后，钩子可能在原来的末页过早执行，并报告：

```text
LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3).
               Rerun to correct this.
```

括号中依次为上一遍和本遍的实际总页数。例如从一页增为三页，第一遍 PDF 已有三页，但钩子仍在第一页执行；再次编译后移至第三页。

这两类完整警告设置 `analysis.rerunRequired: true`，并在诊断处提供 `Rebuild …` 操作。它们不要求文献或索引工具；没有其他工具请求时，`requiredBibliographyTools` 和 `requiredIndexTools` 都为 `[]`。操作使用现有[重编译配置](build.md#从诊断重新编译)。

## 日志与清除

识别兼容逐字节折行、LF / CRLF 和 LaTeX 的缩进续行。末页钩子警告必须包含两个不同的正整数页数和完整的 `Rerun to correct this.` 请求。普通宏包中的相似文字、示例、不完整请求、额外说明及错误级别的消息不会触发该标记或操作。

分析依据本次最终日志。后续编译中警告消失时，标记与操作随之清除；编译管道早期轮次的提示不会覆盖最终结果。完成通知与 `latex.buildStatus` 返回一致的摘要，旧诊断对应的菜单命令失效。

## 复现与测试

使用下面的主文档，将 `child.tex` 的正文从三页缩成一页，或从一页增加为三页。每次修改后连续编译两遍即可观察请求与收敛：

```latex
\documentclass{article}
\AddToHook{shipout/lastpage}{\put(20,-30){Last page marker}}
\begin{document}
\input{child}
\end{document}
```

默认回归核对折行、续行、页数格式、相邻来源边界及两种客户端操作。真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和支持上述钩子的 LaTeX 内核，在 60 / 70 / 79 列日志中检查实际 PDF 页数、钩子执行页、请求和操作的出现与清除，包括右括号单独折行的情况：

```sh
cargo test --locked --test rerun_warnings
cargo test --locked --test rerun_warnings real_last_page -- --include-ignored
cargo test --locked --test protocol wrapped_standard_rerun
```
