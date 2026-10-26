# biblatex resources

[Documentation](README.md) · [Quote Command](biblatex.md) · [Document Key and Rename](bibliography.md) · [Compile Input Check](build-inputs.md)

## Loading and editing

```tex
\usepackage{biblatex}
\addbibresource{references.bib}
\addglobalbib[bibencoding=utf8,label={Shared references}]{shared.bib}
```

`addbibresource` adds default literature resources; `addglobalbib` adds biblatex global resources used by each `refsection`. Both support path completion, file linking, definition jumps, path updates after file or directory moves, as well as navigation of loaded citation keys and cross-file renaming. The command snippet retains the complete `.bib` file name, and the signature help usess `[options]{file}`.

Only a complete filename, including extension, is read per call. `{refs}` only searches for files named `refs` and does not select `refs.bib`; `{refs,part.bib}` is a file name with a comma. Among the resource parameters directly called, `%` is processed according to the literal file name characters and will not swallow subsequent references; path completion and file movement retain these characters and the complete extension.

After dynamic file names are confirmed by actual Biber logs, document dependencies will also be added and indexed by database roles, supporting non-standard extensions or files without extensions. For specific rules, see [Document Compilation and Diagnostics](bibliography-diagnostics.md#database-roles-and-special-suffixes).

Key renaming of shared libraries overrides the associated root document, retains entries of the same name in standalone projects, and respects existing ambiguities and version checks. Resources are indexed as BibTeX data, and text within them that looks like TeX macros is not imported.

## Static support range

By default, native BibTeX data is processed, and the following optional parameters are also recognized:

- `type=file`、`datatype=bibtex`、`location=local`、`glob=false`。
- The literal `bibencoding`, and the resource label `label`; the labels are saved as original text, and the macro text will not be expanded into certifiable static labels.

One curly brace and grouping-protected commas outside option values are allowed. Remote resources, other data formats, wildcard expansions, unknown options, or dynamic values that determine file selection are not inferred as local dependencies. Options containing comments or parameter placeholders, macro-expanded file names, and unconfirmed paths such as leading and trailing spaces, consecutive spaces, and newlines will not be inferred.

Static `\csuse{addglobalbib}`, `\csname addbibresource\endcsname` and confirmable pure aliases share resource analysis; does not perform arbitrary macro expansion and does not fully emulate character class changes in custom commands. Calls that are consumed as data, displayed verbatim, or have been redefined by other macros will not mistakenly provide file editing; resources in the unexecuted macro body will not be used as determined missing file conditions.

`addsectionbib` only registers resources for later selection, and will not add dependencies when they appear alone. Confirmable `refsection` selects to replace the default resources and retain global resources, and the empty list restores the default resources; reference key completion, jump, suspension, missing and duplicate diagnostics follow the actual partition resources. Unconfirmed execution paths or consumer disagreements remain protected, reference lookups, highlighting, and safe renames of static database keys are handled as fully declared identities, and all consumers are checked against new name capture in preparation for renames and both workspace edit formats.

The static resource tags that can be confirmed in the preface support partition selection completion, definition jump, hover, search for references, highlighting and renaming. The last `label` option takes effect; registered labels take precedence over literal file names, and `chosen.bib.alias` can also be used as a label. The rename checks the shared consumer and whether the new name captures the existing tag or filename selection, and rejects edits when integrity cannot be guaranteed; dynamic tags do not provide these symbol edits.

## Compile protection

Static local resources participate in unsaved content checking before first compilation. Completing the missing literature library during compilation will cause the results to expire, preventing the continued release of old missing resource diagnoses; for detailed rules, see [Compilation Input and Expired Results](build-inputs.md).

When Biber/BibTeX reports missing local resources, the server locates the identifiable resource parameters and provides up to 5 filename spelling fixes; retaining full extensions and calling options. For specific rules, see [Document Build diagnostics](bibliography-diagnostics.md#missing-library).


Biber's IDS alias cache is shared across digital partitions: aliases loaded by previous partitions can import original entries in subsequent resource allocations without local IDS claims, or even local primary keys. The current static resource model retains an unknown state for such imports, does not guess missing references, does not return partial reference maps or partial renames. Authentication also covers TeX requests, source code collection members, and database relationships; independent tags and resource tags continue to use their own namespaces.

When IDSs with the same name are declared in different libraries, changing one of the primary keys alone will cause global mapping inconsistency, so editing of the primary key is rejected. After first changing them to independent aliases and updating the references, you can continue to authenticate the primary key name change. The new name of the alias must check all related partitions of the same compiled project to prevent the capture of references or relationships that were not originally defined. Unsaved database changes immediately revoke or restore certification.

The new names of ordinary primary keys and coupled source code primary keys cannot occupy the IDS names of other partitions: when Biber reads real entries with the same name, it will delete the previous global alias mapping, causing subsequent references to change. Ordinary primary keys that do not conflict with each other in different partitions still retain their respective identities.


The belonging entries of an IDS are also affected by the order in which resources are first loaded: Biber processes requests in numerical partition order, and the global entry cache is only updated on the first read from the same database. Reselecting the default resource will create a new digital partition; ending the current partition and returning to partition 0 will still be in the previously processed partition 0. No database is preloaded for partitions that are not requested.

When returning to a previously read database, the cache alias may obtain the primary key data of another database with the same name; the unreferenced real entry may also overwrite the cache or delete the alias with the same name. The server retains the actual request and resource first load status of each digital partition, keeps unknown requests, relationships, and completions that cannot prove local ownership of entries, and rejects partial references or edits that rely on them. Provider ambiguities caused by coexisting primary keys continue to be passed to subsequent cache requests; tags and resource tags are still resolved independently.

The primary key of an IDS with the same name can have independent book or source code collection identities in different digital partitions. Static reads now allocate authenticated navigation, suspension, complete references and candidates on a resource-by-resource basis to avoid misjudgment of non-coexisting primary keys with the same name as the same entry; all requests are still subject to global alias mapping and cache load order. If the same physical reference has different meanings in different executions, continue to reject ambiguous results. Renaming still additionally requires full ownership authentication across resources.

Cross-partition static IDS aliases can be renamed independently: the editor will completely update the alias declaration, TeX reference and database relationship in this partition, and retain the same-name aliases and all primary key spellings of other partitions. New names must pass project-wide partition references, source code collections, relationships, and IDS name capture checks; edits are rejected when authentication is incomplete due to foreign cache imports, shared call ambiguities, and unsaved consumers. The independent renaming of the primary key to which the source code belongs still requires consistent global mapping.

The new name of the primary key will also affect the global entry cache: changing the primary key belonging to the IDS to the ordinary primary key of another database, or changing the ordinary primary key to the primary key belonging to the IDS of another database, may cause subsequent aliases to obtain incorrect data. The server re-authenticates the actual number partition and first load sequence according to the proposed new spelling, and rejects edits that change the source of the data; renames with the same name that can be proven safe, such as later loading of the library, continue to be allowed. This check also covers coupled edits of source collections and original primary keys, and is updated with unsaved databases.

`defcitealias`, `entrydata`, and `addtocategory` in the ordinary resource partition retain complete citation key references, which can participate in precise navigation, search for references, and safe renaming, and also support static naming calls. They do not send referral requests to Biber and therefore do not cause the database to be loaded early; related IDS queries still check the actual cached origin. Conditional queries and source code collection paths continue to be authenticated separately, and partial edits will not be returned when complete execution cannot be proven.

When an IDS alias name has the same name as a static source code collection, if there is no database relationship pointing to the name, unauthenticated relationship strings/splicing, and wildcard loading, navigation, hovering, and full references can be provided in the actual request order: requesting the alias first will lock the database identity, and declaring the collection first and then requesting it will adopt the collection identity. Multiple primary key/alias sets belonging to the same name and another alias request are also authenticated separately. Unprovable results remain unknown when relationships, wildcards, shared physical reference meaning conflicts, or unsaved consumers are involved; this extension continues to reject renames of intersecting names, and candidates still use independent strict authentication.

In a common resource partition, if multiple original entries share an IDS name and maintain the same primary key spelling, the primary key rename will find all the associated entries through any aliases, update all primary keys and their complete references together, and preserve the IDS spelling and independent read identity. The new name must be free within the entire scope of the relevant resource; unsaved consumers, ambiguous shared calls, different ownership spellings, budget overruns, and couplings involving source code collections are still rejected for editing. The output uniformly verifies all file versions, duplicates and overlapping edits.

The handwritten `\bibitem` belongs to the classic document namespace and cannot define the reference key of the biblatex background. Authenticated native entries and static naming calls will not be merged with the database definition, nor will they participate in the renaming of the database primary key; keys that only exist by hand are still reported as missing. Ordinary projects and digital partitions follow the same rules, and source collection coexistence, dynamic keys, redefinition, and unauthenticated forwarding continue to be protected.

The ordinary resource map supports key navigation, reference and complete renaming of static `\ifentryinbib{key}{text}{text}` and `\ifentrycategory{key}{category}{text}{text}`. The two branches can be empty or contain ordinary text and comments. These queries do not request data from Biber and do not load the database ahead of time; proposed new primary keys still check for query key capture. Maintain full graph protection when branches containing commands, parameters, or special TeX syntax, as well as dynamic keys, predicate redefinitions, or source code collections coexist.
