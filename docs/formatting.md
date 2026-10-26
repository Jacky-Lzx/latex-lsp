# Formatting

[Back to documentation](README.md) · [Editing features](editing.md)

Supports standard `textDocument/formatting` document formatting, calling `tex-fmt` by default, and `latexindent` can also be selected. Please install the selected tool to `PATH` of the server, or configure the absolute path to the executable file. Neovim can call `vim.lsp.buf.format()`.

Set using `initializationOptions` (`init_options` for Neovim) or `workspace/didChangeConfiguration`:

```json
{
  "formatting": {
    "onType": true,
    "formatter": "tex-fmt",
    "executable": null,
    "arguments": null,
    "timeoutSeconds": 10
  }
}
```

- `formatter`: `tex-fmt` (default) or `latexindent`.
- `onType`: Default is `true`, indent when the following built-in input is enabled; set to `false` to turn it off individually.
- `executable`: The name of the selected tool is used by default; the full path can be specified.
- `arguments`: `null` uses the default parameters listed below; the array completely replaces the default parameters. The standard input mode of the tool must be retained when customizing. `tex-fmt` uses `--stdin`, and `latexindent` uses `-`. Parameters are passed directly to the process without going through the shell.
- `timeoutSeconds`: Default 10 seconds, allow 1–3600 seconds.

Switch to latexindent, for example:

```lua
init_options = {
  formatting = { formatter = 'latexindent' },
}
```

The default `tex-fmt` parameter is `--stdin --tabsize N`; `--usetabs` is added when the client requests tab indentation. The working directory is the directory where the current document is located, allowing the tool to read the project configuration according to its own rules. Non-file buffers that have not yet been saved use the workspace directory, and inherit the server directory when there is no workspace.

The default `latexindent` parameters include `-l '-y=defaultIndent: "indentation"' -`, and `-c` / `-g` will put auxiliary files and logs into independent temporary directories, which will be deleted after the task is completed. `-l` loads the local settings supported by the tool, and `defaultIndent` adopts the client's `tabSize` and `insertSpaces`. When using custom parameters, indentation, logging, and local configuration are determined by this parameter.

The server writes the current LaTeX buffer text to the tool's standard input and produces a UTF-16 text edit on standard output excluding the same suffixes; the source file is not overwritten. Keep the LF/CRLF of the original document, and return an empty edit list if the content has not changed. Unsaved buffers are also supported; BibTeX uses the following built-in formatters, `.bst` is not formatted yet.

## Range formatting

Support standard `textDocument/rangeFormatting`. Uses the same tools, configuration, and full buffer input as document formatting, so selected entries still use the indentation of the outer environment, and verbatim code retains full context. The server aligns the text before and after formatting by line content, and only returns modifications that are completely within the requested range, leaving content outside the selection unchanged. Wrapping, merging, or other non-split modifications across selection boundaries are skipped; the selection can be enlarged or document formatting used. When you need to indent the entire line, please start selecting from column 0 of the line.

Multiple edits are returned non-overlapping, using the UTF-16 coordinates of the original document. Column numbers that exceed the line length are converged to the end of the line according to LSP rules; ranges that are empty after normalization return an empty edit list. Reverse order, non-existent rows, or range slicing of UTF-16 surrogate pairs returns `InvalidParams`. Row alignment limits the size of the intermediate matching table, and consecutive overwrites that exceed the budget are processed as a whole to avoid large files consuming unbounded memory. Cancellations, timeouts, document or configuration changes are subject to the same protections as document formatting.

## Indent when typing

Supports standard `textDocument/onTypeFormatting`. The trigger characters are line feed and `}`. The client needs to enable input formatting. Uses the built-in local indenter, does not rely on `tex-fmt` / `latexindent`, nor reads their configuration. `formatting.onType: false` can be closed; directory configuration also takes effect.

When wrapping a new line, the indentation of the previous line is used as the basis, and one level is increased or decreased according to the unoffset environment and brace grouping of the line; `\\begingroup` / `\\endgroup`, `\\bgroup` / `\\egroup` are supported, and the `document` environment is not increased by one level. A `}` or `\\end{environment}` at the beginning of the current line is reduced by one level; entering a closing `}` also adjusts the line. The closing character at the beginning of the previous line is already reflected in its indentation and is not reduced repeatedly.

