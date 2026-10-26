# Float height warnings

[Back to documentation](README.md) · [Compile](build.md) · [Typesetting warning analysis](typesetting.md)

LaTeX reports `Float too large for page by ...pt` when the height of the figure or table exceeds the allowed height of the float on the current page. The server retains the original warning and locates the line reported in the log (usually the end-of-float command) when the source can be confirmed, along with diagnostic codes `oversized-float` and `data.excessPt`.

## Compilation results

`analysis.oversizedFloats` summarizes the number of warnings and maximum exceedances in the final log, for example:

```json
{
  "oversizedFloats": {
    "count": 2,
    "maxExcessPt": "19.25"
  }
}
```

- `count`: The number of analyzed float warnings is too high. Duplicate diagnoses from the same source, location and message are counted only once.
- `maxExcessPt`: Maximum excess in TeX pt. Compiler-preserved decimal strings are compared as exact values; `null` when there is no such warning, `0` for `count`.

Floating body warnings are counted separately and are not merged into `analysis.boxWarnings`, nor do they require compilation again. The warning itself does not change builds that successfully generate PDF to failure. `latex/buildFinished` returns the same digest as `latex.buildStatus`; only the final TeX log is used, warnings from earlier rounds are not retained. After modifying the layout and recompiling, the resolved diagnostics were cleared and the summary was reset to zero.

## Adjust content

Check the actual height of the figure or table, the title, and the page settings of the class file, and shrink the image, split the table, or adjust the layout based on the content. For example, the following rule will produce an overly tall plot when tested:

```tex
\begin{figure}
\rule{1cm}{1000pt}
\end{figure}
```

Reduce the height to the range that the page can accommodate and then recompile. The server provides measurements and sources, and the exact size is determined by the content.

## Identification and Budgeting

Only complete standard LaTeX warnings, including the `pt on input line N.` suffix, are counted; examples, incomplete messages, negative numbers, exponents, and non-finite values in packages are not included in the summary. Supports 60 / 79 column log line wrapping by bytes, including decimal points and line breaks in the middle of `input` words, and is compatible with traditional and file line number formats.

When the source or line number cannot be confirmed, the original warning uses a valid fallback position, and the excess reported explicitly in the message can still be counted. When the makefile has been rewritten, additional line number descriptions do not affect the summary.

A maximum of 128 ASCII characters are reserved for a single overage. The entire record that exceeds the budget does not participate in the count and maximum value, and `analysis.truncated: true` is set. This flag is also set when diagnostic input is omitted and the summary only represents the observed portion. Expired results caused by cancellation, timeout, inability to observe the final log, or input changes continue to return `analysis: null`.
