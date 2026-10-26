# Typesetting warning analysis

[Back to documentation](README.md) · [Compile](build.md) · [Compile diagnostics](diagnostics.md)

`analysis.boxWarnings` Aggregates Overfull / Underfull diagnostics in the final TeX log by warning type and direction, helping clients show the number and most severe typeset problems.

For independent statistics on charts exceeding the height of the page, see [Floating Body Height Warning](floats.md).

## Field

Each item contains `kind` (`overfull` / `underfull`), `axis` (`horizontal` / `vertical`) and `count`. Overfull additionally provides the string `maxExcessPt`, which represents the maximum overflow in TeX pt; Underfull provides the integer `maxBadness`, which represents the maximum badness.

For example, the relevant fields in `analysis`:

```json
{
  "boxWarnings": [
    {"kind": "overfull", "axis": "horizontal", "count": 2, "maxExcessPt": "19.0"},
    {"kind": "underfull", "axis": "vertical", "count": 1, "maxBadness": 10000}
  ],
  "truncated": false
}
```

The four categories are counted separately, sorted by `kind`, `axis`, and only the actual categories are returned; when there is no typesetting warning, it is `[]`. The maximum overflow amount uses exact decimal comparison, retains the compiler's numerical text, and avoids floating point rounding; for equal values, choose a stable original writing method.

## Count and source

`count` counts the warning diagnoses retained after final log parsing. Warnings in different files or locations are counted separately; duplicate diagnoses at the same location and with the same content in the log have been merged, and the number will not be increased repeatedly. Supports paragraphs, alignment, single-line detected and output routine warnings, and engine byte-wrapped or formatting with explicit filenames and line numbers.

Only use full standard warning headers, do not infer metrics from sample text of package warnings, box content dumps, or incomplete messages. Even if the source code location cannot be confirmed, the typesetting indicators clearly reported in the log can still be counted; the reliability of the location and source continues to be expressed by item-by-item diagnostics. The generated file line number description appended by the server does not affect indicator recognition.

Up to four categories are returned. When a single overflow exceeds 128 ASCII characters, the entire record is not included in the count and maximum value, and `analysis.truncated: true` is set. This flag is also set when the diagnostic input itself is omitted, in which case the count and maximum represent only the observed portion.

## Compilation status

The summary itself does not change the build success status, warning count, or `rerunRequired`. `latex/buildFinished` and `latex.buildStatus` return the same summary; warnings from earlier compilation passes are not retained, and the corresponding categories disappear after the typesetting is fixed. For results such as cancellation, timeout or expiration, continue to press [Result Analysis](build-analysis.md) to return to `analysis: null`.
