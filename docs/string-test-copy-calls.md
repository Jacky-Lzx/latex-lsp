# 字符串命令副本的直接调用

[返回文档目录](README.md) · [保存与恢复](string-test-copies.md) · [参数提示](signatures.md)

## 保存的含义与参数数量

静态复制得到的控制词可直接调用。副本保留复制时的字符串含义，之后重定义源命令不会改变它：

```tex
\usepackage{etoolbox}
\newbool{draft}
\let\saved\ifblank
\renewcommand{\ifblank}[1]{#1}
\saved{\booltrue{draft}}{Empty}{Tokens}
```

这里的 `booltrue{draft}` 是字面字符串，不改变 boolean，也不参与 boolean 导航、补全或重命名。副本名 `saved` 可以按普通宏重命名；两个条件分支中的符号仍参与索引和编辑。

参数数量来自保存的含义。例如 `\let\ifblank\ifstrequal` 后，`ifblank` 的前两个参数均为字面字符串；反向复制后，`ifstrequal` 只有第一个数据参数。参数提示使用实际调用名和保存的参数数量。

字面参数中的 `input`、声明或逐字命令不会载入文件、定义符号或遮蔽后续代码。目标名称为 `input`、`label`、`newcommand`、`verb` 等已有命令名时，已确认的原生字符串含义也会替换该名称原有的解析规则。

副本自身也可被[撤销定义](string-test-undefinitions.md)；撤销后的调用不再读取原来的字符串参数，之前从它保存的其他副本仍然有效。

## 跨文件更新与歧义

副本含义按文件加载顺序传播，支持链式静态复制。编辑依赖缓冲区、关闭后恢复磁盘内容，以及磁盘文件通知都会更新调用处的字面边界与参数提示；主文档的版本号保持不变。

共享文件在多个上下文中采用不同含义时：

- 都是原生字符串测试但数据参数数量不同：共同的第一个参数保持字面，第二个参数标记为歧义。
- 原生测试与自定义命令并存：可能作为代码执行的数据参数标记为歧义。
- 会修改歧义参数的重命名被拒绝，其他明确的符号仍可编辑；无法确定完整参数形式时不猜测参数提示。

编译流程使用编译前的项目源码快照分析日志。布尔表达式 `test{\saved{...}}` 中的字面数据不会抢占同级可执行 boolean / toggle 的错误位置；未知测试或参数数量存在歧义时保留整行诊断，不猜测修复。单独解析日志、没有项目上下文时仍采用保守的静态规则。

不推断动态名称、任意宏展开、用户宏执行后的副作用或分组结束时的自动恢复。复制原语被重定义或来源无法确认时保留不确定性。

## 验证

```sh
cargo test --locked --test copied_string_tests
cargo test --locked --test protocol copied_string_predicate_arities
cargo test --locked --test boolean_expression_diagnostics copied_string_tests
cargo test --locked --test copied_string_tests -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox，覆盖四种谓词、双向参数数量变更以及三种复制形式。宏名和 boolean 重命名前后，比较字面原文、boolean 状态与 PDF 页数。