Only replace the spaces and tabs at the beginning of the current line and use the client `tabSize` / `insertSpaces`; do not modify the text, commands, comments or line breaks. Supports unsaved buffers, UTF-16 and CRLF. Parentheses, escaped braces, verbatim content, and known token operands in comments are not included in the structure; verbatim blocks are not formatted internally. Environment and grouping commands in a completed macro definition do not affect the indentation after the definition, and the curly braces in the definition body still provide indentation. BibTeX and `.bst` are currently formatting without input.

Environment and group command reuse project-level parameter analysis: commands in user macro parameters do not increase or decrease the environment indentation, and the curly braces of the parameters themselves are still used to edit group content. Supports static `csuse` / `csname` calls, confirmable pure forwarding aliases and cross-file unsaved definitions; primitives are processed according to the parameter syntax of user macros when they are redefined. `noexpand` suppresses only one control token, whitespace and grouping after static naming calls follow the same single-token rules.

This is an indentation aid for adjacent lines without performing any macro expansion or reflowing the entire document. Cross-line or dynamic environment names, lines exceeding 64 KiB, indentations exceeding 4096 columns, and project analysis scope exceeding 2 MiB of source code, 1024 definitions, or 32768 commands are skipped; parameter context is also not guessed when the project index is incomplete. Entering `}` at the end of a line break request in the middle of the text and an ordinary command parameter will not trigger the entire line to be reflowed. You can unify the layout of existing documents with full-text or range formatting.

## BibTeX formatting

BibTeX / biblatex bibliographic buffer supports full-text and range formatting without the need to install external tools. The server reuses document entries and expression parsing, and only adjusts the external white space of the fields of complete ordinary entries: each field is newlined and indented, leaving a space on both sides of `=`, closing the delimiter and starting a new line. Indentation takes the requested `tabSize`/`insertSpaces`; no commas are added or removed.

Field values and their internal wrapping, brace protection, case, string concatenation, key names, field and entry order remain intact. Comments and the whitespace that maintains comment boundaries remain intact, so sections containing comments may retain their original layout. New line breaks follow the LF / CRLF of the document, and the original mixed line breaks will not be unified. Incomplete entries, invalid assignments, no-field entries, and `@string`, `@preamble`, `@comment` remain as is.

Range requests filter directly for blank modifications located within the selection, supporting adjacent single-line entries and unnamed document buffers. LaTeX's `formatter`, `executable` and `arguments` are not used for document formatting. The tasks run in the background and share the concurrency upper limit, cancellation and expiration result verification; the output is also limited to 16 MiB. The server only returns edits and does not write to the literature database.

Formatting is performed in the background, while editing and other requests can still be sent. After the document content, version, language type or formatting configuration changes, `ContentModified` is returned. Please request formatting again. Run up to 4 formatting tasks simultaneously. Cancellation of requests, timeouts, shutdowns, and disconnections terminate tasks; Unix also terminates child process groups.

An error is returned when the tool is missing, non-zero exit, invalid UTF-8, empty output for a non-empty document, or stdout exceeds 16 MiB, and no edits are generated. The actual typesetting and verbatim environment processing follows the rules of the chosen tool.

verify:

```sh
cargo test --locked --lib formatting
cargo test --locked --lib on_type_formatting
cargo test --locked --test protocol on_type_formatting
cargo test --locked --lib bib_formatting -- --include-ignored
cargo test --locked --test protocol formatting_
cargo test --locked --test protocol bibliography_formatting_
cargo test --locked --test protocol range_formatting_
cargo test --locked --test protocol range_formatting_with_real_tex_fmt -- --ignored
cargo test --locked --test protocol formatting_with_real_tex_fmt -- --ignored
cargo test --locked --test protocol formatting_with_real_latexindent -- --ignored
```

The last two items require true `tex-fmt` / `latexindent` respectively, verify four-space indentation, verbatim content, CRLF, UTF-16, repeated formatting and not writing the source file. The default test uses independent tool fixtures to verify parameters, buffer inputs, configuration updates, errors, timeouts, cancellations, expired results, and process cleanup for both backends.
