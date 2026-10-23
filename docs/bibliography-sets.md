# biblatex 文献集合

[文档目录](README.md) · [文献](bibliography.md) · [集合编译诊断](bibliography-diagnostics.md#biber-文献集合)

`\defbibentryset{集合键}{成员键列表}` 声明动态集合，支持集合键和成员键的补全、定义跳转、悬停、引用查找、高亮、语义 token、选择范围及跨文件重命名。

```tex
\addbibresource{refs.bib}
\defbibentryset{collected}{first,second}
\cite{collected}
```

第一个参数声明集合键，第二个参数引用文献成员。重命名 `collected` 更新集合声明及引用，保留成员；重命名 `first` 更新数据库条目、成员列表及其他引用。标签和正文中的同名文字保持原样。集合键也会出现在文献补全和文档符号中。

成员参数支持逗号列表、注释、换行和未完成输入中的补全。参数提示区分 `{set key}` 与 `{keys}`；命令片段补全提供两个参数，已有参数时只补命令名。集合名使用单个静态文献键规则，支持 Unicode 和字面双引号。

只有两个参数均完整、集合名静态时建立声明。宏生成的名称、星号和可选参数形式不生成推测声明。注释及逐字内容中的示例不参与索引。成员中的动态名称不展开，额外分组和后续正文不作为成员引用。

集合与数据库条目、手写文献及 `ids` 别名共用文献键命名空间。同名定义或新名称冲突会拒绝歧义重命名；项目隔离、共享来源检查和版本化编辑沿用文献编辑规则。索引不执行宏展开，也不按 biblatex 的 `refsection` 执行顺序选择多个同名声明。

未保存编辑立即更新声明和成员索引。关闭缓冲区后，已加载的项目来源恢复磁盘内容。Biber 对静态 `@set` 和成员字段的原生警告另见[集合诊断](bibliography-diagnostics.md#biber-文献集合)。

```sh
cargo test --locked --test biblatex_sets
cargo test --locked --test protocol bibliography_dynamic_sets_
cargo test --locked --test biblatex_sets -- --include-ignored
```

真实 Biber 测试通过普通模式 `.bcf` 验证索引中的动态集合，比较独立重命名集合键、成员键后的完整 `.bbl`，包括含双引号的新集合键及 Unicode 成员键。该测试只需 Biber；它不运行 LaTeX 或替代 biblatex 宏展开的验证。
