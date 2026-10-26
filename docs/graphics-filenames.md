# Graphics filenames

[Back to documentation](README.md) · [Image analysis and movement](graphics.md)

## Quotes and whitespace

Image jump, completion, dependency tracking, file movement and missing image repair support graphicx's complete and segmented double-quoted file names. The following writing methods all point to `plot image.png`:

```tex
\includegraphics{"plot image.png"}
\includegraphics{"plot image".png}
\includegraphics{"plot" image.png}
\includegraphics{ "plot  image.png" }
```

Double quotes are used to wrap file names and do not belong to disk file names. Ordinary spaces, tabs, and single newlines are combined according to TeX's rules, and leading and trailing whitespace are ignored; these rules are also followed within quotation marks. Therefore, `"plot  image.png"` looks for `plot image.png` with only one space.

`\includegraphics{"plot image"}` that omits the extension is still searched in the order of the image directory and extension at the calling location. For details, see [Image Analysis and Movement](graphics.md). [Static aliases and named calls](graphics-calls.md) use the same rules.

## Search for quotes in directories

When the project uses pdfLaTeX with default parameters, or uses latexmk of the pdfLaTeX engine, the complete and segmented quotation mark directory of `\graphicspath` is also supported:

```tex
\graphicspath{{"figure files/"}{"other figures"/}}
\includegraphics{"plot image"}
```

The server merges ordinary blanks in the directory and performs jumps, completions, image dependency tracking and missing image repairs according to the currently effective settings. Static aliases, local and global scopes for directory declarations continue to apply. After a directory or main document is moved, quotes, absolute paths, and the applicable `./` prefix are retained; segmented quotes can be organized into a pair of complete quotes. Editing of a moved path that would change the target due to TeX merging whitespace is rejected.

The directory must end with `/`, and the closing quotation mark can enclose the slash; no whitespace can be added after the slash, even if the whitespace is within the quotation marks. For example, `{"figure files/" }` and `{"figure files/ "}` do not perform static directory inference.

XeLaTeX, LuaLaTeX's existence checks for directory quotes differ from the read rules; under these engines or in custom compilation configurations where the engine cannot be confirmed, the server does not infer image targets or mobile edits for quoted directories. If the directory name contains spaces, you can directly write `\graphicspath{{figure files/}}` without adding quotation marks. The server determines based on the actual compilation configuration of the root document, including the applicable `% !TeX program` directive; quotation marks in the image file name parameter continue to be processed according to the above rules.

## Completion, movement and missing image repair

Completion supports unclosed quotes, and filenames that span a single newline. Remaining names are replaced when the cursor is in the middle of a name; directory completion preserves existing path suffixes.

Image movement and missing image repair preserve file name quotes, command calling forms, asterisks, and options. When you need to modify the writing of segmented quotation marks, you can organize them into a pair of complete quotation marks. For example, after moving `plot image.png`, update `"plot image".png` to `"assets/chart image.png"`.

The file name reported by the compiler may not be quoted, and the diagnostic still indicates the full quoted name in the source code; spelling suggestions follow the search directory and extension settings of the image call. Repairs are only provided when the compiler confirms that the missing image can reliably correspond to the source code. For details, see [Build diagnostics](diagnostics.md).

## Support boundaries

Quotes in the full path must be in pairs. Static target inference is not performed for paths containing any macro expansion, empty paragraphs, or internal comments in the file name. Uncompleted quotes can be used for completion during typing, but will not participate in navigation and file movement as a dependency on the parsed image. The compilation input check observes the actual selected images and higher priority candidates. For rules, see [Compilation Input and Expired Results](build-inputs.md).
