# 图片文件名

[返回文档目录](README.md) · [图片解析与移动](graphics.md)

## 引号与空白

图片跳转、补全、依赖跟踪、文件移动和缺图修复支持 graphicx 的完整及分段双引号文件名。以下写法均指向 `plot image.png`：

```tex
\includegraphics{"plot image.png"}
\includegraphics{"plot image".png}
\includegraphics{"plot" image.png}
\includegraphics{ "plot  image.png" }
```

双引号用于包裹文件名，不属于磁盘文件名。普通空格、制表符和单次换行按 TeX 的规则合并，首尾空白被忽略；引号内也遵循这些规则。因此，`"plot  image.png"` 查找的是只有一个空格的 `plot image.png`。

省略扩展名的 `\includegraphics{"plot image"}` 仍按调用位置的图片目录和扩展名顺序搜索，详见[图片解析与移动](graphics.md)。[静态别名与命名调用](graphics-calls.md)使用相同规则。

## 补全、移动与缺图修复

补全支持尚未闭合的引号，以及跨单次换行的文件名。光标位于名称中间时会替换剩余名称；目录补全保留已有的路径后缀。

图片移动和缺图修复保留文件名引号、命令调用形式、星号及选项。需要修改分段引号写法时，可以将其整理为一对完整引号，例如移动 `plot image.png` 后，把 `"plot image".png` 更新为 `"assets/chart image.png"`。

编译器报告的文件名可能不带引号，诊断仍标出源码中的完整带引号名称；拼写建议沿用该图片调用的搜索目录和扩展名设置。只有编译器确认缺图且能可靠对应源码时才提供修复，详见[编译诊断](diagnostics.md)。

## 支持边界

完整路径中的引号必须成对。含任意宏展开、空段落或文件名内部注释的路径不作静态目标推断。未完成引号可用于输入时的补全，但不会作为已解析的图片依赖参与导航和文件移动。编译输入检查观察实际选中的图片及更高优先级候选，规则见[编译输入与过期结果](build-inputs.md)。
