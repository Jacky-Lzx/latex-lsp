# Boolean diagnostics and fixes

[Back to documentation index](README.md) · [Boolean switch and toggle](symbols.md#boolean-switch-and-toggle)

When etoolbox reports `Boolean '\ifdarft' undefined`, the server maps the conditional command name in the log to the base name of the source code `darft`, accurately marks the name and provides within-project spelling repair:

```tex
\newbool{draft}
\setbool{darft}{true}
\ifbool{darft}{Yes}{No}
```

Select ``Replace with boolean `draft` `` to modify only the corresponding name. Supports `setbool`, `booltrue`, `boolfalse`, `ifbool` and `notbool`, retains assignments, branches, comments and prefixes such as `\global`; supports cross-line parameters and repeated calls.

## Candidates and boundaries

Suggestions are from the current project static `newif`, `newbool` / `providebool`, `newboolean` / `provideboolean` declarations, including `newif` which expands the literal `csname`. toggle and ordinary commands use independent namespaces, and declarations from other projects are not involved in the repair. This candidate is not provided when a repeated declaration, conditional command, or any derived assignment command conflicts with another definition.

Built-in `true` / `false` are only read-only condition suggestions for `ifbool` / `notbool`; they have no corresponding assignment commands and cannot be used for assignment repair. Built-in conditions that are overridden by the project definition are not candidates.

Dynamic, incomplete or semantically blank names, macro expansion, operations consumed by macros, redefinitions, and log echoes that cannot be uniquely checked retain entire rows of errors. Ifthen's own errors retain the original row-level position; etoolbox's `Invalid boolean expression` see [Boolean expression diagnostics and repair](boolean-expressions.md).

## Protocol and Verification

The exact diagnostic code is `undefined-boolean`, and the name data is such as `{"boolean":"darft"}`, without the `\if` prefix in the log. Diagnose and record macro sources; provide re-checking of actual parameters, candidates, sources and versions before repair, support versioned editing and old clients, and reject expired repairs. See [Build diagnostics](diagnostics.md#analysis-scope-and-source-confirmation) for general rules, and see [Test Instructions](testing.md#boolean-diagnostic) for verification commands.
