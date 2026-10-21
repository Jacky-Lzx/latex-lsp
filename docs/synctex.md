# SyncTeX 双向查询

[返回 README](../README.md)

查询源码与 PDF 的位置映射，供编辑器和预览插件集成。编译配置及产物有效性规则见[编译](build.md)。

- [用途](#用途)
- [查询协议与返回结果](#查询协议与返回结果)

## 用途

- SyncTeX 查询：源码行定位 PDF 页码/坐标，以及 PDF 坐标反查源码行；返回数据供编辑器或 PDF 预览插件使用，检查未保存来源和过期产物。

## 查询协议与返回结果

成功编译后，安装的 `synctex` 命令行工具可提供源码与 PDF 之间的映射。通过 `workspace/executeCommand` 调用：

```json
{
  "command": "latex.forwardSearch",
  "arguments": [{
    "uri": "file:///project/chapter.tex",
    "position": { "line": 12, "character": 4 }
  }]
}
```

返回 `{rootUri, pdfUri, locations: [{page, x, y, h, v, width, height}, ...]}`。源码 `position` 遵守 LSP 的零起始 UTF-16 约定，查询采用行级映射；PDF `page` 从 1 开始，`x` / `y` 从页面左上角计量，单位为 1/72 英寸。可选的 `h` / `v` / `width` / `height` 描述所在排版框。

```json
{
  "command": "latex.inverseSearch",
  "arguments": [{
    "uri": "file:///project/main.tex",
    "page": 1,
    "x": 150.0,
    "y": 135.0
  }]
}
```

反查也用源文件 `uri` 识别项目，返回 `{rootUri, pdfUri, locations: [Location, ...]}`，其中 `Location` 是可交给编辑器跳转的标准 LSP 位置，范围位于对应行首。查询可能返回多个候选或空列表，服务器不会自动启动 PDF 阅读器或编辑器。查询在后台执行，最多同时运行 4 个，每次最多运行 5 秒、返回 256 项；期间仍可编辑和发送其他请求。标准 `$/cancelRequest`、超时和 shutdown 会终止查询进程及其子进程，返回前会再次检查来源和产物；PDF 和映射文件在查询期间等长替换且保留时间戳时，也会丢弃该次结果，这项查询前后检查在服务器尚未编译过项目时同样生效。要求输出目录内已有 PDF 和 `.synctex.gz` 或 `.synctex`。项目存在未保存内容、来源晚于同步记录，或者当前编译运行中/最近编译未成功时，会要求保存并重新编译。
