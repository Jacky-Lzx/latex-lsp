# Missing delimiter operands

[Back to documentation](README.md) · [Compile Diagnostics](diagnostics.md) · [Missing right delimiter](missing-right-delimiters.md)

A delimiter is required after `\left`, `\right` and `\middle`. When the compiler reports `Missing delimiter (. inserted).`, the server checks the token to be reread and the source code echo, flags the diagnostics on the command with the missing delimiter, and provides a quick fix for `Insert invisible delimiter after '…'`.

```latex
$\left a+b\right)$          % after the fix：$\left. a+b\right)$
$\left(a+b\right x$         % after the fix：$\left(a+b\right. x$
$\left(a\middle b\right)$   % after the fix：$\left(a\middle. b\right)$
```

Fix inserting a dot after a command to represent an invisible delimiter. Original letters, mathematical symbols, whitespace, line breaks, and comments are retained intact. If visible brackets are required, please use `\left(`, `\right)`, etc. according to the meaning of the formula.

This is different from [missing the entire right delimiter command](missing-right-delimiters.md): this operation only supplements the parameters of the existing command, not `\right`, and does not guarantee to eliminate other grouping or mathematical errors. For example, the curly brackets in `\left{` are still grouping; if you originally want to display the set brackets, you should use `\left\{` instead.

## Positioning and Source

The `<to be read again>` frame needs to be next to the error message. The next line must be a single ASCII token that can correspond to the source code, followed by the direct `l.N` source code echo. The token must appear exactly after the command, during which only blanks and comments are allowed; ordinary characters and control sequences such as `\alpha` can be checked. The full echo must match the source code prefix, and the shortened echo must match the unique line.

Supports subfiles, UTF-16 ranges, CRLF, legacy and file line number logging, and cross-line comments. The diagnostic overwrites the command and the actual edit is a zero-width insert at the end of the command. The second line of source code echo and the fully confirmed standard TeX help text will not be used as file entry or exit events; subsequent main file and sibling file diagnostics remain from their respective sources.

Additional user expanded frames, tokens that are inconsistent with the source code after expansion, incomplete or ambiguous echoes, unknown sources, definition text, macro parameters, verbatim content and comments retain the original diagnostics and do not provide editing. The writing method of `^^` and non-ASCII token will not be fixed on a per-token basis for the time being; similar errors within `\big` and other macros will not be directly inferred from primitives.

Fix parameter syntax in confirmed sources when using compile. This operation is not provided when the primitive or related control words are redefined within the project; the document version, source code, diagnostic data and dependencies are rechecked when requesting operations and executing old client commands. After the parameter definition in the dependency changes, the old operation may become invalid. See [Macro Source Rules](literal-macro-scopes.md) for details.

The diagnostic code is `missing-delimiter-operand`. `data.delimiterCommand` saves the command with backslash, `recoveryToken` saves the verified token to be processed, `sourceContext` saves the source code echo, and `sourceLine` saves the echo line number starting from zero; in cross-line calls, the echo line can be different from the diagnostic line.

## verify

```sh
cargo test --locked --test delimiter_operands
cargo test --locked --test delimiter_operands -- --include-ignored
cargo test --locked --test protocol right_delimiter
```

True regression requires pdfLaTeX, XeLaTeX and LuaLaTeX. Fixed 13 errors in subfiles in 60 / 79 columns and two error formats, covering three primitives, control words and script symbols, continuous calls and cross-line comments, confirming that one page PDF is generated, errors are cleared and main file warnings come from the correct source. stdio regression covers Code Action, old client commands, fake replacement rejections, version invalidation and diagnostic cleanup after recompilation.
