# 真实工具回归

[返回文档目录](README.md) · [开发与测试](development.md)

## 环境与入口

默认 `cargo test --locked` 跳过需要外部程序的回归。安装 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、BibTeX、Biber、biblatex、logreq、SyncTeX 和 Neovim 0.10+ 后，可执行全部测试：

```sh
cargo test --locked -- --include-ignored
```

只安装部分工具时，按下面的命令选择对应测试。

## 排版日志与文件移动

真实盒子警告回归需要 pdfLaTeX、XeLaTeX 和 LuaLaTeX，使用较窄的日志输出宽度验证 Overfull 警告的跨行恢复及完整段落范围：

```sh
cargo test --locked --test box_warnings real_ -- --ignored
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

图片调用别名回归同样需要 pdfLaTeX、graphicx 和 etoolbox，验证纯别名、静态命名调用、图片与主文档同时移动后的 recorder 输入，以及缺图定位、拼写修复和重新编译：

```sh
cargo test --locked --test indirect_graphics real_ -- --ignored
```

默认回归另覆盖完整和未完成路径的别名补全、跨输入文件与本地宏包的别名、未执行和不确定调用的排除，以及目录补全和缺图建议的作用域、共享源码歧义、候选图片的过期检查及版本化 stdio 文件移动。

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

