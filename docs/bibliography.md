# Bibliography

[Documentation](README.md) · [Editing features](editing.md) · [Build diagnostics](diagnostics.md)

Citation keys support completion, hover, definition jump, citation search, highlighting and cross-file renaming. Direct calls follow the same project command meaning as statically named calls; ordinary parameters referenced by the redefinition are preserved, and parameter data or shared source ambiguities prevent partial renaming. Editors only use identifiable static source code ranges, retaining text with the same name, captions, comments, and verbatim text.

- [Citation keys and lists](#citation-keys-and-lists)
- [Bibliography outline](#bibliography-outline)
- [Entry type and field completion](completion.md#bibtex-entries-and-fields)
- [Citation commands](#citation-commands)
- [biblatex literature resources](biblatex-resources.md)
- [biblatex reference](biblatex.md)
- [biblatex Bibliography categories](bibliography-categories.md)
- [Manual bibliographies](#manual-bibliographies)
- [Item relationship and renaming](#entry-relationships-and-rename)
- [String abbreviations](#string-abbreviations)
- [URL and DOI links](#url-and-doi-links)

## Citation keys and lists

Document preview and document hover for citation completion show the complete original BibTeX entry, preserving field order, indentation, line breaks, and comments, without appending the directory path. The preview uses BibTeX code blocks and does not expand string macros, splice expressions, or inherited fields. The entry list still uses title, author, and year summaries; citation completion also retains the ability to search by these metadata.

Citation keys support ordinary parentheses, including unpaired parentheses, such as `book(old)`, `book(old`, and `book)old`. `\bibitem`, the citation command, and the BibTeX database in two forms of entry delimitation uniformly support completion, jump, semantic markup, and renaming; cross-entry references and metadata use complete keys, and editing retains comments and text.

The key list of citations also supports cross-line completion, including the supported commands of ordinary `cite`, natbib and biblatex; only the current key is replaced, and other citation keys, description parameters, newlines and list blanks are retained.

TeX comments before and after the key name will be retained, and commas and brackets in the comments will not participate in key name resolution. Macro expansion, nested grouping, and non-contiguous key names spliced by comments are not statically inferred.

A comma after a BibTeX entry key can start a new line; LF, CRLF, and indentation are supported, and navigation and renaming preserve the original whitespace and metadata after the key.

Ordinary whitespace, line breaks, and indentation are also allowed between BibTeX's `@` and entry types, such as `@ book{key, ...}`; special entries and field contents are still treated as full entry boundaries.

BibTeX custom entry types support legal identifiers such as numeric suffixes, underscores, and hyphens, such as `@custom1` and `@custom_type`, and participate in indexing, metadata, and renaming. The compiler's standard two-line warning about unknown style types will be located to the type name corresponding to the `.bib` entry; using the explicitly reported database, key name and line number, the type and key branch can also be located, see [Entry Type Diagnostics](bibliography-diagnostics.md#undefined-entry-type) for details.

## Bibliography outline

The outline range of the document entry covers the entire structure from `@` to the end of the text, and the selection range is still the citation key; the range of the abbreviation `@string` covers the complete assignment, and the selection range is the abbreviation name. Entries continue to show title, author, year preview, retaining Unicode, quotes, and parentheses in custom types and keys.

Clients that support hierarchical notation display the field below the entry, with the field name retaining its original case and allowing direct navigation to the field when selected. The field range covers names, assignments and primitive value expressions, and supports multi-line literals, nested grouping, macros and `#` splicing; repeated fields are listed separately, and field style text inside comments and literal values will not become additional symbols. The contents of `@comment` and `@preamble` do not provide entry or field symbols.

When editing unfinished text, assignments, or field names, the outline uses the scope currently recognized by the parser; constructions that have not yet been closed can still display the outline, and folding continues to require a complete construction. Unnamed documents and other extension files identified by the `bib` / `bibtex` language type also support outlines, and incremental editing updates entries and fields immediately.

The static `ids` alias is listed under the corresponding field as a key symbol, showing the corresponding primary key, and the selection range accurately covers the alias itself. Duplicate aliases are retained separately; macros, splices, repeated fields, and other unconfirmed declarations do not generate speculative symbols.

Legacy clients get tile notation, the field's `containerName` points to the owning citation key, and the alias's container is the original `ids` field name; the jump range retains the actual name. Workspace symbol search supports citation keys, static aliases, and string abbreviations. The field name itself does not participate in the search. Incremental editing updates aliases and ranges instantly, and restores disk content after closing the buffer.

The field-less brace entry `@misc{key}` also participates in outline, citation key navigation and renaming, and allows line breaks and closing after the key. BibTeX requires commas for parenthesized entries, and the empty field form should be written `@misc(key,)`.

## Citation commands

Common `cite`, `nocite`, natbib and supported biblatex commands share the citation key index.

The flat `bibcite` compilation record of traditional LaTeX/BibTeX can display the bibliographic label next to the output citation, and supports independent switches, request ranges and auxiliary file updates; structured natbib and biblatex special records are not inferred yet. See [Auxiliary Document Reference](auxiliary-references.md) for configuration and boundaries.

natbib's `citealt` / `citealp`, `citenum`, `citeyearpar`, `citefullauthor` and `Citet` / `Citep` / `Citealt` / `Citealp` / `Citeauthor` supports citation key completion, navigation, citations, semantic highlighting, safe renaming, compilation warning location and spelling repair, and provides command snippets and signature help. Identifies supported asterisk forms and whitespace and comments before and after the asterisk, preserving reference lists, comment parameters, and text.

Single, multiple group, and volume number citations to biblatex and their asterisk forms are provided in [biblatex Citation Guide](biblatex.md), including title, date, URL, footnotes, and descriptive citations recording only the citation key.

`\entrydata{key}{code}` and its asterisk form support navigation, completion, and renaming of a single static citation key, preserving text of the same name in the execution code; see [Entry Data Context](biblatex.md#entry-data-context) for details.

The single-key checks of `\ifentryexists{key}{true}{false}` and `\ifentryinbib{key}{true}{false}` also participate in document editing, retaining the text with the same name in the two branches; see [Entry Condition Check](biblatex.md#entry-condition-check) for details.

Citations in running text through the `bibentry` package's `\bibentry{key}` support navigation, hover, references, completion, signature help, semantic highlighting, and cross-file rename. Missing-citation warnings from actual builds can identify the key and offer spelling fixes. The argument is treated as one citation key; comments, entry text, and unrelated labels with the same name are preserved. Key lists, dynamic names, unsupported stars, and optional notes are not treated as editable keys.

Ordinary citation commands that accept optional descriptions recognize at most two description parameters; when redundant descriptions occur, the following text will not participate in citation key completion, navigation, renaming, or accurate compilation warning positioning. The description parameter ends at the first unprotected `]` according to TeX rules; `[` in the text, brace-protected square brackets, whitespace, and comments are still preserved, and the signature help follow the same boundaries.

natbib's `defcitealias{key}{display text}` uses a single static citation key, and `citetalias` / `citepalias` reference the same key. Renaming maintains alias settings and references synchronously, retaining comments and display text before and after the key; comma lists, semantic blanks, macros, or dynamic keys spliced by comments are not inferred. The alias setting itself does not generate missing citation key warnings, and does not receive compilation diagnostics for other references in the same line.

## Manual bibliographies

Manual bibliographies `\bibitem[label]{key}` prompts optional display labels and required citation keys. Read at most one optional tag, ending at the first unprotected brace `]`, retaining the literal `[`; text after the key, redundant or unclosed tags, and unsupported asterisks will not trigger citation key editing or prompts.

Annotations before and after declaring an identified key, references navigate to the actual key name and participate in renaming, semantic highlighting, duplicate definition checking and spelling fixes. Editors retain optional documentation tags and article bodies, and skip statements that require macro expansion or contain semantic gaps.

## Entry relationships and rename

`\defbibentryset{set key}{member keys}` supports navigation, completion and independent renaming of dynamic collection key declarations and member references; see [Document Collection](bibliography-sets.md) for details. Collection keys share the document namespace with database entries and aliases, and ambiguous edits are rejected when conflicts exist.

The static `ids` field of biblatex can declare multiple bibliographic aliases for an entry, such as `@book{main,ids={old,legacy},title={Example}}`. In `\cite{old}` or relationship fields, you can complete, jump to alias declaration, view the associated primary key, find references, highlight and rename. Aliases are independent of the primary key: renaming `old` updates only the alias and its references, renaming `main` retains `ids` and the alias references. Alias declarations also support semantic highlighting and selection ranges.

`ids` field names ignore case as per ASCII, accept a literal list of curly braces or double quotes unique to the complete entry, and retain white space on either side of the comma. The list is comma separated by default and Biber custom `xsvsep` configuration is not read. String macros, concatenation, nested grouping, TeX commands, repeated fields, and unclosed entries do not generate speculative aliases. You can view all definitions when a duplicate alias or an alias conflicts with the actual primary key. Renaming will reject the ambiguity; new aliases containing double quotes will convert the double quote list into curly braces. Primary key positioning of compile diagnostics continues to use actual database entries.

Field name completion and hover instructions include `ids` without first using the field in the project. Also available in `ismn`, `label`, `options`, `presort`, `shorthand`, `shorteditor`, `sortname` and `sorttitle` built-in hints; these hints do not serve as data model validation rules.

Postscript, annotations, comments, foreword, introduction Author, patent holder, original language, place of publication, publisher, and descriptions are also provided for the standard `namea`–`namec` / `lista`–`listf` slots; these descriptions are shared with corresponding name completion and hover for biblatex data access commands.

References between document entries support `crossref`, `xref`, `xdata`, `related` and `entryset`, including curly braces or double quotes, and comma-separated lists of the latter three types of fields; you can complete, jump, find references, highlight and rename parent entries. Field names are not case-sensitive, and completion also supports unclosed values. String macros, `#` splicing, or values containing TeX commands need to be evaluated and will not be automatically modified for the time being; the same-named text in titles and comments will be retained. The citation key cannot be renamed to `*` to avoid changing the select-all meaning of `\nocite{*}`.

When the project selects resources by `refsection`, the entry primary key and static `ids` declaration in the database jump to itself, and the metadata of the declaration is displayed on hover. The relationship field only provides targets when pointing to the same statement in all partitions and shared projects containing the current library; when the parent entries with the same name belong to the default library and the selected library, they are navigated separately, and the two libraries are not merged. True duplicate declarations in valid resources preserve multiple targets and do not show any previews; targets are not guessed for unknown execution paths, unclosed entries, duplicate relationship fields, and incomplete indexes. Reference search and highlighting use a complete static identity graph, covering TeX references, five types of relationship fields and all shared consumers; primary keys and IDS maintain independent identities, and the same source code range that is entered repeatedly is returned only once. New roots that have not yet been saved are also included in the verification. Partial results will not be returned when there are binding conflicts with the same source code, delayed code, or incomplete analysis. Certifiable claims support preparation for renames and full, versioned workspace editing; only the full reference to that primary key or IDS is changed, preserving claims of the same name in other partitions. Renaming rejects primary key or alias conflicts in the same region, and also checks whether the new name captures existing TeX references and relational fields; quoted new keys are merged into integer edits and converted to curly braces.

Relational field completion is also filtered by these partitions and shared consumers: only keys corresponding to the same declaration in each related resource collection are proposed, keys unique to the default library and conflicting candidates are excluded. Supports unclosed items and literal values, replaces the entire current list item and preserves adjacent members, whitespace and newlines; preserves the original integer curly brace conversion when a quoted key is selected. `xdata` only recommends the `@xdata` entry and its static IDS alias. The type of the alias comes from the entry, and the primary key must be unique and consistent in each partition; ordinary books, handwritten documents, and source code collections will not be used as XDATA data sources. Type filtering and partition identity verification take effect at the same time, and unsaved entry type modifications will immediately refresh the candidates. Static named collection coverage in normal projects is also verified in execution order; there is no fallback to the old type in the database when the execution, consumer or full index cannot confirm it. XDATA completion containing actual source code collections continues to use source code namespace protection such as IDS and dynamic execution.

Dynamic collections may replace entries with the same name in the database. In partitioned projects with resource selection, the declarations, members and TeX references of the complete static `defbibentryset` support jump and hover by numerical partition and execution order. New partitions with the same resource list still have independent collections; the preface definition belongs to partition 0, and the definitions in the group continue to take effect in this partition. When an explicit reference is first made and then the collection is defined, it continues to point to the original database entry; a standalone `nocite{*}` does not lock the entry. Normal and static named calls follow the same rules, duplicate entries and all shared consumers must give the same target.

Relationship fields also support jump and hover to valid source collections. All five types of relationships first verify complete entries, unique literal fields and database parent keys, and then compare the database's targets in all digital partitions and shared consumers; the same database does not return any targets when two partitions point to collections and books respectively. When a book is locked by an explicit early reference, the relationship continues to display book metadata.

Relational field completion compares candidate claims in all number partitions and consumers; collection metadata is displayed when the source collection covers a book with the same name, and book information is retained when a book is explicitly requested to be locked. Static named collections also provide their own candidate metadata, and repeated executions of the same physical declaration are merged into a single candidate. Source code collection candidates for `crossref`/`xref` must also have consistent underlying database parent entries; collections with only source code declarations cannot be used by Biber database relationships. Active source collections are not candidates for `xdata`, default `related`, or nested `entryset` to avoid type errors, missing collection clones, or corrupted memberships. This candidate is excluded when declaration identities of different partitions conflict, and common entries continue to be available; unfinished relationship values, full suffix replacement, and list and quote editing rules remain available.

Full reference and document highlighting distinguishes collections, members, and database entries with the same name by source URI, range, and numeric partitioning, covering all saved and unsaved consumer and database relationships. Repeated execution of the same physical declaration returns only one declaration; when referenced first and then defined, the unused collection only returns itself, and the book reference remains independent. When shared physical references have binding conflicts in different partitions or consumers, partial graphs are not returned; relationships that only have source code collections but lack database parent keys are not certified as complete graphs.

After the complete identity is authenticated, independent source code collections and database members support preparation for renaming and versioned workspace editing, updating only claims and references for that identity. The new name must have no primary key, alias, or source code declaration conflicts in all related numeric partitions, nor must it capture existing requests, collection members, or database relationships; disjoint numeric partitions can retain declarations of the same name. Static naming, repeated input, unsaved consumers, and entire set of quoted values use the same validation.

Source code collections involving `crossref`/`xref` support complete renaming after the dual dependencies are certified: the database parent key of the same name must be unique, all consumers of the original library and each related numeric partition must be covered by the same collection, and the editor updates the collection, reference, relationship value and implicit parent entry primary key at the same time. Independent book consumers, other collective identities, unknown consumers and `xdata`/`related`/`entryset` relationships continue to be rejected. Unused source code statements after locking books with early requests will not be renamed to avoid adding new collections and members to Biber; collections covering database entries with the same name under `nocite{*}` will not be renamed to avoid exposing previously covered books. Database primary keys covered by active collections are also not partially edited.

TeX quotation and set member completion is based on the actual input position and all consumer authentication candidates, preserving metadata search, full suffix substitution, Unicode, static naming and currently unclosed literals. Accepting an earlier citation candidate may lock the name as a book, with completion showing the book or collection metadata actually used after acceptance; repeated executions of the same physical citation also assume the first request identity. Only collections declared by source code can be used as TeX reference candidates. Collection members do not recommend other source code collections to avoid forming unsupported nested relationships. Candidates exclude the name when there is a binding conflict in different number partitions or shared consumers, and still retain public entries with consistent identities; unsaved provision files and new consumers refresh the results immediately, and report an incomplete list when the configuration limit is reached.

TeX citations and active collection members of certifiable source code collection projects support missing reference warnings and spelling fixes; all digital executions and saved and unsaved consumers must be consistent. Early source-only requests are missing even if a collection with the same name is subsequently defined, and existing database books retain book identity. Collections that have not been suppressed by earlier requests have members processed even if they have not been explicitly referenced; members that are suppressed by declarations do not falsely raise warnings. Fixed the problem of authenticating source code collections and database candidates according to the requested location after accepting edits, which is not affected by the upper limit on the number of completions. Reference warnings confirmed by the compiler continue to use the same filtering and avoid repeated reporting.

Certifiable source code collection projects support disjoint static IDS: the alias and its associated primary key must be unique and not intersect with any tracked source code collection name. After the complete literal IDS list has been verified by all loaded resources, collections, members, aliases and database relationships support precise navigation, independent renaming, completion and missing reference repair; alias previews are accompanied by metadata of their primary keys, and XDATA aliases continue to be filtered by their type. Unsaved provision files and new consumers participate in authentication and restore disk context after shutdown. Non-literal, duplicate, incomplete or conflicting IDS, aliases to which `@set` belongs, and cases where alias names intersect with collection names continue to be protected; `ids=macro name` will not be regarded as a safe namespace just because a static alias is not generated.

When the primary key of a unique static IDS is covered by the source code collection, both ordinary complete projects and explicit resource partitions support read-only definition jump and hover. Reference aliases still jump to the IDS statement, and the preview shows the ownership relationship to avoid displaying the original book information that has been overwritten by the collection; early alias requests will not lock the book identity like early primary key requests. Full reference search, highlighting, alias self-renaming, and missing key diagnostics share the complete identity graph of all consumers; reference key candidate preview affiliations; candidate fields also verify the actual loaded type to avoid indirect use of aliases as dynamic collection members. The primary key to which the source code belongs is renamed and the unique database primary key is moved at the same time, even if there is no relationship field directly referencing the primary key; all original database consumers and digital partitions must be bound to the current collection consistently. Early primary key requests or independent book consumers continue to reject such name changes. Use of an alias indirectly as a member of another dynamic collection, intersection of the alias name itself with the collection, and continued protection of the intersecting XDATA/`@set` owning key.

Complete normal projects also certify source code collections in order of execution, without using refsection or switching resources. Early requests lock database books or missing identities, and late requests bind declared collections; navigation, completion, reference maps, missing diagnostics and renaming use the same rules, and input files and unsaved consumers participate in authentication. Only the preface and no document body fragments retain the original editing behavior. Removing an earlier book key may activate a suppressed collection of the same name and add new members, so such preparations and renames will be rejected. Unresolvable named calls are also treated as potential source declarations and do not fall back on database unions with the same name; literal examples and unrelated static calls retain the existing behavior.

Dynamic or deferred execution, collection declarations with the same name in different locations in the same number partition, nested collections, resources with uncertified IDS, executable `@preamble` or entry `execute` fields, handwritten `bibitem`, unauthenticated data queries and not guessing the target when the analysis is incomplete; the mixing of explicit early references and wildcard loading is also not certified for the time being. The database declaration itself can still be viewed, and resource tags and literal examples remain processed independently. The database `@preamble` and entry `execute` fields can be injected into the collection via the generated BBL. A biblatex project containing these records will remove reference authentication, key candidates, and full key renaming from database models even without explicit reference partition or source collection declarations; static naming, pure forward package loading, and legacy `bibliography` resource declarations are also included in the authentication. Also suppress unproven missing documentation diagnoses. The database declaration itself can still be viewed, and legacy BibTeX, standalone database drafts, and unrelated projects retain their original behavior. The addition, removal and closing of unsaved databases are all re-certified.

Citation keys allow literal double quotes, such as `@misc{O"Connor}` and `\cite{O"Connor}`. Common citations, natbib aliases, biblatex multi-group/volume/data citations, classification assignments and handwritten documents share this rule, supporting full key navigation, completion, semantic highlighting, renaming and missing citation repair. When completing or renaming a double-quoted key into a double-quoted field, the entire value will be converted into curly braces, for example, `crossref="old"` is changed to `crossref={O"Connor}`; other members of the list, blanks, and comments outside the field will be retained, and multiple renames in the same list will be merged into one edit. Multi-line field completion uses the main edit and non-overlapping additional edits on the request line, preserving original line breaks. Keys containing macros, nested groupings, semantic gaps, or spliced by comments are still not inferred.

When BibTeX compilation reports invalid `crossref`, the error is localized to the literal field value of the child entry, and the link actually reads the parent entry in the database. Relevant locations will prompt that parent entries that are preceded and not explicitly referenced may be skipped by BibTeX. For details, see [crossref compilation diagnostics](bibliography-diagnostics.md#invalid-crossref).

When the compiler reports nested `crossref`, the warning also targets the field value and links to the parent entry in the reference chain. Self-references and loops maintain the compiler's warning level, and errors with missing parent entries are displayed separately; see [Nested Reference Diagnostics](bibliography-diagnostics.md#nested-crossref) for details.

When Biber reports an ordinary or XDATA inheritance cycle, diagnostics link the two reported entries and, where it can be confirmed, mark the specific `crossref` target. See [inheritance cycle diagnostics](bibliography-diagnostics.md#biber-circular-inheritance).

When Biber reports excess `crossref` in a collection, excess `entryset` in a member, or missing `entryset` in a collection, the diagnostics is located at the original field name or entry key respectively; see [Document Collection Diagnostics](bibliography-diagnostics.md#biber-bibliography-entry-sets) for details.

Renaming a shared library covers every root document that uses it; entries with the same name in other projects remain independent. Reject edits if there is definition ambiguity, name conflict, or shared reference meaning inconsistency. Supports versioned workspace editing; see [Symbols and Renaming](symbols.md) for specific rules.

## String abbreviations

`@string` declarations and field values, other string declarations, and abbreviated references in `@preamble` support completion, definition jump, hover, reference lookup, highlighting, and cross-file renaming. Names are compared according to BibTeX rules, and ASCII is not case-sensitive; citation keys continue to be case-sensitive, and the two namespaces do not affect each other.

```bibtex
@string{Journal = {Journal of Examples}}
@string{FullTitle = Journal # {, volume } # 2026}
@book{example,
  title = FullTitle,
  publisher = JOURNAL,
  year = 2026,
  month = jan
}
```

Selecting `Journal` in `publisher = JO` will replace the full abbreviation name, and also supports the null value position after `=` and `#`. Candidates are in plain text, preserving splice symbols, literal values, comments, and line breaks. Common `jan` to `dec` provide completion and month description; the actual expansion is determined by the document style.

Declarations appear as constant documentation symbols, and variable semantic highlighting is provided for both declarations and bare references. Hover preview of the original assignment expression and limit the length, no string expansion is performed. Numbers, text within curly braces and double quotes, comments, and the body of `@comment` are not referenced as abbreviations; both entry delimiters, UTF-16, CRLF, unfinished expressions, and unsaved edits use the same index.

Rename reuse project isolation and shared library check, support version editing and only change the name case. Modifications are rejected when there are multiple case equivalence declarations, when shared references point to different or missing definitions in other papers, and when the new name would capture an existing citation. Month names are reserved as common abbreviations and do not offer renaming. Edit Navigation does not parse macro definitions in `.bst` or infer cross-database execution order; unresolved multi-token expressions do not generate abbreviated references.

When an undefined abbreviation is found during compilation, a precise warning and up to 5 spelling suggestions are provided; the database reading order in the log is fixed, and only valid statements that have been in effect before are suggested. Editing is not provided when the input changes or the positioning is ambiguous. For details, see [Undefined String Abbreviation Diagnostics](bibliography-diagnostics.md#undefined-string-abbreviation).

verify:

```sh
cargo test --locked --test bibliography_strings --test semantic
cargo test --locked --test protocol bibliography_abbreviations_
cargo test --locked --test bibliography_strings real_bibtex -- --include-ignored
```

Real BibTeX regression compares the complete content of `.bbl` before and after renaming, covering cross-database splicing, numbers, case and preamble; a custom style is used to verify that self-referential renaming retains the original results.

## URL and DOI links

BibTeX's `url` and `doi` fields provide standard document links, and the editor can directly open the target; confirmed link values also provide string semantic highlighting. Supports case-insensitive field names, braced or double-quoted values, and both entry delimiters.

```bibtex
@article{example,
  title = {An example paper},
  url = {https://example.org/paper?q=one%20two#abstract},
  doi = {10.1234/example}
}
```

`url` supports absolute URIs for `http`, `https`, `mailto`, `ftp`, and `file`. `doi` supports naked identifiers, the optional `doi:` prefix, and HTTP(S) parsed URLs for `doi.org` / `dx.doi.org`. The naked DOI is converted to an HTTPS parsing link, which only checks the name structure and does not check the registration status online; the digital registration code can contain segments.

`#`, `?`, `%` and Unicode in bare DOIs are encoded by identifier content, e.g. `10.1234/a?b#c` opens `https://doi.org/10.1234/a%3Fb%23c` without interpreting the second half as a query or page fragment. DOIs containing separate `.` / `..` path segments will have slashes encoded to prevent browser normalization from changing the identifier. Explicitly parsing a URL preserves its URI encoding, query parameters, and fragments.

Link scope covers only the actual content of the value, preserving grouping, quotes, and surrounding whitespace; compatible with Chinese, emoji, and CRLF. Supports five literal escapes: `\_`, `\%`, `\#`, `\&`, and `\$`. Dynamic macros, packaging commands, string macros, `#` splicing expressions, nested grouping and internal whitespace do not generate links; titles, comments and sample text in `@comment` / `@string` / `@preamble` do not participate in the link index. Duplicate fields in the same entry do not generate links to that field, other fields and entries continue to work normally.

Only closed field values generate links; completed fields can still be opened when editing an entry that has not yet been closed. Neither standalone nor shared libraries require a root document to be selected and no compilation required. Unsaved incremental edits update links and highlights immediately; close buffers and use the current content after disk file changes.

Verify command:

```sh
cargo test --locked --test bibliography_links --test semantic --test navigation
cargo test --locked --test protocol bibliography_links_
```

XDATA candidate types and native inheritance return:

```sh
cargo test --locked --test bibliography_xdata_completion -- --include-ignored
cargo test --locked --test protocol xdata_relation_completions_
```

Standalone IDS regression in source code collection:

```sh
cargo test --locked --test biblatex_source_aliases -- --include-ignored
cargo test --locked --test protocol source_set_disjoint_aliases_
```

Execution identity return of ordinary complete projects:

```sh
cargo test --locked --test biblatex_source_projects -- --include-ignored
cargo test --locked --test protocol source_set_default_
```

The read-only identity of the primary key to which the source code covers the IDS returns:

```sh
cargo test --locked --test biblatex_source_alias_owners
cargo test --locked --test biblatex_source_alias_owners -- --include-ignored
cargo test --locked --test protocol source_overridden_ids_owners_
```

Full alias reference and rename regression for intersecting IDS:

```sh
cargo test --locked --test biblatex_source_alias_references -- --include-ignored
cargo test --locked --test protocol intersecting_ids_alias_edits_
```

Coupled editing and candidate authentication regression of the primary key of the source code IDS:

```sh
cargo test --locked --test biblatex_source_alias_owner_edits --test biblatex_source_alias_candidates -- --include-ignored
cargo test --locked --test protocol intersecting_ids_source_owner_
cargo test --locked --test protocol intersecting_ids_owner_candidates_
```

Biber global IDS cache regression across digital partitions:

```sh
cargo test --locked --test biblatex_alias_loading
cargo test --locked --test biblatex_alias_loading -- --include-ignored
cargo test --locked --test protocol global_biber_alias_loading_
```

Native tests verify both cached imports and new alias capture as well as counterexamples of new primary keys deleting old alias mappings; requires pdfLaTeX, biblatex, and Biber to be installed.


Digital partition first load and IDS entry cache regression:

```sh
cargo test --locked --test biblatex_alias_owner_cache -- --include-ignored
cargo test --locked --test protocol cached_alias_owner_proofs_
```

Return of digital partition read-only identity for the IDS of the same name:

```sh
cargo test --locked --test biblatex_numeric_alias_owners -- --include-ignored
cargo test --locked --test protocol numeric_ids_source_owners_
```

The native test compares the default library book and the selected library source code collection, and verifies the BBL after accepting the candidate; the primary key editor continues to be protected; the independent alias editor in the next stage also has native controls.

Cross-partition alias self-safe editing regression:

```sh
cargo test --locked --test biblatex_numeric_alias_edits -- --include-ignored
cargo test --locked --test protocol numeric_ids_alias_edits_
```

The native test compares the complete BBL of the two partitions before and after the name change, and the requirements are consistent after normalizing the changed aliases; the protocol verifies the two editing formats, versions of all affected files, unsaved cache import and shutdown recovery.

IDS global entry cache regression after primary key rename:

```sh
cargo test --locked --test biblatex_primary_cache_rename -- --include-ignored
cargo test --locked --test protocol primary_cache_capture_renames_
```

Native testing verifies that both forced error renames will change subsequent entry data without Biber warnings, and also verifies that accepted safe renames maintain BBL; the protocol covers versioned edits, unsaved databases, shutdown recovery, and coupled source primary keys.

Partition interaction regression for non-loading citation keys:

```sh
cargo test --locked --test biblatex_nonloading_section_keys -- --include-ignored
cargo test --locked --test protocol numeric_nonloading_keys_
```

Covers full reference and versioned editing for normal and static named calls, undefined name capture, overridden cache ownership, and conditional query protection. The native verification non-loading call does not pre-read resources and compares the complete BBL after the application is renamed.

The read-only identity with the same IDS name as the source code collection returns:

```sh
cargo test --locked --test biblatex_source_named_aliases -- --include-ignored
cargo test --locked --test protocol source_named_ids_reads_
```

Native override of the default/selected partition's early and late requests, the existence of two collections with their own primary key and alias name, another alias and independent collection members. The protocol verifies precise references, rejects intersection renames, unsaved static/string/splicing relationship revocation authentication and shutdown recovery. Native counterexample confirmation relation macro expansion also suppresses collections without warning.

Coupled rename regression of the primary key to which the shared IDS belongs:

```sh
cargo test --locked --test biblatex_shared_alias_owner_rename -- --include-ignored
cargo test --locked --test protocol coupled_raw_ids_owners_
```

The test verifies that the complete BBL of the two partitions is consistent after name change and normalization, complete editing of the three libraries when multiple aliases are associated, independent project isolation and unsaved consumers. The protocol accurately verifies two workspace edit formats, six edits, three file versions, and close recovery; common citation results still only contain their respective claimed identities.

Manual bibliographies and biblatex backend namespace regression:

```sh
cargo test --locked --test biblatex_manual_namespace -- --include-ignored
cargo test --locked --test protocol handwritten_items_keep_biblatex_
```

Covers normal projects, numeric partitions, static named entries, complete database renaming, candidate and missing diagnostics, unsaved input and shutdown recovery. The native comparison verifies that the complete BBL is consistent before and after the name change, and the `bibcite` of the handwritten entry remains unchanged; the protocol accurately verifies that the two editing formats, versions, UTF-16 positions, and traditional auxiliary records will not be mistakenly used for background reference prompts.

Plain text predicate query support in source code collections `ifentryinbib`/`ifentrycategory` (with static named calls): resolves identity by final real request and claim per numeric partition, preserving navigation, full referencing and renaming, but does not pre-read the database or send Biber requests. The collection declaration itself enables the collection to be loaded, so no explicit reference is required. Querying for missing keys does not generate reference warnings; truly missing references and set members are still checked.

All execution contexts of a shared query must be bound to the same identity; redefinition, executable/dynamic branching, IDS cross semantics, and unauthenticated internal command execution contexts continue to be protected. Predicate key completion authenticates candidates against the final partition namespace, ordinary references and collection members are still calculated against the actual requested position.

```sh
cargo test --locked --test biblatex_source_textual_queries -- --include-ignored
cargo test --locked --test protocol source_textual_predicates_
```

Direct and non-redefined `mkbibparens`, `mkbibbrackets`, `parentext` and `brackettext` initialize biblatex internal commands, also supporting calling these wrappers with static `\csuse{name}`/`\csname name\endcsname`. Within the complete grouping parameters of these wrappers, `entrydata`/`entrydata*` of plain text code bodies support navigation, hover, completion, referencing and full renaming of source code collections, including statically named `entrydata` calls. The key is authenticated according to the final partition identity, and the query does not load the database book in advance; early real references still retain the book identity. The package itself also provides the `{text}` signature, and internal common tag references can be interacted with.

The global zero-argument pure forwarding already implemented in the preamble also supports these queries, such as `\wrap{\entrydata{key}{Text}}` after `\def\wrap{\mkbibparens}`, as well as static named calls, multi-level forwarding, and loading statements from the preamble input file. Each claim in the forwarding chain must have been executed when called, and the target must maintain its native meaning; claims such as `newcommand` and `NewDocumentCommand` signed with empty parameters continue to use the same authentication. Unsaved statement changes and recovery after closing are immediately recalculated.

Direct calls from ordinary text, code outside of packages, copied packages, unexecuted or partial forward declarations, packages declared after entering the text, dynamic names, redefinition of named interfaces or packages, incomplete parameters and executable code bodies continue to be protected. When the underlying `mkbibparens`/`mkbibbrackets` forwarded by `parentext`/`brackettext` is redefined, the native parameter execution and initialization authentication are also removed, and another independent packaging is retained. Database `execute`/`@preamble`, shared consumer identity and new name capture inherit full authentication.

The wrapper also relies on native bracket output commands. Project redefinition corresponding to `bibopenparen`/`bibcloseparen`, `bibopenbracket`/`bibclosebracket`, or `bibleftparen`, `bibrightparen`, `bibleftbracket`, `bibrightbracket` , the native text execution and initialization authentication of the relevant packaging and forwarding calls will be removed. These output commands may consume the first command in the text as a parameter, leaving display text with the same name; navigation, completion, and renaming will protect these text. Nested packages use both brackets interchangeably, so changing the output of one may also affect the authentication of another package. Restore the original interaction after removing the redefinition.

`mkbibfootnote{text}` and `mkbibfootnotetext{text}` provide command snippets, signature help and text label interaction. In default `notetype=foot+end` and explicit `footonly`, native footnotes will initialize the bibliographic command on the fly; plain text `entrydata`/`entrydata*` within full parameters supports the same final set query, completion and full rename, including static naming calls and pure forwarding performed in the preface. They do not read optional footnote numbers.

Footnote initialization also relies on footnote primitives, `protecting`, `blxmkbibnote`, `bibfootnotewrapper`, `bibsentence` and native underlying typesetting commands. Relevant redefinitions may discard the query or simply swallow the command, leaving the display text behind; no citation key editing can be provided accordingly. `ExecuteBibliographyOptions[entry types]{options}` provides both native signature help and command snippets. Schema authentication checks the load options in the full source closure, `PassOptionsToPackage`/`PassOptionsToClass` and `ExecuteBibliographyOptions`, and only accepts literal options that are sure to preserve live footnotes and key names without internal whitespace or grouping. `endonly`, dynamic options, direct modification of internal mode counters or unconfirmed dependency continuation protection do not simulate the actual expansion position of the endnote. Restore interaction after removing unsaved redefinitions or schema modifications.

Footnote code preserves latency bounds in otherwise immediate execution analyses; literature query tracking enables immediate parameters only after independent certification. Potential string judgment redefinition in the code is continuously protected by hooks to prevent endnotes from returning incomplete macro renames after subsequent expansion. The executable document code within the text is still authenticated according to the original boundaries, and ordinary text and static structure references continue to be retained.

```sh
cargo test --locked --test biblatex_source_entrydata -- --include-ignored
cargo test --locked --test protocol initialized_entrydata_
```

The native test actually applies the rename and completion edits returned by the LSP, compares the complete BBL of the four packaged normal/asterisk calls when querying before the collection declaration, and confirms that no additional original database books are loaded. The protocol verifies both edit formats, UTF-16/CRLF, ranges and versions of master files and shared queries, unsaved redefinition and execution code, shared new root conflicts, and shutdown recovery.

When the project contains independent source code collections, the primary keys of real databases that belong to different digital partitions and share IDS can be coupled and renamed: conflicts are checked based on actual resource allocation, and all belonging entries and their references connected by aliases are completely collected. Collection members, wildcard loading, and cross-database relationships continue to participate in the check; source code collections that obscure the primary key, intersecting aliases, different primary keys, unauthenticated consumers, or new name capture still refuse to be edited. Authentication of multiple cross-repository prerequisites for the same physical source code set is described below.

```sh
cargo test --locked --test biblatex_source_shared_owners -- --include-ignored
cargo test --locked --test protocol source_shared_raw_owners_
```

When the same physical source code collection declaration is executed in multiple partitions, it is supported to move the real prerequisite primary keys of each partition simultaneously, and the entries can be partially or completely without IDS. All consumers of each original entry must assume this same collective identity, and all associated IDS entries must be included in the edit; referencing the IDS first will not falsify earlier primary key requests. Read-only references retain source code identity and are renamed to additionally include the required database headers. Maximum of 64 prerequisite entries; the database reference only authenticates the original `crossref/xref` parent entry relationship, and the actual edit atomically moves all required headers and their relationship keys, preserving the IDS name. Independent book consumers, missing primary keys, different physical collections, early primary key requests, wildcard masking, unauthenticated cache returns, cross-repository cache new names, and full-item name capture continue to protect. Individual prerequisites carry over from the original certification.

On subsequent partitions back to the parsed library, the final source collection can replace the original prerequisite entries imported from the global cache. Display identities are authenticated against the actual collection claims for each partition; referencing the IDS first does not by itself prevent the final collection, but requesting the real primary key directly first still preserves the original book. Wildcard loads, different physical claims in the same partition, aliases that have been removed by real keys, and ambiguous providers continue to protect. A complete name change still requires that all original consumers adopt the same physical collection identity; the ordinary database graph and the new name cache simulation continue to use the original ownership check, and the analysis is still subject to the 32 MiB work budget.

```sh
cargo test --locked --test biblatex_source_prerequisites -- --include-ignored
cargo test --locked --test protocol source_prerequisite_edits_
```

Compilation tags for local numeric reference contexts support both literal `labelprefix`/`sorting` and global deferred numbers. The default context logged by AUX is bound to the BBL real prefix; across printing contexts for the same key still follows the biblatex real default selection. All executions of a shared physical reference must receive the same complete label; unauthenticated context assignments, naming context declarations, redefinitions, dynamic options, and custom formats continue to be protected. For details, see [Auxiliary Reference and Numbering Tips](auxiliary-references.md).

`ifentryinbib`/`ifentrycategory` key completion in the source code collection project supports direct and static named calls: the candidate takes the final first real request for the partition or the collection declaration, and the query itself does not become an early load request. When a real request has not yet occurred, the set of subsequent statements can be completed; when there is an early `cite`, the metadata of the original book is still displayed. Candidates for ordinary `cite` continue to use cursor request semantics.

Supports empty keys, complete suffix replacement, existing comments, and UTF-16/CRLF editing. Requires complete, plain-text branch and static classification names; unfinished branch, executable code, redefinition, dynamic keys, internal document execution context, and IDS intersection continuation protection. All roots and digital partitions sharing a physical query must obtain the same physical candidate identity, and single candidates that cannot be confirmed are excluded; insufficient budget does not return uncertified candidates.

```sh
cargo test --locked --test biblatex_source_predicate_completion -- --include-ignored
cargo test --locked --test protocol source_predicate_completions_
```

`\assignrefcontextentries[options]{keys}` and its asterisk form support jumps, previews, completions, quotes, highlighting and complete renaming of entry key lists, and also recognize static `csuse`/`csname` calls and authenticated pure forwarding commands. Square brackets are required parameters, even if the option is empty, it must be written as `[]{keys}`; the signature prompt distinguishes the option from the key list. Changing the name only modifies the actual entry key, and does not change options such as `labelprefix`, ordinary text, and verbatim content.

This command allocates context without loading or referencing entries, so missing keys and wildcards `*` do not trigger missing reference diagnostics and do not generate reference number prompts. Numeric partitioning of ordinary resources reuses non-loadable key authentication; context assignments in source collections, captured command copies, and ambiguous operand calls continue to reject partial renames. Direct, static naming of ordinary resources and executed pure forwarding strong/weak allocation support display of deferred numeric labels in order of execution. For options and compilation record boundaries, see [Auxiliary Reference and Numbering Prompt](auxiliary-references.md); weak allocation restores the context switch according to native grouping, and classification allocation also prompts according to the membership authentication number of the current source code and AUX; keyword allocation also prompts according to the member authentication number of the actual loaded BBL. For details on the independent namespace and complete editing of the classification list, see [Bibliography categories](bibliography-categories.md).

`\assignrefcontextkeyws[options]{keywords}` provides signature help and command completion fragments in the form of required square brackets, keyword lists, and asterisks, and also recognizes static named calls and authenticated pure forwarding. Keywords, entry keys, and category names are processed separately, and key navigation or renaming across namespaces is not provided. See [Auxiliary References and Numbering Hints](auxiliary-references.md) for the members, blanks, and coverage sequence of number hints.
