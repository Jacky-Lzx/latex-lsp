# 上下文选项补全

在宏包或文档类的 `[options]`、`includegraphics` 的可选参数、`geometry` / `newgeometry` / `hypersetup` 的设置分组，以及 `setkeys{Gin}` 等已知 family 中，可以补全选项名称和常见枚举值。`itemize`、`enumerate` 和 `description` 的可选参数提供常见 enumitem 选项。

例如 `\usepackage[mar]{geometry}` 可补为 `margin=`；`\includegraphics[keepaspectratio=t]{figure}` 可补全 `true`。支持 geometry、graphicx、hyperref 和常见文档类选项。已有选项不会重复推荐，花括号内的逗号不会切分外层选项，修改选项名会保留已有 `=value`。编辑范围按 UTF-16 计算。

这些是精选的静态选项数据。用户重新定义的同名命令、未知宏包、动态 family 或注释中的片段不会套用这份选项表；服务器不会从宏包源码执行或推导任意自定义键。
