# Static brace diagnostics

[Back to documentation](README.md) · [Command-name operands and declarations](declaration-operands.md) ·[Compiler's redundant right curly brace diagnostics](extra-braces.md)

Check source code for brace pairings while editing, without starting compilation. The source of the diagnostics is `texquill` and the code is `syntax`.

| Diagnostics | Quick Fixes |
| --- | --- |
| `Unclosed brace` | Insert the missing `}` at the end of the document; if there is a comment at the end, break the line first |
| `Unexpected closing brace` | Delete the selected redundant `}` |

## Single token source for let

Raw `let` reads a source token. The following two curly braces are used for assignment and do not turn grouping on or off:

```tex
\let\opening={
\let\closing=}
{Ordinary text.}
```

These two assignments will not trigger curly brace diagnostics or insertion or deletion repair, and subsequent ordinary groupings will still be checked normally. Supports static `csname` targets omitting `=`, whitespace and comments between operands, and explicit `expandafter` expansion:

```tex
\expandafter\let\csname closing\endcsname=}
```

Macro declarations that actually occur after copying still go into the index. `futurelet` will continue to execute the look-ahead token, and then the curly braces will still participate in the matching. For details, see [Look-ahead Target](futurelet.md).

The original `meaning` also reads a token. The source curly braces of `\meaning{` and `\meaning}` do not participate in pairing; when viewed continuously and mixed with `let`, the consumed curly braces will not be mistaken as group parameters by subsequent commands. For the handling of command name references and file dependencies, see [meaning operands](meaning.md).

## Macro text and parameters

The macro definition body and grouping arguments are literal brace-paired when read. Occurrences of `let` in the body do not skip these curly braces; unclosed body and extra closed characters still report errors. `let` in macro argument-delimited text also does not consume the opening curly brace that defines the body.

Checks grouped arguments covering legacy declarations, xparse, environment body, static `csname` targets, and built-in commands. Unknown argument structures take a conservative literal pairing and do not perform any macro expansion, category code changes, or alias calls. When the previous text of this document has been explicitly rewritten as `let`, the subsequent source will no longer be confirmed according to the primitive rules; the recovery of meaning after the end of the grouping will not be simulated.

Compiler errors may come from execution, macro expansion, or other delimiters. For relevant log location and deletion suggestions, see [Excess Right Curly Brace](extra-braces.md).
