# SyncTeX queries

[Back to README](../README.md)

Query the location mapping between source code and PDF for editor and preview plug-in integration. For compilation configuration and product validity rules, see [Compilation](build.md).

- [Use](#purpose)
- [VS Code preview](#vs-code-preview)
- [Query protocol and return results](#query-protocol-and-return-results)

## Purpose

- SyncTeX query: source code lines locate PDF page numbers/coordinates, and PDF coordinates inversely check source code lines; return data for use by editors or PDF preview plug-ins to check unsaved sources and expired products.

## VS Code preview

The built-in viewer supports both directions after saving and successfully compiling with SyncTeX enabled. Use **TeXquill: Forward Search (TeX to PDF)** or Ctrl+Alt+J (Cmd+Alt+J on macOS) to reveal and highlight the source cursor's PDF location while keeping source focus. Ctrl+click (Cmd+click on macOS) in the PDF opens the corresponding source file and line. The extension passes these queries to the server; it does not discover roots or run a separate SyncTeX process. See the [extension guide](../editors/vscode/README.md#synctex-navigation) for setup and limitations.

## Query protocol and return results

After successful compilation, the installed `synctex` command line tool provides mapping between source code and PDF. Called via `workspace/executeCommand`:

```json
{
  "command": "latex.forwardSearch",
  "arguments": [{
    "uri": "file:///project/chapter.tex",
    "position": { "line": 12, "character": 4 }
  }]
}
```

Return `{rootUri, pdfUri, locations: [{page, x, y, h, v, width, height}, ...]}`. Source code `position` adheres to LSP's zero-based UTF-16 convention, and queries use row-level mapping; PDF `page` starts with 1, and `x` / `y` is measured from the upper left corner of the page, in units of 1/72 inches. Optional `h` / `v` / `width` / `height` typesetting box where the description is located.

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

The reverse check also uses the source file `uri` to identify the project and returns `{rootUri, pdfUri, locations: [Location, ...]}`, where `Location` is the standard LSP position that can be given to the editor to jump, and the range is at the beginning of the corresponding line. The query may return multiple candidates or an empty list, and the server will not automatically launch a PDF reader or editor. Queries are executed in the background, with up to 4 running simultaneously, each running for up to 5 seconds, returning 256 items; you can still edit and send other requests during this time. Standard `$/cancelRequest`, timeout and shutdown will terminate the query process and its sub-processes, and the source and product will be checked again before returning; when the PDF and mapping files are replaced with equal lengths during the query and the timestamp is retained, the results will also be discarded. This pre- and post-query check also takes effect when the server has not compiled the project. It is required that the output directory already contains PDF and `.synctex.gz` or `.synctex`. When the project has unsaved content, the source is later than the synchronization record, or the current compilation is running/the latest compilation was unsuccessful, you will be asked to save and recompile. After successfully building input that has recorded the current PDF, timestamp updates caused by saving the same content will not prevent queries, even if latexmk skips recompiling; newly discovered input and actual content changes must still be recompiled.
