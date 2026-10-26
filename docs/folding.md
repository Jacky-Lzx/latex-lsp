# Document folding

[Back to documentation](README.md) · [Editing features](editing.md)

Folding is provided via the standard `textDocument/foldingRange` request, using the current buffer contents without compilation. After full and incremental edits, the next request returns the new range immediately.

## LaTeX

Supports chapters, nested environments, multiline curly braces, continuous full-line `%` comments, and verbatim environments. Chapter collapse ends before the next sibling or higher heading, retaining that heading. A verbatim environment only collapses text boundaries recognized by the parser; curly braces and formula delimiters displayed within them are not scanned.

Supports pairs of `% region title` / `% endregion` and `% #region title` / `% #endregion` comment tags, which can be nested and also work with BibTeX top-level comments. The mark must be a whole line comment, regardless of indentation; no area folding is generated when the corresponding mark is missing. `comment` The environment is marked for comment collapse to facilitate the use of the editor's "Collapse all comments".

Environment name reuses static parameter rules, supporting name peripheral comments and bare ASCII single-character parameters. The normal environment header is still folded normally when it is immediately followed by the `%` comment; the dynamic environment name does not match.

Multiline formulas with the following four explicit delimiters are supported:

```tex
\[
  E = mc^2
\]

$$
  a^2 + b^2 = c^2
$$

\(
  x + y
\)

$
  x - y
$
```

Groups within formulas continue to collapse individually. Formulas in deeper groups such as `\text{for $x$}` will not close the outer formula in advance; when the inner formula spans rows, an independent folding range is also provided.

Comments, URLs, inline verbatim content, verbatim environments, and literal conversion data of `\detokenize` / `\string` do not produce formula folding. Control characters, curly braces, and `$` in the `\let` copy source and `\meaning` operands confirmed by the parser are also not used as delimiters. Single-line formulas, missing terminators, different types of closing characters, and unclosed formulas that span the corresponding brace groups do not cause formula folding; legal brace groups can still be folded.

## BibTeX

Collapse boundary reuse document parser, supports two entry forms `{...}` and `(...)`, as well as case-insensitive `@string`, `@preamble` and `@comment`. `@comment` and contiguous entire lines of `%` comments are marked as comment folding, including comments between fields within the entry; the remaining entries are marked as zone folding. Percent signs in literal values do not participate in comment folding.

```bibtex
@article(example,
  title = "A ) character inside a quoted title",
  note = {A parenthesis ) inside braces}
)
```

Parentheses, percent signs, and text that appears to be an item within a quoted or braced value will not end prematurely or create another item fold. Items missing a closing character are not collapsed. Ordinary text in BibTeX, LaTeX curly braces and formulas within field values are not scanned again according to the LaTeX source code; `.bst` style files do not adopt these rules either.

## Client capabilities

- `lineFoldingOnly: true`: Only the starting and ending lines are returned; duplicates are removed from the same line range.
- Other clients: UTF-16 starting and ending columns are returned, and nested ranges of different columns in the same group of rows are retained separately. CRLF carriage returns do not count towards the end of the column.
- `foldingRange.collapsedText: true`: Returns the first line of text preview, up to 80 Unicode characters, and appends an omission mark; this field is omitted when support is not declared.
- `rangeLimit`: Limit the number of returns after sorting and deduplication, `0` returns an empty list.

Formulas use static delimiter analysis, no macro expansion is performed, and custom delimiters or category code changes are not inferred; the execution semantics of environment aliases are handled by other navigation functions, and collapse is still based on the direct environment structure.

## verify

```sh
cargo test --locked --test folding --test navigation --test fancyvrb_environments
cargo test --locked --test protocol folding_
```

Covers four formula delimiters, nested formulas in text groups, literal operands, BibTeX parentheses keys and quotes, special entries, comment areas and nested tags, incomplete input, UTF-16, CRLF, preview length, client limit, and unsaved incremental edits under stdio.
