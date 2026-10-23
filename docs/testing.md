# 真实工具回归

[返回文档目录](README.md) · [开发与测试](development.md)

## 环境与入口

默认 `cargo test --locked` 跳过需要外部程序的回归。安装 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、BibTeX、Biber、biblatex、logreq、SyncTeX 和 Neovim 0.10+ 后，可执行全部测试：

```sh
cargo test --locked -- --include-ignored
```

只安装部分工具时，按下面的命令选择对应测试。

## 编辑器语言识别

默认回归覆盖编辑器语言类型、非标准文件名、未命名文档与项目输入角色的优先级。真实 Neovim 回归需要 Neovim 0.11+，验证未保存的 `.data` 文件设置 `bib` 文件类型后，条目符号、增量编辑与补全均使用 BibTeX 解析：

```sh
cargo test --locked --test language_ids -- --include-ignored
cargo test --locked --test protocol language
```

biblatex 的 `.bbx`、`.cbx`、`.lbx`、`.dbx` 额外核对根目录扫描、动态监听注册、磁盘创建/更新/删除、未保存缓冲区优先与关闭恢复。字体定义 `.fd` 与 Babel 语言文件 `.ldf` 共用这些扫描和监听回归。显式输入回归检查项目作用域、路径补全、文件移动，以及 `.bbx.tex` 等同名后缀文件的原生优先顺序；`.fd` / `.ldf` 则核对精确文件优先。日志回归包含四种 biblatex 文件与两种字体/语言文件的嵌套上下文、缺失来源屏障、UTF-16 和编号定位，recorder 回归确认输入角色。

```sh
cargo test --locked --test language_ids --test path_completion
cargo test --locked --test protocol latex_and_biblatex_sources_
cargo test --locked --test generated_log real_corrupt_sources_ -- --include-ignored
```

最后一项需要 pdfLaTeX，在 `.aux`、`.bbl`、`.toc` 和六种支持源码中触发错误，分别验证传统与文件行号日志的实际来源。

## 文献字段诊断

重复字段诊断回归覆盖原生值结束行、后移的逗号和关闭符、同一行多个重复项及完整记录顺序、截断和歧义回退、大小写与引号键、UTF-16、CRLF 和来源角色。原生 BibTeX 确认首个值保留；stdio 回归检查 push/pull 诊断在保存增量编辑后移动及修复后清除：

```sh
cargo test --locked --test bibliography_field_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_fields_
```

## 语义高亮

语义高亮回归覆盖 BibTeX 类型、字段、字符串和数字操作数、运算符、关系键、注释及未完成输入。协议回归核对扩展 legend、UTF-16 范围查询、字段/值/注释的连续增量编辑、delta 还原、未变化结果和重开文档：

```sh
cargo test --locked --test semantic
cargo test --locked --test protocol semantic_token
```

自定义字段名回归检查合法标点和 Unicode、完整字段补全范围、后续元数据和关系引用、缩写导航与重命名；原生测试使用自定义 `.bst` 读取这些字段：

```sh
cargo test --locked --test bibtex_field_names -- --include-ignored
cargo test --locked --test protocol bibtex_custom_field_names
```

## 补全

BibTeX 自定义字段回归覆盖跨文献资源候选、项目隔离、本地拼写优先、大小写去重和排序，以及含 `$`、反斜杠、中文与 emoji 的完整字段替换。真实 Neovim 验证片段展开后的字面名称，原生 BibTeX 验证补全保留已有赋值及后续字段；需要同时安装两个工具。协议测试检查未保存的资源增量编辑、打开文件期间的磁盘通知、关闭后恢复磁盘候选和客户端片段能力：

```sh
cargo test --locked --test bibliography_completion -- --include-ignored
cargo test --locked --test protocol custom_bibliography_field_completions
```

命令名操作数回归覆盖内置与跨文件用户命令、声明目标、复制来源、`meaning`、空命令前缀、尚未闭合的名称分组、注释续行、词中替换及 UTF-16 范围；核对未完成输入不会产生空名称声明，注释、逐字和文字化内容不提供命令补全，同时确认普通调用、`futurelet` 继续执行的命令和本文件显式重定义后的调用仍提供参数片段：

```sh
cargo test --locked --test operand_completion
```

参数数量回归检查注释、CRLF、注释拆开的数字、可选默认值及非法和动态数量，核对跨文件宏、环境和成对定界符的签名与补全。外部回归在三个引擎中比较宏、环境与定界符跨文件重命名前后的实际参数值，并保留数量注释：

```sh
cargo test --locked --test signature commented_argument_counts -- --include-ignored
```

默认值预览回归覆盖传统宏/环境与 xparse 默认值，核对注释消除、转义百分号、显式控制空格、控制词边界、Unicode 和长注释。三个引擎使用 `\ifx` 比较原始默认值与预览文本的 token 身份，同时检查参数提示和补全详情一致：

```sh
cargo test --locked --test signature optional_default_previews -- --include-ignored
```

## 重命名

标签、命令、环境、计数器与 PDF 目标的回归覆盖跨文件编辑、注释保留、名称冲突和特殊字符边界。真实编译还核对连续改名后的 PDF 链接、书签和引用值；组合目标覆盖空名称及配对或不配对的方括号，并保留同名普通标签。

以下测试需要当前 LaTeX 发行版、pdfLaTeX、latexmk，以及 hyperref、mathtools、refcount 和 aliascnt：

```sh
cargo test --locked --test rename --test bracket_labels --test counter_brackets --test counter_rename --test commented_labels -- --include-ignored
```

