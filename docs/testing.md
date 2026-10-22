# 真实工具回归

[返回文档目录](README.md) · [开发与测试](development.md)

## 环境与入口

默认 `cargo test --locked` 跳过需要外部程序的回归。安装 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、BibTeX、Biber、biblatex、logreq、SyncTeX 和 Neovim 0.10+ 后，可执行全部测试：

```sh
cargo test --locked -- --include-ignored
```

只安装部分工具时，按下面的命令选择对应测试。

## 排版日志与文件移动

声明操作数回归使用三个 LaTeX 引擎验证复制 `newcommand`、`newif`、`newenvironment` 后的普通调用，以及宏、开关和环境重命名前后的输出一致；同时验证直接与静态 `csname` 目标复制字面花括号，以及未执行的宏正文和参数分隔文本。默认测试另检查各类符号索引、命令名操作数的参数提示与名称补全、静态 `csname`、逐字边界和标签编辑范围，并核对花括号复制不会产生错误修复、真实不配对仍保留精确位置：

```sh
cargo test --locked --test operand_declarations -- --include-ignored
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

跨文件宏来源回归需要这三个引擎及 etoolbox，在 60 / 79 列及传统 / 文件行号日志中验证裸参数、分隔参数、重复声明和别名转发不会误获转义编辑。父文件声明的传统与 xparse 环境、可选参数和 `\begin` 别名也参与验证；零参数或已完成参数之后的实际分隔符在修复后重新编译。复制回归另覆盖 `let`、内核复制命令和 etoolbox 复制接口，验证来源后续重定义、环境副本、捕获的 `begin` 和来源操作数不会误作为调用。默认测试另覆盖加载时序、暂停父文件、动态来源、编译中源码变化和宏文件变化后的旧修复拒绝；协议测试同时覆盖 Code Action 和旧客户端的重复声明、别名及复制变化：

```sh
cargo test --locked --test literal_macro_scopes -- --include-ignored
cargo test --locked --test protocol literal_build_fixes_recheck_cross_file_macro_changes_without_a_main_version_change
```

源码文件名移动回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX，验证首部空格、主干末尾空格、裸路径和引号路径，以及移动后的真实输入与导航：

```sh
cargo test --locked --test source_filename_renames -- --include-ignored
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

## 文献与编译诊断

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

