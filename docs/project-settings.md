# Project settings and scope

Editor initialization options and `workspace/didChangeConfiguration` set default values. The project can override the compilation, formatting and diagnostic options through `.texquill.json` in the workspace directory; the subdirectory configuration inherits the upper-level configuration, and the file closest to the document takes precedence. Only the ancestor directories in the workspace folder where the document is located are searched; when the workspace is not configured, only the documentation is read.

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

The key of `projects` is the main document path relative to the configuration file directory, and the value is the partial configuration. The corresponding override is applied only after the main document is selected; subfiles in the same main document use the override. Each layer applies the normal configuration first, and then applies the matching main document configuration. `build.rootFile` is resolved relative to the configuration directory in which it is declared; other compilation paths still adhere to the compilation guidelines.

Independent root documents in the same workspace each have their own symbol ranges, as do root documents that have not yet been written to disk but have declared `documentclass` or `begin{document}`. Buffers that do not belong to the main document can share symbols of the workspace folder in which they are located; different workspace folders are isolated. Source code and documents explicitly referenced by the current file through the static load command remain in scope, even if these dependencies are located outside the workspace directory.

The server caches the parsing results of the configuration file. Changes in file metadata or file listening events will refresh the cache. Incorrect configuration will produce a `project-configuration` warning at the beginning of the document, and the layer configuration is ignored. File size is up to 1 MiB and checks up to 32 layers upward.

`distribution` and `indexing` are server-level settings, please set them in the editor initialization options or configuration notifications.
