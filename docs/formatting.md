# 格式化

[返回文档目录](README.md) · [编辑功能](editing.md)

支持标准 `textDocument/formatting` 全文格式化，默认调用 `tex-fmt`，也可选择 `latexindent`。请将所选工具安装到服务器的 `PATH`，或配置可执行文件的绝对路径。Neovim 可调用 `vim.lsp.buf.format()`。

使用 `initializationOptions`（Neovim 的 `init_options`）或 `workspace/didChangeConfiguration` 设置：

```json
{
  "formatting": {
    "onType": true,
    "formatter": "tex-fmt",
    "executable": null,
    "arguments": null,
    "timeoutSeconds": 10
  }
}
```

- `formatter`：`tex-fmt`（默认）或 `latexindent`。
- `onType`：默认 `true`，启用下述内置输入时缩进；设为 `false` 可单独关闭。
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

服务器将当前 LaTeX 缓冲区文本写入工具的标准输入，以标准输出生成一个排除相同前后缀的 UTF-16 文本编辑；不覆盖源文件。保持原文档的 LF / CRLF，内容未变化时返回空编辑列表。未保存的缓冲区同样支持；BibTeX 使用下述内置格式化器，`.bst` 暂不格式化。

## 范围格式化

支持标准 `textDocument/rangeFormatting`。使用与全文格式化相同的工具、配置和完整缓冲区输入，因此选中的条目仍使用外层环境的缩进，逐字代码也保留完整上下文。服务器按行内容对齐格式化前后的文本，只返回完全位于请求范围内的修改，选区外内容保持原样。跨越选区边界的换行、合并或其他不可拆分修改会被跳过；可扩大选区或使用全文格式化。需要整理整行缩进时，请从该行第 0 列开始选择。

返回的多个编辑互不重叠，使用原始文档的 UTF-16 坐标。超出行长的列号按 LSP 规则收敛到行尾；归一化后为空的范围返回空编辑列表。逆序、不存在的行或切开 UTF-16 代理对的范围返回 `InvalidParams`。行对齐限制中间匹配表的大小，超过预算的连续改写按一个整体处理，避免大文件消耗无界内存。取消、超时、文档或配置变更与全文格式化采用相同保护。

## 输入时缩进

支持标准 `textDocument/onTypeFormatting`，触发字符为换行和 `}`，需要客户端启用输入时格式化。使用内置局部缩进器，不依赖 `tex-fmt` / `latexindent`，也不读取它们的配置。`formatting.onType: false` 可关闭；目录配置同样生效。

换行时，以前一行的缩进为基础，根据该行未抵消的环境和花括号分组增加或减少一级；支持 `\\begingroup` / `\\endgroup`、`\\bgroup` / `\\egroup`，`document` 环境不增加一级。当前行开头的 `}` 或 `\\end{环境}` 会减少一级；输入闭合的 `}` 时也会调整该行。前一行开头的关闭符已反映在其缩进中，不会重复减少。

只替换当前行行首的空格和制表符，沿用客户端 `tabSize` / `insertSpaces`；不修改文字、命令、注释或换行。支持未保存缓冲区、UTF-16 和 CRLF。注释中的括号、转义花括号、逐字内容及已知 token 操作数不计入结构；逐字块内部不格式化。已完成宏定义中的环境和分组命令不会影响定义后的缩进，定义正文的花括号仍可提供缩进。BibTeX 和 `.bst` 暂无输入时格式化。

这是相邻行的缩进辅助，不执行宏展开，也不会重排整个文档。跨行环境名、超过 64 KiB 的行、超过 4096 列的缩进、超过 2 MiB 的缓冲区或超过 1024 个定义会跳过；在正文中间的换行请求和普通命令参数结束处输入 `}` 不会触发整行重排。可用全文或范围格式化统一已有文档的布局。

## BibTeX 格式化

BibTeX / biblatex 文献缓冲区支持全文与范围格式化，无需安装外部工具。服务器复用文献条目及表达式解析，只调整完整普通条目的字段外部空白：每个字段换行并缩进，`=` 两侧各保留一个空格，关闭定界符另起一行。缩进采用请求的 `tabSize` / `insertSpaces`；不添加或删除逗号。

字段值及其内部换行、花括号保护、大小写、字符串拼接、键名、字段和条目顺序保持原样。注释及维持注释边界的空白保持原样，因此含注释的部分可能保留原有布局。新换行沿用文档的 LF / CRLF，原有混合换行不会被统一。不完整条目、无效赋值、无字段条目以及 `@string`、`@preamble`、`@comment` 保持原样。

范围请求直接筛选位于选区内的空白修改，支持相邻单行条目和未命名文献缓冲区。LaTeX 的 `formatter`、`executable` 和 `arguments` 不用于文献格式化。任务在后台运行，共用并发上限、取消及过期结果校验；输出同样限制为 16 MiB。服务器只返回编辑，不写入文献数据库。

格式化在后台执行，期间仍可编辑和发送其他请求。文档内容、版本、语言类型或格式化配置变化后，返回 `ContentModified`，请重新请求格式化。最多同时运行 4 个格式化任务。取消请求、超时、shutdown 和连接断开会终止任务；Unix 同时终止子进程组。

工具缺失、非零退出、无效 UTF-8、非空文档对应空输出或标准输出超过 16 MiB 时返回错误，不生成编辑。实际排版及逐字环境处理遵循所选工具的规则。

验证：

```sh
cargo test --locked --lib formatting
cargo test --locked --lib on_type_formatting
cargo test --locked --test protocol on_type_formatting
cargo test --locked --lib bib_formatting -- --include-ignored
cargo test --locked --test protocol formatting_
cargo test --locked --test protocol bibliography_formatting_
cargo test --locked --test protocol range_formatting_
cargo test --locked --test protocol range_formatting_with_real_tex_fmt -- --ignored
cargo test --locked --test protocol formatting_with_real_tex_fmt -- --ignored
cargo test --locked --test protocol formatting_with_real_latexindent -- --ignored
```

最后两项分别需要真实 `tex-fmt` / `latexindent`，验证四空格缩进、逐字内容、CRLF、UTF-16、重复格式化及不写入源文件。默认测试使用独立工具夹具验证两种后端的参数、缓冲区输入、配置更新、错误、超时、取消、过期结果和进程清理。
