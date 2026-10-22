# 寄存器与长度

[返回文档目录](README.md) · [符号与重命名](symbols.md) · [计数器与定理](counters.md)

## 声明与重命名

`newlength`、`newsavebox`、`newcount` / `newdimen` / `newskip` / `newmuskip` / `newtoks` / `newbox` 等寄存器分配，以及 `chardef` / `mathchardef`、寄存器绑定和字体声明也进入符号索引，支持变量补全、导航、悬停和跨文件安全重命名；识别直接名称和经 `expandafter` 展开的字面 `csname` 声明。常用长度操作、保存盒命令提供片段及参数提示；重命名会检查已有宏、寄存器、计数器派生命令和常用内置长度/寄存器原语的冲突。

## etoolbox 类型检查

以下命令提供片段、参数提示和名称补全。名称形式的静态首参数支持导航、引用、语义高亮和跨文件重命名；直接形式接受花括号包裹或裸控制序列。

| 检查的类型 | 名称形式 | 直接形式 |
| --- | --- | --- |
| 计数寄存器 | `ifcscounter{quantity}{then}{else}` | `ifdefcounter{\quantity}{then}{else}` |
| 长度（skip 寄存器） | `ifcslength{measure}{then}{else}` | `ifdeflength{\measure}{then}{else}` |
| 尺寸（dimen 寄存器） | `ifcsdimen{distance}{then}{else}` | `ifdefdimen{\distance}{then}{else}` |

`ifcscounter` 检查控制序列的类型。对于 `\newcounter{exercise}` 创建的 LaTeX 计数器，应使用 `ifcscounter{c@exercise}`；仅检查计数器名称是否存在时使用 `ifltxcounter{exercise}`。定理自有计数器的 `c@名称` 也参与导航和重命名。

名称补全允许选择普通宏等控制序列，因为检查结果为假也是合法用法。重命名维护首个名称参数，保留分支正文、同名普通文本、标签、注释与逐字内容。动态名称不作推断；共享文件在不同论文中指向独立声明时拒绝改名。

直接形式只检查首个控制序列，不执行它。将 `newcommand` 或 `verb` 作为被检查对象时，不会据此索引虚假的声明或开始逐字扫描；本文件显式重定义这些检查命令后，不再套用原有操作数规则。

## 长度表达式赋值

etoolbox 的 `deflength` 支持片段、参数提示和长度控制序列的导航与改名。接受裸控制序列或花括号包裹的控制序列，补全命令时保留已有操作数：

```tex
\usepackage{etoolbox}
\newlength{\measure}
\deflength\measure{5pt plus 2pt minus 1pt}
{\deflength{\measure}{\measure*2+1pt}} % 局部赋值
{\global\deflength\measure{\measure+2pt}} % 全局赋值
```

赋值使用 TeX 的 glue 表达式，保留伸长和收缩分量；服务器不求值。改名同步维护表达式中的寄存器引用，保留计算逻辑及局部、全局作用域。

## 未定义长度诊断与修复

etoolbox 报告 `Length '\measuer' undefined` 时，服务器会核对完整的 `deflength` 调用和日志源码回显，精确标出长度控制序列，附带 `undefined-length` 代码及长度名称数据。支持裸控制序列、花括号包裹的控制序列、跨行表达式，以及同一行的多次调用；修复保留表达式和赋值前缀。

拼写建议来自标准长度及当前项目中静态的 `newlength`、`newskip`、`newdimen`、`skipdef`、`dimendef` 声明，也识别字面 `csname` 声明。普通宏、计数/盒子/muskip 寄存器、重复声明和被其他类型覆盖的名称不作为长度候选；其他论文的声明不参与修复。

宏展开、动态或不完整操作数、命令被重定义、日志回显无法唯一确认时保留整行错误。精确诊断记录错误发生时的宏来源，提供修复前重新核对源码、候选类型及参数边界；支持版本化编辑和旧客户端，过期修复会被拒绝。

真实三引擎验证与默认回归命令见[测试说明](testing.md#寄存器与长度)，通用来源校验见[编译诊断](diagnostics.md#分析范围与来源确认)。
