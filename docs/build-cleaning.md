# 清理辅助文件与产物

[返回编译指南](build.md)

通过 `workspace/executeCommand` 调用：

- `latex.cleanAuxiliary`：清理当前根文档、jobName 和产物目录中的已知辅助文件，保留 PDF 与 SyncTeX。
- `latex.cleanArtifacts`：额外清理该任务的 PDF、`.synctex` 和 `.synctex.gz`。

第一个参数可以是文档 URI，或包含 `uri` 与可选布尔值 `dryRun` 的对象。可先预览：

```json
{"command":"latex.cleanAuxiliary","arguments":[{"uri":"file:///project/main.tex","dryRun":true}]}
```

返回 `rootUri`、`outputDirectory`、`artifacts`、`dryRun`、候选 `files`、实际 `removed`、带原因的 `skipped` 和逐文件 `errors`。预览不修改文件或编译状态；实际清理会重新生成并校验清单。I/O 失败可能留下部分文件，已删除的路径仍会完整返回。

只处理当前任务的常见辅助后缀（aux、bbl、blg、bcf、run.xml、fdb_latexmk、fls、log、目录／导航及索引文件）。完整的 `.fls` 记录可以补充产物目录内子章节的 `.aux`；显式配置的命名索引也纳入清单。不会扫描整个目录或执行外部清理程序。`.bib`、`.ist`、源码、已知依赖、打开的缓冲区、目录和符号链接受到保护；Unix 上也保护源码的硬链接。

当前任务或同一产物目录中有活动编译时拒绝清理；与另一已知任务共享 PDF 或索引输出时也拒绝。请先结束编译，或为项目设置不同的 jobName／产物目录。目录不存在时返回空清单，不创建目录。执行前复核输出目录绑定、文件绑定、元数据和内容；计划失效时不开始删除。执行中再次变化会停止并报告错误。文件系统对外部进程的并发更改不提供事务回滚。

实际删除后清除当前任务的旧编译状态与编译诊断，并通过推送／拉取机制更新客户端；辅助清理保留 PDF 的输入历史，完整清理同时使该历史失效。静态编辑诊断继续有效。单次清单最多 4096 个候选，内容校验总预算 128 MiB，recorder 最多 4 MiB／32768 行；超限时拒绝或忽略不完整 recorder，避免无限遍历。
