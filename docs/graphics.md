# Graphics resolution and moves

[Back to documentation](README.md) · [Project and path analysis](projects.md)

Please refer to [Picture File Name](graphics-filenames.md) for quotation marks, blank spaces and related editing rules in file names.

## Search directories and move files

`\includegraphics` will also search the directory declared by `\graphicspath{{figures/}{plots/}}` in the current project, supporting image jump, subdirectory completion and file renaming. When you move an image directory or main document, the directory declaration is updated simultaneously, trying to preserve the short names and omitted extensions of images. The path is based on the current compilation main documentation, and declarations in shared packages also follow this rule. Allow spaces and Chinese, recognize static directories ending with `/`; directories that do not yet exist will also be recalculated when moving the main document. If the shared declaration requires different replacement paths in different projects, the file move edit will be rejected. Directory values do not expand arbitrary macros; coverage and scoping rules for static declarations are as follows.

When the order of extensions is not declared, if extensions are omitted from the image, the compilation root directory and declaration directory will be searched one by one in the default order; supported explicit extensions are first searched by their original names. For example, when the root directory has `plot.png` and `figures/` has `plot.pdf`, `\includegraphics{plot}` selects PDF, and `plot.png` explicitly selects PNG. The directory where the subfile is located will not overwrite these pictures in advance; only the directory of the subfile itself is used as the root. Jumps, completions, input tracking, and file renaming share this search base.

Image renaming will check the search results after the entire batch of files has been moved, and keep the abbreviation that still points to the original image; the abbreviation will use the path and actual extension relative to the new compilation root directory when other images are selected. Occlusion of the same name caused by moving other files in the same batch will also trigger reference updates; editing is refused when the target cannot be safely maintained to avoid using another image despite successful compilation.

## Directory coverage and scope

The server tracks the valid directories of each `\includegraphics` in source code order. The last `\graphicspath` replaces the previous list, and the empty declaration `\graphicspath{}` clears the extra directory; curly braces, grouping primitives, and local settings in the environment are restored after the grouping ends:

```tex
\graphicspath{{figures/}}
{\graphicspath{{drafts/}}\includegraphics{plot}} % uses drafts/
\includegraphics{plot} % restores figures/
\graphicspath{{final/}}
\includegraphics{plot} % uses final/，no longer searches figures/
```

Static input files can be executed repeatedly, and local packages and document classes are only loaded once; when shared source code requires different directories at different execution locations, a single image target is not provided. Directory settings and extension settings are saved and restored separately and follow the following global assignment rules. Jump, completion, spelling suggestions, renaming and compilation candidate checks use the settings corresponding to the calling location. Overwritten directories will not participate in candidate checks for subsequent images.

## Extension declaration

`\DeclareGraphicsExtensions{.png,.pdf}` Searches for images with extensions omitted in declaration order. Each extension searches the compilation root directory first, then the directory for `\graphicspath`, before trying the next extension. For example, when `plot.pdf` and `plot.png` exist at the same time, this statement causes `\includegraphics{plot}` to select PNG.

Explicit extensions use independent image format rules: even if only `.png` is declared, `\includegraphics{plot.pdf}` will still read the existing `plot.pdf` first, and will not change the image due to the simultaneous existence of `plot.pdf.png`. Empty declaration `\DeclareGraphicsExtensions{}` turns off extension inference, explicit filenames continue to be available.

Declarations support comments, whitespace, repeated settings, as well as normal curly braces, `begingroup` / `endgroup` and local settings in the environment. The server tracks the settings of static input files and local packages and document classes in source code order; packages and classes are only loaded once, and ordinary input can be executed repeatedly. After the partial grouping ends, the previous order is restored. A single image target is not provided when the same source code location requires different extension orders in different execution contexts.

## Static aliases and named calls

Image calls, directory declarations, and extension declarations support parameterless pure aliases, alias chains, and static `\csuse` / `\csname` calls. The search settings at the calling location are used for completion, jump, file movement and missing image repair; for examples and support boundaries, see [Static aliases for image commands](graphics-calls.md).

## Global settings and recovery

The settings of `\global\graphicspath{{figures/}}` and `\global\DeclareGraphicsExtensions{.png}` are retained outside the group. When `\globaldefs` is a positive number, directory and extension declarations automatically take effect globally; when it is a negative number, explicit `\global` is also processed as local assignment; when it is zero, the prefix is followed. Global assignment will update the value that the outer group should restore, and the most recent global settings will be restored after subsequent local modifications.

```tex
\DeclareGraphicsExtensions{.pdf}
{\global\DeclareGraphicsExtensions{.png}}
\includegraphics{plot} % prefers PNG
{\globaldefs=-1 \global\DeclareGraphicsExtensions{.pdf}}
\includegraphics{plot} % still prefers PNG
```

`\globaldefs`'s own assignment is saved and restored according to the scope rules before assignment. Prefixes can be passed across static input files; prefixes and `\globaldefs` can be called via proven parameterless pure aliases and static `\csuse` / `\csname`. Literal integers in decimal, octal, hexadecimal, ASCII character codes, and multi-signed forms are supported. Assignments in conditions, dynamic integers, macros that may expand to numbers, and redefined prefixes or calling primitives stop inference.

## Compilation checks and support boundaries

Jumps, image dependencies, target verification when moving files, and missing image spelling repairs share the above order. The compile input check only looks at the selected image and its priority candidates: when a PNG is selected, creating a new low-priority PDF will not expire the results; deleting the PNG or adding an earlier candidate will expire the results.

No arbitrary macro expansion is performed. Dynamic or redefined settings, custom `\DeclareGraphicsRule`, undetermined conditional branches and chapter selection configurations will skip related static inference; inference will also stop when looping input or exceeding the analysis upper limit. The compiler's recorder continues to be used to track which files were actually read. For the client calling sequence of file movement, see [File Operation](file-operations.md).
