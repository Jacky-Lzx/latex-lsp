# 缺失的右定界符

[返回文档目录](README.md) · [编译诊断](diagnostics.md) · [多余的右定界符](right-delimiters.md)

编译器报告 `Missing \right. inserted.` 时，服务器核对恢复帧与源码回显，将诊断定位到结束数学或显式分组的标记，并提供 `Insert invisible '\right.'` 快速修复。例如：

```latex
$\left(a$                      % 修复后：$\left(a\right.$
\(\left(a\)                    % 修复后：\(\left(a\right.\)
\begin{equation}\left(a\end{equation}
% 修复后：\begin{equation}\left(a\right.\end{equation}
```

插入的点是 TeX 的不可见定界符，不会显示为句点。若需要可见的右括号，请按公式含义改用 `\right)` 等写法。

## 支持范围

支持日志直接指向的 `$`（包括 `$$` 的第一个结束符）、`\)`、`\]`、`\endgroup`，以及 `math`、`displaymath`、`equation` 的 `\end{...}`。插入位置在结束标记之前，保留原标记、注释、空白和换行。支持子文件、UTF-16 范围、CRLF、传统与文件行号日志；缩短的源码回显须能在该行唯一匹配。

必须先读到准确的恢复帧 `<inserted text>` / `\right .`，再读到直接的 `l.N` 源码回显。额外的用户宏展开帧、不匹配或歧义回显、未知来源、定义正文、逐字内容、注释及被宏消费的结束标记均不提供此操作。`right`、可静态识别的相关结束命令或数学环境在项目内被重定义时，也会拒绝相应修复。

部分错误会延迟到 `\end{document}` 才报告，例如某些未加载 amsmath 的 `\[...\]` 恢复情况。服务器保留这些编译诊断，不据此在文档末尾补定界符；自定义环境结束处也不推断插入位置。

一次操作插入一个 `\right.`。同一位置缺少多层配对时，重新编译后再处理剩余错误；一些 LuaTeX 版本会在首个错误处重复恢复，达到错误上限后才退出，因此也可能需要分轮修复。此操作不会消除其他独立的括号或分组错误。

## 来源与旧操作

诊断代码为 `missing-right-delimiter`。诊断标出结束标记，实际编辑是其起点处的零宽插入。诊断数据中的 `closingToken` 保存该标记，`sourceContext` 保存源码回显，`sourceLine` 保存从零开始的回显行号。

快速修复复用编译时确认的宏来源和参数语法；请求操作与执行旧客户端命令时重新核对源码、版本、标记及依赖。源码、依赖中的参数定义或 `right` 定义变化后，旧操作可能失效，需重新编译。来源规则见[宏来源说明](literal-macro-scopes.md)。

## 验证

```sh
cargo test --locked --test right_delimiters
cargo test --locked --test right_delimiters real_missing_right -- --ignored
cargo test --locked --test protocol right_delimiter
```

真实回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX。在 60 / 79 列及两种错误格式中逐轮修复子文件，覆盖上述结束标记、宏生成的左定界符、跨行注释与嵌套缺失，最终确认生成一页 PDF 且编译错误清除。stdio 协议回归同时覆盖 Code Action 和旧客户端命令、版本校验、伪造替换拒绝及重编译后的诊断清除。
