# latex-lsp

Rust 编写的 LaTeX 语言服务器，支持补全、导航、重命名、格式化、编译与诊断。

[快速开始](docs/install.md) · [文档](docs/README.md) · [性能测试](docs/benchmarks.md) · [MIT 许可证](LICENSE)

## LSP 支持情况与 texlab 对比

核对日期：2026-10-26。latex-lsp 以当前仓库实现为准；[texlab](https://github.com/latex-lsp/texlab) 以当日获取的 `master` 提交 [`94e6c09`](https://github.com/latex-lsp/texlab/tree/94e6c094ae628125073b2cf70c11dddaaa092140) 为准。

✓ 表示已实现该项功能，✗ 表示未实现可用功能；空处理函数不计为支持。勾选不代表两者覆盖范围、默认设置或效果相同。编辑器需支持对应协议；格式化、编译、ChkTeX 和 SyncTeX 的外部工具要求见下表及链接文档。本表为源码审阅结果，不是两套服务器的完整运行对测。

### 标准 LSP 功能

依据：latex-lsp 的[能力声明与处理实现](src/server/backend.rs)、[请求分发](src/server/protocol.rs)；texlab 的[能力声明与请求处理](https://github.com/latex-lsp/texlab/blob/94e6c094ae628125073b2cf70c11dddaaa092140/crates/texlab/src/server.rs)。

| 功能                                                         | latex-lsp | texlab | 支持范围或差异                                                                                                    |
| ------------------------------------------------------------ | :-------: | :----: | ----------------------------------------------------------------------------------------------------------------- |
| 文档同步                                                     |     ✓     |   ✓    | 打开、关闭、保存与增量文本更新。                                                                                  |
| 补全与补全项解析                                             |     ✓     |   ✓    | `completion` / `completionItem/resolve`；命令、环境、标签、文献与路径等，具体候选覆盖不同。                       |
| 悬浮提示                                                     |     ✓     |   ✓    | `hover`；latex-lsp 还展示唯一用户宏声明前的[文档注释](docs/command-documentation.md)。                            |
| [参数提示](docs/signatures.md)                               |     ✓     |   ✗    | `signatureHelp`；内置命令及静态可确认的跨文件用户宏、环境签名。                                                   |
| 定义跳转                                                     |     ✓     |   ✓    | `definition`；标签、文献、命令与文件等。                                                                          |
| 声明跳转                                                     |     ✓     |   ✗    | `declaration`；latex-lsp 将静态符号的源码定义作为声明位置。                                                       |
| 查找引用                                                     |     ✓     |   ✓    | `references`；标签、文献键和命令等。                                                                              |
| 准备重命名与跨文件重命名                                     |     ✓     |   ✓    | `prepareRename` / `rename`；latex-lsp 另覆盖[自定义环境、计数器等符号](docs/symbols.md)，两者重名及歧义检查不同。 |
| 文档内引用高亮                                               |     ✓     |   ✓    | `documentHighlight`。                                                                                             |
| 文档大纲与工作区符号搜索                                     |     ✓     |   ✓    | `documentSymbol` / `workspace/symbol`。                                                                           |
| [文档折叠](docs/folding.md)                                  |     ✓     |   ✓    | `foldingRange`；LaTeX 结构和 BibTeX 条目等。                                                                      |
| 逐级扩大选区                                                 |     ✓     |   ✗    | `selectionRange`；词、参数、命令及文档结构。                                                                      |
| 环境名称联动编辑                                             |     ✓     |   ✗    | `linkedEditingRange`；同一文档内可确认配对的 `begin` / `end`。texlab 的自定义环境修改命令不等同于此协议。         |
| [用户宏调用层次](docs/call-hierarchy.md)                     |     ✓     |   ✗    | `prepareCallHierarchy` / `callHierarchy/incomingCalls` / `callHierarchy/outgoingCalls`；仅静态可确认的调用。      |
| [行内提示](docs/auxiliary-references.md)                     |     ✓     |   ✓    | `inlayHint`；两者支持标签信息，latex-lsp 还支持部分编译后文献编号。                                               |
| 语义令牌：全文、范围与增量结果                               |     ✓     |   ✗    | `semanticTokens/full` / `range` / `full/delta`；texlab 的范围请求处理为空，未声明语义令牌能力。                   |
| [LaTeX 全文格式化](docs/formatting.md)                       |     ✓     |   ✓    | `formatting`；均可调用 tex-fmt 或 latexindent，需另行安装。latex-lsp 默认 tex-fmt，texlab 默认 latexindent。      |
| BibTeX 全文格式化                                            |     ✓     |   ✓    | 均有内置格式化器，具体排版规则不同；texlab 另可选择外部格式化器。                                                 |
| 范围格式化                                                   |     ✓     |   ✗    | `rangeFormatting`；latex-lsp 支持 LaTeX 与 BibTeX。                                                               |
| 输入时格式化                                                 |     ✓     |   ✗    | `onTypeFormatting`；latex-lsp 在换行、输入 `}` 后内置调整 LaTeX 行首缩进。                                        |
| 推送诊断                                                     |     ✓     |   ✓    | `textDocument/publishDiagnostics`；静态检查、编译日志及可选 ChkTeX。                                              |
| 文档与工作区拉取诊断                                         |     ✓     |   ✗    | `textDocument/diagnostic` / `workspace/diagnostic`；latex-lsp 协商启用，支持结果 ID、未变更结果与刷新。           |
| 快速修复与延迟解析                                           |     ✓     |   ✗    | `codeAction` / `codeAction/resolve`；texlab 当前仅返回空操作列表，解析原样返回。                                  |
| [结构重构与全部修复](docs/refactoring.md)                    |     ✓     |   ✗    | `refactor.rewrite` / `refactor.extract` / `source.fixAll`；正文包裹、提取用户宏及无歧义静态修复。                 |
| 文档内编译入口                                               |     ✓     |   ✗    | `codeLens`；latex-lsp 返回关联主文档的编译命令。                                                                  |
| [颜色预览与颜色表示转换](docs/editing.md#颜色预览与命名颜色) |     ✓     |   ✗    | `documentColor` / `colorPresentation`；texlab 的颜色名称补全不等同于颜色预览协议。                                |
| 文档链接                                                     |     ✓     |   ✓    | `documentLink`；支持的文件、URL 等链接类型不同。                                                                  |
| [文件移动时更新源码引用](docs/file-operations.md)            |     ✓     |   ✗    | `workspace/willRenameFiles`；移动后通过 `didRenameFiles` 更新索引。                                               |
| 文件创建、删除通知                                           |     ✓     |   ✗    | `workspace/didCreateFiles` / `didDeleteFiles`；texlab 可通过文件监听更新，此处只比较文件操作协议。                |
| 工作区文件夹动态增删                                         |     ✓     |   ✗    | `workspace/didChangeWorkspaceFolders`；texlab 可读取初始化时的多个文件夹，但没有动态增删处理。                    |
| 类型定义、实现跳转与类型层次                                 |     ✗     |   ✗    | `typeDefinition` / `implementation` / `prepareTypeHierarchy` 等，均未实现。                                       |

### LaTeX 项目、编译与 PDF 集成

以下包含服务器扩展命令，不属于标准 LSP 语言功能。latex-lsp 使用 `workspace/executeCommand` 下的 `latex.*` 命令；texlab 使用 `texlab.*` 命令及 `textDocument/build`、`textDocument/forwardSearch` 等扩展请求，接口不能直接互换。

| 功能                                             | latex-lsp | texlab | 支持范围或差异                                                                                 |
| ------------------------------------------------ | :-------: | :----: | ---------------------------------------------------------------------------------------------- |
| 主文档识别与跨文件依赖                           |     ✓     |   ✓    | 根据源码依赖组织项目，支持根文件配置；解析规则和项目隔离策略不同。                             |
| 后台编译与保存后编译                             |     ✓     |   ✓    | 默认 latexmk，可配置其他编译工具；需要相应 TeX 工具。                                          |
| 编译取消与进度通知                               |     ✓     |   ✓    | 两者均提供取消命令，并可使用客户端原生工作进度。                                               |
| 编译错误与警告诊断                               |     ✓     |   ✓    | 分析编译日志；具体消息类型及定位精度不同。                                                     |
| ChkTeX 检查                                      |     ✓     |   ✓    | 需安装 ChkTeX；latex-lsp 保存时检查，texlab 可配置打开／保存及编辑时检查。                     |
| [辅助文件与完整产物清理](docs/build-cleaning.md) |     ✓     |   ✓    | latex-lsp 按已记录产物清理并支持预览；texlab 调用 `latexmk -c` / `-C`。                        |
| [源码位置查询 PDF 页码与坐标](docs/synctex.md)   |     ✓     |   ✗    | latex-lsp 调用 `synctex` 返回位置数据；texlab 的正向搜索调用阅读器，不返回此类坐标。           |
| PDF 坐标反查源码位置                             |     ✓     |   ✗    | latex-lsp 提供 `latex.inverseSearch` 查询；texlab 的反向搜索接收阅读器已解析的源码位置。       |
| 自动调用 PDF 阅读器正向搜索                      |     ✗     |   ✓    | texlab 可配置阅读器命令，并在编译后执行；latex-lsp 返回坐标，交由客户端集成。                  |
| 阅读器反向搜索回调与编辑器跳转                   |     ✗     |   ✓    | texlab 通过 IPC 接收源码位置，再调用 `window/showDocument`；latex-lsp 的查询结果由客户端处理。 |
| 导出项目依赖图                                   |     ✗     |   ✓    | texlab 的 `texlab.showDependencyGraph` 返回 DOT 文本；latex-lsp 暂无对应导出命令。             |

texlab 扩展依据：[编译、搜索与命令处理](https://github.com/latex-lsp/texlab/blob/94e6c094ae628125073b2cf70c11dddaaa092140/crates/texlab/src/server.rs)、[格式化实现](https://github.com/latex-lsp/texlab/blob/94e6c094ae628125073b2cf70c11dddaaa092140/crates/texlab/src/features/formatting.rs)、[格式化默认配置](https://github.com/latex-lsp/texlab/blob/94e6c094ae628125073b2cf70c11dddaaa092140/crates/base-db/src/config.rs)、[工具命令实现](https://github.com/latex-lsp/texlab/tree/94e6c094ae628125073b2cf70c11dddaaa092140/crates/commands/src)。完整使用说明见[编辑功能](docs/editing.md)、[编译指南](docs/build.md)与[文档目录](docs/README.md)。
