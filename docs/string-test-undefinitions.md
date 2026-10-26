# Undefinition and string tests

[Back to documentation](README.md) · [Save and restore](string-test-copies.md) ·[Directly call the copy](string-test-copy-calls.md)

etoolbox's undefine command removes the meaning of the current control word. The server takes its target as the command name operand, updating the parameter bounds of the string test and its copies in load order.

| Command | Static target example |
| --- | --- |
| `undef` | `\undef\saved` or `\undef{\saved}` |
| `gundef` | `\gundef\saved` or `\gundef{\saved}` |
| `csundef` | `\csundef{saved}` |
| `csgundef` | `\csgundef{saved}` |

Provide signature help and `undef` / `gundef` completion fragments. The target is still an ordinary macro reference; navigation and renaming recognize undo positions when the symbol is uniquely identifiable. The undo operation does not generate a new macro declaration, nor does the corresponding file, declaration, or verbatim syntax execute because the target was originally called `input`, `label`, `newcommand`, or `verb`.

## Copy, Undo and Redefine

```tex
\usepackage{etoolbox}
\newbool{draft}
\let\original\ifblank
\undef\ifblank
\newcommand{\ifblank}[1]{#1}
\ifblank{\booltrue{draft}}
\original{\boolfalse{draft}}{Empty}{Tokens}
```

`original` retains the original string meaning saved before cancellation. The new `ifblank` is processed as a custom macro, so when the boolean `draft` is renamed, the declaration and `booltrue` are updated, and the `boolfalse{draft}` of `original` is retained as literal data.

Copies made after undoing no longer inherit the meaning of the removed string. Simply calling a revoked control word will produce a compilation error; the server will no longer apply the original string data parameters and signature help to it, and the subsequent curly brace content will be indexed as normal code. The compilation log can still pinpoint the undefined command error to the actual call.

The static `input` immediately following the undo operation is still involved in project analysis, so redefining the target in other files will also update subsequent calls. When an identified custom macro reads a control word as a bare operand, the control word will not in turn consume the subsequent code; this rule does not need to be undefined in advance. For details, see [String Test Command as Macro Parameter](string-test-operands.md).

## Update and support boundaries

Dependent incremental editing, restoring disk content after closing the buffer, and disk notifications will refresh the relevant meanings, and the main document version number remains unchanged. When a shared file in a different root may be either string data or code that is executable after undoing, rejecting renames that modify the ambiguous parameters, and other unambiguous symbols can still be edited.

Only infer confirmable static targets. Dynamic command names or undo interfaces whose meaning cannot be determined leave uncertainty and do not guess at the editing of the associated string parameters. Undo operations within the body of string data and uncalled macros are not performed at the point of declaration. Redefined undo commands are processed within known parameter boundaries and are not treated as definite delete operations.

The static analysis scope of the string context is used here, no arbitrary macro expansion is performed, and automatic recovery at the end of the group is not simulated; the difference between global effects and local undoes of `gundef` / `csgundef` is not used to infer unmodeled group scope.

## verify

```sh
cargo test --locked --test undefined_string_tests
cargo test --locked --test protocol string_undefinitions
cargo test --locked --test undefined_string_tests -- --include-ignored
```

True regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox. Four undo interfaces and four string predicates respectively verify the macro name, boolean strings before and after renaming, status and PDF page number; also check the error location and subsequent code execution after undoing.
