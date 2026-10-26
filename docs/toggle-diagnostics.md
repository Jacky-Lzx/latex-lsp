# Toggle diagnostics and fixes

[Back to documentation index](README.md) · [Boolean switch and toggle](symbols.md#boolean-switch-and-toggle)

When etoolbox reports `Toggle 'darft' undefined`, the server checks the source code and log echo, accurately marks the name of the direct operation, and provides spelling fixes within the current project:

```tex
\newtoggle{draft}
\settoggle{darft}{true}
\iftoggle{darft}{Yes}{No}
```

Select ``Replace with toggle `draft` `` to modify only the corresponding name, retaining the assignment, branch text, and prefixes such as `\global`. Supports `settoggle`, `toggletrue`, `togglefalse`, `iftoggle` and `nottoggle`, including cross-row parameters and repeated calls on the same row.

## Candidates and boundaries

Suggested static `newtoggle` / `providetoggle` declarations from the current project. Symbols with the same name in booleans, ordinary commands, and other projects are not candidates; this candidate is not provided when repeated declarations or internal `etb@tgl@name` control sequences conflict with macros, registers, environments, etc.

Dynamic or incomplete arguments, redefined operations, macro expansion, operations consumed as parameters by macros, and source code echoes that cannot be uniquely matched retain the entire line of diagnostics. For the `togl {...}` error in Boolean expressions, see [Boolean Expression Diagnostics and Repair](boolean-expressions.md).

## Protocol and Verification

The precise diagnostic code is `undefined-toggle`, the name data is such as `{"toggle":"darft"}`, and the source of the project macro when the error occurs is recorded. Recheck parameter boundaries, candidate declarations, sources, and document versions before repairing; be compatible with versioned editing and old clients, and reject out-of-date repairs. See [Build diagnostics](diagnostics.md#analysis-scope-and-source-confirmation) for general rules, and see [Test Instructions](testing.md#toggle-diagnostic) for verification commands.
