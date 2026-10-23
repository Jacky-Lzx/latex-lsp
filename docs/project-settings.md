# 项目配置与作用域

编辑器初始化选项和 `workspace/didChangeConfiguration` 设置默认值。项目可以通过工作区目录下的 `.latex-lsp.json` 覆盖编译、格式化和诊断选项；子目录配置继承上级配置，距离文档最近的文件优先。仅查找文档所在工作区文件夹内的祖先目录；未配置工作区时只读取文档目录。

```json
{
  "formatting": { "formatter": "tex-fmt" },
  "build": { "onSave": false },
  "diagnostics": { "chktex": { "enabled": true } },
  "projects": {
    "book.tex": { "build": { "engine": "lualatex" } },
    "slides.tex": {
      "build": { "engine": "xelatex" },
      "formatting": { "formatter": "latexindent" }
    }
  }
}
```

`projects` 的键是相对于该配置文件目录的主文档路径，值是部分配置。选中主文档后才应用对应覆盖项；同一主文档的子文件使用该覆盖项。每层先应用普通配置，再应用匹配的主文档配置。`build.rootFile` 相对于声明它的配置目录解析；其他编译路径仍遵守编译指南。

同一工作区内的独立主文档各自限定符号范围，尚未写入磁盘、但已声明 `documentclass` 或 `begin{document}` 的主文档也保持独立。没有所属主文档的缓冲区可共享所在工作区文件夹的符号；不同工作区文件夹之间隔离。

服务器缓存配置文件的解析结果，文件元数据变化或文件监听事件会刷新缓存。错误配置会在文档开头产生 `project-configuration` 警告，该层配置被忽略。文件最大 1 MiB，最多向上检查 32 层。

`distribution` 和 `indexing` 属于服务器级设置，请在编辑器初始化选项或配置通知中设置。
