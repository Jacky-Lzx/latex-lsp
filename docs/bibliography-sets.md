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

集合与数据库条目、手写文献及 `ids` 别名共用文献键命名空间。同名定义或新名称冲突会拒绝歧义重命名；项目隔离、共享来源检查和版本化编辑沿用文献编辑规则。只有前言的片段保持上述符号编辑规则。普通完整项目与含资源选择的 `refsection` 项目按数字分区和实际输入顺序认证静态集合身份，详见[分区文献身份](bibliography.md)。同一资源列表的不同数字分区不合并，先显式引用再定义集合时仍可能使用数据库条目。分组不会使集合定义恢复到分组前的状态。

分区项目的数据库关系字段补全按全部数字分区认证候选，展示实际生效的源码集合或书籍信息，支持静态命名集合及未完成输入。源码集合仅作为已有数据库父键的 `crossref`／`xref` 候选；没有数据库条目的源码集合、活动集合的 XDATA、默认 related 克隆及嵌套 entryset 均不提供候选。TeX 引用和集合成员补全、缺失键诊断、完整引用与改名共用全部消费者和数字分区的身份认证。唯一静态 IDS 与源码集合名称及所属主键互不相交时支持这些交互；所属主键被集合覆盖时另外支持只读导航和悬停。动态或延迟定义、冲突声明、嵌套集合、未认证的 IDS、可执行 `@preamble` 和手写文献仍保留保护。索引不执行任意宏展开。

未保存编辑立即更新声明和成员索引。关闭缓冲区后，已加载的项目来源恢复磁盘内容。Biber 对静态 `@set` 和成员字段的原生警告另见[集合诊断](bibliography-diagnostics.md#biber-文献集合)。

```sh
cargo test --locked --test biblatex_sets
cargo test --locked --test protocol bibliography_dynamic_sets_
cargo test --locked --test biblatex_sets -- --include-ignored
```

真实 Biber 测试通过普通模式 `.bcf` 验证索引中的动态集合，比较独立重命名集合键、成员键后的完整 `.bbl`，包括含双引号的新集合键及 Unicode 成员键。该测试只需 Biber；它不运行 LaTeX 或替代 biblatex 宏展开的验证。

分区集合只读身份回归覆盖普通／静态命名调用、成员、前言与分组、数字分区、首次引用顺序、通配加载、未保存输入及共享消费者、Unicode／CRLF 和分析预算。原生回归需要 pdfLaTeX、biblatex 和 Biber，核对十种执行顺序下 BBL 的实际条目类型与导航目标：

```sh
cargo test --locked --test biblatex_source_sections
cargo test --locked --test biblatex_source_sections -- --include-ignored
cargo test --locked --test protocol source_set_navigation_
```

数据库关系字段的源码集合导航另有回归，覆盖五类字段、数字分区冲突、首次请求锁定、UTF-16／CRLF、重复字段及未保存消费者。原生测试比较集合与书籍两种父条目及实际继承字段：

```sh
cargo test --locked --test biblatex_source_relations -- --include-ignored
cargo test --locked --test protocol source_set_relation_
```

源码集合的关系字段补全回归覆盖五类字段、完整键替换、引号转换、未完成输入及候选身份冲突；原生测试接受补全后比较实际条目类型、继承字段和最终编译结果：

```sh
cargo test --locked --test biblatex_source_completion -- --include-ignored
cargo test --locked --test protocol source_set_relation_completion_
```

源码关系候选的原生边界测试还比较有无数据库父条目的 `crossref`／`xref`，核对 Biber 的缺失父条目警告和 BBL 中是否保留该关系，并检查活动集合的 XDATA、related 克隆及嵌套集合警告。
