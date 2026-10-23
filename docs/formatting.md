# 格式化

[返回文档目录](README.md) · [编辑功能](editing.md)

支持标准 `textDocument/formatting` 全文格式化，默认调用 `tex-fmt`，也可选择 `latexindent`。请将所选工具安装到服务器的 `PATH`，或配置可执行文件的绝对路径。Neovim 可调用 `vim.lsp.buf.format()`。

使用 `initializationOptions`（Neovim 的 `init_options`）或 `workspace/didChangeConfiguration` 设置：

```json
{
  "formatting": {
    "formatter": "tex-fmt",
    "executable": null,
    "arguments": null,
    "timeoutSeconds": 10
  }
}
```

- `formatter`：`tex-fmt`（默认）或 `latexindent`。
- `executable`：默认采用所选工具的名称；可指定完整路径。
- `arguments`：`null` 使用下述默认参数；数组会完整替换默认参数。自定义时必须保留工具的标准输入模式，`tex-fmt` 使用 `--stdin`，`latexindent` 使用 `-`。参数直接传给进程，不经过 shell。
- `timeoutSeconds`：默认 10 秒，允许 1–3600 秒。

切换到 latexindent，例如：

```lua
init_options = {
  formatting = { formatter = 'latexindent' },
}
```

默认 `tex-fmt` 参数为 `--stdin --tabsize N`；客户端请求制表符缩进时增加 `--usetabs`。工作目录为当前文档所在目录，让工具按自身规则读取项目配置。尚未保存的非文件缓冲区使用工作区目录，没有工作区时继承服务器目录。

默认 `latexindent` 参数包括 `-l '-y=defaultIndent: "缩进"' -`，并通过 `-c` / `-g` 将辅助文件和日志放入独立临时目录，任务结束后删除。`-l` 加载工具支持的本地设置，`defaultIndent` 采用客户端的 `tabSize` 和 `insertSpaces`。使用自定义参数时，缩进、日志和本地配置均由这份参数决定。

服务器将当前缓冲区文本写入工具的标准输入，以标准输出生成一个排除相同前后缀的 UTF-16 文本编辑；不覆盖源文件。保持原文档的 LF / CRLF，内容未变化时返回空编辑列表。未保存的 LaTeX 缓冲区同样支持；BibTeX 与 `.bst` 暂不格式化。不提供范围格式化。

格式化在后台执行，期间仍可编辑和发送其他请求。文档内容、版本、语言类型或格式化配置变化后，返回 `ContentModified`，请重新请求格式化。最多同时运行 4 个格式化任务。取消请求、超时、shutdown 和连接断开会终止任务；Unix 同时终止子进程组。

工具缺失、非零退出、无效 UTF-8、非空文档对应空输出或标准输出超过 16 MiB 时返回错误，不生成编辑。实际排版及逐字环境处理遵循所选工具的规则。

验证：

```sh
cargo test --locked --lib formatting
cargo test --locked --test protocol formatting_
cargo test --locked --test protocol formatting_with_real_tex_fmt -- --ignored
cargo test --locked --test protocol formatting_with_real_latexindent -- --ignored
```

最后两项分别需要真实 `tex-fmt` / `latexindent`，验证四空格缩进、逐字内容、CRLF、UTF-16、重复格式化及不写入源文件。默认测试使用独立工具夹具验证两种后端的参数、缓冲区输入、配置更新、错误、超时、取消、过期结果和进程清理。
