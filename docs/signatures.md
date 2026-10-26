# Signature help

[Back to documentation](README.md) · [Editing features](editing.md) · [Complete](completion.md)

Provide parameter signatures for common commands, environments, and cross-file user macros/environments, distinguish between required and optional parameters, and track current parameters in nested calls. Editors can request prompts via the standard `textDocument/signatureHelp`, which Neovim can call `vim.lsp.buf.signature_help()`.

Continuous explanation comments before unique user macro or environment declarations are also displayed in the signature help, retaining the declaration type, definition file and line number, as well as the original signature and current parameters. Supports direct calls, static `csuse` / `csname` calls, and cross-file unsaved modifications; pure forwarding aliases that can infer signatures use the description of the alias itself. Macros and environments with the same name are parsed separately, and a certain description is not selected when making repeated declarations. Comments are displayed as plain text. For extraction rules and budgets, see [User Statement Document](command-documentation.md).

## Declaration and optional parameters

Traditional `newcommand` / `renewcommand` / `providecommand`, `DeclareRobustCommand` and robust variants of etoolbox, as well as `newenvironment` / `renewenvironment` The declaration itself also prompts the name, number of parameters, default values and definition text, recognizes asterisks and two macro name grouping forms; completion retains existing control word names.

The short title of the chapter and figure title and the radical degree of `sqrt` follow the boundary of the first `]` that is not protected by curly braces; the literal `[` will not swallow the signature help of subsequent titles or expressions, and the prompt will stop after the call is completed. The asterisk chapter command directly prompts for the title, does not accept the short title parameter, and does not treat the subsequent text as the title.

The optional label of `item` uses the same right bracket boundary as the option of `printbibliography`, supporting direct, static naming and pure forwarding calls; the text after the option no longer displays the signature help of these commands.

Standard footer commands support `\footnote[number]{text}`, `\footnotetext[number]{text}`, and `\footnotemark[number]`, including unfinished parameters, static naming, and pure forward calls. Command completion provides footnote body fragments, retaining existing numbering and text. `footnotemark` does not read the required parameters. The curly braces that follow are groupings that are executed independently, and the labels in them retain navigation, references, and complete renaming. User redefinition within the project takes precedence over built-in parameter rules.

The header of `\begin{minted}[options]{language}` also provides signature help, supporting unfinished parameters, comment continuation lines and Unicode positions; the code body and the end of the closing line continue to be processed verbatim. See [minted code](minted.md) for redefinition and project scope rules.

FancyVerb's `Verbatim`, `LVerbatim`, `BVerbatim` and their asterisk forms prompt `[options]`; `SaveVerbatim` also prompts `{name}`, and `VerbatimOut` also prompts `{file}`. Unfinished parameters and comment continuations are also supported. An ordinary newline will end the header search without options, and the square brackets or curly braces on the next line are verbatim; the save name and file name with options can be read across ordinary newlines. Complete parameters, headers, discarded tails, and code text will not be prompted. In-project redefinition, undefinition, and FancyVerb environment customization will suppress the corresponding built-in signatures, and unsaved setting changes will take effect immediately.

`\begin{lstlisting}[options]` also supports header hints, including unfinished options, brace-protected closing brackets, comment continuation lines, and Unicode. The prompt is stopped after the option ends, square brackets after ordinary line breaks retain verbatim content; LaTeX commands executed normally after closing the markup retain their respective signatures. Redefinition, undefinition and `lstnewenvironment` declaration within the project will suppress the corresponding built-in prompts, and unsaved setting modifications will take effect immediately.

`\lstinline[options]` prompts header options for inline code, supports unfinished options, comment continuation lines, and the first right square bracket not protected by curly braces; the code body remains verbatim. Command completion provides the `\lstinline|code|` fragment. When the code already exists, only the command name is completed, and the original curly braces or delimiters are retained.

## Comments in number of parameters

The number of parameters is read according to TeX comment rules, supporting comment continuation lines, CRLF and numbers separated by comments. For example:

```tex
\newcommand{\pair}[2% two arguments
][DEF]{#1:#2}
\pair{Body}
```

The call signature is `\pair[#1=DEF]{#2}`; command completion only inserts a placeholder for the required second parameter, leaving the first parameter at its default value. The macro also works when declared in other files of the project.

The number of arguments is limited to 0–9 for legacy macros and environments; to 1–9 for `DeclarePairedDelimiterX` and `DeclarePairedDelimiterXPP`. Spaces between ordinary numbers will not be removed by annotation processing, for example, `1 2` will not become `12`. Dynamic counting, escape tokens, illegal content, overflow and out-of-bounds quantities do not infer signatures and do not perform macro expansion.

Macro parameter consumption analysis uses the same quantity rules; functions such as color renaming continue to exclude commands passed in as other macro arguments to avoid modifying the names in the data as actual calls. See [Symbols and Renaming](symbols.md) for the remaining rules for symbols and renaming.

When the string test command is used as the control word parameter of a user macro, it prompts for the known signature of the outer macro; for the cursor boundaries inside the control word, at the end of the file, and after the end of the parameter, see [String commands in macro parameters](string-test-operands.md#signature-help).

## Default value preview

Default values in signature help and command completion details are displayed according to TeX comment rules. For example, `F` after the continuation line of `DE% comment` is displayed as `DEF`; CRLF, continuous comment lines and Unicode are supported. Applies to legacy macros/environments, as well as xparse's `O`, `D`, `R` defaults.

The preview retains escape tokens such as `\%`, explicit control spaces `\ `, and retains the separation between control words and subsequent letters when necessary to avoid spelling `\relax` and the following `DEF` into another command name. Normal blank compression, the preview can display up to 80 characters; comments are eliminated first, and then the preview is intercepted.

The default preview does not perform macro expansion. Completion continues by omitting optional parameters and inserting only required parameter placeholders, using the default behavior in the declaration.
