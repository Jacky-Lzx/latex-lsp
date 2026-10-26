# Special character diagnostics and fixes

[Back to documentation](README.md) · [Compilation diagnostics](diagnostics.md)

When the compiler reports bare `&`, `#`, `_`, `^` or redundant alignment delimiters in the text, the server will check the source code echo of the log, accurately mark the error characters, and provide escape quick fixes.

For the deletion operation of redundant `}`, see [redundant right curly bracket](extra-braces.md).

`Double subscript.` / `Double superscript.` in math mode diagnosed and repaired using independent [repeat superscript and subscript](math-scripts.md).

## use

Save the file and execute `latex.build`, requesting the editor's code operation at the wrong location:

| Compilation errors | Quick fixes | Modify results |
| --- | --- | --- |
| `Misplaced alignment tab character &.` | `Escape '&' as '\&'` | `&` → `\&` |
| `Extra alignment tab has been changed to \cr.` | `Escape '&' as '\&'` | `&` → `\&` |
| `You can't use` followed by `macro parameter character #` mode error | `Escape '#' as '\#'` | `#` → `\#` |
| `Missing $ inserted.`, echo confirmation `_` | `Escape '_' as '\_'` | `_` → `\_` |
| `Missing $ inserted.`, echo confirmation `^` | `Escape '^' as '\textasciicircum{}'` | `^` → `\textasciicircum{}` |

Support for `#` above bug in horizontal, vertical, math, restricted horizontal and internal vertical modes. For example:

```tex
Before & and & after.
Before # after.
```

Each errored character generates separate diagnostics and repairs. After applying item by item it becomes:

```tex
Before \& and \& after.
Before \# after.
```

Save and compile again and the resolved diagnostics will be cleared. The fix replaces only the selected character, preserving the rest of the same row; the legal `&` in the table will not produce this kind of fix.

## Extraneous alignment separators

TeX reports `Extra alignment tab has been changed to \cr.` when there are more `&` in a table or array than the column format allows. The server checks the internal `\endtemplate` read frame or the `<template>` frame ending with this command before locating the error to the actual delimiter; supports normal and expanded error contexts, `tabular`, math `array`, and raw `\halign`.

Accurate diagnostics uses the `extra-alignment-tab` code, and still saves characters and echoes through `data.character` and `data.sourceContext`. Escape operations are provided only when the source code can be confirmed.

If you need to display the literal `&` here, you can change:

```tex
\begin{tabular}{ll}A & B & C\\\end{tabular}
```

Fixed to:

```tex
\begin{tabular}{ll}A & B \& C\\\end{tabular}
```

If a third column is required here, the column format should be adjusted (for example, to `{lll}`). The escaping operation will change the selected delimiter to the literal character in the current cell, which needs to be selected according to the content.

Unmatched inner frames, extra user macro expansion frames, duplicate shortened echoes, and recognized macro parameters in loaded project sources maintain full line diagnostics. Delimiters can still be positioned after normal typesetting commands without parameters, such as `\hfil&`. For cross-file bare parameters, separated parameters and source boundaries, see [Macro Source Description](literal-macro-scopes.md).

## Underscores and carets in text

`Missing $ inserted.` may be caused by missing math delimiters or by entering `_` / `^` in normal text. If the target is a formula, the correct mathematical delimiter should be added; the escaping operation is suitable for situations where literal characters really need to be displayed, such as changing `file_name` to `file\_name`.

The caret is output using `\textasciicircum{}`; `\^` is an accent command and will treat the following characters as accent objects, so it will not be used as the replacement text for this operation. Existing mathematical superscripts and subscripts (e.g. `$x_1^2$`) remain intact.

Precise diagnostics and escape operations are only provided if the log contains the exact `<inserted text>` / `$` recovery frame and the source code echo that follows ends with the actual errored `_` or `^`. Collateral errors generated at `\par`, end of paragraph, or `\end{document}` during TeX recovery will continue to retain the original diagnostics; recompiling after applying the fix will clear the corresponding collateral errors. Unwinding the stack in command arguments, other reasons for missing math mode, and unclear error locations do not provide this operation.

## Positioning and editing verification

- Support traditional `!` log and "filename:linenumber:error" format; subfile errors are located to the actual source, and the position is calculated in UTF-16 units of LSP.
- The complete `l.N` echo must be consistent with the source code prefix; the echo shortened to `...` by TeX must be uniquely matched in the entire line.
- The `unescaped-special-character` code is used for accurate diagnostics of special characters in the text, and `extra-alignment-tab` is used for redundant alignment delimiters; `data.character` saves the characters, and `data.sourceContext` saves the source code echo for verification.
- Supports versioned Code Action editing, and the `latex.applyQuickFix` command for legacy clients. Compilation diagnostics, current source code, and macro parameters are rechecked when a fix is requested; documentation versions are also rechecked when old client commands are executed. For the processing of cross-file macro changes, see [Repair and Recheck](literal-macro-scopes.md#fix-revalidation).

Macro expansion stacks, unconfirmed sources, invalid line numbers, source code changes, or non-unique echoes retain the original diagnostics and do not provide escape editing. Comments, verbatim content, macro definition bodies, and escaped characters are also not included in the repair. This function provides suggestions based on the compilation errors that have occurred, and does not treat all occurrences of `&` / `#` / `_` / `^` as errors.

For the verification method of the real engine, see [Real Tool Regression](testing.md).
