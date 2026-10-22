# 缺失的定界符参数

[返回文档目录](README.md) · [编译诊断](diagnostics.md) · [缺失的右定界符](missing-right-delimiters.md)

`\left`、`\right` 和 `\middle` 后需要一个定界符。编译器报告 `Missing delimiter (. inserted).` 时，服务器核对待重新读取的 token 和源码回显，将诊断标在缺少定界符的命令上，并提供 `Insert invisible delimiter after '…'` 快速修复。

```latex
$\left a+b\right)$          % 修复后：$\left. a+b\right)$
$\left(a+b\right x$         % 修复后：$\left(a+b\right. x$
$\left(a\middle b\right)$   % 修复后：$\left(a\middle. b\right)$
```

修复在命令后插入一个点，表示不可见定界符。原来的字母、数学符号、空白、换行和注释原样保留。若需要可见括号，请按公式含义改用 `\left(`、`\right)` 等写法。

这与[缺失整个右定界符命令](missing-right-delimiters.md)不同：本操作只补已有命令的参数，不补 `\right`，也不保证消除其他分组或数学错误。比如 `\left{` 中的花括号仍然是分组；若本来想显示集合括号，应改用 `\left\{`。

## 定位与来源

需要紧邻错误消息的 `<to be read again>` 帧，下一行必须是单个可对应源码的 ASCII token，再接直接的 `l.N` 源码回显。该 token 必须恰好出现在命令之后，期间仅允许空白和注释；普通字符和 `\alpha` 等控制序列都可核对。完整回显须匹配源码前缀，缩短回显须在该行唯一匹配。

支持子文件、UTF-16 范围、CRLF、传统与文件行号日志，以及跨行注释。诊断覆盖命令，实际编辑是命令末尾的零宽插入。第二行源码回显和已经完整确认的标准 TeX 帮助文本不会作为文件进入或退出事件；之后的主文件和兄弟文件诊断保持各自来源。

额外用户展开帧、展开后与源码不一致的 token、不完整或歧义回显、未知来源、定义正文、宏参数、逐字内容和注释保留原诊断，不提供编辑。`^^` 写法及非 ASCII token 暂不按单 token 修复；`\big` 等宏内部的同类错误也不作直接原语推断。

修复使用编译时已确认来源中的参数语法。原语或相关控制词被项目内重定义时不提供此操作；请求操作和执行旧客户端命令时重新校验文档版本、源码、诊断数据与依赖。依赖中的参数定义变化后，旧操作可能失效。详见[宏来源规则](literal-macro-scopes.md)。

诊断代码为 `missing-delimiter-operand`。`data.delimiterCommand` 保存带反斜杠的命令，`recoveryToken` 保存已核对的待处理 token，`sourceContext` 保存源码回显，`sourceLine` 保存从零开始的回显行号；跨行调用中，回显行可与诊断行不同。

## 验证

```sh
cargo test --locked --test delimiter_operands
cargo test --locked --test delimiter_operands -- --include-ignored
cargo test --locked --test protocol right_delimiter
```

真实回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX。在 60 / 79 列与两种错误格式中修复子文件的 13 处错误，覆盖三个原语、控制词与脚本符号、连续调用和跨行注释，确认生成一页 PDF、错误清除且主文件警告来源正确。stdio 回归覆盖 Code Action、旧客户端命令、伪造替换拒绝、版本失效和重编译后的诊断清除。
