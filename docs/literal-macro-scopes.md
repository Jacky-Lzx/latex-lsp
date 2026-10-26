# Macro sources for special character fixes

[Back to documentation](README.md) · [Special character diagnostics](special-characters.md) · [Duplicate superscript and subscript](math-scripts.md)

The server checks the macro parameters against the project file loaded during compilation. `&`, `#`, `_`, and `^` in text special characters, redundant alignment delimiters, and repeated superscripts and subscripts, if consumed by statically identifiable macro parameters, retain the entire line of diagnostics and do not provide character replacement.

## Cross-file parameters

For example, `macros.tex` declares:

```tex
\def\bad#1#2{&}
```

Main file uses:

```tex
\documentclass{article}
\input{macros}
\begin{document}
\begin{tabular}{ll}A & B\bad x& C\\\end{tabular}
\end{document}
```

TeX may stop the source code echo of `Extra alignment tab` at `\bad x&`. Here `&` is the second parameter of the macro, and the actual delimiter comes from the macro expansion; the server will not recommend escaping this parameter to `\&`. Delimited parameters are also checked, for example `\def\bad#1&{&}`.

If `\bad` is changed to `\def\bad{}` with zero parameters, the same `&` is the delimiter for direct execution, and accurate diagnostics and escape repair can be obtained. Normally executed box contents, such as `\hbox{#}`, still support the character positioning of `#`.

## Repeat declarations and environment parameters

Duplicate declarations within the same file may also change the number of parameters:

```tex
\def\bad{}
\def\bad#1#2{&}
```

The server retains recognized candidate parameter forms. As long as one of the forms consumes the offending character, no direct replacement is provided; the check is also forwarded along pure control word aliases, terminating when a loop is encountered. Candidates are only used to exclude character edits and do not provide definite signatures, source loading, or navigation results for ambiguous macros.

The environment's start controls share a namespace with ordinary macros. When the parent file defines `\newenvironment{bad}[2]{&}{}` and the child file uses `\begin{bad}x&`, the second naked parameter will also not be repaired; traditional optional parameters, the xparse environment, and the pure alias of `\begin` are supported.

Repeated declarations without parameters and calls in which parameters have been completely read can still locate the delimiter for subsequent direct execution, such as `&` in `\begin{bad}xy&`. The parameter text and replacement body in the declaration are not treated as independent calls.

When there is a confirmed project source, complete project parameter analysis is used; aliases and target macros defined in different files will also be checked together to avoid using characters after the completed grouping parameters as parameters. Unresolvable declarations continue to remain undefined.

## Command and environment copy

Parameter candidates also recognize static copy declarations: `\let` (with optional `=`), `NewCommandCopy` / `RenewCommandCopy` / `DeclareCommandCopy`, the corresponding environment copy commands, and etoolbox's `cslet`, `letcs`, `csletcs`. Support for bare control words, grouped control words, static names, and explicit `expandafter` expanded targets `csname` allowed by these interfaces.

For example:

```tex
\def\consume#1#2{&}
\let\bad\consume
\def\consume{}
```

The copy `\bad` still retains the two parameters when copying, and `&` in subsequent `\bad x&` will not be used as a delimiter for direct execution. Static analysis preserves identifiable source declarations, does not simulate arbitrary assignment orders, and does not treat copy relationships as ordinary late-binding aliases; subsequent changes in the source do not automatically treat old copies as zero arguments.

The source control words in the copy statement are operands, not an actual call. For example `\let\saved\input` will not read subsequent files just because `input` was copied. When the copy keyword itself is redefined or the source is dynamic, the operand boundaries are not confirmed accordingly.

These candidates only participate in compile diagnostics and fix verification, and do not infer deterministic signature help or static file loading behavior for the replica. After the grouping parameters have been completely read, the real delimiter behind can still be located; after the macro file is changed, the repair menu and old client commands will recheck the copy relationship.

## Source Boundary

- Use project sources that were already loaded when the error occurred; macro signatures are not provided for files that have not yet been loaded.
- Parent file declarations that are paused when executing child files are only used to exclude possible macro arguments and are not used to confirm the meaning of the macro at the current position.
- Dynamically loaded or unconfirmed sources, macro files that change during compilation, retain the entire line of diagnostics.
- The analysis is static and does not perform arbitrary TeX macro expansion. The original source code echo, macro expansion frame, annotation and verbatim content inspection continue to apply.

## Fix revalidation

`data.macroSources` for accurate diagnostics records the loaded sources participating in the analysis; when there is a paused parent file, `data.macroPausedSources` is also recorded. The source must belong to the current project and the compiled source confirmed by the server.

The server rechecks the current macro parameters when requesting a Code Action or executing `latex.applyQuickFix` for an old client. Even if the main file version does not change, after the macro file is changed from zero parameters to two parameters, the original character repair will be rejected. The old command also checks the master file version, current diagnostics, and replacement content.

For test commands and external tool requirements, see [Real Tool Regression](testing.md).
