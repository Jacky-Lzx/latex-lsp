# expl3 编辑

在 `\ExplSyntaxOn` 与 `\ExplSyntaxOff` 之间，服务器将 `_` 和 `:` 作为控制序列名称的一部分。`ProvidesExplPackage`、`ProvidesExplClass`、`ProvidesExplFile` 的完整声明也开启此模式；普通 TeX 分组结束时恢复进入分组前的模式。注释、逐字代码以及宏正文中延迟执行的模式切换不会改变外围词法状态。

支持直接静态目标的以下声明：

- `cs_new`、`cs_set`、`cs_gset` 及其 `protected` / `nopar` 形式的 `:Npn`、`:Npx`、`:Npe`、`:Nn`、`:Nx`、`:Ne`；`cs_new_eq:NN`、`cs_set_eq:NN`、`cs_gset_eq:NN` 别名。
- `tl`、`str`、`int`、`seq`、`prop`、`bool`、`dim`、`skip`、`muskip`、`clist`、`fp`、`box`、`ior`、`iow`、`coffin` 的 `new:N` 与 `const:Nn` / `const:Nx` / `const:Ne`。

项目函数和变量提供完整名称补全、定义、引用、悬停、跨文件重命名、文档符号及语义高亮。补全还包含常用 expl3 基础函数。重命名保留函数的 `:nn` 等参数签名，以及变量的 `l_` / `g_` / `c_` 范围和 `_tl` 等类型后缀；声明不唯一或与已有名称冲突时拒绝操作。

参数提示由项目声明或已知基础函数的名称后缀推导，区分 `N` 单 token 和 `n` 等分组参数，以及 `T` / `F` 分支。显式 `:Npn` 参数文本必须是与名称签名一致的普通 `#1 ... #9`；分隔参数文本、`p` / `w` 参数不推测提示。

这是静态编辑支持：不执行展开、任意 catcode 改写或条件定义，不生成 `:c` 动态名称，也不展开 `cs_generate_variant:Nn` 产生的隐式声明。复杂局部遮蔽和运行时声明需要手动检查；普通 LaTeX 模式仍使用原来的控制词规则。
