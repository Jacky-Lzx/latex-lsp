# Saving and restoring string commands

[Back to documentation](README.md) · [Cross-file context](string-test-projects.md) · [Command-name operands](declaration-operands.md)

The copy can also be called directly, retaining the number of data parameters at the time of copying. For examples, signature help, shared file ambiguities and compilation diagnostics, see [Direct call of copies](string-test-copy-calls.md).

## Explicit recovery

`ifstrequal`, `ifstrempty`, `ifblank` and `notblank` support copying and saving the original meaning through static commands and then restoring it explicitly. Copying captures the meaning when execution reaches that position; redefining the source command later does not alter the already saved copy.

For example main file:

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\let\saved\ifblank
\begin{document}
\input{override}
\ifblank{\booltrue{draft}}
\input{restore}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\end{document}
```

`override.tex`：

```tex
\renewcommand{\ifblank}[1]{#1}
```

`restore.tex`：

```tex
\let\ifblank\saved
```

When boolean `draft` is renamed, `booltrue` in declarations and custom calls will be updated, and the restored `boolfalse{draft}` is retained as string data. The name of the saved copy, `saved`, can also be updated through ordinary macro renaming, and the recovery effect will be maintained.

## Copy and parameter boundaries

Supports unredefined `let` (with optional `=`), `cslet`, `letcs`, `csletcs`, `NewCommandCopy`, `RenewCommandCopy` and `DeclareCommandCopy`, source and target names must be statically resolvable. Chain copying, nested file loading, and saving custom meanings and then copying them back are all propagated in the actual order of analysis. The overwritten copy is no longer treated as the original string command; copying of string data and macro bodies that have not yet been called is not performed at the declaration point.

Confirmed custom parameter forms are also passed to the calling file, including normal parameters, optional parameters, xparse declarations and static TeX delimited parameters. Therefore, the single-parameter customized `ifblank` will not accidentally swallow the following `input`, and the restored file will still participate in project analysis. Argument boundaries are used to determine reading order, no arbitrary macro expansion is performed.

Buffer-dependent opens, incremental edits, close recovery, and disk notifications update subsequent string semantics, and the main document version number remains unchanged. When repeated loading or multiple root documents make the same parameter have both a literal and an executable meaning, edits that would modify that parameter are rejected, and other explicit symbols can still be renamed.

When undoing a meaning via `undef` / `gundef` / `csundef` / `csgundef`, earlier copies still retain the captured content, and later copies no longer inherit the deleted string syntax, see [Undoing Definition](string-test-undefinitions.md).

## Support boundaries

Recognizes the implications of direct calls to static copies and recovery after the four test commands above, without inferring side effects of user macro calls or automatic recovery at the end of a group. When copying a target or source name that requires macro expansion, its effect is not inferred. Uncertainty remains when the source meaning cannot be confirmed or the copy primitive is redefined; conflicts in the number of data parameters in multiple contexts are handled according to [Common Literal Prefixes and Ambiguity Rules](string-test-copy-calls.md#cross-file-updates-and-ambiguities).

## verify

```sh
cargo test --locked --test string_test_projects
cargo test --locked --test protocol restored_string_predicates
cargo test --locked --test string_test_projects -- --include-ignored
```

True regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox. The three engines respectively cover the native, customized and restored calls of the four string tests, and compare the original string text, boolean status and PDF page number before and after renaming.
