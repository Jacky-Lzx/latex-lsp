# 图片命令的静态别名

[返回文档目录](README.md) · [图片搜索与移动](graphics.md)

## 图片调用

图片跳转、路径补全、依赖跟踪和文件重命名支持无参数、正文仅含单个控制序列的纯别名、别名链，以及静态 `\csuse` / `\csname` 调用：

```tex
\usepackage{graphicx,etoolbox}
\def\Image{\includegraphics}
\newcommand{\Figure}{\Image}
\graphicspath{{figures/}}
\DeclareGraphicsExtensions{.png,.pdf}
\Image*[width=1cm]{plot}
\Figure{plot}
\csuse{Image}{plot}
\csname Image\endcsname{plot}
```

通过静态输入文件和本地宏包定义的别名同样可用。图片目录和扩展名按实际调用位置的设置解析；路径尚未输入完整时也可补全。移动图片或主文档时保留调用形式、星号与选项，并核对移动后的实际图片目标。共享源码在不同主文档下解析不一致时，不猜测单一目标。

编译后由 recorder 发现的项目源码也参与图片调用分析，包括这些文件中的直接图片调用、静态别名和宏参数边界。相关图片可跳转、补全、跟踪依赖和随文件移动更新引用；仅被代码展示命令读取的文件不会因此安装宏定义。共享源码仍按各编译根文档分别解析。recorder 文件中的重定义或参数消费会阻止不可靠的图片引用编辑；搜索设置的执行顺序无法确认时继续跳过目标推断。

编译器确认缺图且源码回显能唯一对应调用时，诊断标出文件名，并按当前有效目录和扩展名提供拼写建议；修复只替换文件名。源码、别名含义或候选文件变化后，旧修复会重新校验。详细规则见[编译诊断](diagnostics.md)。

## 搜索设置

目录和扩展名声明也支持上述纯别名和静态命名调用：

```tex
\def\Paths{\graphicspath}
\Paths{{figures/}{plots/}}
\def\Extensions{\DeclareGraphicsExtensions}
\Extensions{.png,.pdf}
\csname DeclareGraphicsExtensions\endcsname{.pdf,.png}
```

服务器使用调用位置后面的参数，按源码顺序、分组和全局赋值规则跟踪设置。搜索优先级及作用域示例见[图片搜索与移动](graphics.md)。

## 支持边界

`\csuse` 需要文档加载提供该命令的宏包，例如 etoolbox。静态分析不执行任意宏展开：带参数的转发宏、`\let` 捕获的含义、复杂包装、动态命令名和含义不确定的重定义不作推断。作为其他宏参数消费、用于逐字展示或仅出现在未执行宏正文中的调用，不作为实际图片依赖。

无法确认的调用、参数不完整或格式错误、共享执行上下文冲突，以及 recorder 独有且执行顺序未知的设置，不推测单一图片目标。recorder 仍用于跟踪编译器实际读取的文件。构建有效性检查关注选中图片及优先于它的候选；增加低优先级图片不会使结果过期。
