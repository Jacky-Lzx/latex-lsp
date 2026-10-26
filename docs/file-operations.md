# File operations

[Back to documentation](README.md) · [Project and path analysis](projects.md)

Supports the creation, deletion, renaming and movement of files and directories. The file path is modified by the editor, and the server is responsible for maintaining source code references and indexes.

## Rename and move

`workspace/willRenameFiles` Updates input files, libraries, BibTeX styles, local packages and document classes, images, file URLs, and `% !TeX root` paths. Directory movement will recalculate the internal relative path, and try to keep the writing method of omitting the extension. For the image search directory and masking rules for files with the same name, see [Image Search and Movement](projects.md#image-search-and-movement).

The chapter list of `includeonly` will also be updated simultaneously, retaining comments, list separators, CRLF and internal spaces in the file name; the selection list itself will not load chapters. PDF expiration checking for missing chapters uses the same parsing as spell repair, and is conservative when encountering dynamic or discontinuous names.

Keep the new suffix when the input source code is changed to another extension. For example, `\input{chapter}` is updated to `\input{chapter.ltx}` after `.tex` is changed to `.ltx`, and the root command is updated simultaneously. The following loading methods require the target to retain the corresponding suffix:

| Reference method | Suffixes that must be retained |
| --- | --- |
| `include` / `includeonly` | `.tex` |
| Package loading | `.sty` |
| Document class loading | `.cls` |
| `bibliographystyle` | `.bst` |
| `bibliography` | `.bib` |

The leading space of the file name is reserved with `./`, and the space at the end of the trunk is reserved with an explicit suffix; whitespace that will be merged or ignored by TeX does not generate reference editing. For specific rules, see [Source File Name and Movement](source-filenames.md).

References in shared source code require that the edits required by each project are consistent; new paths that contain macro expansion, undetermined targets, or cannot be expressed safely will be rejected. The explicitly configured `build.rootFile` needs to be modified synchronously after the main file is renamed.

## File URL

The `file:` link in `\url`, `\href` and four-parameter `\hyperref` will update the file path while retaining the query parameters and fragments in the source code, including percent encoding, TeX escaping and empty `?` / `#` separator. For example, when changing `manual.pdf` to `guide.pdf`:

```tex
\href{file:///project/manual.pdf?download=1#page=3}{Manual}
```

Updated to:

```tex
\href{file:///project/guide.pdf?download=1#page=3}{Manual}
```

The four-parameter `\hyperref{file:///project/manual.pdf?download=1}{chapter}{intro}{Chapter}` only modifies the path in the first parameter, retaining the category, name and text; `#chapter.intro` used for navigation will not write the first parameter additionally. Comments, verbatim content, plain text, and HTTP links are not involved in file renaming.

## Client calling sequence

The editor or file management plug-in needs to be executed in the following order:

1. Send `workspace/willRenameFiles` to apply the returned edits. Edit the URI that points to the document before the move and preserves the version constraints of the open document.
2. Move files or directories on the disk.
3. Send `workspace/didRenameFiles` to cause the server to update the document URI, refresh the disk index, and stop compilation using the old input path.

## Create and delete

Standard `workspace/didCreateFiles` / `workspace/didDeleteFiles` notifications recursively update the source index, refresh navigation, completion, and duplicate definition diagnostics, retain unsaved buffers, cancel affected compilations, and clear old compilation diagnostics. Ability to synchronize file operations in the editor without file listening; the server continues to retain recorder input to check if old PDFs are out of date.
