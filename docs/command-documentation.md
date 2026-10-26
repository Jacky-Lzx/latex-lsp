# User declaration documentation comments

[Back to documentation](README.md) · [Symbols and renaming](symbols.md)

The user macro's floating prompt displays the continuous explanation comments before the declaration, while retaining the original name, declaration type, definition file and line number. For example:

```tex
% Format a highlighted term.
%
% #1 is the term text.
\newcommand{\term}[1]{\textbf{#1}}
```

This description can be seen when hovering over `\term`. Supports cross-file definitions, unsaved buffers, Chinese, emoji, CRLF, cross-line declaration parameters within the current project, and declaration prefixes such as `long` / `global` / `protected`. When a definition buffer is modified or closed, the description is updated with the existing document lifecycle.

Only use real comments confirmed by the parser. Remove the starting indent, the consecutive `%` and a space after each line, retaining the paragraph and internal layout. Physical blank lines, other code, inline comments trailing code, and TeX/Bib directives, arara, and Vim modeline end description blocks; no description is appended when the line of declaration is preceded by other code. Comments in macro parameter headers do not replace the description before declaration.

Descriptions are sent as plain text, with Markdown, HTML, URLs, and backslashes preserved literal. The current project must have only one user statement with the same name. In case of ambiguity, no comments are attached to any statement; macros with the same name in other independent projects do not affect each other. The original help for built-in commands remains intact.

Each block has a maximum of 32 lines and 8192 bytes. No comments will be appended when exceeding the limit. This function provides instructions in the source code, does not execute TeX, does not infer the meaning of parameters, and does not treat the macro text as a document.

[Signature help](signatures.md) Also displays the description before the unique user macro or environment declaration, retaining the signature, current parameters and definition position; direct calls and static named calls share this rule. Pure forwarding aliases display their own declared description when the signature is inferable, and the environment uses a macro-independent declaration namespace.

The same comment rules are used for user environment hover prompts, `begin` / `end` name completion, environment copy source completion, and direct control word completion. Descriptions are available for legacy, xparse, Environ, and environment replication declarations; environments that appear but are not declared retain their original brief information. The uniqueness of macros and environments is judged separately. Macros with the same name will not replace the environment statement, and repeated environment statements will not select a certain comment. The environment completion document follows the client capability negotiation and expiration verification in [Completion](completion.md), and retains all edits to cross-line names.

```sh
cargo test --locked --lib command_docs
cargo test --locked --lib environment_documentation
cargo test --locked --test protocol user_command_documentation
cargo test --locked --test protocol user_environment_documentation
```
