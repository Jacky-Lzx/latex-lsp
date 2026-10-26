# Conditional file hooks

[Back to documentation](README.md) · [Cross-file context](string-test-projects.md) · [Projects and files](projects.md)

## Hook execution order

Meaning propagation of string tests supports direct `InputIfFileExists` and `IfFileExists` calls without redefinition. The file name and the two hook parameters must use complete curly braces, and the file name must be statically parsable.

`InputIfFileExists` When a file is found, the command with the second parameter is parsed first, and then the subfile is read; the third parameter does not propagate the statement backwards. Only the third parameter is propagated if the file confirms that it does not exist. `IfFileExists` uses the same branch selection, but does not read the source code in the checked file.

For example:

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\newcommand{\identity}[1]{#1}
\begin{document}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\InputIfFileExists{./checked}{\let\ifblank\identity}{}
\end{document}
```

`checked.tex`：

```tex
\ifblank{\booltrue{draft}}
```

Before the subfile is executed, `ifblank` has been changed into a single-parameter macro, so the parameters of `booltrue` can be navigated and renamed. `boolfalse` in the earlier call of the main file is still string data, and the name is changed to retain the original text. Nested file inspection and static input within hooks are also supported.

## Local conditions and updates

The search follows the project's existing main documentation, current file directory, and suffix rules. A found local file determines the success branch; an explicit path with a directory (such as `./missing`) determines the failure branch when none of the local candidates exist. Naked names that cannot be found in the project may still come from the TeX distribution, and failing branches cannot be selected based solely on local absences.

The source code of unchecked hooks continues to participate in the static symbol index, but the declarations and inputs therein are not propagated as side effects of the execution of this file condition. Dynamic file names, redefined check commands, unfinished parameters, and unconfirmed conditions remain uncertain, and related string parameters do not provide speculative automatic renaming. No arbitrary macro expansion is performed, and no other TeX conditions are simulated.

Existence checks can also reference non-source files such as CSV and images. Creation, deletion or file listening notifications will re-analyze the relevant context, the text and version number of the main document remain unchanged, and non-source files will not enter the LaTeX symbol index. The file conditions of shared source code are parsed separately for each root document; when the same parameter has different meanings, the parameter is refused to be modified, and other symbols can still be renamed.

## verify

```sh
cargo test --locked --test conditional_string_tests
cargo test --locked --test protocol conditional_string_hooks
cargo test --locked --test conditional_string_tests -- --include-ignored
```

True regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox. Check the success and failure branches of the two commands in the three engines respectively, and compare the hooks, subfile execution order, boolean status and PDF page count before and after renaming.