源码文件移动的三引擎回归见[排版日志与文件移动](#排版日志与文件移动)，支持范围见[标签与交叉引用](labels.md)、[PDF 目标与书签](pdf-destinations.md)和[符号与重命名](symbols.md)。

## 计数器与定理

默认测试检查跨文件导航与改名、补全、参数提示、语义高亮、名称冲突及非法星号和动态名称的排除：

```sh
cargo test --locked --test etoolbox_counters --test counters --test counter_rename --test counter_brackets --test counter_fixes --test undefined_counters --test shortened_counter_context --test theorem
```

etoolbox 的真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox。在三个引擎中比较普通计数器与定理改名前后的局部赋值、全局赋值和存在性检查，覆盖含 `]` 的新名称；同时用传统与文件行号日志验证未定义计数器的精确定位，应用拼写修复后重新编译。检查分支正文、表达式和同名独立符号不被误改：

```sh
cargo test --locked --test etoolbox_counters -- --include-ignored
```

支持范围与示例见[计数器与定理](counters.md)。

## 寄存器与长度

默认回归覆盖 etoolbox 寄存器类型检查的名称导航与改名、派生计数器名称、补全、参数提示、操作数与分支边界，以及共享文件歧义。真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox，逐次改名长度、尺寸、计数寄存器、LaTeX 计数器、定理和普通宏，核对类型判断结果、局部/全局赋值及 glue 的伸长和收缩分量保持一致：

```sh
cargo test --locked --test etoolbox_registers
cargo test --locked --test etoolbox_registers -- --include-ignored
```

操作示例与静态分析边界见[寄存器与长度](registers.md)。

## 未定义长度诊断

[长度诊断与修复](length-diagnostics.md)回归检查 `deflength` 的精确范围、裸控制序列及跨行表达式、重复与缩短源码回显、候选寄存器类型、项目隔离及版本化和旧客户端修复。外部回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox，使用三个引擎的 60 / 79 列日志，在传统与文件行号格式中修复自定义长度、尺寸和内置长度，并核对重新编译后的实际值：

```sh
cargo test --locked --test undefined_lengths
cargo test --locked --test undefined_lengths -- --include-ignored
cargo test --locked --test protocol undefined_length_diagnostics_offer_versioned_fixes_and_reject_changed_candidate_types
```

## boolean 诊断

默认回归检查 etoolbox 五种直接操作的基础名称定位、跨行与 UTF-16 范围、重复和截断源码回显、声明及派生命令冲突、项目隔离、只读内置条件、版本化及旧客户端修复。外部回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX、ifthen 和 etoolbox；在三个引擎的 60 / 79 列及两种日志格式中修复后重新编译，核对局部、全局赋值、分支与同名 toggle 的独立性：

```sh
cargo test --locked --test undefined_booleans
cargo test --locked --test undefined_booleans -- --include-ignored
cargo test --locked --test protocol undefined_boolean_diagnostics_offer_versioned_fixes_and_reject_changed_candidate_namespaces
```

支持范围见[boolean 诊断与修复](boolean-diagnostics.md)。

## toggle 诊断

默认回归核对五种直接操作的实际参数读取边界、UTF-16 和跨行定位、重复及缩短回显、候选命名空间、项目隔离、生成命令冲突及过期修复。真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox，分别使用 60 / 79 列和两种日志格式，应用修复后重新编译并核对局部、全局赋值及分支结果：

```sh
cargo test --locked --test undefined_toggles
cargo test --locked --test undefined_toggles -- --include-ignored
cargo test --locked --test protocol undefined_toggle_diagnostics_offer_versioned_fixes_and_reject_changed_candidate_namespaces
```

支持范围见[toggle 诊断与修复](toggle-diagnostics.md)。

## 布尔表达式诊断

默认、stdio 协议及三引擎回归的命令与验证范围见[布尔表达式验证](boolean-expressions.md#验证)。真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox；功能边界见[布尔表达式诊断与修复](boolean-expressions.md)。

## 字符串测试

字符串测试的数据边界、编辑器协议及三引擎验证命令见[字符串测试验证](string-tests.md#验证)；导入重定义、共享文件、增量编辑、关闭恢复和磁盘变更的回归见[跨文件上下文验证](string-test-projects.md#验证)。副本直接调用、参数数量变更、宏名与 boolean 重命名的回归见[字符串副本验证](string-test-copy-calls.md#验证)。条件文件钩子的分支选择、执行顺序及非源码文件通知回归见[条件文件钩子验证](conditional-string-tests.md#验证)。撤销操作数、重新定义、共享上下文和三引擎回归见[撤销定义验证](string-test-undefinitions.md#验证)。外部回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox。

## 排版日志与文件移动

页数回归检查输出路径中的伪页数、PDF / DVI / XDV 格式、引号、日志折行、不完整摘要和读取预算。外部回归使用三个 LaTeX 引擎的 60 / 79 列日志，核对编译结果返回实际 PDF 页数：

```sh
cargo test --locked --test page_counts -- --include-ignored
```

空文档回归检查标准与 LuaTeX 的无页面报告、成功退出时的旧 PDF 保留、日志与管道优先级及日志省略边界。外部回归在三个引擎的 60 / 79 列日志中连续生成 PDF、编译空文档、恢复正文，核对结果不会复用旧产物，且重新加入正文后可以成功编译：

```sh
cargo test --locked --test no_pages -- --include-ignored
```

信息日志回归检查五类标准 `Info` 消息、文件名括号、字节折行、空白与宏包续行、较长消息、真正的文件切换、折行错误路径和日志省略边界。明确属于续行的错误与警告示例、源码回显、资源及 PDF 输出文字不会发布为诊断。外部回归在三个 LaTeX 引擎的 60 / 79 列输出及传统 / 文件行号格式中核对诊断数量、警告归属、错误行号和未定义命令的精确范围：

```sh
cargo test --locked --test information_logs -- --include-ignored
```

标准重编译警告回归覆盖 LaTeX、natbib、longtable 与 hyperref 的 60–82 列逐字节折行、LF / CRLF、行尾空格、第一句后的续行、额外说明和相邻文件错误。三引擎回归使用 60 / 63 / 79 列日志，分别重命名跨文件标签和文献键，或修改表格后续分块的列宽，核对请求重新出现、实际 PDF 页数及收敛后的清空。协议回归检查完整警告、两种客户端操作响应、最终日志优先级和旧操作失效：

```sh
cargo test --locked --test rerun_warnings
cargo test --locked --test rerun_warnings -- --include-ignored
cargo test --locked --test protocol wrapped_standard_rerun
```

[末页变更回归](last-page.md)在三个引擎的 60 / 70 / 79 列日志中连续缩短和加长跨文件正文，核对临时额外页、末页钩子的旧页与正确页、实际 PDF 页数，以及再次编译后的请求和操作清除：

```sh
cargo test --locked --test rerun_warnings real_last_page -- --include-ignored
```

rerunfilecheck 回归核对标准 `Rerun.` 请求、hyperref 完整书签及反向文献引用提示、折行、满行后的额外空行、示例文字和 makeindex 请求的边界，并检查结果分析与重编译操作一致。默认回归另核对文件名中的括号、看似完整的警告正文及不完整警告之后的真实文件切换。外部回归在三个 LaTeX 引擎的 60 / 79 列日志中验证特殊文件名、后续主文件与子文件的诊断归属、首次文件变化后的请求及下一遍内容不变后的清空：

```sh
cargo test --locked --test rerunfilecheck -- --include-ignored
```

反向引用回归先生成稳定文献引用，再将引用移至下一页，核对三引擎的页数、重新出现的请求及再次收敛后的清空；索引和术语表的真实日志回归确保 makeindex 说明不会生成单纯重跑 TeX 的操作。stdio 回归覆盖标准 CodeAction / 旧式 Command 客户端、完成通知与状态一致、最终日志优先级、诊断清除和旧菜单操作失效：

```sh
cargo test --locked --test protocol backreference_rerun
```

声明操作数回归使用三个 LaTeX 引擎验证复制 `newcommand`、`newif`、`newenvironment` 后的普通调用，以及宏、开关和环境重命名前后的输出一致；同时验证直接与静态 `csname` 目标复制字面花括号，以及未执行的宏正文和参数分隔文本。默认测试另检查各类符号索引、命令名操作数的参数提示与名称补全、静态 `csname`、逐字边界和标签编辑范围，并核对花括号复制不会产生错误修复、真实不配对仍保留精确位置：

```sh
cargo test --locked --test operand_declarations -- --include-ignored
```

`meaning` 的默认回归检查声明与逐字扫描边界、字面花括号、参数提示和标签编辑排除、直接与静态名称的跨文件导航及重命名，并验证依赖候选不会混入复制来源后的普通文本。上面的三引擎回归同时检查查看 `newcommand`、普通宏和 `verb` 后的执行结果、宏正文、参数分隔文本及本文件改写 `meaning` 后的实际调用；重命名前后比较输出。项目上下文的根文件和依赖规则另有回归，覆盖声明目标、复制来源和含义不确定的查看操作：

默认单元测试另核对命令 token 的文字化边界，以及跨文件或不确定改写 `meaning` 时，宏包、文档类和文献候选文件的变化仍会使编译结果过期。

```sh
cargo test --locked --test meaning_operands
cargo test --locked --test quoted_roots --test source_dependencies --test file_macro_operands
```

`futurelet` 重命名回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX，比较改名前后的前瞻判断与继续执行结果，覆盖直接目标、静态 `csname` 目标和中文新名称。默认测试检查跨文件编辑、非法目标、命令继续加载文件及签名边界；stdio 回归核对 UTF-16 名称范围和文档版本。命令片段另由真实 Neovim 展开验证：

```sh
cargo test --locked --test futurelet -- --include-ignored
cargo test --locked --test protocol futurelet_targets_return_versioned_cross_file_renames_over_stdio
cargo test --locked --test snippets -- --include-ignored
```

Unicode 输入错误回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX。在 60 / 79 列日志及传统 / 文件行号格式中核对子文件字符、重复位置、emoji 和零宽连接符，验证 pdfLaTeX 显式映射后的清空，以及 Unicode 引擎的缺字分类：

```sh
cargo test --locked --test unsupported_unicode -- --include-ignored
```

特殊字符修复回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX。在 60 / 79 列日志中检查子文件中的裸 `&` / `#` / `_` / `^`，应用快速修复后重新编译，核对合法表格、数学上下标、转义字符、注释和逐字内容保持原样。多余对齐分隔符的回归另覆盖 `tabular`、数学 `array`、原始 `halign`、传统与文件行号日志、展开的内部模板帧及宏参数排除：

```sh
cargo test --locked --test special_character_fixes -- --include-ignored
```

其中多余右花括号的回归覆盖普通文本、数学模式、`begingroup` 和 `left` 分组，在三个引擎、60 / 79 列日志、传统与文件行号格式、默认与展开上下文中验证单字符删除与重新编译，同时保留合法分组、转义字符和逐字内容，排除宏展开来源。单独运行：

```sh
cargo test --locked --test special_character_fixes extra_closing -- --include-ignored
```

重复上下标回归在三个引擎、60 / 79 列日志、传统与文件行号格式及默认与展开上下文中核对行内、显示、分组和注释续行公式。应用空基底操作后重新编译，同时检查隐藏宏展开帧时的裸参数与分隔参数不会被修改：

```sh
cargo test --locked --test special_character_fixes double_script -- --include-ignored
```

数学定界符修复的来源、协议和三引擎回归见[多余的右定界符](right-delimiters.md#验证)、[缺失的右定界符](missing-right-delimiters.md#验证)和[缺失的定界符参数](delimiter-operands.md#验证)。

跨文件宏来源回归需要这三个引擎及 etoolbox，在 60 / 79 列及传统 / 文件行号日志中验证裸参数、分隔参数、重复声明和别名转发不会误获转义编辑。父文件声明的传统与 xparse 环境、可选参数和 `\begin` 别名也参与验证；零参数或已完成参数之后的实际分隔符在修复后重新编译。复制回归另覆盖 `let`、内核复制命令和 etoolbox 复制接口，验证来源后续重定义、环境副本、捕获的 `begin` 和来源操作数不会误作为调用。默认测试另覆盖加载时序、暂停父文件、动态来源、编译中源码变化和宏文件变化后的旧修复拒绝；协议测试同时覆盖 Code Action 和旧客户端的重复声明、别名及复制变化：

```sh
cargo test --locked --test literal_macro_scopes -- --include-ignored
cargo test --locked --test protocol literal_build_fixes_recheck_cross_file_macro_changes_without_a_main_version_change
```

源码文件名移动回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX，验证首部空格、主干末尾空格、裸路径和引号路径，以及移动后的真实输入与导航：

```sh
cargo test --locked --test source_filename_renames -- --include-ignored
```

NFSS 字体声明与选择的默认回归核对六类声明和七类带参数选择命令的所有参数、注释与嵌套设置、`\DeclareMathSizes` 的有星号和无星号形式、片段协商、保留已有参数及用户签名覆盖。另检查无参数 `\selectfont` 及其别名后的命令边界、文件错误修复及本地重定义。stdio 回归使用 `.fd` 文档验证编辑器实际请求；外部回归填充实际补全片段，在 pdfLaTeX、XeLaTeX 和 LuaLaTeX 中编译，核对选中的 Computer Modern 字体、编码、字形、数学字号以及行距因子应用后的基线距离：

```sh
cargo test --locked --test nfss
cargo test --locked --test protocol nfss_declarations_
cargo test --locked --test nfss real_completed_nfss_ -- --include-ignored
```

字体替换回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 Computer Modern 字体，在 60 / 79 列日志中验证长字体名、字体与字号替换，以及修复后摘要清空。缺字回归需要 XeLaTeX：

```sh
cargo test --locked --test font_substitutions -- --include-ignored
cargo test --locked --test missing_characters -- --include-ignored
```

真实盒子警告回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX，使用较窄的日志输出宽度验证 Overfull 警告的跨行恢复及完整段落范围；结果分析回归另核对四类指标和排版修复后的清空行为：

```sh
cargo test --locked --test box_warnings real_ -- --ignored
cargo test --locked --test box_analysis -- --include-ignored
```

浮动体过高回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX，在 60 / 79 列日志、传统和文件行号格式中核对子文件图表的警告行、数量及最大超出量，缩小内容后重新编译并确认摘要清空。默认测试另覆盖小数点处折行、相邻诊断边界、精确十进制比较及数值预算：

```sh
cargo test --locked --test oversized_floats -- --include-ignored
```

真实文件 URL 重命名回归需要 pdfLaTeX 和 hyperref，验证改名前后生成的 PDF 仍保留查询参数、页面目标及四参数链接的目标：

```sh
cargo test --locked --test file_operations real_file_url_renames -- --ignored
```

真实图片扩展名声明回归需要 pdfLaTeX、graphicx 和 etoolbox，验证 PNG 优先、显式 PDF、局部与全局作用域，以及图片和主文档移动后的编译输入：

```sh
cargo test --locked --test graphics_extensions real_ -- --ignored
```

全局设置回归覆盖 `\global`、正负 `\globaldefs`、嵌套分组、声明与前缀的静态别名和命名调用，以及字面整数，逐项核对导航、改名前后实际读取的图片。目录作用域回归另验证多次覆盖、局部恢复、全局赋值、静态别名，以及图片目录和主文档同时移动前后的真实编译输入：

```sh
cargo test --locked --test graphics_paths real_ -- --ignored
```

图片调用别名回归同样需要 pdfLaTeX、graphicx 和 etoolbox，验证纯别名、静态命名调用、图片与主文档同时移动后的 recorder 输入，以及缺图定位、拼写修复和重新编译。另验证宏展开后动态载入的源码在 recorder 发现后恢复图片导航、依赖与改名，并在移动图片后继续编译成功：

```sh
cargo test --locked --test indirect_graphics real_ -- --ignored
```

带引号的图片路径回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX、graphicx 和 etoolbox，验证完整与分段引号、空白合并、图片移动后的 recorder 输入，以及缺图定位、拼写修复和重新编译。搜索目录回归另核对引擎对目录引号的差异、目录与主文档同时移动后的实际图片，以及缺图修复后的再次编译：

```sh
cargo test --locked --test quoted_graphics real_ -- --ignored
```

默认回归另覆盖完整和未完成路径的别名补全、跨输入文件与本地宏包的别名、recorder 来源中的宏定义与参数边界、仅作代码展示的文件排除、共享根文件冲突、未执行和不确定调用的排除，以及目录补全和缺图建议的作用域、共享源码歧义、候选图片的过期检查及版本化 stdio 文件移动。

## listings 代码

`listings_environments` 覆盖头部选项、名称分组、注释续行及首行尾部中的伪关闭标记，核对正文和关闭行尾部的符号、依赖、版本化重命名及文件移动；另覆盖活动换行、CRLF、Unicode、精确关闭字符串、折叠列、联动编辑、高亮和未闭合修复。头部签名回归覆盖完整及未完成选项、注释和参数边界、项目重定义、内部读取器覆盖与未保存设置刷新。协议回归增量修改头部及真实关闭标记，检查诊断、符号、参数提示与编辑范围的刷新：

`listings_inline` 覆盖 `*` 与 Unicode 分隔符、花括号的首个字面右括号、头部空白及注释续行、头部命令索引、行尾恢复、重定义、代码隔离、补全片段、已有代码保留、悬停、参数提示与高亮。另覆盖已知宏参数、稳定的命令复制及复制链、纯静态转发、宏定义正文中的嵌套花括号、转义、注释与跨行代码，以及普通作用域、零参数命令、被读取为参数的控制词和未知宏的反例。协议回归在星号、花括号、带选项、缺少结束符及宏参数包装、复制宏的参数语法之间增量切换，核对版本化重命名范围和逐字隔离；原生编译核对预分词与普通正文的边界差异。

```sh
cargo test --locked --test listings_inline --test listings_environments
cargo test --locked --test protocol listings_environment_
cargo test --locked --test protocol listings_inline_
```

安装 pdfLaTeX、XeLaTeX 和 listings 后，为第一个命令添加 `-- --include-ignored`；原生回归核对标签重命名前后的实际编译输入、aux 标签、头部提示、伪关闭标记及正文结束规则。行内代码回归另核对头部命令的实际执行，以及保留原生行尾错误时后续标签的恢复。

## FancyVerb 环境参数提示

默认回归覆盖十种环境的选项、保存名称和输出文件提示，包含未完成参数、注释续行、CRLF、Unicode、首个未受花括号保护的右方括号、无选项时的活动换行、项目重定义与未保存设置刷新。协议回归逐次输入头部、完成参数并添加正文和关闭行尾部，核对提示及依赖隔离：

```sh
cargo test --locked --test fancyvrb_environments
cargo test --locked --test protocol fancyvrb_environment_header_
```

安装 pdfLaTeX、XeLaTeX 和 fancyvrb 后，为第一个命令添加 `-- --include-ignored`；原生回归在实际编译及版本化标签重命名场景中核对头部提示与逐字正文边界。

## minted 代码

minted 的默认回归覆盖内联逐字代码、已知宏参数和复制宏中的预分词边界、普通作用域及零参数包装反例、跨行和 Unicode 定界符、环境头部的完整与未完成参数提示、注释及可选参数边界、项目重定义与未保存设置刷新，以及文件补全与移动、数据/源码角色、全局设置保护和缺失文件修复。协议回归检查头部参数的逐次增量输入、代码与关闭行尾部的提示隔离，以及代码文件内容变化和构建中路径编辑对 PDF / SyncTeX 历史的影响：

```sh
cargo test --locked --test minted --test minted_environments --test minted_inputs --test missing_minted_inputs
cargo test --locked --test protocol minted_inline_
cargo test --locked --test protocol minted_environment_
cargo test --locked --test protocol static_code_
cargo test --locked --test protocol in_flight_code_path_
```

安装 pdfLaTeX 与 minted v3 后，为四个测试目标添加 `-- --include-ignored`。内联代码使用关闭 shell escape 的逐字模式，预分词回归另用 XeLaTeX 核对注释、转义、嵌套分组及标签重命名后代码中的伪命令和文件包含不执行；路径、环境及双模式缺失文件修复回归另需 PATH 上的 `latexminted`。高亮回归使用受限 shell escape 和立即高亮模式，核对生成文件中的独立内容标记，确认 Python 实际读取的空格及 Unicode 文件名。环境回归核对头部及正文的伪结束标记、逐字与高亮模式、结束行尾部的原生错误、版本化标签重命名及实际 aux 输出。支持范围见 [minted 代码](minted.md)。

## 文献与编译诊断

动态集合回归核对声明与成员的独立重命名、含引号的新键、注释与未完成列表补全、两个参数的提示、命令片段、语义声明与选择范围、项目隔离、同名及新键冲突、错误形式和缓冲区更新。真实 Biber 通过普通模式控制文件处理索引中的集合，比较集合键、成员键依次重命名后的完整 `.bbl`；此测试不运行 biblatex 宏。stdio 覆盖旧客户端与版本化编辑、范围移动、动态名称移除声明、关闭文档后恢复磁盘定义及成员编辑：

```sh
cargo test --locked --test biblatex_sets -- --include-ignored
cargo test --locked --test protocol bibliography_dynamic_sets_
```

扩展姓名组成部分回归覆盖完整模板、字段与条目边界、Unicode 模型字段、NFD 键、字面值与宏表达式、字段重复和模型碰撞、映射字段、来源顺序和失效、超限缓存及无猜测修复。真实 Biber 验证 AUTHOR 与 TRANſLATOR、Unicode 未知组成部分、范围移动及修复清除、字符串宏、模型碰撞和 source map；只需 Biber。stdio 检查推送和拉取、增量移动、宏表达式范围、映射字段的键回退和修复清除：

```sh
cargo test --locked --test biber_namepart_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_nameparts_
```

Biber 集合回归覆盖三种原生模板、字段名的 Unicode 模型匹配、动态值、重复及不完整字段、来源历史顺序和失效、规范化键歧义、超限来源与无猜测修复。真实 Biber 测试从安装的 `--tool-config` 及其 `Constants.pm` 构造匹配版本的 `.bcf`，移除工具默认 source map 后在普通模式运行，验证原生 `.blg` 和 `.bbl`、范围移动、修复清除、模型名称碰撞及 source map 生成的字段；只需 Biber，不依赖 `biblatex.sty`。stdio 检查推送和拉取、增量编辑、字段与键回退、警告下成功构建及修复清除：

```sh
cargo test --locked --test biber_set_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_sets_
```

Biber 循环继承回归核对致命错误发生前的来源、跨库读取顺序、超限来源、错误后的字段警告回退、后续输入无法恢复历史、规范化键歧义、字段回退、自引用、相关位置上限和完整模板。真实 Biber 验证普通与 XDATA 的双条目循环、自循环、字段级 XDATA、宏、source map 及别名引用，比较源码移动和打断循环后的结果。stdio 同时验证推送和拉取、增量移动、动态字段与字段级 XDATA、后续警告来源失效、失败构建和修复后的清除及成功状态：

```sh
cargo test --locked --test biber_cycle_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_inheritance_cycles_
```

Biber 别名冲突回归覆盖三种完整模板、静态与动态字段、跨库读取顺序、NFD 与大小写、重复所属键及候选上限、历史失效，以及单行 1500 条混合 Unicode 别名冲突的全部 UTF-16 范围。真实 Biber 验证两种条目顺序下三类冲突的范围移动与修复清除，以及宏展开别名的主文档回退和 source map 移走 IDS 后的原始范围；stdio 同时检查推送和拉取、增量移动、动态别名回退到主文档、相关位置和无猜测修复：

```sh
cargo test --locked --test biber_alias_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_alias_conflicts_
```

biblatex `ids` 别名回归覆盖静态列表、ASCII 字段名匹配、主键与别名独立重命名、歧义拒绝、项目隔离、UTF-16/CRLF、语义声明及增量更新。真实 Biber 使用 `--tool --output-resolve-crossrefs` 核对别名和主键重命名后子条目仍继承同一标题，并验证双引号列表改为含引号的新别名；只需 Biber。stdio 同时检查旧客户端及版本化编辑、范围移动、动态字段移除索引和关闭文档后恢复磁盘内容：

```sh
cargo test --locked --test bibliography_aliases -- --include-ignored
cargo test --locked --test protocol bibliography_ids_aliases_
```

文献日志记录边界回归覆盖输入路径、源码回显和 Biber 消息中的级别/行号样式文本，另核对实际 `INFO` 记录才可建立数据库角色。条目类型诊断回归验证类型与键的对应关系、分行头、歧义、来源角色和 UTF-16；真实 BibTeX 测试在修正类型后重新编译，协议用例覆盖推送与拉取诊断：

```sh
cargo test --locked --test bibliography_log_records --test bibliography_type_diagnostics -- --include-ignored
cargo test --locked --test biber_data_sources biber_database_roles_
cargo test --locked --test protocol bibliography_type_warnings_
```

BibTeX 字符串缩写的默认回归覆盖值表达式、补全、大小写、UTF-16、项目隔离、共享引用和安全重命名。真实工具回归只需 BibTeX，比较跨数据库、preamble 和自引用在重命名前后的完整 `.bbl` 内容：

```sh
cargo test --locked --test bibliography_strings -- --include-ignored
cargo test --locked --test protocol bibliography_abbreviations_
```

未定义字符串缩写回归覆盖两行日志、精确范围、输入顺序、无效声明、歧义位置和旧修复失效。真实 BibTeX 测试在修复后重新编译并检查警告消失及 `.bbl` 内容；stdio 测试同时验证推送/拉取诊断、版本化编辑与旧客户端的 `workspace/applyEdit`：

```sh
cargo test --locked --test bibliography_string_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_string_warnings_
```

重复文献条目测试核对大小写等价键、分行键头、同一行歧义、数据库顺序、来源角色及相关位置上限。真实 BibTeX 回归验证实际报错的行号与跨库原始位置，stdio 测试覆盖推送和拉取诊断中的精确范围与 `relatedInformation`：

```sh
cargo test --locked --test bibliography_duplicate_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_entries_
```

Biber 字段诊断回归覆盖无效字段、条目类型限制、日期与日期范围、数据类型、边界及模式约束、ISBN/ISSN/ISMN、完整特殊文件名、数据库角色、源码回显、级别与记录完整性。另核对字面值、空值、字符串拼接、字段归属、继承与歧义回退、Unicode 规范化碰撞、CRLF、缓存更新和单行 6000 条诊断的全部范围。输入及模型字段匹配另有单行 1500 个条目、7500 条混合诊断逐项核对，覆盖完整 Unicode 折叠、原始拼写分别定位及碰撞后的键回退。无文件名的默认年份、月份、ISBN 和畸形范围警告额外核对完整此前历史、跨库规范化键唯一性、不可读候选、读取及来源数量上限、符号链接去重，以及值内类似日志文字的边界。真实 Biber 使用最小自定义模型验证组合/分解字符、ß/ẞ、两种 sigma、带点/无点 I 和连字，核对值移动、修复清除、模型字段碰撞及未知字段独立定位；默认模式额外核对 `IſBN` 别名及其与 `ISBN` 的碰撞、月份修复，以及 `PAGES` / `Pageſ` 畸形范围的独立原始值定位。stdio 同时验证输入及模型命名空间的推送和拉取、增量保存、警告模式切换、默认字段五项并列警告清除与成功构建：

```sh
cargo test --locked --test biber_field_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_field_warnings_
cargo test --locked --test protocol biber_unqualified_field_warnings_
```

Biber XDATA 回归覆盖三种完整模板、section 边界、原生级别、静态列表精确项、宏及拼接、继承、重复字段与规范等价目标、未闭合条目、此前历史及跨库键唯一性。单行 1500 个条目中的 4500 个 XDATA 引用与 1500 条年份警告混合验证全部 UTF-16 范围。真实 Biber 核对缺失目标的两条消息、错误目标类型、列表项移动、补齐后的清除，以及实际宏展开、source map、重复字段和目标的键回退。stdio 验证推送、拉取、保存后的移动、动态字段回退和修复清除：

```sh
cargo test --locked --test biber_xdata_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_xdata_warnings_
```

字段级 XDATA 测试覆盖四种原生消息、字段与键边界、原生源位置文本、单一字面值和完整表达式、输入及模型命名空间区别、规范等价字段碰撞、继承/映射/不完整字段回退和历史失效。单行 1500 个条目、6000 条混合字段警告逐项验证 UTF-16 范围，包含 `Straße` 与 `STRASSE` 的独立位置。真实 Biber 验证姓名列表和普通列表的位置修复、源字段补齐、类型修复、值移动和清除，并确认宏原始表达式、source map 生成字段及自定义模型的定位。stdio 覆盖四种警告并列、推送与拉取、增量移动、映射字段回退、保存后清除及成功构建：

```sh
cargo test --locked --test biber_xdata_field_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_granular_xdata_warnings_
```

Biber 关系引用回归覆盖四种原生关系模板、section 与级别边界、单值和列表目标、同目标跨字段的独立范围、动态/继承/重复字段回退、此前输入顺序、缓存建立后的历史失效及规范化键碰撞。单行 1500 个条目的 6000 条普通关系警告与 1500 条 XDATA 警告逐项核对 UTF-16 位置。真实 Biber 验证四种关系的目标移动和补齐清除，以及字符串宏和实际 source map 的键回退；stdio 同时核对四类诊断的推送、拉取、增量保存、动态字段回退和成功构建状态：

```sh
cargo test --locked --test biber_relation_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_relation_warnings_
```

Biber 姓名回归覆盖三种原生模板、原生级别、规范化键唯一性、特殊键引号、姓名中的文件名及类似后缀文字、此前输入顺序、来源不可读或超限、不猜测姓名修复，以及单行 3000 条混合错误和警告的全部 UTF-16 键范围。真实 Biber 验证两个姓名错误之后的年份警告仍能精确定位，并核对 LaTeX 解码后的姓名、连续 `and`、Unicode 自定义模型姓名字段、范围移动和修复清除。未知或不完整错误继续使后续历史失效。stdio 同时核对推送和拉取、两个错误后的字段位置、键移动、仅警告时的成功状态及全部清除：

```sh
cargo test --locked --test biber_name_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_name_errors_
```

Biber 日期覆盖回归核对被覆盖的原始值与关联日期各自的唯一性、完整模板、此前输入历史、规范化键碰撞、继承边界、无自动删除或改写，以及单行 1500 个条目、3000 条警告的主范围及关联范围。真实 Biber 对两种字段顺序和日期范围分别验证移动及分量一致后的清除，实际 source map 验证日期、年份或两者改名后的原始范围回退。stdio 同时验证推送和拉取、增量编辑、关联日期消失、原始年份缺失时的键回退及修复清除：

```sh
cargo test --locked --test biber_overwritten_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_overwritten_fields_
```

Biber 条目类型回归核对原生记录完整性、特殊文件名、精确条目归属、规范化键碰撞、未闭合正文、未确认及不可读来源、类型不一致时的键回退和无自动 `misc` 替换。Unicode 折叠覆盖组合/分解形式、ß/ẞ 展开、希腊两种 sigma、带点与无点 I、连字，单行 3000 个条目和 6000 条类型及字段警告逐项验证原始 UTF-16 范围。真实 Biber 逐项验证范围移动、类型修复清除，并使用实际 source map 将有效 `book` 改为无效 `boook`，确认只标记键；stdio 验证类型移动、原始类型与映射类型不一致时的回退及清除，均覆盖推送和拉取：

```sh
cargo test --locked --test biber_type_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_type_warnings_
```

Biber 必填字段回归区分单一字段与至少填写一项的字段组，核对原始要求、完整特殊文件名、数据库角色、单字段和组模板边界、Unicode 规范化碰撞、来源失败、不借用其他库，以及保存后范围刷新和无猜测插入修复。单行 1500 个组合字符键对应 3000 条单字段及备选组警告，逐项核对原始 UTF-16 范围。真实 Biber 校验空条目、条目键移动、补齐必填要求后的警告清除；stdio 同时验证推送、拉取、增量保存和成功构建状态：

```sh
cargo test --locked --test biber_required_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_required_fields_
```

Biber 重复键回归覆盖无行号歧义、同文件全部候选、跨库顺序、精确大小写、来源缺失及超限、特殊文件名、符号链接、记录级别和语法错误后的截断键。Unicode 回归核对组合/分解形式、韩文与组合标记的规范等价碰撞、兼容字符区别，以及多种规范等价拼写共享的候选上限。另验证同一 Unicode 长行中 3000 个重复键及 3000 对大小写键的全部范围、每键上限之后的 CRLF 位置，以及缓存建立后发生错误时停止跨库关联。真实 Biber 测试比较默认模式与 `--noskipduplicates` 的实际输出，并验证规范等价声明的位置移动和修复清除；stdio 同时验证 ASCII 与规范化 Unicode 跨库相关位置、增量保存、成功构建以及推送与拉取：

```sh
cargo test --locked --test biber_duplicate_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_duplicate_warnings_
```

Biber 数据库键大小写警告回归覆盖区分大小写的规范等价键、跨库读取顺序、主键规范化碰撞、关联键的多个规范等价候选及相关位置上限、错误与截断历史、来源角色、完整特殊文件名、符号链接、UTF-16/CRLF 和有界读取。真实 Biber 测试确认两种拼写均保留于输出，源码组合字符与日志分解字符精确对应，保存后诊断移动、修复后清除；stdio 验证 ASCII 及 Unicode 的推送与拉取、跨库相关位置及成功构建状态：

```sh
cargo test --locked --test bibliography_case_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_case_warnings_
```

`crossref` 错误与嵌套警告测试核对字段所属条目、字面值范围、重复键与字段、动态值、来源角色、日志截断、扫描上限及相关父条目位置。真实 BibTeX 回归验证父条目缺失、前置未引用、后置、显式引用、通配引用、空值、外围空白，以及普通嵌套、自引用、循环、动态上级引用和缺失祖先。嵌套警告与祖先缺失错误须独立定位、保留各自级别；不匹配或缺失续行不能吞掉下一条记录。stdio 覆盖推送、拉取、警告下构建成功、警告与错误共存，以及修复后重编译清除诊断：

```sh
cargo test --locked --test bibliography_crossref_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_crossref_errors_
cargo test --locked --test protocol bibliography_nested_crossrefs_
```

文献大纲回归核对完整条目与字符串范围、字段层次、赋值范围、未完成构造、注释和嵌套字面量、重复键和字段、UTF-16/CRLF、语言类型及大文件的字段归属。静态 IDS 别名核对字段子符号、所属主键、重复声明与动态值排除，以及单行 2000 个 Unicode 别名的全部选择范围。真实 BibTeX 回归验证无字段花括号条目、分行键和引号键；stdio 覆盖层级与平铺客户端、未命名及其他扩展名文档、增量编辑和旧字段清除，另检查别名搜索、静态转动态后清除及关闭文档恢复磁盘内容。

```sh
cargo test --locked --test bibliography_symbols -- --include-ignored
cargo test --locked --test protocol bibliography_outlines_
cargo test --locked --test protocol bibliography_alias_symbols_
```

引号文献键回归覆盖各类引用参数、手写文献、条目关系列表、双引号字段转花括号、UTF-16/CRLF、注释和说明保留、动态值排除、项目隔离及共享歧义。stdio 验证版本化与传统工作区编辑、补全后增量更新和跨文件重命名；真实 pdfLaTeX/BibTeX 回归连续重命名父条目，比较完整 `.bbl` 并核对 crossref 继承和缺失引用警告。

```sh
cargo test --locked --test quoted_citations -- --include-ignored
cargo test --locked --test protocol quoted_citation_keys_
```

选择范围回归核对完整文献键、标签列表和范围端点、结构化引用的说明/页码/键/字段角色、BibTeX 关系值与条目层级、跨行命令及尾随正文、CRLF 注释、UTF-16 和未完成参数。stdio 回归验证键名及字段定界符增量修改后范围刷新、关闭重开、位置顺序及无效代理对位置拒绝。

另覆盖 BibTeX 普通字段、字符串赋值与 preamble 的字面内容/定界符/操作数/拼接表达式/完整构造，两种条目定界符、受保护的引号与百分号、跨值标点隔离、末尾空白、空表达式和未完成输入。实际 stdio 编辑在引号、嵌套花括号、拼接与未闭合值之间转换，核对范围刷新并复查缩写导航；真实 BibTeX 字符串回归验证既有输出保持一致。

```sh
cargo test --locked --test selection_ranges
cargo test --locked --test protocol selection_ranges
```

`entrydata` 默认回归覆盖普通与星号调用、单键与代码角色、注释和跨行补全、Unicode/引号键、语义 token、选择范围、动态键及列表排除、别名、项目隔离、共享含义歧义和嵌套普通引用的警告归属。stdio 回归覆盖导航、悬停、补全、参数提示、引用、高亮、两种工作区编辑格式，以及增量改为键列表后清除原先的引用：

```sh
cargo test --locked --test biblatex_entrydata
cargo test --locked --test protocol biblatex_entrydata_
```

条目条件查询回归覆盖两个命令的单键编辑、两个分支与等待参数、片段、用户签名、注释/UTF-16、语义与选择范围、动态键及不支持的调用、别名项目隔离、共享歧义和嵌套引用警告归属。stdio 检查两种工作区编辑、三个参数角色、增量转成列表后的引用清除及关闭重开：

```sh
cargo test --locked --test biblatex_entry_queries
cargo test --locked --test protocol biblatex_entry_queries_
```

数据选择器回归覆盖六个命令的字段/字面列表/姓名列表候选隔离、大小写与词中替换、注释和未完成参数、UTF-16/CRLF、可选格式的方括号边界、精确悬停及与文献键的独立性。另核对动态参数、逐字内容、命令名操作数、用户定义及别名改写的排除，并验证缓冲区改写后的候选刷新。stdio 在普通文本和片段客户端中应用实际增量补全，再核对悬停、原有键导航、三个参数类型、用户重定义清除和关闭重开恢复：

字段读取和三个 undef 检查额外覆盖首参数与两个分支的独立性、参数提示、计数器/别名计数器/定理及 newif 派生命令冲突，还有其他项目的声明隔离；相同 stdio 回归扩展到全部十一个命令。

字段比较回归检查两个字段参数的独立替换、字符串/命令操作数与分支边界、动态首参数后的第二字段、注释续行及未完成参数。五个比较命令同时核对列表类型隔离、四参数签名、嵌套调用、用户定义与 `newif` 覆盖，以及缓冲区恢复。

宏比较目标还覆盖命令名的纯文本补全、空前缀、未完成分组，以及被比较的文献、声明、逐字命令不被执行。复用 `operand_completion` 回归检查执行分支及显式重新定义后的调用片段；stdio 同时检查比较目标的词中替换和外层四参数提示。

`ifentrytype` 额外核对默认输出类型与特殊条目/输入别名的区分、共享类型描述、与同名文献键及字段的独立性、跨行注释编辑、分支参数提示、用户定义及 `newif` 覆盖后的恢复。

派生命令回归另检查命令名补全采用纯名称、撤下不再适用的内置说明与签名，以及移除覆盖后恢复文献参数片段。

```sh
cargo test --locked --test biblatex_data_selectors
cargo test --locked --test biblatex_data_comparisons
cargo test --locked --test biblatex_entry_types
cargo test --locked --test protocol biblatex_data_selectors_
```

连续 latexmk 编译回归覆盖已有产物、缓存失败后的错误位置保留、成功的无操作编译、修正后的诊断清空，以及自定义输出目录和作业名；真实工具部分需要 pdfLaTeX 和 XeLaTeX。默认测试另核对启动失败、超时及其他工具失败不会误用旧日志，并通过 stdio 验证重复诊断发布与修正后的清空：

```sh
cargo test --locked --test repeated_build -- --include-ignored
cargo test --locked --test protocol repeated_latexmk_failures
```

真实 biblatex 引用及重编译请求回归还需要安装 biblatex 和 logreq，使用 pdfLaTeX 与 BibTeX：

```sh
cargo test --locked --test biblatex_resources real_ -- --ignored
cargo test --locked --test biblatex_rerun real_biblatex -- --ignored
cargo test --locked --test biblatex_commands real_biblatex -- --ignored
cargo test --locked --test biblatex_data_citations real_ -- --ignored
cargo test --locked --test biblatex_categories real_ -- --ignored
cargo test --locked --test multicite real_biblatex -- --ignored
cargo test --locked --test volume_citations real_volume -- --ignored
```

文献资源回归验证全局资源在文件及文献键改名后的 pdfLaTeX / BibTeX 编译，以及 Biber 控制文件中的精确资源名称；后一项只生成 `.bcf`，无需运行 Biber。

文献分类回归验证重命名文献键后，分类名保持不变、辅助文件中的分配同步更新，且按分类筛选的参考文献仍能生成 PDF。默认测试另覆盖分类键的编辑、参数提示、补全、诊断隔离、共享源码歧义及版本化 stdio 重命名。

缺失文献资源修复另有两组真实工具回归：

```sh
cargo test --locked --test biblatex_resource_fixes real_biber -- --ignored
cargo test --locked --test biblatex_resource_fixes real_bibtex -- --ignored
```

前者需要 Biber，使用 `--tool` 验证真实缺文件日志及修复后数据库可读取，不验证完整 PDF 编译；后者需要 pdfLaTeX、BibTeX、biblatex 和 logreq，验证应用文件名修复后重新生成 PDF。默认测试另覆盖特殊文件名、选项保留、版本化编辑、过期修复和 stdio 协议。

Biber 非标准后缀数据库另有真实日志回归：

```sh
cargo test --locked --test biber_data_sources real_biber -- --ignored
```

该测试使用 Biber `--tool` 读取不同后缀及无扩展名数据库，验证依赖发现和重复文献键警告的定位；不要求匹配版本的 biblatex，也不生成 PDF。默认协议回归覆盖数据库角色、导航、版本化重命名、文件移动及编译期间输入变更。

普通文献警告回归覆盖同库及跨库重复键、完整大小写、不可读或超限数据库、符号链接去重、特殊后缀、UTF-16/CRLF、独立行号及多警告数据库。真实 BibTeX 验证样式生成的字段警告在条目移动后更新、字段补齐后消失；stdio 覆盖 push/pull、保存编辑后唯一与歧义状态的切换及修复后清除。

```sh
cargo test --locked --test bibliography_log_records -- --include-ignored
cargo test --locked --test protocol generic_bibliography_entry_warnings_
```

重复字段诊断另覆盖 3000 个同一行 Unicode 值、跨行值与 Unicode 字段名、交错的数据库警告组，以及首次警告后出现语法错误时的回退。真实 BibTeX 同样验证 3000 个被忽略的字段逐一定位、首次值仍用于输出、移除重复字段后诊断清空；stdio 核对同组多个警告在保存增量编辑后一起移动、保留相关首次赋值并在修复后清除。

```sh
cargo test --locked --test bibliography_field_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_fields_
```

