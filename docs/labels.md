# Labels and cross-references

[Documentation](README.md) · [Editing features](editing.md) · [PDF Target and Bookmark](pdf-destinations.md)

Tags support completion, navigation, hover, reference lookup, highlighting and cross-file renaming. The following describes key name boundaries, each package interface, and positioning rules for compilation diagnostics.

## Key name and editing range

- Normal brackets in tag keys (including unpaired brackets) support navigation, completion, hover, reference lookup, semantic highlighting, and cross-file renaming; single tags, lists, range endpoints, and hyperref / refcount parameters share literal key boundaries, preserving comments around names.

Ordinary tag references are also matched according to the complete key name. `label{plain}` and `label{ plain }` belong to different tags. Navigation, reference, duplicate definition diagnostics and renaming will not merge other tags with blank spaces; tags with blank spaces will not be entered into the static symbol index for the time being. The key list of `cref` / `Cref` continues to be processed according to the list rules. Renaming retains commas and spaces between keys. Undefined reference diagnostics also checks key names according to the call type.

In tags and document lists, TeX comments before and after key names will be retained, and commas and brackets in comments are not involved in key name resolution; navigation, renaming, completion, compilation warning repair, and semantic highlighting share the actual source code scope. Macro expansion, nested grouping, and non-contiguous key names spliced by comments are not statically inferred.

Single-tagged declarations, references, range endpoints, and the tag parameters of hyperref and refcount also recognize TeX comments before and after the name. Navigation and renaming retain normal whitespace semantics in names; when completion is accepted, peripheral whitespace in the old name is cleared individually and comments are retained, and compile warnings are still checked against the line number of the actual closing argument.

## Common tag references

- `subref` / `subref*` subgraphs and subtable references of `subcaption` support label navigation, completion, hover, reference search and cross-file renaming; compilation warnings are located in the source code key, and label spelling fixes are provided. Keep asterisks, titles, comments, and verbatim text.

- varioref references: `\vref`, `\Vref`, `\vpageref` and `\fullref` Spelling fixes for shared tab navigation, reference lookup, renaming, completion and compile warnings. Comments, optional descriptions and legal asterisks are retained, and command snippets and signature help are provided; `vref` / `Vref` accepts a same-page description, `vpageref` accepts a same-page description and a different-page prefix, and `fullref` uses a single label parameter. Unsupported asterisks or redundant optional parameters skip label editing. The two endpoints `\vrefrange[same-page]{first}{last}` and `\vpagerefrange[same-page]{first}{last}` respectively support these features; the latter accepts asterisks. Compilation warnings for cross-line calls, even if the end line of the second argument is reported, locates the actual missing endpoint. The description ends at the first `]` that is not protected by curly braces, and the literal `[` in the description is retained; the text left after ending the description early will not be mistaken for label editing, and signature help follow the same rules.

- `crefrange` / `Crefrange`, `cpagerefrange` / `Cpagerefrange` and asterisk forms of `cleveref` support tab navigation, completion, hover, reference lookup and cross-file renaming for both endpoints, and provide signature help, command snippets, semantic highlighting and spelling fixes for compile warnings. Endpoints are treated as individual labels, preserving asterisks, comments, and newlines; invalid optional descriptions are not treated as labels.

- `cpageref` / `Cpageref` page number lists and asterisk forms of `cleveref` use the same tag editing function to recognize comma-separated tags one by one; completion, renaming and compile warning fixes preserve other tags, separated commas and whitespace.

Tag list completion supports cross-line calls. `cref` / `Cref` also follows the actual asterisk and parameter boundaries, and does not provide tag editing after invalid optional descriptions.

`getrefnumber` / `getpagerefnumber`, `getrefbykeydefault`, `IfRefUndefinedExpandable` / `IfRefUndefinedBabel` and `refused` of `refcount` support tag completion, navigation, hover, referencing and renaming, and provide command snippets and signature help. Only the first parameter is used as a label, and field names, default values, and conditional branches are retained; compilation warnings from other references in the same line will not be assigned to read or conditional judgment commands that do not generate warnings.

refcount's `\getrefbykeydefault`, `\IfRefUndefinedExpandable`, `\IfRefUndefinedBabel` and four `setcounterref`/`addtocounterref`/`setcounterpageref`/`addtocounterpageref` interfaces also support precise compilation error and spelling fixes for individual macros within key parameters. Match the actual parameter consumption boundaries and internal error prefixes of each interface, and retain the default values, conditional branches and counter names. The reference number can be extracted when the field selection parameter is empty; row-level diagnostics are temporarily reserved for dynamic field selection parameters or counter names.

The label and asterisk forms of `Ref` and `autopageref` also support the above editing functions and recognize the blank or comment before the asterisk. Without an asterisk, ordinary reference and hyperlink reference warnings may be generated at the same time. Diagnostics and spelling repair calls are checked separately; hyperlink warnings will not be attributed to asterisk calls that only generate ordinary warnings or read commands that do not generate warnings.

## hyperref tag

- Static tags for `\hyperref[label]{text}` and `\hyperref[{label}]{text}` support completion, navigation, hover, referencing and cross-file renaming; compiled `Hyper reference` warns about targeting tags and provides spelling fixes, supporting cross-line links. The URL form of the four required parameters provides separate signature help, retaining the link text, address and percent signs therein. When completion, spelling repair, and renaming introduce names containing `[` / `]`, labels are automatically protected with curly braces; existing protective braces are retained. Tags not protected by curly braces end at the first `]` to distinguish tags with the literal `[` from other names; navigation, completion, renaming, and warning positioning use actual tag boundaries, and optional tags of `hyperdef` also follow this rule.

- The optional tag of `\hyperdef[label]{category}{name}{text}` is added to the index as a declaration and shares navigation, completion, duplicate definition checking and safe renaming with ordinary tags. Renaming preserves the category, name, and display text of the PDF target; supports `[{label}]` protection form, and provides command snippets and signature help.

Comments can be preserved inside and outside hyperref's protective curly braces; navigation and renaming can continue after completion or renaming of names containing square brackets.
