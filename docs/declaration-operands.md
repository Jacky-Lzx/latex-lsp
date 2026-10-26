# Command-name operands and declarations

[Back to documentation](README.md) · [Symbols and renaming](symbols.md)

The control word read in a declaration or copy statement is the command name operand and is not equivalent to executing the command. For example:

```tex
\newcommand{\ordinary}[1]{#1}
\let\saved\newcommand
\ordinary{Body}
```

Only `ordinary` and `saved` are declared here. The copy source `newcommand` will not be executed again, nor will the subsequent `ordinary` call be registered as the second statement. Renaming `ordinary` can still update its actual declaration and call.

## Indexing and editing

Recognizes direct declaration targets, as well as control word sources for `let`, kernel command copy, and `cslet`; compatible with allowed grouping notation and static `csname` targets with explicit `expandafter` expansion. The command name itself is retained as a macro reference, supporting navigation and renaming of the source command.

The copied statement command does not read plain text backwards, avoiding the generation of spurious macros, registers, environments, counters, Boolean switches, toggle, labels, document entries, PDF targets, colors, or save text. Operands also do not provide name completion, signature help, key name highlighting, or rename editing for subsequent groupings. For example, `{real}` in `\let\saved\ref{real}` is a common group and will be retained when renaming the label with the same name.

Copying `verb`, `begin`, etc. commands will also not initiate a verbatim scan or environment pairing at the copy location. The actual commands that appear after copying are still parsed normally; for the continuation execution rules of `futurelet`, see [Look-ahead Target](futurelet.md).

The command name completion at the operand position only replaces the name and does not insert the calling parameters; see [Completion Rules](completion.md#command-name-operands) for details. `meaning` For similar indexing and editing rules when viewing commands, see [meaning operands](meaning.md).

The targets of `undef` / `gundef` are also read as unexpanded control words, supporting bare control words and single control word grouping; `csundef` / `csgundef` use static names. The undo location is a macro reference rather than a new statement, and the original syntax of the target will not be executed. For details, see [Undo Definition and String Test](string-test-undefinitions.md).

## Boundary

The source of the original `let` is a token; copying `{` will not treat the entire following set of contents as an operand. For the matching rules between source curly braces and macro text, see [Static Braces Diagnostics](brace-diagnostics.md). Static analysis preserves the actual subsequent declarations and does not perform any macro expansion, category code changes, or assignment procedures, nor does it infer a definite call signature for the copy.

When the declaration/copy keyword has been explicitly rewritten through direct, named or static `csname` declaration earlier in this document, subsequent operands will no longer be confirmed according to the original rules of the keyword. This judgment retains uncertainty and does not simulate the recovery of meaning after the grouping ends.

For possible parameter consumption relationships in compilation diagnostics, see also [Macro Sources for Special Character Repair](literal-macro-scopes.md).
