# futurelet lookahead targets

[Back to documentation](README.md) · [Symbols and renaming](symbols.md)

`\futurelet`'s direct control word target enters the macro symbol index, supporting completion, definition jump, hover, quote, symbol list and cross-file renaming. For example:

```tex
\def\inspect#1{\ifx\lookahead A Found A\fi #1}
\futurelet\lookahead\inspect A
```

TeX first assigns the meaning of `A` to `\lookahead`, and then executes `\inspect A`. When renaming `lookahead` from a declaration or reference, references in the definition body, direct calls, and static `csname` references are synchronously maintained, and `inspect`, lookahead characters, comments, and verbatim content are preserved.

## Static name target

Explicit, adjacent `expandafter` expands the complete static `csname` target:

```tex
\expandafter\futurelet\csname lookahead\endcsname\inspect A
```

Targets used only by static names can be renamed to Chinese or hyphenated names; the new name must be able to form a TeX control word when there is a direct control word declaration or reference. Inheriting project scope, shared origin ambiguity, and name conflict checking, LSP editing preserves versions of open documents.

## Execution and analysis boundaries

The continue execution command and look-ahead token after `futurelet` will be put back into the input stream and cannot be processed as source parameters that are not executed in ordinary copy statements. For example, in `\futurelet\lookahead\input{child}`, `input` is still the actual file loaded.

Targets take the form of bare control words or the static `csname` above; braced targets, dynamic names, and arbitrary expansion sequences are not inferred as static declarations. The argument signatures of copy calls are not inferred from lookahead tokens, nor are subsequent assignments or TeX grouped scoping simulated.

Command completion provides the `\futurelet\lookahead\continuation A` fragment, leaving spaces between the continued execution of the command and the letter token to avoid merging into a control word.
