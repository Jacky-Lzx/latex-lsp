# String tests

[Return to Documentation](README.md) · [Symbols and Renaming](symbols.md) · [Boolean Expression Diagnostics](boolean-expressions.md)

The following test of etoolbox handles the parameters as unexpanded token data and then selects the conditional branch:

| Command | Data parameters | Judgment |
| --- | --- | --- |
| `ifstrequal` | Two strings | Character forms of unexpanded content are the same |
| `ifstrempty` | A string | Empty string; ordinary spaces still belong to the content |
| `ifblank` | A string | Empty string or spaces only |
| `notblank` | A string | Contains non-space token |

Each command subsequently accepts `{then}{else}`. Provides command completion fragments and signature help; prompts for outer test commands are displayed within the string, and prompts for nested commands can continue to be displayed within the branch.

## Data and References

```tex
\newbool{draft}
\ifstrequal{\ifbool{draft}}{\ifbool{release}}
  {Same}
  {Different.\booltrue{draft}}
```

Here the two strings are compared as-is. When the boolean `draft` is renamed to `release`, the `booltrue` parameters in the declaration and branch are updated, the two strings used for comparison retain the original text, and the result is still `Different`.

Macros, statements such as `newbool`, labels, references, environments, and file commands such as `input` in the string are all data, and no corresponding symbols, references or project dependencies are established. The data area does not provide definition, renaming, hover and name completion, and semantic highlighting by string. Both branches are still indexed according to the source code and support normal navigation, completion and versioned cross-file renaming; the server does not simulate conditional evaluation.

## Read boundaries

Supports complete brace parameters and single-token parameters with confirmable boundaries, such as `\ifstrempty\macro{Empty}{Nonempty}`. Comments, CRLFs, escape characters, and nested braces between parameters retain their original text. Commands such as `verb` in the data are only read as tokens, and the curly braces still follow the ordinary TeX parameter grouping rules.

Unclosed data parameters remain literal, and missing brace diagnostics are preserved. If the end of the file is the `%` comment in the parameter, the repair of the insertion and deletion of `}` will first wrap the line to ensure that the brackets take effect.

Unassigned direct calls are parsed according to the built-in command syntax; [Static copy call](string-test-copy-calls.md) uses the meaning and number of data parameters saved during copying. Explicit redefinitions in the current file and previously loaded local dependencies stop using the command's built-in data boundaries; see [Cross-file context](string-test-projects.md) for loading order, unsaved content, editing restrictions on shared files, and static support boundaries.

When the user macro reads the test command as an unexpanded control word parameter, the string test syntax is not executed at this location, and the subsequent code is parsed according to the actual parameter boundary; for the processing of cross-file declarations, unknown macros, and shared files, see [String test command as macro parameter](string-test-operands.md).

## Boolean expression diagnostics

`input` in `test {\ifstrequal{\input{ghost}}{A}}` is still string data. When these confirmed test parameters contain command tokens, the outer `bool` / `togl` operands can still get [accurate diagnostics and spelling fix](boolean-expressions.md). Expression positioning requires the use of full curly braces for test command string parameters and checks for test command redefinitions within the project; any user test macros continue to retain line-level diagnostics.

## verify

The default regression covers indexing, dependencies, name editing, completion, signature help, single tokens, comments, unclosed parameters and this file redefinition. For cross-file redefined regression commands, see [Project Context Verification](string-test-projects.md#verify). The following protocol regressions check literal data, versioned cross-file edits, and incremental modifications:

```sh
cargo test --locked --test string_predicates
cargo test --locked --test protocol string_predicates
```

True regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox. After renaming boolean and toggle respectively, check the string, conditional branch and PDF; then introduce the expression spelling error and check the same result after accurate repair and recompilation:

```sh
cargo test --locked --test string_predicates -- --include-ignored
```
