# User macro call hierarchy

[Back to documentation](README.md) · [Symbols and renaming](symbols.md) ·[Signature help](signatures.md)

Supports standards `textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls` and `callHierarchy/outgoingCalls`. Open the call hierarchy at a user macro name or call site to see which macros, which source files call it directly, and which other user macros are called by the macro's replacement text. Neovim works with `vim.lsp.buf.incoming_calls()` and `vim.lsp.buf.outgoing_calls()`.

For example:

```tex
\newcommand{\leaf}{Text}
\newcommand{\parent}{\leaf\leaf}
\parent
\leaf
```

The caller of `\leaf` contains `\parent` (two calling locations) and the current source file (one location at the top level of the file). Multiple locations from the same caller/target are combined into one result. After selecting the file caller, you can continue to view the outbound calls at the top level of the file; the positions within the macro body are classified into the corresponding macros. Recursive macros return their own direct calling edges and do not recursively expand the entire graph.

Uses current buffer with project dependencies, supports cross-file, unsaved edits, unnamed buffers, UTF-16 and CRLF. The scope of the macro node covers the declaration and replacement text, and the name range is used for jumps; the direct call position corresponds to the actual control sequence and does not include its parameters. Different root documents are isolated by existing project scope.

Static `\csuse{leaf}` and `\csname leaf\endcsname` can also point to the same `\leaf` node, supporting opening the call hierarchy from the name. The call position contains the entire named expression, including the name grouping or `endcsname`, and does not contain subsequent arguments to the target macro. The name must be statically confirmable; when `csuse`, `csname` or `endcsname` is explicitly redefined within the project, the corresponding expression is conservatively processed as a normal user command. A direct call to the redefine command itself can still be seen.

## Support boundaries

Currently limited to the only user commands in the project that have a complete replacement body. Traditional `newcommand` series, xparse command declarations, TeX `def` series, and etoolbox named definitions reuse existing parameter parsing. When there are multiple declarations with the same name, the definition is incomplete, or the definition is nested in the body of other definitions, the macro node is not provided; built-in commands and system package implementations do not enter this diagram.

Edges represent direct or static named calls in the source code, without executing TeX. Comments, verbatim content, literal conversion/testing, command name checking and copying operands, `noexpand` operands, macro parameter data and declaration parameter headers do not form a call. The target of a named declaration also does not form a call; calls in the body of the complete declaration are subsumed by this macro. Environment definitions, nested or other unidentified definition bodies also do not count as calls to outer macros or top-level files. Whether macro arguments are ultimately executed or not depends on expansion, and the graph only retains calls where existing argument analysis allows confirmation.

Dynamic names, captured meanings of copy commands, or arbitrary macro substitutions are not currently expanded; runtime branches, call counts, or execution order are not inferred. The existing Find References feature can still view name uses that are not part of the call.

`noexpand` follows the single token boundary in parameter analysis: only suppresses the following control sequence and does not treat the entire group as a macro parameter. The lexical blank after ordinary control words and literal `endcsname` is ignored, and the actual blank after the name grouping of `csuse` is still token; therefore, `\csuse{noexpand}\leaf` suppresses `leaf`, and `\csuse{noexpand} \leaf` still calls `leaf`. Confirmable forward-only aliases use the same rules. When the user explicitly redefines `noexpand` using its declared parameter syntax, the call to the user macro itself also enters the figure.

The data returned for the node retains the originally requested item origin and content fingerprint. Subsequent requests re-parse the current scope; when the node file content/version changes, the definition is removed, or becomes non-unique, `ContentModified` (`-32801`) is returned. Please reopen the calling hierarchy. Missing valid node data returns `InvalidParams`. The caller list can be updated with the latest contents of other files.

An explicit error is returned if the index is incomplete; a single analysis is also limited to 1024 LaTeX documents, 16 MiB total, 4096 declarations, 100,000 commands, and 20,000 calls, and an error is returned instead of a partial figure if the limit is exceeded.

## verify

```sh
cargo test --locked --lib call_hierarchy
cargo test --locked --test protocol call_hierarchy
cargo test --locked --lib real_noexpand_ -- --include-ignored
```

Covers project isolation, calls across files and at the top of files, duplicate locations, recursion, declaration syntax, macro parameters and literal data, nested definitions, UTF-16/CRLF, incremental editing, ambiguity, expired or invalid nodes, and analysis budgets.

The last one requires pdfLaTeX and etoolbox to compare primitives/user redefinitions with actual output times, named call whitespace, and grouped call edges.
