# 安装与启动

[返回 README](../README.md) · [编辑器配置](editors.md)

安装当前稳定版 Rust。编辑功能无需 TeX 发行版；编译需要相应工具位于服务器的 `PATH` 中，默认使用 latexmk 与 pdfLaTeX，配置见[编译指南](build.md)。

从源码构建并启动：

```sh
cargo build --release --locked
./target/release/latex-lsp --stdio
```

服务器直接使用 Tokio 实现 stdio JSON-RPC 运行层，`lsp-types` 提供协议数据类型。请求超时或取消时会移除响应接收端，迟到和重复的客户端回复会安全忽略。

服务器收到 `shutdown`、`exit`，或 stdio 输入流关闭时，会取消并回收编译与 SyncTeX 进程组，避免编辑器断开后遗留外部工具。未先完成 `shutdown` 就发送 `exit` 时，仍按 LSP 返回退出码 `1`。

进程从标准输入读取 LSP 消息，并向标准输出写入带 `Content-Length` 的 JSON-RPC 消息；通常由编辑器启动。直接运行时等待输入属于正常行为。也可以安装到 Cargo 的可执行文件目录：

```sh
cargo install --path . --locked
latex-lsp --version
```

