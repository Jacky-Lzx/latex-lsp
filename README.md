<img src="assets/icon.svg" width="64" height="64" alt="">

# latex-lsp

用 Rust 实现的轻量 LaTeX Language Server，通过 stdio 提供 LSP 服务。编辑功能无需安装 TeX 发行版；编译需要相应的 TeX 工具。

## 功能

- 补全、悬停、参数提示、跨文件跳转与引用查找。
- 标签、文献键、宏、环境等符号的安全重命名，以及文件移动后的路径更新。
- 实时诊断与快速修复、语义高亮、折叠和文档符号。
- 后台编译、取消与保存后编译，解析 TeX、BibTeX 和 Biber 的错误与警告。
- 编译结果分析与 SyncTeX 双向位置查询，供编辑器或 PDF 插件集成。

## 快速开始

安装当前稳定版 Rust，在仓库目录执行：

```sh
cargo install --path . --locked
latex-lsp --version
```

将 `latex-lsp --stdio` 配置为编辑器的语言服务器，具体步骤见[安装与编辑器配置](docs/editors.md)。可用 [examples/demo/main.tex](examples/demo/main.tex) 试用跨文件导航、补全与重命名。

编译需要 TeX 工具；安装、引擎切换和操作方式见[编译指南](docs/build.md)。

## 文档

[文档目录](docs/README.md)提供编辑功能、重命名、项目配置、编译诊断、SyncTeX 和开发测试的详细说明。

## 范围

采用静态解析，不执行任意 TeX 宏展开，不读取 TeX 发行版的包数据库。不内置 PDF 预览、格式化或数学语义检查；重命名和修复遇到歧义时会拒绝编辑。

## 许可证

[MIT](LICENSE)
