# Bibliography build diagnostics

[Documentation](README.md) · [Build diagnostics](diagnostics.md#bibtex-and-biber) · [Documentation Resources](biblatex-resources.md)

Processing can be done using the default latexmk or [explicit documentation backend](bibliography-processing.md). The server reads the `.blg` file newly added or updated in this compilation, analyzes BibTeX's duplicate entries, syntax errors and field warnings, as well as Biber's errors/warnings; it locates the `.bib` file when it can identify database and citation keys. A failed to locate message is displayed in the main document. The old `.blg` will not be treated as an error in this compilation. The `.bib` and `.bst` actually read in the log will also be added to the project dependency, supporting the macro-expanded document path, citation key renaming, root file identification, unsaved check and expired PDF check; styles in the system directory will not be indexed in batches. The bibliography record is retained independently of `.fls` until a new bibliography log updates it, to avoid losing bibliographic sources when running only TeX.

## Database roles and special suffixes

Biber's complete `INFO - Found BibTeX data source 'filename'` record confirms that the file is read as a BibTeX database. The database can use other extensions such as `.data` or no extension; even if the name ends with `.tex`, `.png` or `.bst`, it will still be indexed by the document data, and the text in it that looks like a LaTeX macro will not be imported. Warnings, example text, incomplete logging, or truncated logs do not create new database dependencies.

These sources participate in document navigation, versioned citation key renaming, file movement, unsaved checks, and compilation result validity checks. The database role is retained with the file move and updated with the new complete document log; the database and style still follow the search order of `BIBINPUTS` / `BSTINPUTS` respectively, and the symbolic links use the actual target.

The full log name of the database is also used for quoted paths, bare paths, and entry field positioning in warnings, including Biber's `Duplicate entry key` and `Datamodel: Entry` messages. Unidentified explicit files remain in the main document and do not borrow entries of the same name from other databases.

## Logging boundaries

BibTeX's database, style, and auxiliary file records are processed as full file names; text such as `ERROR -`, `WARN -`, `---line … of file …`, etc. in the path will not generate additional errors. The ` : ` source code echo after the syntax error is also retained as context, and the log style text inside will not become a new diagnostics.

Biber's level is read from the current record header and supports the common `WARN - …`, `[0] WARN - …` and `[0] Utils.pm:395> WARN - …` formats. `ERROR -` in the warning text does not change the level, and BibTeX line number styles in the text do not change the diagnostic source. Examples of errors in `INFO` / `DEBUG` messages do not generate errors; the database role only accepts the complete `Found BibTeX data source '…'` message from the actual `INFO` record.

## Missing style files

BibTeX reports a diagnostic code of `missing-bibliography-style` when it cannot open a style file. When the final TeX log confirmed that there was only one corresponding static `bibliographystyle` declaration in the source read, the error was localized to the exact style name, and spelling suggestions for up to 5 local `.bst` in the same directory were provided; comma names, explicit suffixes, and surrounding comments were preserved. Errors in dynamic or spliced names, declaration ambiguities, incomplete sources, and scan overruns are retained in the main document and no guesswork fixes are provided. It is no longer repaired when the style has been restored or the diagnostics has expired.

## Missing library

When BibTeX cannot open the database, the diagnostic code is `missing-bibliography-database`. When there is a unique static `bibliography` / `nobibliography` declaration in the actual read project source code, the missing items in the list are accurately marked and up to 5 local `.bib` spelling suggestions in the same directory are provided; other databases, comments, directories and explicit suffixes are retained. Matches whitespace in names according to BibTeX rules, candidates do not introduce whitespace or commas. The system package implementation does not serve as a user statement, and local packages, document classes, and indexed external sources still participate in verification; main document errors are retained when dynamic names, redefinitions, ambiguities, incomplete sources, or log truncation occur, and no guesswork fixes are provided. Fix for compatibility with versioned editors and older clients, and review whether versions and files have been restored before applying.

Biber's full error `Cannot find 'reffs.bib'!` also uses `missing-bibliography-database`, with support for locating the only active `addbibresource` / `addglobalbib` filename. Missing library errors in the BibTeX backend also support these two commands. Missing control files, warnings, sample text, and incomplete messages are not treated as such missing errors.

Resource parameters are matched by full literal filename, preserving options, spaces, apostrophes, commas, `%`, Unicode, and explicit extensions. Repair only replaces file parameters and preserves the full extension; for example, `{reffs.bib}` can be repaired to `{refs.bib}`, and `{refs}` can be explicitly repaired to `{refs.bib}`. `.bib` is still not automatically added when searching for dependencies. For details, see [Document Resource Loading Rules](biblatex-resources.md).

Up to 5 candidates in the same directory are provided. When the suffix is explicitly specified, only files with the same suffix are compared. Modification locations are not guessed when remote, wildcard, dynamic resources, redefinition, duplicate declaration, unconfirmed source, or log truncation; legacy fixes are not provided when the file has been made up or the meaning of the current call has changed.

## Indirect call and repair verification

LSP compilation results also locate missing libraries and styles in static named calls and pure aliases, and provide the same filename spelling fixes. The positioning is based on the source of the project actually read during this compilation and verified by the input snapshot; no positioning basis is provided for unexecuted commands, uncalled macro bodies and unloaded alias definitions in macro parameters. The repair also reviews the recorded macro scope and current project calls, retaining indirect call forms, comments, and database lists; editing is not provided when the meaning has changed or the source is incomplete.

## latexmk missing library warning

The server also generates a `missing-bibliography-database` warning by default when latexmk confirms that the database is missing and skips BibTeX, providing the same pinpointing and fixing. Compilation continues to respect latexmk's policies and existing input consistency checks, and projects with pre-generated `.bbl` can still generate PDFs. The warning only adopts the complete driver list after the last target log check, and existing native BibTeX errors take precedence; no-operation compilation that does not output a new list will not add a missing library warning. Other executable programs, different job logs, incomplete or truncated output do not enter this analysis.

## Biber extended name components

Full `Invalid namepart 'part' found in extended name format name 'field' in entry 'key', ignoring` warning for Biber Use `invalid-bibliography-namepart`. For example `bogus` in `author={family=Doe, given=Jane, bogus=Extra}` can be reported by the tool as an unknown component; the valid name is determined by the actual Biber data model and the server does not statically negate custom name parts.

The diagnostics retains the native `namepart`, belonging key and model field name, and locates the complete original field value. Literal values use the inner scope, null values retain the delimiter, and string macros and concatenation expressions use the entire rvalue. Biber may have expanded or transformed the name, so it does not guess the position of an original word from the log, nor does it automatically rewrite the name or data model.

Field matching uses Biber's full Unicode folding, e.g. `TRANſLATOR` corresponds to `translator`. Fallback to the unique entry key when fields are missing, duplicated, model names collide, or entries are incomplete; locate the database only when all previous inputs are readable and the keys are unique after decomposition according to the specification. Diagnostics of source history invalidation, key ambiguity and retention of root document when cache exceeds limit. After saving, recompile and update the location, and clear the push/pull diagnostics after repairing.

```sh
cargo test --locked --test biber_namepart_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_nameparts_
```

## Biber Bibliography entry sets

Biber's native collection warnings use `invalid-bibliography-set`: `crossref` is no longer required in the collection entry, `entryset` is no longer required in the collection member, and `entryset` is missing from the collection. The original field name of the first two types of marks, `reason` in the data, is `obsolete-field`; the entry key of the latter type of mark, `reason`, is `missing-entryset`. Diagnostics retains the levels and messages reported by the tool, and does not statically determine whether the collection is legal when there is no native report.

Field names are matched according to Biber's full Unicode fold, supporting original spellings and dynamic value expressions such as `CROSSREF`, `croſſref`. Duplicate fields, model name collisions, incomplete assignments, or source map generated fields fall back to unique entry keys and are flagged `unresolvedField`. Reports missing `entryset` always use the entry key because the source map may have removed the original fields.

This type of warning does not have a database name. The database can only be located if the previous input history is complete, all sources are readable, the specifications are equivalent, and the case-sensitive keys are unique. Retain root document diagnostics when the source is unknown, the key is ambiguous, or the cache exceeds the limit. Recompile updates position after saving, clears push/pull diagnostics after fix; does not provide guesswork for field deletion or insertion.

```sh
cargo test --locked --test biber_set_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_sets_
```

## Biber field verification

Biber native `Datamodel: Entry 'key' (refs.bib): Field 'field' invalid in data model - ignoring` and `Invalid field 'field' for entrytype 'book'` use diagnostic code `invalid-bibliography-field` to mark unique field names in unambiguous entries. Whether fields are supported is determined by the actual Biber data model. The server only adopts tool reports and does not statically speculate whether custom fields are valid.

`invalid-bibliography-field-value` marks invalid values and supports format warnings, data types, upper and lower bounds, pattern matching for dates and the end of date ranges, and ISBN, ISSN, and ISMN verification messages. A complete single literal value marks the internal content, and an empty literal value covers the delimiter; string abbreviation, concatenation, and other complete expressions mark the entire value expression, and macro expansion is not performed. Both messages and levels maintain native results and do not automatically delete fields or overwrite values.

Messages with filenames only take explicitly named databases, supporting other extensions for confirmed database roles, full special filenames, and symbolic links. Fields come only from unique valid assignments in the complete body of the entry; duplicate fields, unclosed bodies, invalid expressions, and missing fields (including fields provided only by inheritance) retain the location of unique entry keys and do not borrow from parent entries or other databases. The first line of the file is retained when the entry key is missing or ambiguous; the main document is retained when the source is unreadable, unconfirmed, or exceeds the limit. Preserve main document warnings if unescaped filenames and values in date messages are indistinguishable.

By default, when Biber does not enable data model verification, the complete `legacy year field 'value' in entry 'key' is not an integer - this will probably not sort properly.`, `month` warning, `ISBN 'value' in entry 'key' is invalid - run biber with '--validate_datamodel' for details.`, and `Range field 'pages' in entry 'key' is malformed, falling back to literal` of the same template also use value diagnostic codes. These messages have no file names and only locate the corresponding field value if the previous database read history is complete, all candidate files are readable, and the normalized key is unique among all candidates; symbolic links are deduplicated according to the actual target and do not borrow subsequent input. Range fields are positioned according to the input spelling, and support default page numbers and custom model range fields; the message does not indicate a specific fragment, so the complete value is marked. Preserve main document warnings for missing history, syntax or unknown errors, log truncation, normalization key collisions, or read limits. Quotes, filenames, and log-like text in raw values are not otherwise interpreted as locations, and dates, ISBNs, or page ranges are not automatically modified.

Matches distinguish namespaces according to Biber's Unicode rules: keys are case-preserved and decomposed normally; unknown field, date, and malformed range warnings preserve ASCII folding after combination of input fields; entry type restrictions, data type and other data model checks, default year, month, and ISBN warnings use full Unicode folding. For example, the custom model `strasse` can correspond to the original `Straße`, and the default model `isbn` can correspond to `IſBN`. `STRASSE` and `Straße` in the same entry constitute ambiguity for model warnings, falling back to the key even if there are fields with exactly the same spelling; unknown field messages can still locate the two original fields respectively, and malformed range warnings can also locate the values of `pages` and `pageſ` respectively. The scope always points to the original source, preserving UTF-16 and CRLF; both field indexes share a conversion per original position, and the cache is built by bounded source. Recompiling after saving will update the scope, and both push and pull diagnostics will be cleared after repair.

```sh
cargo test --locked --test biber_field_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_field_warnings_
cargo test --locked --test protocol biber_unqualified_field_warnings_
```

## Biber XDATA Quote

Use `invalid-bibliography-xdata` when Biber reports that the `xdata` target is missing or the target is not an XDATA entry. Supports full `I didn't find a database entry for xdata entry … - ignoring (section …)`, `Entry … references XDATA entry … which does not exist, not resolving (section …)` and `which is not an XDATA entry` templates, retaining native messages, levels and section numbers. Missing targets may produce two different native messages, both retained.

Previously when the database history was complete and the entry key was unique across all readable input, the diagnostic marked the entry `xdata` as the only target item in the literal list, preserving other list items, whitespace, Unicode, and CRLF. Entry keys and targets are matched by case-sensitive NFD. Fallback to unique entry keys for fields generated by macros, splicing, inheritance, source map, duplicate fields, duplicate targets, or unclosed entries; retain main document diagnostics when sources are missing, unreadable, out of bounds, history errors, or canonical equivalent key collisions. The section number is used to preserve tool context, and source selection still conservatively checks all previous databases. No automatic guessing of missing entries, no changing of target types, or expansion of macros.

```sh
cargo test --locked --test biber_xdata_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_xdata_warnings_
```

Field-level XDATA messages also use `invalid-bibliography-xdata`, covering references that do not specify a source field, the source field does not exist, the field types on both sides are incompatible, and the source location of a list of names or a normal list does not exist. When a message explicitly gives a reference to a field, the diagnostic marks the complete value of its unique original assignment; string macros, concatenations, and mixlists retain the entire value expression and do not guess at an internal fragment. When the original field is missing, duplicated or incomplete, it falls back to the corresponding key. The source and history restrictions follow the above rules.

Such native messages preserve input field spelling, combined ASCII lowercase and NFD matching, `Straße` and `STRASSE` respectively; do not use full Unicode folding of the data model. Diagnostics retain `parent`, `sourceField` and textual `sourcePosition` given explicitly by the message. Messages that do not specify a source field do not infer the parent entry. XDATA tags and delimiters can be configured so that the server does not parse or rewrite references from the original value itself.

```sh
cargo test --locked --test biber_xdata_field_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_granular_xdata_warnings_
```

## Biber relationship reference

Biber's complete `I didn't find a database entry for … 'parent' in entry 'child' - ignoring (section …)` message uses `missing-bibliography-relation` and supports four native relationships: `crossref`, `xref`, `related entry` and `static set member`. They correspond to the original `crossref`, `xref`, `related` and `entryset` fields respectively, retaining the native message, level and section number; static collection member warnings are reported by Biber when it actually processes the collection entries.

Positioning shares complete previous input history with XDATA and canonical key uniqueness checks, single relationships mark unique literal targets, and list relationships mark exact list items. The same target is indexed separately in different relational fields; duplicate fields or canonically equivalent targets within the same field remain ambiguous. Macros, splicing, inheritance, source map and incomplete fields fall back to their own keys, and unconfirmed sources fall back to the main document. No guesswork renaming, inserting missing entries, or expanding dynamic values. After saving, recompile the update range, and the push and pull diagnostics after completing the target will be cleared.

```sh
cargo test --locked --test biber_relation_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_relation_warnings_
```

## Biber circular inheritance

Biber's full `Circular inheritance between 'parent'<->'child'` and `Circular XDATA inheritance between 'parent'<->'child'` use `cyclic-bibliography-inheritance`, retaining the native error level and two entry keys. Only the complete database read history before the current error is used, all candidates must be readable and the reported subkeys must be unique after a case-sensitive NFD match.

Ordinary circular inheritance can target the parent key in the child entry's unique static `crossref`; fallback to the child entry key when string macros, splicing, repeated fields, source map generated fields, or referencing the parent entry through the `ids` alias. XDATA's entry-level and field-level loops use the same native template, both mark sub-entry keys, and do not guess specific fields. Reported parent entry via `relatedInformation` link; retains main document error on child key ambiguity and lists up to 16 candidate locations. Self-looping does not list the same range as the main diagnostic repeatedly.

Circular inheritance is a fatal error. The current message uses the input record before the error is reported, and subsequent diagnostics is still processed according to the failure history, and subsequent input will not restore this state. If there have been previously unknown errors, missing or exceeded sources, unreadable files, or truncated logs, the current loop will also retain the main document errors. Do not automatically delete fields or break reference chains; recompile and refresh the position after saving, and clear push and pull diagnostics after repair.

```sh
cargo test --locked --test biber_cycle_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_inheritance_cycles_
```

## Biber name format

Biber's full name message uses `malformed-bibliography-name`, covering the following native situations:

- `Name "name" has too many commas, skipping entry 'key'`: Too many commas.
- `Name "name" is malformed (consecutive commas): skipping entry 'key'`: Consecutive commas.
- `Name in key 'key' is empty (probably consecutive 'and'): skipping entry 'key'`: Empty name.

Diagnostics retains the actual level, message, citation key and `reason`. The first two categories also save the `name` reported by Biber. Comma errors still fail the build; empty names show up as native warnings. Names may be LaTeX decoded or may come from a custom model's name field, so only unique entry keys are marked, no guessing of the original name range or automatic fixes for commas, `and` are provided.

Messages do not have file names and require that the previous input history is complete, all candidates are readable, and the keys are unique after case-sensitive canonical decomposition. Subsequent databases do not participate in positioning; the main document is retained in case of key collision, missing source, log truncation or read limit. Positional multiplexing bounded key cache, supports Unicode, CRLF and special file names.

These three complete messages mean that only one entry is skipped, and Biber will still process subsequent entries, so the existing input history is retained and subsequent name errors and field warnings can continue to be accurately located. Syntax errors, unknown errors, and incomplete name error templates continue to invalidate history; this rule also applies to duplicate keys, case correlation, and default field diagnostics. After saving, recompile the update range, and clear the push and pull diagnostics after repairing.

```sh
cargo test --locked --test biber_name_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_name_errors_
```

## Biber Date Override

The complete `Overwriting field 'year' with year value from field 'date' for entry 'key'` and corresponding `month` warnings use the standalone code `overwritten-bibliography-field`, marking the original year or month value being overwritten. Diagnostics retain tool messages, levels, and `overwrittenBy: "date"` data; the value itself can be valid, just different from the date components actually used by Biber, and does not automatically decide which field should be deleted or modified.

When the entry has a unique, complete original `date` assignment, the standard `relatedInformation` is associated with its value, making it easier to jump from the overwritten value to view the date. Range dates associate the entire value, and string and concatenated expressions maintain the original expression range. Fallback to a unique entry key when the original year or month is missing, duplicated, or incomplete; `date` does not guess the associated position when it is missing, ambiguous, or generated from a source map, and does not borrow fields from other entries.

Such messages have no file names, reuse the complete history of previous Biber inputs, case-sensitive normalized key uniqueness, and bounded origin caching. Unconfirmed source, ambiguous keys, log truncation, previous syntax or unknown errors, retain root document diagnostics on read failure. After saving, recompile and update the main location and associated locations; after adjusting the date or making the components consistent, the push and pull diagnoses are cleared together.

```sh
cargo test --locked --test biber_overwritten_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_overwritten_fields_
```

## Biber entry type

The complete `Datamodel: Entry 'key' (refs.bib): Invalid entry type 'boook' - defaulting to 'misc'` message for Biber data model validation uses the diagnostic code `invalid-bibliography-entry-type`, marking the original type name in the unique entry. Valid types are determined by the actual tool's data model; diagnostics retain native message, level, and `entryType` data and do not statically negate custom types or automatically change source code to `misc`.

Type names are matched according to Biber's full Unicode case folding and standard decomposition, for example, `Straße` / `STRAẞE` corresponds to `strasse`, and Greek suffix sigma, dotted I and ligatures are also processed according to native results. The reference key is still only case-sensitive and canonical decomposition, and the field name follows its own matching rules; the position always covers the original source code, retaining the actual number of characters and the UTF-16 range.

Locate only within explicitly named databases. Mark the type name only when the complete entry, unique normalized key, and matching original type can all be confirmed; mark the unique entry key and append `unresolvedType` when the text is not closed, or when the source map and other processing make the report type different from the source code. When the key is missing or ambiguous, the first line of the source is retained; when the source is unconfirmed, unreadable, or exceeds the limit, the main document is retained and no other libraries are borrowed. Types and field positions share a bounded source cache. Recompiling after saving will update the range. After repair, push and pull diagnostics will be cleared.

```sh
cargo test --locked --test biber_type_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_type_warnings_
```

## Biber required fields

The complete `Missing mandatory field 'author'` and `Missing mandatory field - one of 'date, year' must be defined` messages verified by the Biber data model use diagnostic code `missing-bibliography-field`. Only the requirements of the actual tool report are adopted, and there is no static assumption about which fields the entry must have; the field group indicates that at least one item must be filled in, and it will not be split into multiple all-required errors. The single field in the diagnostic data is saved in `field`, and the alternative group is saved in `fields`.

Warnings target unique entry keys in an explicitly named database, breaking up matches according to Biber's Unicode specification and preserving the original UTF-16 range. When the key is missing or there are multiple declarations after normalization, the first line of the file is used, and no other libraries are borrowed; the main document is retained when the source is missing, unconfirmed, unreadable, or exceeds limits. Complete special file names, databases with other suffixes, and symbolic links follow existing source rules.

Messages and levels remain native results, with no guessing of field values, no selection of alternative fields for the user, and no automatic field insertion. Recompiling after saving will update the entry key position, and clear push and pull diagnostics after completing the requirements; multiple warnings in the same log reuse the bounded key position cache.

```sh
cargo test --locked --test biber_required_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_required_fields_
```

## Biber document alias conflict

Biber's three complete classes of `ids` warnings use `conflicting-bibliography-alias`: `Entry alias … already has an alias …, skipping ...`, `Citekey alias … is also a real entry key, skipping ...` (the `Entry alias` prefix is also accepted), and `BAD RECURSION! Entry alias … is identical to the entry key, skipping ...`. They respectively indicate that the alias is occupied by other entries, the alias conflicts with the real primary key, and the alias is equal to its own primary key; retain the native message and level and do not generate warnings by itself when there is no compiler report.

When the previous input history is complete and all sources are readable, the diagnostic declares the index using the static `ids`, matching the alias to the associated primary key by case-sensitive NFD. When the skipped declaration and associated primary key can be uniquely identified, the alias is accurately marked and the existing declaration is linked. Multiple candidates, primary key collisions, dynamic or repeated fields, unclosed text, etc. retain main document warnings and list up to 16 confirmable related positions; skipped entries will not be inferred based on the order of duplicate messages. Positions are not borrowed due to unknown errors, truncation, reads, or after the number of sources exceeds the limit. Only conflicts are reported, and entries that should be deleted or renamed are not guessed; recompile the update range after saving, and the repaired push and pull diagnostics will be cleared.

```sh
cargo test --locked --test biber_alias_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_alias_conflicts_
```

## Biber duplicate citation keys

The complete `Duplicate entry key: 'alpha' in file 'refs.bib', skipping ...` message uses diagnostic code `duplicate-bibliography-key`; the native format without `, skipping ...` is also supported when `--noskipduplicates` is turned on. Primary keys are case-sensitive matching only in explicitly named databases, and unique declarations use the exact key range; when there are multiple declarations with the same name in the same file, the first line range of the file is retained, candidates are listed through `relatedInformation`, and specific declarations are not guessed based on no line number messages.

The matching complies with Biber's Unicode specification decomposition. The combined form `café` and the decomposed form `café` belong to the same key; when both forms exist in the same library, the ambiguity is retained and the original declaration range is listed, and the decomposed form in the report is not mistaken as the only declaration. Uppercase and lowercase and compatible characters are still matched by their respective keys, and compatible decomposition is not performed. Canonical equivalent spellings share an upper limit of 16 candidates.

The relevant location is the current file and the database read from the previous log confirmation, with a maximum of 16 entries. Symbolic links are deduplicated according to the actual target; the only primary location will not be listed as a relevant location again. Unresolved sources, unreadable or oversized files retain the main document warning; missing primary keys do not borrow from other databases. Log truncation, syntax or unknown errors, and incomplete source history will stop cross-repository correlation, but candidates in the current explicit file will still be displayed. Inherit full file names, database roles, and bounded cache rules with case warnings.

Diagnostics retain native levels and messages. Biber's selection of duplicate keys varies with options, and the server does not mark candidates for retention or deletion, nor does it provide automatic deletion or renaming. Recompiling after saving will refresh the relevant locations, and clear push and pull diagnostics after repair. UTF-16 range and first row range of cache keys by source in the same log, supporting databases where a large number of entries are concentrated in the same long row.

```sh
cargo test --locked --test biber_duplicate_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_duplicate_warnings_
```

## Biber citation key case warning

Biber's complete `Possible typo (case mismatch) between datasource keys: 'ALPHA' and 'alpha' in file 'refs.bib'` message uses the diagnostic code `bibliography-key-case-mismatch`, marking exactly the first unique key in the database that is unambiguously reported. Matching is case-sensitive and supports Unicode, UTF-16, CRLF, and full file names including apostrophes; files with other suffixes must have confirmed database roles.

Both primary keys and associated keys are decomposed and matched according to Biber's Unicode specification, but the scope points to the original source code. For example, `CAFÉ` / `café` in the log can correspond to `CAFÉ` / `café` in the source code. If there are still multiple declarations of the primary key after normalization, the first line of the file is retained; multiple canonical equivalent declarations of another spelling are retained as independent associated positions. Name rules for normal BibTeX navigation and other backends are not affected.

`relatedInformation` links the position of another spelling in the current database and the previously confirmed reading database, up to 16 places; multiple candidates are retained when there are multiple declarations with the same name. Subsequent databases will not participate in positioning. When log truncation, previous Biber syntax or unknown errors, unresolved sources, or read history exceeds the limit, only the currently explicitly named database is used; missing or duplicate primary keys retain the first line warning of the file and do not borrow from other databases. Preserve main document warnings when the source is unreadable, unconfirmed, or exceeds the bounded parsing cache, and the absence of relevant keys does not affect the original message.

Biber treats the two keys as different entries and may retain both; diagnostics maintain native warning levels and do not provide automatic merging or renaming. The upper and lower case messages between the reference key and the database key are still processed according to the original log rules. After fixing and recompiling, push and pull diagnostics clear old results simultaneously.

```sh
cargo test --locked --test bibliography_case_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_case_warnings_
```

## Undefined string abbreviation

The two-line `Warning--string name "jouranl" is undefined` / `--line 2 of file refs.bib` warning for BibTeX uses the diagnostic code `undefined-bibliography-string`, marking exactly the only bare abbreviation reference in that line. Supports field, `@string`, `@preamble` and multi-line `#` splicing, ASCII case-insensitive matching, and preserves UTF-16 and CRLF positions. Literal values, declaration names, citation keys, and comments do not serve as references; when the same line is repeatedly cited, the entire line of diagnostics is retained, and the modification position is not guessed. Warnings are also retained when the source is missing, the location is invalid, or the location line does not immediately follow the warning.

The quick fix provides up to 5 similar names, taking only the valid `@string` declaration in the database that was read before the warning occurred, and the declaration that precedes the reference in the current library. Declarations with the same name use the most recent spelling; post-declarations, independent items, and implicit month names are not candidates. The month abbreviation is provided by the style and the server does not parse the `.bst` macro definition.

Fixed review of read database and positionable style content, current buffer, version and compilation input snapshot, compatible with versioned editing and old clients. Preserve diagnostics and stop providing fixes when log truncation, database history is missing or abnormal, previous syntax errors occurred, sources exceeded limits, or input changes. Maximum read of 2 MiB per file, cumulative maximum of 16 MiB; incomplete `@string` assignment is not a candidate for repair.

Verify command:

```sh
cargo test --locked --test bibliography_string_diagnostics
cargo test --locked --test bibliography_string_diagnostics real_bibtex -- --include-ignored
cargo test --locked --test protocol bibliography_string_warnings_
```

## Undefined entry type

Two lines of BibTeX, `Warning--entry type for "entry" isn't style-file defined` / `--line 2 of file refs.bib`, use diagnostic code `undefined-bibliography-entry-type`. The warning line number points to the line where the key name is located, and the server marks the type name of the corresponding entry by parsing the key and type relationship of the index record. For example, `@artcle` tag `artcle` can also be positioned when the type and key are separated.

The source, row number, and full key together determine the entry; when a key with the same name appears in multiple databases or on different rows, use the explicitly reported database and row number. Supports Unicode, CRLF, both delimiters, and unfinished body; retain full-line warnings when there are multiple matching keys on the same line. Preserve main document when source or location is missing Warning, aliases for auxiliary files and styles do not provide type scope by pointing to the same database.

Whether a type is supported is determined by the current documentation style, only warnings acknowledged by the compiler are taken into account here. The server has not yet parsed the type definition in the style, so type spelling fixes are not generated; editorial completion of entry types still works.

```sh
cargo test --locked --test bibliography_type_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_type_warnings_
```

## Invalid crossref

The two lines `A bad cross reference---entry "child"` / `refers to entry "parent", which doesn't exist` in native BibTeX use error code `bad-bibliography-crossref`, targeting the only literal `crossref` value in the subentry. Field names are ASCII case-insensitive and support braces, quotes, two entry delimiters, UTF-16, CRLF, and database symbolic links.

ASCII whitespace before and after the literal value is matched as reported by BibTeX, and the range only covers the parent key; null values also retain the complete error, and the range covers their curly braces or quotes, making it easy to see in the editor.

Locate databases that only use pre-error log acknowledgment reads and require the complete subentry, unique key, and unique `crossref` field. Keep the original errors in the main document for dynamic values, splicing, duplicate fields, ambiguous keys, missing databases, abnormal reading order, previous syntax errors, truncated logs, or scan overruns. The first row error remains when the second row is missing, and subsequent independent diagnostics continue to be processed. The limits of 2 MiB per file, 16 MiB cumulative, and 1024 inputs continue, with an additional limit of scanning to a maximum of 16384 entries.

Provides up to 16 related positions via `relatedInformation` when a parent entry with the same name exists in the read database. When the parent entry is ranked before the child entry, it means that BibTeX may skip the preceding parent entry that is not explicitly referenced: the parent entry does exist, but it may still report "does not exist". Explicitly referencing parent entries, using wildcard references, and placing parent entries after child entries affect BibTeX's selection; the server does not treat physical presence as a basis for successful parsing, nor does it automatically move entries or provide spelling fixes that are not validated by selection rules.

```sh
cargo test --locked --test bibliography_crossref_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_crossref_errors_
```

## Nested crossref

The two lines `Warning--you've nested cross references--entry "child"` / `refers to entry "parent", which also refers to something` of native BibTeX use diagnostic code `nested-bibliography-crossref`. The warning preserves the complete child, parent entry relationship, targeting the literal `crossref` value of the child entry, and linking the parent entry key in `relatedInformation`. Parent entries can come before or after child entries; the relevant specification indicates the nested reference chain reported by the compiler and does not interpret resolved parent entries as missing.

Ordinary nesting, self-references, and loops are handled as per the actual compiler log, the level remains at warnings, and these warnings alone do not change a successful build into a failure. If the parent entry references a missing ancestor, the subsequent `bad-bibliography-crossref` error is independently located to the field value of the parent entry and the error level is retained; both diagnoses are displayed at the same time.

Positioning reuses the above database history, unique keys, unique fields, complete text and read limit. Dynamic or ambiguous subentry references, missing sources, truncated logs, and missing or unmatched continuation lines will all retain warnings in the main document; they will not swallow up the next independent record. This diagnostics only uses native compiler reports, does not statically speculate on dynamic inheritance relationships, and does not automatically rewrite the inheritance chain. After fixing and recompiling, both push and pull diagnostics clear old results.

```sh
cargo test --locked --test bibliography_crossref_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_nested_crossrefs_
```

## Repeating fields

Native BibTeX's `Warning--I'm ignoring entry's extra "title" field` followed by `--line … of file …` uses diagnostic code `repeated-bibliography-field`. Diagnostic precision marks ignored field names, `relatedInformation` links the first assigned field name in the same entry. Preserves warning levels, original key and field information, supports ASCII case changes, keys with quotes or apostrophes, custom field names, UTF-16, CRLF, two entry delimiters, and database symbolic links.

BibTeX's report line points to the comma or entry close seen after parsing the field value, possibly several lines after the field name. Locate multiplexed full value expressions and parsing boundaries, including `#` splicing and delimiters moved to the next line. When there are multiple duplicate fields in the same line, only if the corresponding warning number in the complete log is consistent with the number of source code candidates, they will be positioned separately in source code order; different fields or report lines are counted independently.

The same group of warnings reuses field matching and position conversion results, and thousands of repeated fields still display the exact range and first assignment one by one. Cross-line literals, Unicode field names, and CRLFs retain their original positions; subsequent warnings recheck sequence positioning when subsequent syntax errors render the read history incomplete. The location cache is only used in this log and is tied to the actual parsed source.

Preserve root document warnings for missing continuation lines or unconfirmed database sources; preserve confirmable report lines when row numbers are invalid, fields or entries are ambiguous, assignments are invalid, logs are incomplete, or the sequence cannot be confirmed. Literal text, comments, special constructs, and other entries do not provide field positions. The warning is positioned by a value that the compiler actually ignores; after saving changes and recompiling, push/pull diagnostics update the location, clearing old results after eliminating duplicates.

```sh
cargo test --locked --test bibliography_field_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_fields_
```

## Source and entry positioning

Ordinary BibTeX style warnings (such as `Warning--empty author in entry`) and Biber entry messages reuse the bounded database parsing cache of this log to avoid re-reading and parsing the entire library for each warning. When there is no independent row number, the exact key range is only used if both candidate databases can read it and the complete key name corresponds to a unique entry; duplicate keys in the same library or multiple libraries will not select the first entry by default. Names are matched according to the original case, and database symbolic links and repeated input paths are deduplicated according to the actual files.

Explicit sources are only searched within this file; explicit sources are not affected when other databases are unreadable. Without an explicit source, a database that is logged but missing, unreadable, or exceeds the cache limit prevents guessing at the entry location and the message remains in the main document. The limit of 2 MiB for a single file, 16 MiB in total, and a maximum of 1024 files is shared with other native document diagnoses; the cache will be released when this log parsing is completed, and the modifications will be saved and recompiled for relocation.

The native BibTeX `Repeated entry---line … of file …` error uses diagnostic code `repeated-bibliography-entry`. When there is only one document entry key in the report row, mark the key exactly and list the positions of previously read entries in the database with the same name, up to 16 entries via standard `relatedInformation`. Ignore ASCII case according to BibTeX rules and retain the spelling of the original key; support key and entry type line breaking, UTF-16, CRLF and symbolic links. Post-entries, databases not yet read, string declarations, and comments do not provide relative positions. When there are multiple possible entries in the same line, the source is missing, or the line number is invalid, the original line diagnostics is retained and the citation key is not guessed.

```sh
cargo test --locked --test bibliography_duplicate_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_entries_
```

BibTeX line-numbered bibliography and style errors recognize the type by the input name in the log and then locate the actual file of the symbolic link; correct line numbers are retained when the target uses an unrelated extension. Auxiliary files will not be mistaken for source files because the link target has a source code extension; when the path is missing or is actually a directory, the diagnostics remains in the main file.

Document paths in Biber messages are matched on full filename boundaries, `refs.bib.backup`, `refs.bib2`, or `.bib` in directory names will not borrow the location of an existing `refs.bib`. Explicit `in file refs.bib` and `in file 'refs.bib'` take precedence over filenames in the error text; plain text `in parser` does not prevent the subsequent true source from being identified. `in 'wrong.bib'` or `file 'wrong.bib'` within quotation marks will not overwrite the explicit source; unconfirmed sources retain the original error message and display it in the main file.

Without an explicit source field, the full filename, `refs.bib:`, and the path in parentheses can still be located if enclosed in independent quotes; inline filenames are not truncated from the larger quoted text. Unquoted paths preserve apostrophes, commas, and `.bib` in directories, and are compatible with message punctuation and Unicode whitespace delimiters.

Quoted document paths explicitly noted by the log are treated as full names, preserving spaces, apostrophes, and the `.bib` fragment inside the filename. Punctuation immediately following quotation marks in file names such as `refs.bib':copy.bib` is also retained as part of the name, and `line 700` in the file name does not overwrite the real line number; these full names are also used for dependent indexing, jumps, and citation key renaming. The entry warning only locates the citation key in the specified database; when the specified file does not exist, the path is a directory, or the quotation marks cannot be confirmed, the source of the main file mark is not confirmed, the prefix, suffix or the position of other databases is not borrowed, and the entry range of other files is not applied to the current file.

When punctuation and spaces after quotation marks also belong to the file name (e.g., `refs.bib': copy.bib`), use the full database name confirmed by the literature log to disambiguate it. When multiple recorded names match at the same time, the complete long name is given priority, and the file is found according to the common search path rules; when the complete file is missing, the original error is displayed in the main file, and the location of the short file is not borrowed. `in file`, `entry`, and `line` in the file name are not treated as log fields, nor does the logged short name truncate another well-bounded long name in the message.

Diagnostics also uses the full name confirmed by the log when quotes are not added to the literature path, preserving spaces and quotes inside the file name, and supporting pairs of brackets in the message. For example, `refs.bib line 700 copy.bib` will not be cut into `refs.bib`, and `line 700` will not overwrite the real line number. Both explicit sources and independent paths adhere to these rules; when a complete source is missing, an original error is displayed in the main file and the source is marked as unconfirmed.

When the explicit `file` / `in file` source is not listed in the input log, the full `.bib` path including spaces can also be located. `line`, `entry` text within the filename range and other quoted filenames in the message do not provide an error location; when the complete file is missing or the name uses a suffix such as `.bib.backup`, the source is left unconfirmed. Generic description text (such as `in parser: ...` or `in 'parser'`) does not become the source of diagnostics before subsequent confirmable file fields.

When logs and diagnostics use different path writing methods, they are also recognized by the same input: absolute paths, relative paths, `./` and directory links pointing to the same database are supported, and the actual meaning of `..` after symbolic links is retained. The match follows the order of the tool working directory and `BIBINPUTS`, and uses the log to confirm the read target; files with the same name in other directories will not impersonate the source. Keep the complete message and mark the source as unconfirmed when the source is missing or switched, and do not borrow the range of document entries that interfere with the file. Other files that are explicitly specified by the diagnostics and actually exist can still be located normally even if they are not included in the input record.

`in key` at the end of the BibTeX warning searches by the original content of the citation key, preserving the quotes in the key name; even if the key name is `entry.bib` and a file with the same name exists on disk, the entry in the actual database is located. Biber's `entry 'key' in file 'path'` identifies the citation key and source path respectively; `.bib` in the key name will not allow the file with the same name to overwrite the actual database, and the unclosed key field will not be regarded as the source path. Explicitly specified source files still take precedence.

Biber's `entry` / `Entry` field supports single and double quotation marks, completely retains the citation keys with apostrophes such as `O'Connor2026`, and is positioned according to the UTF-16 range. Filenames, other quoted content, and words with the same name as `subentry` are not treated as entry fields; truncated, unclosed explicit keys do not borrow positions from other entries at the end of the message.

Biber line numbers only take the complete `line integer` field except the source field and quotation mark content. File names such as `line 700 refs.bib`, error content within quotation marks, and words such as `outline` will not cover the real line number; entry warnings also retain the precise range of key names. Warnings generated by BibTeX style are positioned by the last citation key even if the text contains `line 700`; BibTeX errors with source file and line numbers continue to use the tool's independent line number format.

Document dependency and log positioning share search rules, identify the working directory of BibTeX and the database/style search path reported by latexmk, and avoid positioning errors caused by files with the same name in the main directory and output directory. Spaces, Chinese characters, and database names containing single quotes in Biber logs will also be retained.
