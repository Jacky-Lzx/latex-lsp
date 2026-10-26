# Build result analysis

[Back to documentation](README.md) · [Build guide](build.md)

`analysis` of the results summarizes confirmed duplicate compilation requests (`rerunRequired`), unresolved cross-reference keys (`unresolvedReferences`), literature citation keys (`unresolvedCitations`), missing PDF targets (`unresolvedDestinations`), and duplicate PDFs in this final TeX log target (`duplicateDestinations`) and duplicate tag key (`duplicateLabels`).

Unresolved references are only identified from the beginning of the standard LaTeX, package or class alert text; similar text in normal alert examples or file names remains as alerts and is not counted in the reference key. Keys are deduplicated and sorted; PDF strings output by the PDF target name decoding engine are escaped, and citation, document, and duplicate tag keys remain original. Log wrapping does not add spaces before parentheses that reference keys or misread the parentheses as file boundaries.

Missing definitions are not themselves considered duplicate compilation requests. When the table column width needs to be updated, identify the explicit recompilation prompt of `longtable`; the final log after column width convergence no longer retains this request. Also recognizes hyperref's explicit recompilation hint about PDF page tags (`/PageLabels`); for example, XeLaTeX needs to complete page tags after the first build, and clears this mark when subsequent logs are no longer requested.

After using the last page hook, changes in the total number of pages may cause temporary extra pages or hook execution on old pages. Explicit requests for both types of kernels also set recompile flags and provide actions; see [Last Page Changes and Recompilation](last-page.md) for specific warnings, actual PDF page count, and convergence behavior.

There is a maximum of 128 keys per class, with a maximum of 128 Unicode characters per key; keys that exceed the budget do not truncate the identity, but omit it and set `truncated`. This flag is also `true` when diagnostic input is omitted.

Results that are canceled, timed out, unable to observe the final log, or discarded due to input changes return `analysis: null`; warnings that have been resolved in earlier compilation rounds will not enter the analysis.

## Recompilation warning for line breaks

LaTeX cross-references are compatible with known requests for last pages, natbib documents, longtable column widths, and hyperref page tags. When restoring, the connectors and original spaces in the words are retained. For example, `cross` in the 60-column log is still `cross-references` when the new line is `-references`; the existing spaces at the end of the line will not be trimmed and the two words will be spliced.

After the first sentence ends with a period, the request for an explicit line break still belongs to the same warning. Recognize line continuation prefixes of the same package and indented continuation lines of LaTeX; independent natbib short hints continue to be valid, but if there are continuation lines, the full text is used to judge. Examples, additional clarifications, and incomplete requests will not trigger recompilation by borrowing from previous valid phrases. File switches, adjacent errors, or omitted log fragments are not used to complete outstanding requests.

Result summarization uses the same rules as recompile operations. A final check by natbib may be issued when reading back the generated `.aux`; operations initiated at this diagnostic still compile the original root document. After a tag or citation key is renamed across files, subsequent builds will reanalyze references; once the log stabilizes, the tags and operations will be cleared.

## File changes and recompilation

rerunfilecheck supports standard `Rerun.` requests after file changes, as well as hyperref's complete bookmarks and reverse literature reference recompilation prompts, and is compatible with log line breaks and package continuation line prefixes; result analysis uses the same rules as [recompile operation](build.md#recompile-from-diagnostics). Similar text with additional explanations and index and glossary prompts that require running makeindex do not provide this operation. When the files no longer change during subsequent compilations, the recompilation flags and operations are cleared along with the corresponding warnings.

After enabling back reference functions such as hyperref's `backref=page`, `.brf` file changes will generate `Rerun to get bibliographical references right.` requests. This request sets `rerunRequired: true` and can be handled by the direct TeX engine or by an existing recompilation configuration. This type of request only requires running LaTeX again; `requiredBibliographyTools` becomes `[]` when there are no other documents processing the request. The request will reappear when the reference page number changes again, and subsequent stable logs will clear the tags and operations; console prompts from earlier rounds of the compilation pipeline will not overwrite the final log.

Explicit line breaks may leave extra empty lines when the warning header happens to fill a TeX output line. The server retains the complete description of the same package thereafter, including the request body that continues to wrap; index and glossary descriptions that require running makeindex are not lost or misinterpreted as ordinary LaTeX recompilation requests. True file switching and adjacent diagnostics remain from their respective sources.

