# LaTeX LSP for VS Code

使用本仓库的 Rust 语言服务器提供 LaTeX / TeX / BibTeX / BST 补全、导航、重命名、诊断、格式化、快速修复与编译入口。扩展复用 VS Code 自带的 LaTeX 和 BibTeX 语法高亮，并补充 `.bst` 文件类型。

## 安装

当前扩展从源码打包，语言服务器需单独安装；VSIX 不包含服务器或 TeX 发行版。首先在仓库根目录执行：

```sh
cargo install --path . --locked
```

然后构建扩展（Node.js 22 或更新版本）：

```sh
cd editors/vscode
npm ci
npm run package
code --install-extension latex-lsp.vsix
```

在 VS Code 中打开 LaTeX 项目文件夹，再打开 `.tex` 或 `.bib`。如果服务器不在 VS Code 的 PATH 中，在用户 `settings.json` 中指定绝对路径：

```json
{
  "latex-lsp.serverPath": "/absolute/path/to/latex-lsp",
  "latex-lsp.settings": {
    "build": {
      "engine": "xelatex",
      "onSave": true
    }
  }
}
```

本地开发也可以将 `serverPath` 指向本仓库 `target/release/latex-lsp`。路径不展开 `~`、`${workspaceFolder}` 或 shell 表达式。默认命令是 `latex-lsp --stdio`；请勿将参数拼接在 `serverPath` 中。

编译需要安装相应 TeX 工具，默认使用 latexmk 与 pdfLaTeX。LaTeX 格式化默认需要 tex-fmt，也可配置 latexindent；BibTeX 使用内置格式化器。GUI 启动的 VS Code 若找不到工具，可在用户设置中设置 `latex-lsp.serverEnvironment`，例如 `{"PATH": "/Library/TeX/texbin:/opt/homebrew/bin:/usr/bin:/bin"}`。

扩展在受信任的本地文件工作区运行；使用 Remote SSH、WSL 或 Dev Container 时，扩展和服务器运行在远端，`serverPath` 与 TeX 工具也必须位于远端。

## 配置与使用

- `latex-lsp.serverPath`：服务器可执行文件，修改后自动重启。
- `latex-lsp.serverEnvironment`：覆盖服务器继承的环境变量，修改后自动重启。
- `latex-lsp.settings`：完整服务器配置对象；初始化时传入，修改后通过 `workspace/didChangeConfiguration` 更新，无需重启。空对象保留服务器默认值，包括默认关闭保存后编译。
- `latex-lsp.trace.server`：`off` / `messages` / `verbose`，用于诊断协议通信。

`settings` 支持 `build`、`formatting`、`diagnostics`、`distribution`、`indexing`、`inlayHints`。字段与服务端一致，见[编译配置](https://github.com/Jacky-Lzx/latex-lsp/blob/main/docs/build-configuration.md)及[文档目录](https://github.com/Jacky-Lzx/latex-lsp/blob/main/docs/README.md)。服务器拒绝无效设置并保留之前的有效配置，原因会写入输出面板。

一个 VS Code 窗口使用一个服务器，支持多个工作区文件夹。`latex-lsp.settings` 是整个窗口的默认配置；各文件夹或项目的差异放入 `.latex-lsp.json`，见[项目配置](https://github.com/Jacky-Lzx/latex-lsp/blob/main/docs/project-settings.md)。不要把多个无关项目的公共父目录作为单一项目根目录。

通过 VS Code 自带的补全、跳转、重命名、格式化和快速修复操作使用标准语言功能。新建文档选择 LaTeX / BibTeX 语言后即可补全；编译前需保存项目及依赖文件。格式化输入、联动编辑和行内提示等功能仍遵守 VS Code 自身的开关。

命令面板提供：

| 命令 | 用途 |
| --- | --- |
| LaTeX LSP: Build | 编译当前文档所属项目 |
| LaTeX LSP: Rebuild | 强制重编译 |
| LaTeX LSP: Cancel Build | 取消当前文档所属项目的编译；没有活动编辑器时取消所有编译 |
| LaTeX LSP: Show Build Status | 将当前项目状态写入输出面板 |
| LaTeX LSP: Open PDF | 使用 VS Code 注册的 PDF 打开方式打开成功编译的 PDF |
| LaTeX LSP: Restart Server | 停止旧服务器并重新连接，重新同步打开的文档 |
| LaTeX LSP: Show Output | 打开服务器日志与编译结果 |

文档中的 CodeLens 也可以直接触发编译。编译进度使用 VS Code 原生进度显示；开始和完成详情写入 “LaTeX LSP” 输出面板，编译失败会显示通知。扩展不会自动保存文档；服务端会拒绝编译含未保存依赖的项目。

本扩展提供打开 PDF 的入口，暂未实现内嵌 PDF 阅读器或 SyncTeX 双向跳转。与其他 LaTeX 扩展同时启用时，可能出现重复的补全、诊断或编译；建议在当前工作区只启用一套语言服务及自动编译流程。

## 开发与验证

在仓库根目录运行 `cargo build --release --locked`，然后用 VS Code 打开本目录：

```sh
code editors/vscode
```

执行 `npm ci`，按 F5 启动 Extension Development Host，在新窗口设置 `serverPath` 并打开测试项目。`.vscode/launch.json` 会在启动前编译 TypeScript。

```sh
npm run check
npm test
npm run package
```

`npm test` 默认下载稳定版 VS Code，并在隔离的临时用户目录、扩展目录和 LaTeX 项目中运行真实宿主测试，需要可用的 pdfLaTeX 和图形环境。测试覆盖跨文件跳转和重命名、未命名文档补全、BibTeX、CodeLens 编译、打开 PDF、重编译、配置热更新、保存后编译、编译取消及子进程回收、诊断、重启后的未保存内容同步和错误路径修复后的重连。Linux 无显示环境时使用 `xvfb-run -a npm test`。

可通过 `VSCODE_EXECUTABLE_PATH` 使用已安装的 VS Code 主程序，通过 `LATEX_LSP_TEST_SERVER` 指定服务器。macOS 示例：

```sh
VSCODE_EXECUTABLE_PATH='/Applications/Visual Studio Code.app/Contents/MacOS/Code' npm test
```

扩展和服务器可以分别发布版本；变更自定义命令或配置协议时，应在同一提交中更新双方并运行宿主测试。

打包工具的间接依赖 `@azure/msal-node` 使用的 UUID 库通过 npm override 升级至兼容 CommonJS 的 11.x 修复版，避免锁入旧版 UUID 的已知漏洞。该依赖只参与开发和打包，不进入 VSIX。
