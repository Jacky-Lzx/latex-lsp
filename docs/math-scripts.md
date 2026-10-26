# Duplicate math scripts

[Back to documentation](README.md) · [Compilation diagnostics](diagnostics.md) · [Text special characters](special-characters.md)

When TeX reports `Double subscript.` or `Double superscript.`, the same mathematical base already has similar superscripts and subscripts. The source code echo of the server check log accurately identifies the second `_` or `^`, and provides a quick fix to explicitly add an empty base.

## use

| Errors | Diagnostic Codes | Quick Fixes | Modify Examples |
| --- | --- | --- | --- |
| `Double subscript.` | `double-subscript` | `Insert empty base before '_'` | `x_1_2` → `x_1{}_2` |
| `Double superscript.` | `double-superscript` | `Insert empty base before '^'` | `y^1^2` → `y^1{}^2` |

The operation only replaces the selected script symbol with `{}_` or `{}^`, retaining the existing superscript and subscript content and subsequent grouping. For example:

```tex
$x_{ab}_{cd}$ and $y^{ab}^{cd}$
```

After selecting repair respectively, you get:

```tex
$x_{ab}{}_{cd}$ and $y^{ab}{}^{cd}$
```

The empty base allows subsequent superscripts and subscripts to be attached to new mathematical atoms. If you want to combine them into a superscript and subscript, or express a nested relationship, you should adjust the curly braces according to the meaning of the formula, such as `x_{12}` or `x_{1_2}`. Normal `x_1^2` does not belong to repeated superscripts and subscripts; `_` / `^` escapes in the text are still processed by [Special Character Diagnostics](special-characters.md).

Supports inline and display formulas, grouping superscripts and subscripts, and comment continuation lines. Multiple errors on the same line are located separately; recompile after repair, and the resolved diagnoses are cleared.

## Source confirmation

`data.character` for accurate diagnostics saves the script symbols, and `data.sourceContext` saves the source code echo for verification. Supports legacy `!` logs, file line number format, subfiles, CRLF and UTF-16 locations. The full echo must match the source code prefix, and the shortened echo must match uniquely in the entire line.

Macro expansion frames, mismatched or non-unique echoes, unknown sources, and invalid line numbers retain full line diagnostics. Comments, verbatim content, macro definition bodies, and escaped symbols do not provide this operation; script symbols in macro parameters recognized in the source of the loaded project are not considered to be superscripts or subscripts for direct execution. For cross-file source boundaries, see [Macro Source Description](literal-macro-scopes.md).

Supports versioned Code Action and `latex.applyQuickFix` for legacy clients. The current diagnostics, messages, characters, source code and macro parameters are checked when requesting repair; the document version is also checked when the old command is executed, and the overwritten replacement content is rejected.
