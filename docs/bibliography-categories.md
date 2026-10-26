# biblatex categories

[Document Catalog](README.md) · [Documentation](bibliography.md) · [biblatex Reference](biblatex.md)

Use `\DeclareBibliographyCategory{category}` to declare the category, and then assign the citation key via `\addtocategory{category}{keys}`. For example:

```tex
\documentclass{article}
\usepackage[backend=bibtex,style=numeric]{biblatex}
\addbibresource{refs.bib}
\DeclareBibliographyCategory{selected}
\addtocategory{selected}{alpha,beta}
\begin{document}
\nocite{alpha,beta}
\printbibliography[category=selected]
\end{document}
```

`refs.bib` requires two entries: `alpha` and `beta`. The classification assignments themselves do not reference the entries; they still need to be included in the reference via `\cite` or `\nocite`. For compilation tools and configuration, see [Build guide](build.md).

Two category commands provide command snippets and signature help. The second parameter of `\addtocategory` supports citation key completion, hover, definition jump, reference search, document highlighting, semantic highlighting and cross-file renaming. The key list supports spanning lines and comments, and unclosed lists can also be completed; editing only replaces the current key, retaining other keys, comments, and original line breaks.

When you rename the `.bib` entry, the classification assignments and references using that database are updated simultaneously. The first parameter is the category name, which will be retained even if it has the same name as the citation key; the category name uses independent symbol indexing and supports completion, suspension, definition jump, reference search, document highlighting, preparation for renaming, and cross-file renaming. Shared databases and ambiguity handling follow [Document Renaming Rules](bibliography.md#entry-relationships-and-rename).

Citation key editing only recognizes static key lists in the second curly brace argument; macro expansion, nested keys, or non-contiguous key names spliced by comments are not inferred. This command does not accept asterisks and optional descriptions, nor does it treat the subsequent third group or text as a citation key.

Classification assignments do not generate citation warnings for missing citation keys, so warnings and spelling fixes for real citations on the same line are only targeted to the citation command.

The first parameter of `\ifentrycategory{key}{category}{true}{false}` supports completion, navigation, hover, citation, highlighting and renaming of a single citation key; the second parameter uses the category name. When an item is renamed, only the key is updated, when a category is renamed, only the category is updated, and the normal text in the branch is retained. The query does not produce ordinary missing reference warnings and does not accept asterisks, optional specifications, or comma key lists.


Classification names are available for the `DeclareBibliographyCategory` declaration, the first parameter of `addtocategory`, the second parameter of `ifentrycategory` and the first parameter of `ifcategory`, and the `category` / `notcategory` options of `printbibliography`. The `category=name` in the static `defbibfilter` Boolean expression also participates in editing; the filter name, other filter values, the establishment/not establishment code and the text of the same name in the title are retained. There is no need to install biblatex or run a compile to use these source code features.

Normal calls, static `csuse` / `csname` and unique zero-argument pure forward aliases use the command meaning of the current project. Potential references in the full macro body participate in editing; known parameter data and latent names in token operands prevent partial renaming, and confirmed literal examples retain the original text. Command redefinitions, unsaved definitions, and independent projects are handled separately; shared sources must be consistent among all consumers, and the same category must be associated with unique claims. Copy commands with ambiguous aliases are used only to reject incomplete renames, not to provide editable calls accordingly.

Single brace protection in options, comments around names, and cross-line whitespace remain intact. Completion replaces the entire remaining suffix of the name, supports arguments that have not been closed, and stops before subsequent TeX tokens or whitespace. Whitespace within the name, dynamic names, names spliced by comments, and unsupported filter expressions are not expanded; category renaming is rejected when these category usages exist in the project. The new name must be a static identifier without square brackets and must not conflict with other names in the same namespace.

A single index covers only interrelated projects, with a limit of 32 MiB of source code cumulative, 200,000 commands, 8192 declarations, 20,000 items per source, and 100,000 cumulative name explanations. A single filter expression can read up to 100,000 lexical tokens; partial edits are not returned when the limit is exceeded, the item index is truncated, or the consumer meaning is inconsistent.

verify:

```sh
cargo test --locked --test bibliography_categories
cargo test --locked --test protocol bibliography_category_names
cargo test --locked --test bibliography_categories real_biblatex_category -- --include-ignored
```

The last item requires pdfLaTeX, BibTeX and biblatex to verify that the category members and filter output are still valid after the name change.

`\assignrefcontextcats[options]{categories}` and its asterisk form support completion, jump, preview, quotation, highlighting and complete renaming of the category name list. Square brackets are required parameters, and empty options are written as `[]{categories}`; command snippets and signature hints retain this boundary. Static naming and pure forward calls also participate in indexing, and comments, other list items, context prefixes, citation keys, and verbatim content remain in the original text.

An empty list does not assign any categories and can be used to complete category names. Dynamic or incomplete classification lists, command operands, captured duplicates, inconsistent meanings of shared files, and exceeding the analysis budget reject partial renames; target names also do not capture other classifications that appear only in the allocation list. Contextual numerical prompts and keyword assignments continue to be independently certified.

New and unsaved master files also participate in shared classification meaning and claim identity authentication; different or missing claims prevent renaming that affects shared files, and compatibility claims and unrelated items remain separate.

Database fields `execute` and `@preamble` can be used to call classification commands from the generated BBL. Since these codes have not yet been included in the classification index, when such records exist for related projects, we refuse to prepare or perform complete classification renaming to avoid compilation errors caused by missing classification parameters in fields. The check covers all shared consumers, including newly created and unsaved master files; re-authenticate after closing or removing the execution code. Category navigation for known source code continues to be available, and literal examples and unrelated projects remain separate.
