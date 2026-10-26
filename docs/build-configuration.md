# Build configuration

[Back to documentation](README.md) · [Build guide](build.md)

Using the `initializationOptions` (Neovim's `init_options`) setting, `workspace/didChangeConfiguration` can also be sent live, whose `settings` can be used directly using the following objects or wrapped in the `latex` field. Invalid configurations are rejected and the original configuration is retained.

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

`rootFile` is relative to the root directory of the current workspace, and `outputDirectory` is relative to the directory where the main document is located. `executable` can be changed to `pdflatex`, `xelatex`, `lualatex` or `tectonic`. When `arguments` is omitted, the corresponding default parameters are automatically selected. `%f` in the custom parameters is replaced with the absolute path of the root file, `%o` is replaced with the output directory, and `%j` is replaced with the artifact name (remove the extension from the main file name when not configured); the parameters are passed directly to the process without going through the shell. The default TeX/latexmk parameter turns off shell escape. Timeout range is 1–3600 seconds.

When using the default latexmk parameters and not setting `outputDirectory` (or setting it to `null`), the server reads `.latexmkrc` in the main documentation, reads `latexmkrc` if it does not exist, and automatically uses the output directory directly assigned therein:

```perl
$out_dir = 'build';
```

This way, compilation, PDF/log positioning, and SyncTeX all use `build/` next to the main document. Explicit `outputDirectory` takes precedence over rc files; falls back to `build/texquill/` when not set. Supports single-quoted and double-quoted strings without interpolation, comments and multiple direct assignments (the last one takes effect); `$out_dir = '';` represents the main documentation. The disk configuration is re-read for each compilation. Configuration changes during compilation will cause the results to expire.

Configurations are superimposed in order of system, user, and project; user configuration takes priority with `$XDG_CONFIG_HOME/latexmk/latexmkrc`, otherwise `~/.latexmkrc` is used, and system configuration follows `LATEXMKRCSYS`. Each layer uses latexmk's first existing file rule. Main file program directives and explicit LSP settings take precedence over rc.

Ordinary queries only do bounded static reads and do not execute Perl. When encountering conditions, functions, string concatenation, environment variable interpolation, regular or configurations containing other rc, the `-dir-report-only` and controlled `-e` reporting codes of the actual latexmk are called at the compilation entry to obtain the effective engine and directory without starting TeX. When native parsing returns an empty output directory, a known static directory is used, otherwise the main file directory is used; `outputDirectory` can be explicitly set when a fixed location is required. Parse waits up to 5 seconds (or less for `timeoutSeconds`), retaining old logs on failure and reporting the reason. Dynamic configurations are re-evaluated each time compilation is started; queries, diagnostics, and SyncTeX use the latest parsed results verified against the configuration content and the process environment, and do not start processes due to status polling.

`configuration` for start, completion notification and compilation status includes `sources`, `issues` and `native`. The native report records the configuration files loaded by latexmk and supplements the dependencies of the literals `do 'file'` / `require 'file'`; dynamically calculated include paths should be loaded through latexmk's `process_rc_file` in order to report and monitor them. Continue to use LSP's directory settings when calling the TeX engine and Tectonic directly.


`build.rebuildArguments` separately specifies the complete parameter list for forced compilation, and also supports `%f`, `%o`, and `%j`; ordinary `latex.build` still uses `arguments`. When `arguments` is customized or unknown compilation tools are used, `rebuildArguments` must be explicitly set before `latex.rebuild` can be called, otherwise an error will be reported before modifying the old log. For example, when customizing latexmk parameters, you can set `"rebuildArguments": ["-g", "-pdf", "-interaction=nonstopmode", "-file-line-error", "-synctex=1", "-no-shell-escape", "-outdir=%o", "%f"]`; the parameters should include the complete compilation options required by the tool. When setting `jobName`, you also need to pass in `%j` yourself. In the recompilation notice using this custom parameter, `engine` is `null`.

For example, `init_options = { build = { executable = 'xelatex', onSave = true } }` can be added to the Neovim configuration. When explicit subsequent processing is not configured, manually use XeLaTeX/pdfLaTeX to run it only once; when multiple passes of processing are required, you can use latexmk, or configure the following document and index pipelines.

`build.jobName` can set the common file name of PDF, log, recorder and SyncTeX. For example, `"jobName": "论文 draft.v2"` generates `论文 draft.v2.pdf`, and the main file can still be called `main.tex`. latexmk and the three direct TeX engines will automatically add `-jobname`; custom `arguments` needs to pass `%j` by yourself. The name supports letters, numbers, spaces, dots, underscores and dashes, and cannot contain directories or leading or trailing spaces. Tectonic default invocation does not support this option. `jobName` of the start and completion notifications returns the actual artifact name; if two projects are written to the same PDF, parallel compilation will be refused, and they can be compiled again after the previous one is completed.

When using latexmk, `engine` can be set to `pdflatex`, `xelatex` or `lualatex`, and the server will select the corresponding multi-pass compilation mode. When not specified, `% !TeX program = xelatex` or `% !TeX TS-program = lualatex` in the first 20 lines of the main file is read first, and then `$pdf_mode` directly assigned in `.latexmkrc` (`latexmkrc` if it does not exist) next to the main document is read: `1` is pdfLaTeX, `4` is LuaLaTeX, `5` is XeLaTeX; when not set, pdfLaTeX is used. In this way, the default parameters will not overwrite the project's `$pdf_mode = 5;` to pdfLaTeX, and the same settings will be used for normal compilation, forced recompilation and engine notification. Dynamic expressions are obtained via the native parsing described above; other output modes still require explicit selection of supported `engine`.

The program directive ignores case and verbatim context and only accepts these three engine names; program directives in subfiles do not overwrite the main file. Explicit `engine` takes precedence over directives and rc files, explicitly selected direct engines `executable` and custom `arguments` retain their calling mode. For example, `init_options = { build = { engine = 'xelatex', onSave = true } }` can use latexmk to automatically handle multi-pass compilation and documentation of XeLaTeX.

`build.indexTools` [Index and Glossary Processing Pipeline](index-processing.md#explicit-compilation-pipeline) can be configured explicitly to run the tool after generating input and force a recompile; not enabled by default. Tool tasks share cancellation, save, and artifact conflict checking with the current compilation.

`build.bibliographyTool` can explicitly configure [BibTeX / Biber process](bibliography-processing.md) to temporarily store document products, check processing logs, and run subsequent TeX passes; it is not enabled by default and can be configured together with the indexing task.

Missing settings in rc and failure to parse are two states. When static parsing cannot be determined, it will enter native parsing; when both fail, an error will be reported before starting TeX and replacing the log, and the undetermined engine will not be silently overwritten into pdfLaTeX.

`build.auxiliaryDirectory` and `build.finalOutputDirectory` are used for default latexmk calls (other tools require custom parameters), covering `$aux_dir` and `$out2_dir` respectively. Read from project rc when not set, null or default inherited `$out_dir`. `outputDirectory` still represents the TeX output directory; log, recorder, bibliography, index, and chapter auxiliary files use the auxiliary directory, and PDF and SyncTeX use the final directory. The default latexmk parameters are passed in these directories synchronously; cleaning only removes known artifacts in these directories and checks the path bindings for each directory. Custom parameters need to pass their own tool options consistent with these settings.

Customize `arguments` / `rebuildArguments` and call it as it is. The engine or directory will not be automatically inferred. Please configure the product location explicitly. rc dependency monitoring takes effect independently: system, user and project rc are tracked by default; these default sources are excluded when `-norc` / `--norc` exists; each `-r file` / `--r file` and its literal containing files are still tracked. The configuration path supports `%f`, `%o`, and `%j`, and is expanded only once like the compilation parameters. Force compilation of the rc collection using `rebuildArguments`; rc content modifications, deletions, and candidate file creation will expire the results of the compilation, even if the editor does not send file monitoring events, the content will be checked on completion.
