# latex-lsp

用 Rust 实现的 LaTeX Language Server，提供补全、导航、重命名、诊断、后台编译与结果分析，以及 SyncTeX 查询。

## 安装与使用

安装稳定版 Rust，在仓库目录执行：

```sh
cargo install --path . --locked
```

将 `latex-lsp --stdio` 配置为编辑器的语言服务器。编辑功能无需 TeX 发行版，编译需要相应的 TeX 工具。

[使用文档](docs/README.md) · [编辑器配置](docs/editors.md) · [编译指南](docs/build.md) · [示例项目](examples/demo/main.tex) · [MIT 许可证](LICENSE)
