# Extra closing braces

[Back to documentation](README.md) · [Compilation diagnostics](diagnostics.md) ·[Special character escape](special-characters.md)

When the compiler reports redundant `}`, the server will check the source code echo in the log, accurately mark the character, and provide a quick fix for `Remove extra '}'`. The operation only deletes the selected character, and then recompiles to confirm whether the error is resolved.

## Supported errors

| Compiler Messages | Common Locations |
| --- | --- |
| `Too many }'s.` | Extraneous right curly brace in normal text |
| `Extra }, or forgotten $.` | Math Mode |
| `Extra }, or forgotten \endgroup.` | `\begingroup ... \endgroup` Group |
| `Extra }, or forgotten \right.` | `\left ... \right` delimiter |

For example, choose a quick fix for the error location below:

```tex
{Valid} Before } after.
```

will get:

```tex
{Valid} Before  after.
```

Deletion preserves other groupings and whitespace. When there are multiple errors on the same line, each character is located and repaired separately.

If the error actually results from omission of `$`, `\endgroup` or `\right`, the corresponding delimiter should be completed. The deletion suggestion applies to cases where `}` is indeed overwritten; the server retains the complete original message of the compiler for optional repair methods.

## Source confirmation and client support

- Use the `extra-closing-brace` code for accurate diagnostics; `data.character` is `}`, and `data.sourceContext` saves the echo for verification.
- Supports legacy `!` log and file line number formats, subfiles, CRLF and UTF-16 locations. The complete echo must match the source code prefix; the echo shortened by `...` must be uniquely matched.
- Macro expansion, unidentified sources, invalid line numbers, unmatched or non-unique echoes retain whole line diagnostics. Comments, verbatim content, macro definition bodies and `\}` do not provide deletion operations.
- Support for versioned Code Action and `latex.applyQuickFix` for legacy clients. Requesting and performing fixes will recheck current diagnostics, source code, and documentation versions, and reject out-of-date or overwritten edits.

This feature provides fixes based on compilation errors that have occurred. For the pairing check during editing, the literal source of `let` and the macro text rules, see [Static Braces Diagnostics](brace-diagnostics.md).
