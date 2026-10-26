# Boolean expression diagnostics and fixes

[Back to documentation index](README.md) · [boolean diagnostic](boolean-diagnostics.md) · [toggle diagnostic](toggle-diagnostics.md)

etoolbox's `ifboolexpr`, `ifboolexpe`, `whileboolexpr` and `unlessboolexpr` will report the undefined operand as `Invalid boolean expression`, the specific name is in the help text of the log. The server retains this description and accurately marks the name in `bool {...}` or `togl {...}` after checking the full expression and source code echo.

```tex
\newbool{draft}
\newtoggle{draft}
\ifboolexpr{not (bool {darft} and togl {darft})}{Yes}{No}
```

The two misspellings provide ``Replace with boolean `draft` `` and ``Replace with toggle `draft` `` respectively, preserving operators, groupings, operands from another namespace, and branches. boolean candidates and toggle candidates each follow the corresponding [Declaration Rules](boolean-diagnostics.md#candidates-and-boundaries) and [Conflict Check](toggle-diagnostics.md#candidates-and-boundaries). The `bool` operand also supports the uncovered built-in `true` / `false` suggestions.

## Expression bounds

Supports static names, `not` / `and` / `or`, parentheses, comments and cross-line parameters, including following keywords such as `nottogl{...}`. `test {...}` supports unredefined `ifstrequal`, `ifstrempty`, `ifblank`, and `notblank`, requiring full curly braces for string parameters. [Confirmed static copy](string-test-copy-calls.md) is also recognized at compile time, using the number of data parameters saved at the call site. These [string tests](string-tests.md) do not expand commands in the data, similar operand text and command tokens in strings are not used as executable references, and do not prevent the precise positioning of outer operands.

Dynamic names, arbitrary macro-generated expressions or tests, redefinitions, broken or unidentified syntax, and duplicate names in the same namespace within the same expression retain row-level diagnostics. Macro-generated names and nested tests may produce the same echo as the outer operand, so the source code cannot be searched by error name alone. The original errors are also retained when the analysis exceeds the budget.

The loop command has read the expression and loop body before reporting an error, and the log must confirm the complete reading boundary; the conditional command only reads the expression before reporting an error. The fix does not modify the loop body, conditional branches, or assignments.

The shortened source code echo must be unique within the corresponding source code line and consistent with the read endpoint of the call. When the full name is provided in the help text, the echo can cover only the second half of the name; if only the closing curly brace is left, or the long loop body makes the operands completely invisible, line-level diagnostics are retained.

## Comments and source code editing

Comment continuation lines within keywords follow TeX's reading rules. For example, the following `bo` and `ol` form `bool`:

```tex
\newbool{draft}
\ifboolexpr{bo% keep this comment
  ol{draft}}{Yes}{No}
```

`bool`, `togl`, `not`, `and`, `or` and `test` all support this writing method, including CRLF, next line indentation and continuous comments. Navigation, referencing, hovering, name completion, and cross-file renaming recognize corresponding operands; compile diagnostics and spelling fixes use the source scope of the name itself. The editor retains keywords, comments and indentation, and the boolean and toggle with the same name are updated respectively.

Ordinary spaces, blank lines, and escaped percent signs will not be spliced into keywords. Similar operand literals in literal strings in `test` are not referenced as boolean / toggle references; the names themselves must still be static literal content.

## Logs and protocols

The diagnostic message appends the `Boolean 'name' undefined.` or `Toggle 'name' undefined.` given by the compiler, leaving separate errors for the two namespaces. When the standard help text is missing, incomplete, or cannot be confirmed to be related to the current error, the name will not be guessed; the line break description will be restored after the confirmation is complete and will not span other diagnoses.

Accurate diagnostic reuse `undefined-boolean` / `undefined-toggle` code, name data and macro source verification. Versioned edits, legacy clients, and obsolescence fixes follow [General Rules](diagnostics.md#analysis-scope-and-source-confirmation). Even if the main file version remains unchanged, the old fix will be rejected after the test command is redefined in the dependent file; the location must be reconfirmed after adding duplicate operands in the same namespace.

## verify

Default regression checks for two namespaces, read boundaries for expressions and loops, literal test strings, dynamic or ambiguous operands, help text correlation and wrapping, duplicate and shortened echoes, cross-line and UTF-16 ranges, and versioning and legacy client fixes:

```sh
cargo test --locked --test boolean_expression_diagnostics
cargo test --locked --test protocol expression_fixes_recheck_cross_file_tests_without_a_main_version_change
```

Comment continuation lines also add editing, diagnostics, and versioning cross-file rename regressions:

```sh
cargo test --locked --test expression_keywords
cargo test --locked --test protocol commented_expression_keywords_return_versioned_cross_file_renames_over_stdio
```

True regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox:

```sh
cargo test --locked --test boolean_expression_diagnostics -- --include-ignored
cargo test --locked --test expression_keywords -- --include-ignored
```

The three engines use 60/79 column and traditional/file line number logs respectively. Each group verified 11 operand fixes, recompiled to generate PDF, and checked the actual results of conditional branches, loop times, built-in conditions, cross-line expressions, and long names; it also confirmed that dynamic names and user test macros retain line-level diagnostics. For other external tool tests, see [Real Tool Regression](testing.md).

Real regression of comment continuation lines in three engines using traditional and file line number logs respectively. First, independently rename cross-file boolean and toggle, and check the results of branches, literal strings, and two loops; then introduce spelling errors in the two namespaces, verify the exact range, fix and recompile the same results.
