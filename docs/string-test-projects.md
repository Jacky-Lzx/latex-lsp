# String tests across files

[Back to documentation](README.md) · [String test](string-tests.md) · [Projects and files](projects.md)

## Loading order

Data boundaries for `ifstrequal`, `ifstrempty`, `ifblank`, and `notblank` are determined by the context of the direct call. After the current file or loaded local dependency explicitly redefines the command, subsequent calls stop using the built-in string syntax, and the symbols in the parameters are indexed as normal source code.

For example main file:

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\begin{document}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\input{definitions}
\ifblank{\booltrue{draft}}
\end{document}
```

`definitions.tex`：

```tex
\renewcommand{\ifblank}[1]{#1}
```

When the boolean `draft` is renamed, `booltrue` in the declaration and the second call will be updated; `boolfalse` in the first call is still string data and the original text is retained.

Supports direct source code inclusion and nested loading of local packages and document classes. [Chapter Selection](selected-string-tests.md) Filters the semantic propagation of `include` according to loading order. Redefinitions in excluded chapters will not affect subsequent string calls. The previous redefinition in the parent file is passed to the child file, and the redefinition of the child file also affects the call after returning to the parent file; the load command in the native string does not take effect. Loads and redefinitions that are only declared but not yet called in the macro body will not be propagated at the point of declaration.

Rely on unsaved content in the editor. Opening, incremental editing, closing dependency buffers and disk change notifications will update the relevant parsing; when closing, the disk content is restored, and the version number of the main document remains unchanged. New static input exposed by custom calls will also be loaded, including files outside the workspace directory, subject to [File Index Restrictions](projects.md#workspace-index-and-project-isolation).

[Save and Explicit Restore](string-test-copies.md) explains the meaning of command copy capture, cross-file parameter boundaries, and renaming rules before and after restore.

## Macro patches and deferral codes

The expanded/global form of `pretocmd`, `apptocmd`, `patchcmd`, `appto`/`preto` and the corresponding `cs...` interface may modify existing string readers without adding new source code declarations. The server revokes the target's native string authentication in the order in which it was called, without predicting the success of the patch or the replaced macro text. Static named calls and possibly pure forwarding/copying aliases also participate in protection, including aliases from unsaved serving files; these possible relationships are only used to reject incomplete edits. Dynamic targets or uncertified patch prefixes leave the meaning of the associated strings unknown.

For example, `\pretocmd{\ifblank}{#1}{}{}` will cause the original string parameters to be actually executed. At this time, only changing the declaration and the macro names in the two branches will miss a real call; the server rejects this partial edit, and irrelevant labels can still be completely renamed. Corresponding interactions can be restored after removing the patch, or explicitly restoring a confirmed native copy.

Potential modifications in patched replacement code, success/failure callbacks, and hooks or footers are also included in ongoing protection. The code may be executed again after the recovery operation, and the original meaning cannot be reconfirmed just because you see a recovery. Code parameters retain bounds in immediate execution analysis, and structure editors still recognize macros and static references within them. Ordinary macro bodies that have not yet been called are not executed at the declaration position.

The above patch/addition interface provides signature help; the prefix of `patchcmd[prefix]{command}{search}{replacement}{success}{failure}` is optional, and the command operands can be written directly as control sequences. Command completion snippets preserve TeX backslashes and existing arguments, and do not treat search text as executable code.

## Shared files and support boundaries

When the same source code is loaded multiple times in multiple main documents or in one main document, there may be two interpretations of string data and executable parameters at the same time. Such parameters do not provide single navigation or prepare rename results; renames initiated from the declaration that would modify it will also be rejected. Other symbols with the same semantics as in the file can still be renamed.

With static analysis, context propagation here is limited to direct file loading and explicit command assignment. For the local condition selection, pre-hook and file event update of `InputIfFileExists` / `IfFileExists`, see [Conditional File Hook](conditional-string-tests.md). It does not infer other TeX conditions, load aliases, dynamic command names, side effects after user macro calls or command recovery after grouping, and does not rely on package implementations within the TeX distribution. Conditional branches are still indexed according to the source code; when the loop, recursion depth or number of analyzes exceeds the limit, no arbitrary string interpretation will be selected, and relevant editing will be conservatively restricted.

## verify

Default regression covers load order, nested source code and packages, unsaved content, external dependencies, shared files, repeated loading, and boundaries between literal commands and deferred macro bodies. stdio regression checks for incremental edits, shutdown recovery, disk notifications, and main document versioned edits:

```sh
cargo test --locked --test string_test_projects
cargo test --locked --test protocol imported_string_predicates
cargo test --locked --test patched_string_tests -- --include-ignored
cargo test --locked --test protocol macro_patch_guards_
```

The three-engine regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox. Check the original string text, actual boolean status, renaming result and PDF page number before and after import redefinition:

```sh
cargo test --locked --test string_test_projects -- --include-ignored
```
