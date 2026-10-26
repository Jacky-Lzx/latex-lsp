# Last-page changes and reruns

[Back to documentation](README.md) · [Compilation result analysis](build-analysis.md)

When using the `shipout/lastpage` hook, LaTeX determines where to execute the last page content based on the actual total number of pages in the previous pass. After editing changes the total number of pages, even if the PDF is successfully generated this time, the content of the last page may still need to be compiled again to be correctly placed.

## Two types of explicit requests

After the document is shortened, the last page expected by the hook may no longer exist. LaTeX temporarily appends a page to hold the contents of the last page and reports:

```text
LaTeX Warning: Temporary extra page added at the end. Rerun to get it removed.
```

For example, when reducing from three pages to one, a two-page PDF may be generated in the first pass; one page will be restored after compilation again. `pages` in the results always reports the actual number of pages generated this time, including temporary pages.

When the document becomes longer, the hook may execute prematurely on the original last page and report:

```text
LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3).
               Rerun to correct this.
```

The actual total number of pages in the previous pass and this pass is in parentheses. For example, from one page to three pages, the first PDF has three pages, but the hook is still executed on the first page; after compiling again, it moves to the third page.

Both types of complete warnings set `analysis.rerunRequired: true` and provide `Rebuild …` action at diagnostics. They do not require bibliography or indexing tools; `requiredBibliographyTools` and `requiredIndexTools` are both `[]` when no other tools are requested. The operation uses the existing [Recompile Configuration](build.md#recompile-from-diagnostics).

## Log and Clear

Recognizes indented continuation lines compatible with byte-by-byte wrapping, LF/CRLF, and LaTeX. The last page hook warning must contain two distinct positive integer page numbers and the full `Rerun to correct this.` request. Similar text, examples, incomplete requests, additional instructions, and error-level messages in ordinary packages will not trigger this flag or action.

The analysis is based on this final log. When the warning disappears on subsequent compilations, the flags and operations are cleared; hints from earlier passes in the compilation pipeline do not overwrite the final result. The completion notification returns the same summary as `latex.buildStatus`, and the menu commands corresponding to the old diagnostics are invalid.

## Reproduction and testing

Using the root document below, reduce the text of `child.tex` from three pages to one, or increase it from one page to three pages. Compile twice continuously after each modification to observe the request and convergence:

```latex
\documentclass{article}
\AddToHook{shipout/lastpage}{\put(20,-30){Last page marker}}
\begin{document}
\input{child}
\end{document}
```

The default regression checks line breaks, line continuations, page number formats, adjacent source boundaries, and two client operations. Real regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and a LaTeX kernel that supports the above hooks. Check the actual PDF page count, hook execution page, occurrence and removal of requests and operations in the 60 / 70 / 79 column log, including the case of the right bracket wrapping alone:

```sh
cargo test --locked --test rerun_warnings
cargo test --locked --test rerun_warnings real_last_page -- --include-ignored
cargo test --locked --test protocol wrapped_standard_rerun
```
