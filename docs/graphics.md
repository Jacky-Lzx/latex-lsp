# 图片解析与移动

[返回文档目录](README.md) · [项目与路径解析](projects.md)

## 搜索目录与文件移动

`\includegraphics` 还会搜索当前项目中 `\graphicspath{{figures/}{plots/}}` 声明的目录，支持图片跳转、子目录补全和文件重命名。移动图片目录或主文档时会同步更新目录声明，并尽量保留图片的简短名称和省略的扩展名。路径以当前编译主文档目录为基准，共享包里的声明也遵守这一规则。允许空格和中文，识别以 `/` 结尾的静态目录；移动主文档时也会重算尚不存在的目录。如果共享声明在不同项目中需要不同的替换路径，会拒绝该次文件移动编辑。这些目录声明不展开宏，也不模拟条件、局部作用域或多次声明的覆盖顺序。

未声明扩展名顺序时，图片省略扩展名会按默认顺序逐个搜索编译根目录及声明目录；支持的显式扩展名先按原名查找。例如根目录有 `plot.png`、`figures/` 有 `plot.pdf` 时，`\includegraphics{plot}` 选中 PDF，显式 `plot.png` 选中 PNG。子文件所在目录不会提前覆盖这些图片；以子文件自身为根时才使用它的目录。跳转、补全、输入跟踪及文件重命名共用这一搜索基准。

图片重命名会检查整批文件移动后的搜索结果，保留仍指向原图片的简写；简写将选中其他图片时，改用相对于新编译根目录的路径和实际扩展名。同批移动其他文件造成的同名遮蔽也会触发引用更新；无法安全保持目标时拒绝编辑，避免编译成功却使用了另一张图片。

## 扩展名声明

`\DeclareGraphicsExtensions{.png,.pdf}` 按声明顺序搜索省略扩展名的图片。每个扩展名都先搜索编译根目录，再搜索 `\graphicspath` 的目录，然后才尝试下一个扩展名。例如同时存在 `plot.pdf` 和 `plot.png` 时，这项声明使 `\includegraphics{plot}` 选中 PNG。

显式扩展名使用独立的图片格式规则：即使只声明 `.png`，`\includegraphics{plot.pdf}` 仍优先读取存在的 `plot.pdf`，不会因同时存在 `plot.pdf.png` 而换图。空声明 `\DeclareGraphicsExtensions{}` 关闭扩展名推断，显式文件名继续可用。

声明支持注释、空白、重复设置，以及普通花括号、`begingroup` / `endgroup` 和环境中的局部设置。服务器按源码顺序跟踪静态输入文件和本地宏包、文档类的设置；包和类仅加载一次，普通输入可重复执行。局部分组结束后恢复此前顺序。相同源码位置在不同执行上下文中需要不同扩展名顺序时，不提供单一图片目标。

## 静态别名与命名调用

扩展名声明支持无参数、正文仅含单个控制序列的纯别名，包括别名链，以及静态 `\csuse` / `\csname` 调用。例如：

```tex
\def\Extensions{\DeclareGraphicsExtensions}
\Extensions{.png,.pdf}
\csname DeclareGraphicsExtensions\endcsname{.pdf,.png}
```

服务器使用调用位置后面的参数，按相同的源码顺序和分组规则跟踪设置。通过静态输入文件和本地包定义的别名同样可用；`\csuse` 需要文档已加载提供该命令的宏包，例如 etoolbox。带参数的转发宏、`\let` 捕获的含义、复杂包装和动态命令名不作静态求值；无法确定的调用或 recorder 独有的设置不会推测单一图片目标。

## 全局设置与恢复

`\global\DeclareGraphicsExtensions{.png}` 的设置保留到分组外。`\globaldefs` 为正数时，扩展名声明自动全局生效；为负数时，显式 `\global` 也按局部赋值处理；为零时遵循前缀。全局赋值会更新外层分组应恢复的值，后续局部修改结束后恢复最近的全局设置。

```tex
\DeclareGraphicsExtensions{.pdf}
{\global\DeclareGraphicsExtensions{.png}}
\includegraphics{plot} % 优先 PNG
{\globaldefs=-1 \global\DeclareGraphicsExtensions{.pdf}}
\includegraphics{plot} % 仍优先 PNG
```

`\globaldefs` 自身的赋值按赋值前的作用域规则保存和恢复。前缀可跨静态输入文件传递；前缀和 `\globaldefs` 可通过已证明的无参数纯别名及静态 `\csuse` / `\csname` 调用。支持十进制、八进制、十六进制、ASCII 字符代码及多重正负号形式的字面整数。条件中的赋值、动态整数、可能继续展开为数字的宏，以及重定义的前缀或调用原语，会停止相关推断。

## 编译检查与支持边界

跳转、图片依赖、文件移动时的目标校验和缺失图片拼写修复共用上述顺序。编译输入检查只观察选中图片及优先于它的候选：选中 PNG 后，新建低优先级 PDF 不会使结果过期；删除 PNG 或新增更早的候选会使结果过期。

不执行任意宏展开。动态或重定义的设置、自定义 `\DeclareGraphicsRule`、无法确定的条件分支和选章配置会跳过相关静态推断；循环输入或超过分析上限时也停止推断。编译器的 recorder 记录继续用于跟踪实际读取的文件。文件移动的客户端调用顺序见[文件操作](file-operations.md)。
