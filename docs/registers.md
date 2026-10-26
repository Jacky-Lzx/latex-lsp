# Registers and lengths

[Back to documentation](README.md) · [Symbols and renaming](symbols.md) · [Counters and theorems](counters.md)

## Declaration and renaming

`newlength`, `newsavebox`, `newcount` / `newdimen` / `newskip` / `newmuskip` / `newtoks` / `newbox` and other register allocations, and `chardef` / `mathchardef`, register bindings, and font declarations are also entered into the symbol index, supporting variable completion, navigation, hover, and safe renaming across files; recognizing direct names and literal `csname` declarations expanded by `expandafter`. Common length operations and save box commands provide fragment and signature help; renaming will check for conflicts with existing macros, registers, counter derivation commands, and common built-in length/register primitives.

## etoolbox type checking

The following commands provide snippets, signature help, and name completion. The static first parameter in the name form supports navigation, referencing, semantic highlighting and cross-file renaming; the direct form accepts brace-wrapped or bare control sequences.

| type of check | name form | direct form |
| --- | --- | --- |
| Counting register | `ifcscounter{quantity}{then}{else}` | `ifdefcounter{\quantity}{then}{else}` |
| Length (skip register) | `ifcslength{measure}{then}{else}` | `ifdeflength{\measure}{then}{else}` |
| Dimensions (dimen register) | `ifcsdimen{distance}{then}{else}` | `ifdefdimen{\distance}{then}{else}` |

`ifcscounter` Check the type of control sequence. For LaTeX counters created by `\newcounter{exercise}`, use `ifcscounter{c@exercise}`; use `ifltxcounter{exercise}` only to check if the counter name exists. Theorem own counter `c@name` also participates in navigation and renaming.

Name completion allows the selection of control sequences such as ordinary macros, since a false check is also a legal use. Rename maintains the first name parameter, retaining the branch body, ordinary text with the same name, labels, comments, and verbatim content. Dynamic names are not inferred; shared files refuse to be renamed when pointing to independent claims in different papers.

The direct form only checks the first control sequence and does not execute it. When `newcommand` or `verb` is used as the checked object, false claims will not be indexed or word-by-word scans will be started based on this; after this document explicitly redefines these check commands, the original operand rules will no longer apply.

## Length expression assignment

etoolbox's `deflength` supports navigation and renaming of segments, signature help, and length-controlled sequences. Accepts bare control sequences or control sequences wrapped in curly braces, retaining existing operands when completing the command:

```tex
\usepackage{etoolbox}
\newlength{\measure}
\deflength\measure{5pt plus 2pt minus 1pt}
{\deflength{\measure}{\measure*2+1pt}} % local assignment
{\global\deflength\measure{\measure+2pt}} % global assignment
```

The assignment uses TeX's glue expression, preserving the stretch and shrink components; the server does not evaluate them. Renaming synchronously maintains register references in expressions, retaining calculation logic and local and global scopes.

For error location, spelling repair and candidate rules of undefined length, see [Length Diagnostics and Repair](length-diagnostics.md). For the real three-engine verification and default regression commands, see [Test Instructions](testing.md#register-and-length).
