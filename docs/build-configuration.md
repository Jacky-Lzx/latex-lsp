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
    "auxiliaryDirectory": null,
    "finalOutputDirectory": null,
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

这样，编译、PDF / 日志定位和 SyncTeX 都使用主文档旁的 `build/`。显式的 `outputDirectory` 优先于 rc 文件；未设置时回退到 `build/latex-lsp/`。支持单引号和不含插值的双引号字符串、注释及多次直接赋值（最后一次生效）；`$out_dir = '';` 表示主文档目录。每次编译重新读取磁盘配置，编译中配置改变会使结果过期。

配置按系统、用户、项目顺序叠加；用户配置优先采用 `$XDG_CONFIG_HOME/latexmk/latexmkrc`，否则采用 `~/.latexmkrc`，系统配置遵守 `LATEXMKRCSYS`。每层采用 latexmk 的首个存在文件规则。主文件 program 指令和显式 LSP 设置优先于 rc。

普通查询只做有界的静态读取，不执行 Perl。遇到条件、函数、字符串拼接、环境变量插值、正则或包含其他 rc 的配置时，在编译入口调用实际 latexmk 的 `-dir-report-only` 与受控的 `-e` 报告代码，获取有效引擎和目录，不启动 TeX。解析最多等待 5 秒（或更短的 `timeoutSeconds`），失败时保留旧日志并报告原因。动态配置每次开始编译时重新求值；查询、诊断和 SyncTeX 使用按配置内容与进程环境校验的最近解析结果，不因状态轮询启动进程。

开始、完成通知和编译状态的 `configuration` 包含 `sources`、`issues` 与 `native`。原生报告记录 latexmk 加载的配置文件，并补充字面量 `do 'file'` / `require 'file'` 的依赖；动态计算的包含路径应通过 latexmk 的 `process_rc_file` 加载，以便报告和监测它们。直接调用 TeX 引擎和 Tectonic 时继续使用 LSP 的目录设置。


`build.rebuildArguments` 单独指定强制编译的完整参数列表，也支持 `%f`、`%o`、`%j`；普通 `latex.build` 仍使用 `arguments`。自定义了 `arguments` 或使用未知编译工具时，必须显式设置 `rebuildArguments` 才能调用 `latex.rebuild`，否则在修改旧日志前报错。例如自定义 latexmk 参数时，可设 `"rebuildArguments": ["-g", "-pdf", "-interaction=nonstopmode", "-file-line-error", "-synctex=1", "-no-shell-escape", "-outdir=%o", "%f"]`；参数应包含工具所需的完整编译选项，设置 `jobName` 时也需自行传入 `%j`。使用这份自定义参数的重编译通知中，`engine` 为 `null`。

例如 Neovim 配置中可加入 `init_options = { build = { executable = 'xelatex', onSave = true } }`。未配置显式后续处理时，手动使用 XeLaTeX/pdfLaTeX 只运行一遍；需要多遍处理时可以使用 latexmk，或配置下述文献与索引管线。

`build.jobName` 可设置 PDF、日志、recorder 和 SyncTeX 的共同文件名，例如 `"jobName": "论文 draft.v2"` 生成 `论文 draft.v2.pdf`，主文件仍可以叫 `main.tex`。latexmk 和三个直接 TeX 引擎会自动添加 `-jobname`；自定义 `arguments` 需自行传递 `%j`。名称支持字母、数字、空格、点、下划线和短横线，不能包含目录或前后空格。Tectonic 默认调用不支持该选项。开始和完成通知的 `jobName` 返回实际产物名称；两个项目若将写入同一个 PDF，会拒绝并行编译，待前一次结束后可再次编译。

使用 latexmk 时，`engine` 可设为 `pdflatex`、`xelatex` 或 `lualatex`，服务器会选择相应的多遍编译模式。未指定时，优先读取主文件前 20 行的 `% !TeX program = xelatex` 或 `% !TeX TS-program = lualatex`，再读取主文档旁 `.latexmkrc`（不存在时为 `latexmkrc`）中直接赋值的 `$pdf_mode`：`1` 为 pdfLaTeX、`4` 为 LuaLaTeX、`5` 为 XeLaTeX；未设置时使用 pdfLaTeX。这样默认参数不会将项目的 `$pdf_mode = 5;` 覆盖为 pdfLaTeX，普通编译、强制重编译和引擎通知采用同一设置。动态表达式通过上述原生解析获取；其他输出模式仍需显式选择受支持的 `engine`。

program 指令忽略大小写和逐字环境，只接受这三个引擎名称；子文件中的 program 指令不会覆盖主文件。显式 `engine` 优先于指令和 rc 文件，显式选择的直接引擎 `executable` 和自定义 `arguments` 保持其调用方式。例如 `init_options = { build = { engine = 'xelatex', onSave = true } }` 可用 latexmk 自动处理 XeLaTeX 的多遍编译及文献。

`build.indexTools` 可显式配置[索引与术语表处理管线](index-processing.md#显式编译管线)，在生成输入后运行工具并强制重编译；默认不启用。工具任务与当前编译共享取消、保存和产物冲突检查。

`build.bibliographyTool` 可显式配置 [BibTeX / Biber 流程](bibliography-processing.md)，暂存文献产物、核对处理日志并运行后续 TeX 遍次；默认不启用，可与索引任务共同配置。

rc 中缺少设置与无法解析是两种状态。静态解析无法判断时会进入原生解析；两者均失败时，在启动 TeX 和替换日志前报错，不会将无法判断的引擎静默覆盖为 pdfLaTeX。

`build.auxiliaryDirectory` 与 `build.finalOutputDirectory` 分别覆盖 `$aux_dir` 和 `$out2_dir`。未设置时从项目 rc 读取，空值或缺省继承 `$out_dir`。`outputDirectory` 仍表示 TeX 输出目录；日志、recorder、文献、索引和章节辅助文件使用辅助目录，PDF 与 SyncTeX 使用最终目录。默认 latexmk 参数同步传入这些目录；清理仅删除这些目录中的已知产物，并检查每个目录的路径绑定。自定义参数需自行传递与这些设置一致的工具选项。
