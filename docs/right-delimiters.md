# Extra right delimiters

[Back to documentation](README.md) · [Compile Diagnostics](diagnostics.md) · [Missing right delimiter](missing-right-delimiters.md)

When TeX reports `Extra \right.`, there is no `\left` paired with this command in the current math group. The server checks the log source code echo, locates the diagnostics to the actual `\right`, and provides `Remove unmatched '\right'` quick repair.

## use

Fix to remove selected `\right`, keeping the normal delimiter after it. For example:

```latex
$a\right)$                    % after the fix：$a)$
$a\left(b\right)\right]$      % after the fix：$a\left(b\right)]$
$a\right\rangle$              % after the fix：$a\rangle$
```

This cancels automatic size matching there. If you originally want the entire expression to use automatic size, you should add the corresponding `\left` according to the meaning of the formula; the server will not guess the position of the delimiter.

The invisible delimiter `\right.` is processed separately: replace `\right` with `{}`, and remove this point to avoid generating visible periods or splicing adjacent control words. For example, `$\alpha\right.b$` becomes `$\alpha{}b$`.

Supports whitespace, line crossings and comments between `\right` and delimiters. Repair only edits commands and necessary invisible points, retaining comments and line breaks; multiple errors on the same line are located separately. The paired `\left...\right` will not be modified due to the presence of an error next to it.

## Confirmed delimiter

Supports literals `(`, `)`, `[`, `]`, `/`, `|`, `.`. For standard naming delimiters, also check that the delimiter's own `\delimiter` expansion frame is consistent with the name in the source code:

- `langle` / `rangle`、`lbrace` / `rbrace`。
- `lfloor` / `rfloor`、`lceil` / `rceil`。
- `vert` / `Vert`、`backslash`。
- `uparrow` / `downarrow` / `updownarrow` and their first letters in capital letters.
- `lgroup` / `rgroup`、`lmoustache` / `rmoustache`。

Ordinary Greek letters, common mathematical symbols, large operators, and parameterless operators such as `sin` and `lim` will not swallow up subsequent `\right`; user redefinitions within the project are still processed according to their actual parameters.

## Sources and old operations

The diagnostic code is `unmatched-right-delimiter`. `data.delimiter` retains the source code delimiter, `sourceContext` saves the source code echo, and `sourceLine` is the echo line number starting from zero; in cross-line calls, this line can be different from the marked command line. Naming delimiter `delimiterFrame` holds the confirmed expansion name.

Supports legacy `!` logs, file line number format, subfiles, CRLF and UTF-16 ranges. The full echo must match the source code prefix; the shortened echo must match uniquely in the entire line. Incomplete or unmatched echoes, unknown sources, user macro expansion stacks, definition bodies, comments, verbatim text, and unsupported dynamic delimiters retain the original diagnostics and do not provide editing. Name delimiters that lack corresponding standard expansion frames are not inferred.

Fix to inherit parameter syntax from sources loaded at compile time, excluding commands consumed by macros; this fix is not provided when `right` or the corresponding naming delimiter is redefined. Re-verify the source code, document version and macro source when requesting operations and executing old client commands; after the parameter definition in the dependency changes, the old fix becomes invalid. For source analysis rules, see [Macro Source Description](literal-macro-scopes.md).

## verify

```sh
cargo test --locked --test right_delimiters
cargo test --locked --test right_delimiters -- --include-ignored
cargo test --locked --test protocol right_delimiter
```

Real regression requires pdfLaTeX, XeLaTeX and LuaLaTeX, fixing cross-file text in 60 / 79 columns and two error formats, checking literal delimiters, all the above named delimiters, invisible points and comments, before actually generating a page of PDF and confirming error clearance.
