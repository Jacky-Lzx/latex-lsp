<img src="assets/icon.svg" width="64" height="64" alt="">

# latex-lsp

用 Rust 实现的 LaTeX Language Server，通过 stdio 提供补全、导航、安全重命名、诊断、后台编译、编译结果分析和 SyncTeX 查询。

## 快速开始

安装当前稳定版 Rust，在仓库目录执行：

```sh
cargo install --path . --locked
latex-lsp --version
```

将 `latex-lsp --stdio` 配置为编辑器的语言服务器，见[安装与编辑器配置](docs/editors.md)。编辑功能无需 TeX 发行版；编译需要相应的 TeX 工具，见[编译指南](docs/build.md)。

## 文档

[文档目录](docs/README.md)包含编辑、文献、重命名、项目配置、编译诊断、SyncTeX 和开发指南。可用[示例项目](examples/demo/main.tex)试用。

采用静态解析；具体能力与限制见[解析范围](docs/development.md#解析范围)。

## 许可证

[MIT](LICENSE)
