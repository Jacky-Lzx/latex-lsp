# Source filenames and moves

[Back to documentation](README.md) · [File operations](file-operations.md) · [Projects and files](projects.md)

After the file is moved, the reference must continue to select the same target. `workspace/willRenameFiles` Also checks the search order of the new location and the actual meaning of the path in LaTeX, preserving the version constraints of the opened source; the client still applies edits and moves disk files in [File Operation Order](file-operations.md#client-calling-sequence).

## Spaces and extensions

A single internal space can be used for static file references. For example, if `chapter.tex` is changed to `chapter name.tex`, the server will update `\input{chapter}` to `\input{chapter name}`. The original ungrouped `\input chapter` will be filled with curly braces as needed; existing quotation marks will be retained.

When the file name starts with a space, it will be ignored by LaTeX if written directly at the beginning of the parameter. The server adds `./`, leaving spaces inside the path. For example target ` leading.tex` uses:

```tex
\input{./ leading}
```

When the backbone of the file name ends in a space, keep the explicit suffix to prevent the editor from trimming the valid spaces. For example target `chapter .tex` uses:

```tex
\input{chapter .tex}
```

These writing methods are applicable to `input`, `include` and static `InputIfFileExists` / `IfFileExists` file parameters; relative paths will continue to be recalculated when moving the root document or directory. Navigation, dependency indexing and renaming after the move is completed follow the same target.

## Paths that cannot be expressed directly

Consecutive spaces, tabs, and newlines in source code quotations may be merged or ignored by TeX; control characters cannot be written directly as ordinary source code file names. If the new path will change the target, the server rejects the reference update and does not return the partial edit. Placing normal quotes around the entire argument does not preserve whitespace that was already merged during TeX tokenization. This rule does not expand dynamic file names or insert macros that concatenate file names.

`% !TeX root` comments and `file:` links are processed according to their respective semantics, and the space merging rules of source code parameters are not applied; internal continuous spaces can be retained, and the file URL uses percent encoding. For the respective path rules for pictures and biblatex resources, see [Picture File Name](graphics-filenames.md) and [biblatex Documentation Resources](biblatex-resources.md). Packages, document classes and other loading commands still comply with [suffix requirements](file-operations.md#rename-and-move).
