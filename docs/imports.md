# Source loading with the import family

Supports the static two-parameter form of `import`, `subimport`, `inputfrom`, `subinputfrom`, `includefrom` and `subincludefrom`:

```latex
\import{parts/}{chapter}
% parts/chapter.tex inside：
\input{local}
\subimport{figures/}{diagram}
```

Imported source code participates in bullet points, definition jumps, references, and symbol renaming. The `sub` format inherits the current import directory. The ordinary `input` / `include` being imported will use this directory first, and then fall back to the main documentation. The filename parameter provides path completion, source code linking and signature help. Unexecuted imports in macro definition bodies, verbatim codes, and literal conversions will not add dependencies.

Context traverses up to 256 items and respects index dependency depth. Currently only direct static commands are analyzed, dynamic directories, command aliases or macro expansion are not performed; a single jump is not provided when multiple import contexts select different files. When file movement involves importing relative paths, automatic rewriting will be rejected and the import directory needs to be updated explicitly; symbol renaming is not subject to this restriction.
