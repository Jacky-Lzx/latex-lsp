# import 系列源码加载

支持 `import`、`subimport`、`inputfrom`、`subinputfrom`、`includefrom` 和 `subincludefrom` 的静态双参数形式：

```latex
\import{parts/}{chapter}
% parts/chapter.tex 内：
\input{local}
\subimport{figures/}{diagram}
```

导入的源码参与项目符号、定义跳转、引用和符号重命名。`sub` 形式继承当前导入目录，导入中的普通 `input` / `include` 优先使用该目录，再回退到主文档目录。文件名参数提供路径补全、源码链接和参数提示。宏定义正文、逐字代码和字面转换中的未执行导入不会加入依赖。

上下文遍历最多 256 项，并遵守索引依赖深度。当前仅分析直接静态命令，不执行动态目录、命令别名或宏展开；多个导入上下文选择不同文件时不提供单一跳转。文件移动涉及导入相对路径时会拒绝自动改写，需显式更新导入目录；符号重命名不受此限制。
