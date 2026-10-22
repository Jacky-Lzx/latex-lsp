# futurelet 前瞻目标

[返回文档目录](README.md) · [符号与重命名](symbols.md)

`\futurelet` 的直接控制词目标进入宏符号索引，支持补全、定义跳转、悬停、引用、符号列表和跨文件重命名。例如：

```tex
\def\inspect#1{\ifx\lookahead A Found A\fi #1}
\futurelet\lookahead\inspect A
```

TeX 先将 `A` 的含义赋给 `\lookahead`，再执行 `\inspect A`。从声明或引用处重命名 `lookahead` 时，会同步维护定义体中的引用、直接调用及静态 `csname` 引用，保留 `inspect`、前瞻字符、注释和逐字内容。

## 静态名称目标

显式、相邻的 `expandafter` 可展开完整的静态 `csname` 目标：

```tex
\expandafter\futurelet\csname lookahead\endcsname\inspect A
```

只通过静态名称使用的目标可重命名为中文或含连字符的名称；存在直接控制词声明或引用时，新名称须能组成 TeX 控制词。沿用项目范围、共享来源歧义和名称冲突检查，LSP 编辑保留已打开文档的版本。

## 执行与分析边界

`futurelet` 后的继续执行命令及前瞻 token 会放回输入流，不能作为普通复制语句中不执行的来源参数处理。例如 `\futurelet\lookahead\input{child}` 中，`input` 仍是实际文件加载。

目标采用裸控制词或上述静态 `csname` 形式；花括号目标、动态名称及任意展开序列不推断为静态声明。不根据前瞻 token 推断副本调用的参数签名，也不模拟后续赋值或 TeX 分组作用域。

命令补全提供 `\futurelet\lookahead\continuation A` 片段，继续执行命令和字母 token 之间保留空格，避免合并成一个控制词。
