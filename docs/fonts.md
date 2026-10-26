# Font declarations, diagnostics, and analysis

[Back to documentation](README.md) · [Compile](build.md) · [Compile diagnostics](diagnostics.md)

When compilation successfully generates a PDF, font substitution and missing word warnings may still indicate that the typesetting is different than expected. Results summaries help clients focus on displaying these issues, with location-by-location diagnostics retaining the original levels and counts.

## NFSS font declaration

The font definition file `.fd` uses LaTeX source code analysis to participate in project scanning, file monitoring and explicit `\input{...fd}` path completion. The following kernel declaration provides command descriptions, argument snippets, and per-argument hints:

| Command | Parameter order |
| --- | --- |
| `\DeclareFontEncoding` | Encoding, text settings, math settings |
| `\DeclareFontEncodingDefaults` | Default text settings, default math settings |
| `\DeclareFontFamily` | Encoding, font family, loading settings |
| `\DeclareFontShape` | Encoding, font family, font weight, font style, font size specifications, loading settings |
| `\DeclareFontSubstitution` | Encoding, default font family, font weight, font style |
| `\DeclareMathSizes` | Main text size, mathematical font size, superscript font size, secondary superscript font size |

`\DeclareMathSizes*` also supports four signature help, and the default fragment is inserted in the form without asterisks. The other five categories of statements do not accept asterisks. Prompts can be grouped across comments, line breaks, and nested settings; these encoding and font family parameters are not registered as command definitions. If the client does not support snippets, only the command name will be inserted. If there are parameters, the original parameters will be retained. Customized commands with the same name in the project use their own signatures.

For example, declare an independent encoding and font family using Computer Modern:

```tex
\DeclareFontEncoding{LSP}{}{}
\DeclareFontFamily{LSP}{lsptest}{}
\DeclareFontShape{LSP}{lsptest}{m}{n}{<-> cmr10}{}
\DeclareFontSubstitution{LSP}{lsptest}{m}{n}
\DeclareMathSizes{10}{10}{7}{5}
```

Encoding, font family, and font file name do not currently provide registry-driven candidates or navigation; font size specifications are still filled in according to the original LaTeX syntax.

## Select font and line spacing

`\fontencoding{encoding}`, `\fontfamily{family}`, `\fontseries{series}`, `\fontshape{shape}`, `\fontsize{size}{baseline skip}` and `\linespread{factor}` provide parameter snippets and hints. They set the request properties and require a subsequent call to `\selectfont` without parameters before they are applied; `\usefont{encoding}{family}{series}{shape}` selects the font immediately and retains the current font size. The distance between the font size and the baseline without units is treated as pt, and the baseline distance is also scaled by the line spacing factor.

```tex
{\fontencoding{OT1}\fontfamily{cmr}\fontseries{m}\fontshape{n}
 \linespread{1.25}\fontsize{10}{12}\selectfont
 Body text uses a 10pt font with a 15pt baseline skip.}
```

Select commands share existing parameter preservation, client fragment negotiation, and user signature override rules. `\selectfont` and its static alias will not consume the parameters of subsequent commands, and subsequent commands can still obtain prompts and source code repairs normally.

## Font and font size replacement

The complete standard `LaTeX Font Warning` in the final log is divided into two categories:

- `analysis.fontSubstitutions`: Requested font and actual substitute font, fields are `requested` and `replacement`.
- `analysis.fontSizeSubstitutions`: Font size replacement, fields are `font`, `requestedSize` and `replacementSize`.

The font uses NFSS's `encoding/family/series/shape` four-segment identity; the font size retains the decimal string output by the compiler and is not converted to a floating point number. For example, the relevant fields in `analysis`:

```json
{
  "fontSubstitutions": [
    {"requested": "OT1/missing/m/n", "replacement": "OT1/cmr/m/n"}
  ],
  "fontSizeSubstitutions": [
    {"font": "OT1/limited/m/n", "requestedSize": "14.7", "replacementSize": "10"}
  ],
  "truncated": false
}
```

