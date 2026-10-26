# Chapter selection and string semantics

[Back to documentation](README.md) · [Cross-file context](string-test-projects.md) · [Conditional file hook](conditional-string-tests.md)

## Loading order

Cross-file semantic propagation of string tests respects unredefined direct `\includeonly` and `\include` calls. Each root document maintains chapter selections separately: all chapters are included when no selection is declared, and all chapters are excluded from the empty list; subsequent selections replace previous selections, and selections in loaded settings files also take effect in the loading order. Normal `\input` is not filtered by chapter selection.

For example main file:

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\includeonly{}
\begin{document}
\include{definitions}
\ifblank{\booltrue{draft}}{}{}
\boolfalse{draft}
\end{document}
```

`definitions.tex`：

```tex
\renewcommand{\ifblank}[1]{#1}
```

When a section is excluded, redefinition therein does not change the meaning of subsequent `ifblank`. Renaming the boolean `draft` updates the declaration and `boolfalse`, preserving `booltrue{draft}` in the string data. After changing the selection to `\includeonly{definitions}`, the redefinition in the chapter takes effect, and `booltrue` in subsequent calls will also participate in the renaming.

The selection can also come from [Conditional File Hook](conditional-string-tests.md): only the selected hook is propagated; `InputIfFileExists` executes the successful hook first, and then reads the file. Subsequent selections in the read file can overwrite the selections in the hook. Raw string data, macro bodies that have not yet been called, and operands of copy commands do not perform chapter selection at declaration.

## Name matching

- Both the list and `include` parameters can omit the `.tex` at the end; `chapter.tex.tex` and `chapter.tex` are still different chapter names.
- Preserving path spelling, `./chapter` does not match `chapter` even though they point to the same disk file.
- Ignore whitespace around list entries and valid TeX comments, retaining whitespace inside static file names.
- The file name must be statically parsable, and the list and file parameters must be complete.

## Shared files, updates and boundaries

Excluded sections and their dependencies continue to be indexed by the editor, but do not propagate context as additional entries to the actual executed shared file. The same shared file in other root documents or other actual loading locations will still be analyzed according to their respective selections. When a string parameter has two meanings: data and executable source code, rejecting the rename will modify the parameter, and other symbols with clear semantics in the same file can still be edited.

Buffer-dependent unsaved content, incremental edits, shutdown recovery, and disk change notifications all recalculate string semantics, and the text version of the main document remains unchanged.

When dynamic selection, a redefined selection command or an include command cannot determine the actual chapter, ambiguity is retained for subsequent string parameters after encountering the relevant `include`; when only an unknown selection is declared and the include call has not been encountered, the meaning of the existing string will not be changed. No arbitrary macro expansion is performed here, nor is call aliasing or grouping recovery of chapter selections inferred.

For the supported scope of compilation input and expired result checking, see [Compilation Input Rules](build-inputs.md); multiple selections that cannot be unanimously confirmed are still handled according to the conservative rules of this document.

## verify

```sh
cargo test --locked --test selected_string_tests
cargo test --locked --test protocol selected_string_predicates
cargo test --locked --test selected_string_tests -- --include-ignored
```

Real regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox, overrides default selection all, empty lists, explicit suffixes, different path spellings and selection overrides in settings files, compares chapter execution, boolean status and PDF page count before and after renaming.
