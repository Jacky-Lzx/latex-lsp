# 编译配置

[返回文档目录](README.md) · [编译指南](build.md)

使用 `initializationOptions`（Neovim 的 `init_options`）设置，运行中也可发送 `workspace/didChangeConfiguration`，其 `settings` 可直接使用下列对象或包在 `latex` 字段内。无效配置会被拒绝并保留原配置。

```json
{
  "build": {
    "executable": "latexmk",
    "arguments": null,
    "rebuildArguments": null,
    "engine": null,
    "outputDirectory": null,
    "rootFile": null,
    "onSave": false,
    "timeoutSeconds": 120,
    "indexTools": [],
    "bibliographyTool": null
  }
}
```

`rootFile` 相对于当前工作区根目录，`outputDirectory` 相对于主文档所在目录。`executable` 可改为 `pdflatex`、`xelatex`、`lualatex` 或 `tectonic`，省略 `arguments` 时自动选择对应的默认参数。自定义参数中的 `%f` 替换为根文件绝对路径，`%o` 替换为输出目录，`%j` 替换为产物名称（未配置时为主文件名去掉扩展名）；参数直接传给进程，不经过 shell。默认 TeX/latexmk 参数关闭 shell escape。超时范围为 1–3600 秒。

使用默认 latexmk 参数且未设置 `outputDirectory`（或设为 `null`）时，服务器读取主文档目录中的 `.latexmkrc`，不存在时读取 `latexmkrc`，自动采用其中直接赋值的输出目录：

```perl
$out_dir = 'build';
```

这样，编译、PDF / 日志定位和 SyncTeX 都使用主文档旁的 `build/`。显式的 `outputDirectory` 优先于 rc 文件；没有可识别设置时回退到 `build/latex-lsp/`。支持单引号和不含插值的双引号字符串、注释及多次直接赋值（最后一次生效）；`$out_dir = '';` 表示主文档目录。每次编译重新读取磁盘配置，编译中配置改变会使结果过期。

这里采用静态读取，不执行 Perl；条件、函数、字符串拼接、环境变量插值及其他动态表达式不作求值，也不自动读取用户级或系统级 rc 文件。遇到这些用法、自定义 `arguments`、单独配置 `$aux_dir` 或 `$out2_dir` 时，请显式设置 `outputDirectory` 并让编译参数的产物位置与之保持一致。直接调用 TeX 引擎和 Tectonic 时继续使用 LSP 的输出目录设置。

`build.rebuildArguments` 单独指定强制编译的完整参数列表，也支持 `%f`、`%o`、`%j`；普通 `latex.build` 仍使用 `arguments`。自定义了 `arguments` 或使用未知编译工具时，必须显式设置 `rebuildArguments` 才能调用 `latex.rebuild`，否则在修改旧日志前报错。例如自定义 latexmk 参数时，可设 `"rebuildArguments": ["-g", "-pdf", "-interaction=nonstopmode", "-file-line-error", "-synctex=1", "-no-shell-escape", "-outdir=%o", "%f"]`；参数应包含工具所需的完整编译选项，设置 `jobName` 时也需自行传入 `%j`。使用这份自定义参数的重编译通知中，`engine` 为 `null`。

例如 Neovim 配置中可加入 `init_options = { build = { executable = 'xelatex', onSave = true } }`。未配置显式后续处理时，手动使用 XeLaTeX/pdfLaTeX 只运行一遍；需要多遍处理时可以使用 latexmk，或配置下述文献与索引管线。

`build.jobName` 可设置 PDF、日志、recorder 和 SyncTeX 的共同文件名，例如 `"jobName": "论文 draft.v2"` 生成 `论文 draft.v2.pdf`，主文件仍可以叫 `main.tex`。latexmk 和三个直接 TeX 引擎会自动添加 `-jobname`；自定义 `arguments` 需自行传递 `%j`。名称支持字母、数字、空格、点、下划线和短横线，不能包含目录或前后空格。Tectonic 默认调用不支持该选项。开始和完成通知的 `jobName` 返回实际产物名称；两个项目若将写入同一个 PDF，会拒绝并行编译，待前一次结束后可再次编译。

使用 latexmk 时，`engine` 可设为 `pdflatex`、`xelatex` 或 `lualatex`，服务器会选择相应的多遍编译模式。未指定时，优先读取主文件前 20 行的 `% !TeX program = xelatex` 或 `% !TeX TS-program = lualatex`，再读取主文档旁 `.latexmkrc`（不存在时为 `latexmkrc`）中直接赋值的 `$pdf_mode`：`1` 为 pdfLaTeX、`4` 为 LuaLaTeX、`5` 为 XeLaTeX；没有可识别设置时使用 pdfLaTeX。这样默认参数不会将项目的 `$pdf_mode = 5;` 覆盖为 pdfLaTeX，普通编译、强制重编译和引擎通知采用同一设置。读取限制与 `$out_dir` 相同，动态表达式和其他输出模式不作推断，请显式设置 `engine`。

program 指令忽略大小写和逐字环境，只接受这三个引擎名称；子文件中的 program 指令不会覆盖主文件。显式 `engine` 优先于指令和 rc 文件，显式选择的直接引擎 `executable` 和自定义 `arguments` 保持其调用方式。例如 `init_options = { build = { engine = 'xelatex', onSave = true } }` 可用 latexmk 自动处理 XeLaTeX 的多遍编译及文献。

`build.indexTools` 可显式配置[索引与术语表处理管线](index-processing.md#显式编译管线)，在生成输入后运行工具并强制重编译；默认不启用。工具任务与当前编译共享取消、保存和产物冲突检查。

`build.bibliographyTool` 可显式配置 [BibTeX / Biber 流程](bibliography-processing.md)，暂存文献产物、核对处理日志并运行后续 TeX 遍次；默认不启用，可与索引任务共同配置。
