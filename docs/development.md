# 开发与测试

[返回 README](../README.md)

开发环境、质量检查与模块职责。使用说明见[项目 README](../README.md)。

- [构建检查与测试](#构建检查与测试)
- [解析范围](#解析范围)
- [代码结构与分析缓存](#代码结构与分析缓存)

## 构建检查与测试

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
# 已安装 latexmk、pdfLaTeX、XeLaTeX、LuaLaTeX、BibTeX、Biber、biblatex、logreq、SyncTeX 和 Neovim 0.10+ 时，执行真实编译及编辑器测试：
cargo test --locked -- --include-ignored
```

开发和测试构建默认关闭调试信息与增量缓存，以控制磁盘占用。需要源码级调试时，可按需设置 `CARGO_PROFILE_DEV_DEBUG=2` 或 `CARGO_PROFILE_TEST_DEBUG=2`；发布构建继续使用独立配置。

测试包含 UTF-16/增量编辑、语法容错、跨文件导航、文献补全、磁盘/未保存内容同步，以及启动真实进程验证的 stdio 协议流程。

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

## 解析范围

采用静态解析，不执行任意 TeX 宏展开，不读取 TeX 发行版的包数据库。不内置 PDF 预览、格式化或数学语义检查；重命名和修复遇到歧义时会拒绝编辑。

## 代码结构与分析缓存

同一次编译日志中的重复颜色、颜色模型、计数器、环境及缺失输入文件、宏包、文档类错误会复用当前来源集合的宏签名和各类文档调用分析，减少大文档多错误时的重复解析。缓存只保留最近的宏来源集合及每类错误的一份文档分析；来源集合变化时重新分析，每条错误仍核对源码内容，不跨编译保留缓存。

代码按职责拆分：`text.rs` 处理位置与编辑，`syntax.rs` 解析 LaTeX/BibTeX，`citations.rs` 共享多组、卷号及字段和列表引用的参数边界，`bib_resources.rs` 统一 biblatex 静态本地资源的选项、完整文件名与补全边界，`workspace.rs` 管理文件索引与根文档，`features.rs` 实现语言功能，`signature.rs` 提供命令参数提示，`navigation.rs` 处理工作区搜索、折叠和链接，`editing.rs` 处理快速修复、高亮和选择范围，`file_operations.rs` 维护文件移动后的引用，`build.rs` 管理后台编译，`build_log.rs` 分析日志，`recorder.rs` 读取实际输入记录，`synctex.rs` 查询源码/PDF 映射，`config.rs` 校验配置，`dispatch.rs` 保留消息顺序并释放耗时查询的等待，`lib.rs` 对接 LSP，`main.rs` 启动服务。