When the same substitution appears in different locations or files, it is merged and sorted by field tuples; each of the two categories has a maximum of 128 entries. Maximum 256 Unicode characters per font identity and 128 ASCII characters per font size. Omit full records when over budget and set `analysis.truncated: true` without truncating the name.

The recovery engine wraps long font names and sizes in bytes, retaining the original whitespace, Unicode, and parentheses within the name; does not speculate on substitution relationships when the complete warning cannot be confirmed. Only summary warnings such as “Some font shapes…” or “Size substitutions…” do not generate specific records. Font size substitutions below the LaTeX `fontsubfuzz` threshold without warning will also not be inferred.

The two types of substitutions themselves do not set `rerunRequired` and do not change the build success status or the number of warnings. `latex/buildFinished` and `latex.buildStatus` return the same summary, and only this final log is used; after repair, the two arrays are restored to `[]`, and the substitutions in earlier compilation rounds are not retained. For results such as cancellation, timeout or expiration, continue to press [Result Analysis](build-analysis.md) to return to `analysis: null`.

## Missing word summary

`analysis.missingCharacters` summarizes missing word errors and warnings in the final log, sorted by character code point. Compatible with warning and error byte wrapping, does not insert spaces in font names. When the same character appears in different locations, files or fonts, it is merged into one item; `fonts` retains the deduplicated and sorted engine font description, removes an ending punctuation added by the engine, and retains the punctuation of the font itself; the line number description of the auxiliary file attached to the server does not enter the font description. `severity` uses LSP levels: `1` is an error, `2` is a warning; errors are reserved when the same character has both levels. Summary does not change position-by-position diagnostics and build error numbers.

For example, the relevant fields in `analysis`:

```json
{
  "missingCharacters": [
    {
      "character": "中",
      "codepoint": "U+4E2D",
      "severity": 1,
      "fonts": ["[lmroman10-regular]:mapping=tex-text;"]
    }
  ],
  "truncated": false
}
```

Only missing character messages whose literal characters are consistent with legal Unicode code points are included; when the source code location cannot be confirmed, the characters and fonts confirmed by the log can still be summarized, and whether the positioning is reliable will continue to be expressed by diagnostics. The summary is regenerated for each compilation, and an empty array is returned after the missing words are resolved; the output of earlier compilation rounds will not be retained in the summary of the final log.

A maximum of 128 characters is reserved, with a maximum of 8 font descriptions per character and a maximum of 256 Unicode characters per description. Characters or font descriptions that exceed the limit are omitted and `analysis.truncated` is set; the name is not truncated. When the font description of a certain character is all too long, the character, level and empty `fonts` array are still retained. Subsequent errors for an already included character will still increase its level, even if the new font description exceeds the limit.

The error that LaTeX does not set the Unicode character mapping is recorded separately in `analysis.unsupportedUnicodeCharacters`; for the difference and handling method of missing characters in the font, see [Unicode input error](unicode-errors.md).

## Missing word positioning

When the source and line number are confirmed, and the literal characters in the missing character message are consistent with legal Unicode code points, the diagnostics is accompanied by the `missing-character` code and `character`, `codepoint` data. Source code echo can uniquely check the continuous direct literal text, accurately mark each position of the missing word, and support sub-file, UTF-16, CRLF and shortened echo. For example, even if the three missing characters of `中😀中` share the same echo, three ranges will be retained and three errors will be counted; duplicate logs will not repeatedly add diagnoses at the same location.

Locates characters that do not borrow from the previous word across actual whitespace. Whitespace after control words, which TeX ignores, does not confirm text fragment boundaries; such cases, as well as macro expansion, generated characters, ignored regions, ambiguous echoes, and invalid line numbers, retain line-level positions and do not infer the source based on nearby characters of the same name. The same log parsing reuses the positioning results of the same missing word context, and retains the diagnostics of different files, line numbers, echoes and fonts.

