# Direct calls to saved string commands

[Back to documentation](README.md) · [Save and restore](string-test-copies.md) · [Signature help](signatures.md)

## Saved meaning and number of parameters

The control words obtained by static copying can be called directly. The copy retains the meaning of the string at the time of copying, and subsequent redefinition of the source command will not change it:

```tex
\usepackage{etoolbox}
\newbool{draft}
\let\saved\ifblank
\renewcommand{\ifblank}[1]{#1}
\saved{\booltrue{draft}}{Empty}{Tokens}
```

`booltrue{draft}` here is a literal string, does not change boolean, and does not participate in boolean navigation, completion or renaming. The copy name `saved` can be renamed as a normal macro; the symbols in the two conditional branches still participate in indexing and editing.

The number of parameters comes from the saved meaning. For example, after `\let\ifblank\ifstrequal`, the first two parameters of `ifblank` are literal strings; after reverse copying, `ifstrequal` only has the first data parameter. The signature help usess the actual call name and the number of saved parameters.

`input` in a literal argument, declaration, or verbatim command does not load files, define symbols, or obscure subsequent code. When the target name is `input`, `label`, `newcommand`, `verb`, etc. and there is an existing command name, the confirmed native string meaning will also replace the original parsing rules of the name.

The copy itself can also be [undone](string-test-undefinitions.md); the call after undoing no longer reads the original string parameters, and other copies previously saved from it are still valid.

## Cross-file updates and ambiguities

Copy meanings are propagated in file loading order, supporting chained static copying. Editing dependency buffers, restoring disk contents after closing, and disk file notifications update the literal boundaries and signature help at the call site; the version number of the main document remains unchanged.

When a shared file takes on different meanings in multiple contexts:

- Both are native string tests but have different numbers of data parameters: the common first parameter remains literal, and the second parameter is marked as ambiguous.
- Native tests coexist with custom commands: data parameters that may be executed as code are marked as ambiguous.
- Renames that would modify ambiguous parameters are rejected, other unambiguous symbols can still be edited; signature help are not guessed when the complete parameter form cannot be determined.

The compilation process uses the project source code snapshot before compilation to analyze the logs. Literal data in the Boolean expression `test{\saved{...}}` will not preempt the error position of the sibling executable boolean / toggle; when there is an ambiguity in the number of unknown tests or parameters, the entire line of diagnostics is retained, and no guesswork is fixed. Conservative static rules are still used when parsing logs alone and without project context.

Do not infer dynamic names, arbitrary macro expansion, side effects after user macro execution, or automatic recovery at the end of grouping. Preserve uncertainty when replication primitives are redefined or their origin cannot be confirmed.

## verify

```sh
cargo test --locked --test copied_string_tests
cargo test --locked --test protocol copied_string_predicate_arities
cargo test --locked --test boolean_expression_diagnostics copied_string_tests
cargo test --locked --test copied_string_tests -- --include-ignored
```

Real regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox, covering four predicates, bidirectional parameter number changes, and three copy forms. Before and after renaming the macro name and boolean, compare the literal original text, boolean status and PDF page number.
