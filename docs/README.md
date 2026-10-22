# 文档

[返回项目 README](../README.md)

支持补全、导航、跨文件重命名、诊断、快速修复和后台编译。初次使用请先[安装服务器](install.md)，再[配置编辑器](editors.md)；生成 PDF 的配置见[编译指南](build.md)。

## 入门

- [安装与启动](install.md)：环境要求、构建、安装及进程协议。
- [编辑器配置](editors.md)：Neovim 配置、其他编辑器及示例。

## 编辑与项目

- [编辑功能](editing.md)：补全、导航、高亮和颜色。
- [文献](bibliography.md)：文献键、natbib、手写文献和条目关系。
- [biblatex 文献资源](biblatex-resources.md)：加载命令、路径编辑和静态支持范围。
- [biblatex 引用](biblatex.md)：字段、列表、姓名、单次、多组和卷号引用。
- [biblatex 文献分类](bibliography-categories.md)：分类分配、文献键编辑及重命名边界。
- [符号与重命名](symbols.md)：宏、环境、计数器等符号的支持范围。
- [项目与文件](projects.md)：工作区索引、根文件、依赖和路径解析。
- [图片解析与移动](graphics.md)：搜索目录、扩展名顺序、局部与全局设置和图片重命名。
- [图片文件名](graphics-filenames.md)：文件与搜索目录的引号、引擎差异、空白、路径补全和缺图修复。
- [图片命令的静态别名](graphics-calls.md)：图片调用与搜索设置的别名、命名调用和支持边界。
- [文件操作](file-operations.md)：文件移动协议、引用更新、文件 URL、创建和删除。
- [源码文件名与移动](source-filenames.md)：文件名空格、后缀保留和引用可表达性。

## 编译与诊断

- [编译](build.md)：命令、配置、通知、结果分析和产物检查。
- [编译输入与过期结果](build-inputs.md)：保存检查、取消、依赖追踪和产物有效性。
- [编译诊断](diagnostics.md)：错误定位、警告和拼写修复规则。
- [Unicode 输入错误](unicode-errors.md)：不支持的字符、码点摘要与定位边界。
- [特殊字符诊断与修复](special-characters.md)：字面特殊字符、多余对齐分隔符的精确定位及转义修复。
- [多余右花括号](extra-braces.md)：正文、数学及显式分组中的字符定位与删除修复。
- [字体诊断与结果分析](fonts.md)：字体和字号替换、缺字摘要及定位边界。
- [排版警告分析](typesetting.md)：Overfull / Underfull 分类统计与最大指标。
- [浮动体高度警告](floats.md)：过高图表的源码行、警告数量与最大超出量。
- [文献编译诊断](bibliography-diagnostics.md)：BibTeX / Biber 来源定位、缺失资源和文件名修复。
- [SyncTeX 双向查询](synctex.md)：源码与 PDF 的查询协议。

## 开发与测试

- [开发与测试](development.md)：检查命令和代码结构。
- [真实工具回归](testing.md)：外部程序要求、PDF 编译及编辑器测试命令。
