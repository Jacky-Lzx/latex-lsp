# String test commands as macro arguments

[Back to documentation](README.md) · [String test](string-tests.md) · [Signature help](signatures.md)

When the user macro reads `ifblank`, `notblank`, `ifstrempty` or `ifstrequal` as an unexpanded control word parameter, the command reference is retained at this position and subsequent parameters are not read by string test call. This rule also applies to [Saved copy of string command](string-test-copy-calls.md) without first copying, redefining, or undoing the original command.

## Parameter boundaries and cross-file loading

For example `defs.tex`:

```tex
\newcommand{\discard}[1]{}
```

Main file:

```tex
\documentclass{article}
\usepackage{etoolbox}
\newbool{draft}
\input{defs}
\begin{document}
\discard\ifblank\input{after}
\ifblank{\boolfalse{draft}}{Empty}{Tokens}
\end{document}
```

`discard` reads a single `ifblank` token, and the subsequent `input{after}` is still actually loaded; `boolfalse{draft}` in an independent call is retained as string data. If `after.tex` contains `\booltrue{draft}`, renaming the boolean `draft` will update the declaration and operations in the file, retaining the original text in the string. Declarations and calls to `discard` can still be navigated and renamed across files.

Confirm parameter syntax according to the loading order before the call site, support static declarations in the current file and loaded dependencies, including:

| Declaration example | Call example |
| --- | --- |
| `\newcommand{\discard}[1]{}` | `\discard\ifblank` |
| `\newcommand{\discard}[2]{}` | `\discard{Unused}\ifblank` |
| `\newcommand{\discard}[2][Default]{}` | `\discard[note]\ifblank` |
| `\NewDocumentCommand{\discard}{O{Default} m}{}` | `\discard[note]\ifblank` |
| `\def\discard#1!{}` | `\discard\ifblank!` |

Incremental modifications to dependent buffers, disk recovery after shutdown, and file change notifications will refresh parameter boundaries, and the main document does not need to be modified. After changing `discard` from one parameter to zero parameters, the subsequent `ifblank` resumes calling the analysis according to the actual string; the navigation and versioned rename are updated accordingly.

## Signature help

Displays the outer layer `\discard{#1}` when prompted inside the control word parameter of `\discard\ifblank`. If the cursor is at the end of the control word at the end of the file, it is still considered that the parameter is being entered; if there is already a delimiter such as `{` behind it, the control word parameter has ended and the outer prompt will stop. This operand does not display signature help for stand-alone `ifblank` calls.

## Uncertain parameter syntax

Conservative argument fallbacks for unknown macros are not used as evidence for determining consumption control words. For example:

```tex
\let\saved\ifblank
\unknown\saved{\booltrue{draft}\input{ghost}}{Y}{N}
```

When the parameter syntax of `unknown` cannot be confirmed, the server continues to protect the string data of `saved` and does not load `ghost` accordingly or treats the boolean in the data as an actual reference.

When a shared file uses different numbers of parameters in different roots, each root context is retained separately; if the same location could therefore be both string data and normal code, renames that modify the ambiguous location will be rejected. Other explicit symbols can still be edited, see [Cross-file context](string-test-projects.md) for details. Only static parameter reading is inferred here, and the execution of any user macro text or parameter re-expansion is not simulated.

## verify

```sh
cargo test --locked --test string_test_operands
cargo test --locked --test protocol imported_string_predicate_operands
cargo test --locked --test string_test_operands -- --include-ignored
```

True regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox. Covers four predicates and multiple parameter syntaxes, renames outer macros and cross-file booleans respectively, and checks subsequent file loading, string original text, actual status output and PDF page number.
