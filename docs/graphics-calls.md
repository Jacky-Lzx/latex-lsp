# Static aliases for graphics commands

[Back to documentation](README.md) · [Image search and movement](graphics.md)

## Image call

Image jumps, path completion, dependency tracking, and file renaming support pure aliases, alias chains, and static `\csuse` / `\csname` calls with no parameters and whose body contains only a single control sequence:

```tex
\usepackage{graphicx,etoolbox}
\def\Image{\includegraphics}
\newcommand{\Figure}{\Image}
\graphicspath{{figures/}}
\DeclareGraphicsExtensions{.png,.pdf}
\Image*[width=1cm]{plot}
\Figure{plot}
\csuse{Image}{plot}
\csname Image\endcsname{plot}
```

Aliases defined via static input files and local packages are also available. The image directory and extension are parsed according to the settings of the actual calling location; the path can also be completed even if the path has not been fully entered. Keep the calling form, stars, and options when moving an image or main document, and check the actual image destination after the move. When shared source code is parsed inconsistently under different root documents, no single target is guessed.

Project source code discovered by recorder after compilation also participates in image call analysis, including direct image calls, static aliases, and macro parameter boundaries in these files. Related images can be jumped, completed, tracked for dependencies, and referenced as files are moved; files that are only read by the code display command will not have macro definitions installed. Shared source code is still parsed separately for each compilation root document. Redefinition or parameter consumption in the recorder file prevents unreliable image reference editing; search settings continue to skip target inference when the execution order of the search settings cannot be confirmed.

When the compiler confirms that the image is missing and the source code echo can uniquely correspond to the call, the diagnostics will mark the file name and provide spelling suggestions based on the current valid directory and extension; the repair will only replace the file name. When the source code, alias meaning, or candidate files change, old fixes are re-verified. For detailed rules, see [Build diagnostics](diagnostics.md).

## Search settings

Directory and extension declarations also support the pure alias and static named calls described above:

```tex
\def\Paths{\graphicspath}
\Paths{{figures/}{plots/}}
\def\Extensions{\DeclareGraphicsExtensions}
\Extensions{.png,.pdf}
\csname DeclareGraphicsExtensions\endcsname{.pdf,.png}
```

The server tracks settings by source order, grouping, and global assignment rules using the parameters following the call site. For examples of search priority and scope, see [Image Search and Movement](graphics.md).

## Support boundaries

`\csuse` requires document loading of a package that provides this command, such as etoolbox. Static analysis does not perform arbitrary macro expansion: forwarded macros with parameters, the meaning of `\let` capture, complex wrappers, dynamic command names, and redefinitions of uncertain meaning are not inferred. As a call consumed as other macro parameters, used for verbatim display, or only appearing in the body of an unexecuted macro, it is not relied upon as an actual image.

Unconfirmed calls, incomplete or malformed parameters, shared execution context conflicts, and recorder-unique settings with unknown execution order do not speculate on a single image target. The recorder is still used to keep track of which files the compiler actually read. Build validity checks focus on the selected image and the candidates that precede it; adding low-priority images does not expire the results.