When the left and right brackets appear in the file name at the log line break, try to restore the complete known warning first, and then consume the corresponding line after confirming it to avoid treating the file name as an entry or exit event of the source code. 60 / 79 Filenames in column output can contain seemingly complete `has changed. Rerun.` text; recognition uses actual complete filenames without premature end warning. Incomplete warnings still retain the true file switching, and subsequent diagnostics of the main file and sub-files are attributed to their respective sources.

## Document processing and recompilation

`analysis.requiredBibliographyTools` Summary biblatex Explicitly request a rerun of the literature tool, using the lowercase names `biber` / `bibtex`, deduplicated and sorted. A complete document processing request will also set `rerunRequired: true`, since LaTeX needs to be run after document processing; the presence of an undefined reference alone does not infer the required tool.

For example, a first compile of the BibTeX backend might return:

```json
{
  "rerunRequired": true,
  "requiredBibliographyTools": ["bibtex"]
}
```

After running BibTeX, if biblatex only requires running LaTeX again, the tool list becomes `[]`, and `rerunRequired` remains `true`. When the final log is no longer requested, the two are `[]` and `false` respectively. Pagination changes, explicit LaTeX rerun prompts for language loading are also analyzed; normal warning examples, incomplete requests, and similar text in other packages do not set these fields.

The summary is based on this final log, and requests that have been fulfilled in the previous round will not be retained. `latex/buildFinished` and `latex.buildStatus` return the same fields; cancellation, timeout and expiration results still return `analysis: null` according to the original rules. For the tool scope of recompile operation, see [Recompile from Diagnostics](build.md#recompile-from-diagnostics).

## Indexing and Glossary Tools

`analysis.requiredIndexTools` Summary rerunfilecheck Explicitly requested `makeindex`, deduplicated and sorted. A complete index or glossary request also sets `rerunRequired: true`; this list is independent of `requiredBibliographyTools`. For the applicable boundaries of processing commands, log snapshot rules and recompilation operations, see [Index and Glossary Processing](index-processing.md).

## Summary of formatting warnings

`analysis.boxWarnings` summarizes the number of Overfull / Underfull diagnostics in horizontal and vertical directions, as well as the maximum overflow or badness. For fields, counting and budget rules, see [Typesetting Warning Analysis](typesetting.md).

`analysis.oversizedFloats` separately summarizes the number and maximum excess of floating body over-height warnings, see [Floating Body Height Warning](floats.md).

## Font replacement summary

`analysis.fontSubstitutions` and `analysis.fontSizeSubstitutions` respectively summarize the font and font size replacement in the final log. For fields, deduplication and budget rules, see [Font Diagnostics and Result Analysis](fonts.md#font-and-font-size-replacement).

## Missing word summary

`analysis.missingCharacters` Summarizes missing characters by character and font, and retains the highest severity level. For field examples, budget and positioning rules, see [Abstract of missing words](fonts.md#missing-word-summary).

## Unicode input error summary

`analysis.unsupportedUnicodeCharacters` Summary of Unicode characters that LaTeX does not set a map for in the final log, providing characters and code points. See [Unicode input error](unicode-errors.md) for fields, budgets and processing methods.

## PDF pages

The `pages` field in completion notifications and `latex.buildStatus` comes from the final TeX output summary and is provided only when the current build returns a valid PDF. Parsing supports PDF, DVI, and XDV summaries, multiple passes, quoted filenames, and wrapped logs. DVI/XDV summaries support build workflows that produce an intermediate file before converting it to PDF.

Literal characters such as `(9 pages).` and `main.pdf (9 pages).` in the output directory or file name will not be used as page numbers. After reading the complete summary, the server confirms the output file name and page number, and does not treat the line break at the directory name as the end of the summary; when the last summary is incomplete or spans an omitted log, `null` is returned, and the page number from the previous round is not used.

When the last clear output report is "No page output", the number of pages in earlier rounds will not be retained, and the old PDF in the directory will not be regarded as this successful product; for details, see [Input and Product Verification](build-inputs.md#input-and-product-verification). Final logs take precedence over reports from earlier passes in the compilation pipeline, and normal warning examples are not reported as such.
