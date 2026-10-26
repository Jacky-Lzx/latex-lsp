# Glossary, acronym, and index symbols

Terminology and abbreviations support static keys for `newglossaryentry`, `longnewglossaryentry`, `newacronym`, `newabbreviation`, `newacro`, `acrodef`, `DeclareAcronym`. Common `gls`, `glspl`, `glsentry...`, `acrshort`, `acrlong`, `ac` series calls provide key completion, statement jump, quote, hover, rename, document symbol and semantic highlighting. `loadglsentries` The loaded source file participates in project dependencies and scope.

Term keys are processed separately from labels and citation keys. Keys with the same name will not rename each other. Undefined terms and duplicate declarations produce immediate diagnostics; undefined diagnostics can be turned off via `diagnostics.undefinedGlossary`. Renaming of shared declarations will overwrite individual usage projects, and will be rejected if declaration ambiguity is encountered.

The static index entries of `index` / `sindex` support the same navigation and editing functions. The name takes the sorted text before `@` or `|`, allowing spaces and `!` levels; renaming retains the printing form and packaging commands. For example, renaming `\index{Ada Lovelace@Lovelace|textbf}` only replaces `Ada Lovelace`. Each index item is itself a declaration, and the jump will list all positions of the item.

`makeindex[name=people]` and `newindex{people}...` declare named indexes; `index[people]` and `printindex[people]` provide independent index name completion, navigation, renaming and undefined diagnostics. This diagnostic can be switched off via `diagnostics.undefinedIndexes`.

Only static names are analyzed. Comments, verbatim code, deferred macro text, dynamic keys, and MakeIndex quote escapes do not participate in symbol rewriting. Does not simulate custom macro expansion or run glossary handlers.
