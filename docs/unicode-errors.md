# Unicode input errors

[Back to documentation](README.md) · [Compilation diagnostics](diagnostics.md) · [Fonts and missing characters](fonts.md)

pdfLaTeX reports `LaTeX Error: Unicode character … not set up for use with LaTeX.` when it encounters a Unicode character that has no defined mapping. The server retains compilation error levels, identifies specific characters when possible, and summarizes these characters and code points in the compilation results.

## position

When the source and line number are confirmed, the diagnostic carries the `unsupported-unicode-character` code, `data.character` and `data.codepoint` characters and the `U+…` code point respectively. This information is provided only if a single Unicode scalar in the log is consistent with a legal code point; incomplete messages, code point mismatches, or similar messages from other packages are not inferred.

Direct source code echo must be able to match the complete prefix, or uniquely match the shortened suffix in the line. Supports subfiles, UTF-16, CRLF, repeated characters on the same line, and message wrapping for 60 / 79 column logs. A blank line inserted when pdfTeX reaches exactly the printing width, and will only be merged if subsequent content will form a complete, consistent error message; no other diagnostics or file switches will be skipped.

Macro expansion, hidden expansion in command arguments, comments, verbatim content, macro definition bodies, and ambiguous echo retain full line position. When the source cannot be confirmed or the line number is invalid, characters after the parent file or the first and last lines are not borrowed; characters can still be summarized based on the complete error message.

The combined emoji may contain multiple Unicode scalars, and the compiler will diagnose them separately when reporting errors. For example, `👩‍💻` can contain three items: `U+1F469`, `U+200D`, and `U+1F4BB`.

## Summary of compilation results

`latex/buildFinished` and `analysis.unsupportedUnicodeCharacters` of `latex.buildStatus` are sorted by code point, and only one item of the same character is retained in different locations and files:

```json
{
  "unsupportedUnicodeCharacters": [
    {"character": "中", "codepoint": "U+4E2D"},
    {"character": "😀", "codepoint": "U+1F600"}
  ],
  "truncated": false
}
```

The summary only includes complete standard messages of the error level, which are regenerated based on the final log each time, and the records before the previous round of compilation or repair are not retained; `[]` is returned after the error is resolved. Reserves up to 128 distinct characters, sets `analysis.truncated: true` when budget is exceeded or diagnostics are truncated. Position-by-position diagnostics and error numbers remain independent: three character positions on the same line can produce three errors, and the summary only retains the different characters among them.

`Missing character: There is no … in font …` when the font is missing glyphs continues into `analysis.missingCharacters`, retaining the font and severity level. Both types of problems can exist at the same time. For details, see [Abstract of Missing Words](fonts.md#missing-word-summary). Cancellation, timeout and expired results still return `analysis: null` according to [Compilation Result Analysis](build-analysis.md).

## Processing method

If you need to preserve pdfLaTeX, you can define semantically explicit mappings for a small number of symbols in the preamble. For example, sort the Unicode minus sign into a mathematical minus sign:

```tex
\DeclareUnicodeCharacter{2212}{\ensuremath{-}}
```

The mapped alternative content needs to be chosen based on the meaning of the characters. Chinese and multilingual texts usually require the selection of appropriate Unicode engines, language packages and fonts; just switching to XeLaTeX or LuaLaTeX may still cause font missing characters. For configuration methods, see [Compilation Configuration](build-configuration.md). The server provides diagnostic information and preserves the user's choice of character representation and compilation engine.

For the real tool regression command, see [Test Documentation](testing.md).
