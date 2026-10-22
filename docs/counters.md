# 计数器与定理

[返回文档目录](README.md) · [符号与重命名](symbols.md) · [编译诊断](diagnostics.md#命令环境与计数器)

## 补全与参数提示

计数器操作支持命令片段、参数提示和名称补全，包括 `setcounter` / `addtocounter`、`stepcounter` / `refstepcounter`、`value` / `arabic` 等格式化命令、`counterwithin` / `counterwithout` / `numberwithin`。名称候选来自内置计数器、当前项目的 `newcounter` / `newaliascnt` 和拥有计数器的定理声明；支持新计数器、别名及定理的共享/重置参数，保留已有名称后缀、空白和其他实参。不同论文之间的候选按依赖隔离，宏生成的名称不作推断。

## 导航与重命名

静态 `newcounter` / `newaliascnt` 声明支持跳转、悬停、引用、文档/工作区符号及跨文件重命名。重命名联动维护计数器操作、子计数器重置、定理共享编号、别名和格式化命令，包括字面 `csname`、`aliascntresetthe` 及别名元数据；同名的独立宏和 cleveref 的显示文字保持原样。名称冲突、共享引用在其他论文中指向独立声明、或同名定理与计数器对格式化命令的所有权有歧义时拒绝改动。

重命名静态 `\newtheorem{claim}{Claim}` 时，同步维护 `\newtheorem{lemma}[claim]{Lemma}`、`\newtheorem{fact}{Fact}[claim]`、`\newcounter{inner}[claim]`、常见计数器操作与 `counterwithin` / `counterwithout` / `numberwithin`，以及 `\claim`、`\endclaim`、`\theclaim` 等派生命令；支持字面 `\csname theclaim\endcsname`，不展开动态宏。共享计数器的定理保留底层计数器，无编号定理不修改独立计数器。新名称与计数器或派生命令冲突、或共享引用属于其他项目中的独立计数器时拒绝改动；存在直接派生命令调用时，新名称需能组成 TeX 控制序列。

## etoolbox 计数器操作

`defcounter{counter}{expression}` 和 `ifltxcounter{counter}{then}{else}` 的静态计数器名称支持补全、跳转、引用、重命名及语义高亮，两个命令均提供片段与参数提示。自定义计数器和拥有计数器的定理使用相同规则。

```tex
\usepackage{etoolbox}
\newcounter{exercise}
\defcounter{exercise}{2+3}
\ifltxcounter{exercise}{Exists}{Missing}
```

`defcounter` 默认局部赋值，支持 `\global` 前缀。重命名只维护计数器名称与相关引用，保留表达式、分支正文、同名独立宏、标签、注释和逐字内容。宏生成的动态名称不作推断；含语义空白的名称不与普通名称合并，显式补全可以将整个名称替换为候选。

编译报告 `defcounter` 使用了未定义计数器时，支持名称的精确定位及拼写修复。`ifltxcounter` 对不存在的计数器执行第二个分支，不作为未定义计数器错误的来源；同一行有检查和赋值时，修复只修改日志确认的赋值参数。

这两个命令没有星号形式，非法星号调用不提供计数器导航、重命名或名称补全；`counterwithin*` 和 `counterwithout*` 仍支持计数器操作。

## 标签赋值

`setcounterref` / `addtocounterref`、`setcounterpageref` / `addtocounterpageref` 分别识别首个计数器参数和第二个标签参数，提供各自的补全、导航、引用、重命名、编译诊断和拼写修复。同名计数器与标签仍属于不同的符号，修复和重命名只改动相应参数；支持命令片段、参数提示及跨行标签的补全。常用的带星号引用（如 `ref*`、`pageref*`）也支持名称补全和参数提示。

## 验证

etoolbox 的 `ifcscounter{c@名称}` 按控制序列检查底层寄存器类型；它与 `ifltxcounter{名称}` 的区别见[寄存器与长度](registers.md#etoolbox-类型检查)。

计数器与定理的真实编译、改名及拼写修复回归见[测试说明](testing.md#计数器与定理)。
