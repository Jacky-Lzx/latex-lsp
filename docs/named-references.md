# 静态命名引用的编辑

[返回文档目录](README.md) · [标签](labels.md) · [文献](bibliography.md)

`\csuse{ref}{key}`、`\csname ref\endcsname{key}` 及相应文献调用的实际键支持跳转、悬浮、引用查找、文档高亮、准备重命名和跨文件重命名。引用列表、范围、可选说明和结构化 biblatex 参数沿用直接调用的键边界；命令名称分组和说明文字不属于引用键。标签与文献名称分别解析。

实际键参数也提供上下文补全。已输入的键前缀和剩余后缀一起替换，保留其他列表成员、可选说明及后续参数；支持未完成分组、UTF-16 和 CRLF。补全沿用直接调用的标签／文献候选与参数解析，在命令名称参数、说明文字、动态名称和参数数据中不提供引用键候选。共享来源只有在所有消费者中都能确认同一调用与键位置时才补全，相关未保存重定义会立即更新结果。

例如：

```tex
\label{intro}
\newcommand{\later}{\csuse{ref}{intro}}
\csname ref\endcsname{intro}
```

重命名 `intro` 会同时更新声明、宏正文中的静态引用及最后一行的键。完整声明的替换正文可以参与静态编辑，诊断则不预测未展开的宏正文会产生哪些警告。动态名称、任意宏展开和复制命令的捕获含义不在此功能范围内。

索引比较每个主文档对共享来源的解释。命名接口或目标命令被明确重定义时，相关参数按普通用户命令处理；不同消费者的含义不一致，或潜在引用位于不能确认的参数数据／token 操作数中时，拒绝相关键的重命名。已确认的逐字和字面示例保留原文。独立项目的同名定义和重定义互不影响。

编辑使用现有文档版本和项目共享范围校验。单次分析覆盖相互关联的项目，限制为累计 32 MiB 源码、200000 个命令、8192 个声明、每个来源 20000 个键及累计 100000 个键解释；项目索引不完整或超出预算时不返回部分重命名编辑。

命名引用的未定义诊断、拼写修复与编号提示见[静态诊断](static-diagnostics.md)和[辅助文件引用](auxiliary-references.md)。

验证：

```sh
cargo test --locked --test named_reference_calls
cargo test --locked --test named_reference_calls real_named_key -- --include-ignored
cargo test --locked --test protocol named_reference_keys_navigate
cargo test --locked --test protocol named_key_completions_return_precise
```
