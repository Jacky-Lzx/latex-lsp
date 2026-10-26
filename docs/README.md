# Documentation

[Back to project README](../README.md)

TeXquill supports completion, navigation, cross-file rename, formatting, diagnostics, quick fixes, and background builds. Start with [installation](install.md) and [editor configuration](editors.md); see the [build guide](build.md) to generate PDFs.

## Getting started

- [Installation and startup](install.md): Requirements, building from source, installation, protocol lifecycle, and migration.
- [Editor configuration](editors.md): Neovim and VS Code setup, other editors, and a sample project.

## Editing and projects

- [Editing features](editing.md): Completion, navigation, highlights, and colors.
- [Formatting](formatting.md): LaTeX and BibTeX document, range, and on-type formatting; tex-fmt, latexindent, arguments, and project settings.
- [Document folding](folding.md): Sections, environments, multiline equations, BibTeX entries, and client capabilities.
- [External documents and compiled reference information](auxiliary-references.md): xr prefixes, auxiliary-file numbers, pages and titles, hover, and inlay hints.
- [Static named references](named-references.md): Navigation, reference collection, and cross-file rename for named reference keys.
- [Labels and cross-references](labels.md): Key names, comments, range references, and hyperref labels.
- [PDF destinations and bookmarks](pdf-destinations.md): Internal links, composite destinations, and bookmark rename.
- [Contextual option completion](options.md): Keys and enumerated values for packages, document classes, graphics, and configuration commands.
- [Completion](completion.md): Call snippets, existing arguments, command-name operands, and name replacement.
- [Signature help](signatures.md): Cross-file signatures, declaration arguments, optional argument boundaries, and comments in argument counts.
- [Kernel hooks](kernel-hooks.md): Completion, signature help, name navigation, structural references, and deferred execution boundaries.
- [Citation search completion](citation-search.md): Authors, titles, years, combined queries, and fuzzy key matching.
- [Bibliography](bibliography.md): Citation keys, biblatex aliases, natbib, manual bibliographies, entry relationships, string abbreviations, and URL/DOI links.
- [biblatex resources](biblatex-resources.md): Loading commands, path edits, and static analysis boundaries.
- [biblatex citations](biblatex.md): Fields, lists, names, single citations, multicites, and volume citations.
- [biblatex categories](bibliography-categories.md): Category assignment, citation-key edits, and rename boundaries.
- [biblatex entry sets](bibliography-sets.md): Dynamic set declarations, member references, independent rename, and native set diagnostics.
- [minted code](minted.md): Verbatim code, file path edits, build dependencies, and missing-file fixes.
- [expl3 editing](expl3.md): Full function names, variable declarations, navigation, rename, and signature help.
- [Structural refactoring and fix all](refactoring.md): Text wrapping, command extraction, and safe batch fixes.
- [Glossary, acronym, and index symbols](glossary-index-symbols.md): Independent namespaces, completion, navigation, rename, and diagnostics.
- [Symbols and rename](symbols.md): Supported macros, environments, counters, and other symbols.
- [User macro call hierarchy](call-hierarchy.md): Incoming and outgoing calls, cross-file locations, and static analysis boundaries.
- [User declaration documentation comments](command-documentation.md): Macro and environment hover, completion, argument documentation, incremental updates, and limits.
- [String tests](string-tests.md): etoolbox literal data, conditional branches, and signature help.
- [String tests across files](string-test-projects.md): Redefinitions, load order, and rename boundaries for shared files.
- [Saving and restoring string commands](string-test-copies.md): Static copies, restored literal data, and cross-file argument boundaries.
- [Direct calls to saved string commands](string-test-copy-calls.md): Captured data argument counts, signature help, shared-file ambiguity, and build diagnostics.
- [Undefinition and string tests](string-test-undefinitions.md): Undefinition operands, saved copies, redefinitions, dynamic targets, and incremental updates.
- [String test commands as macro arguments](string-test-operands.md): Cross-file argument syntax, control-word operands, signature boundaries, and unknown macros.
- [Conditional file hooks](conditional-string-tests.md): File-existence branches, pre-hook order, string semantics, and rename.
- [Chapter selection and string semantics](selected-string-tests.md): `includeonly` load order, name matching, shared files, and rename boundaries.
- [Counters and theorems](counters.md): Numbering relationships, cross-file rename, etoolbox operations, and refcount label assignments.
- [Registers and lengths](registers.md): Register declarations, type checks, length expressions, and safe rename.
- [futurelet lookahead targets](futurelet.md): Declarations, static names, cross-file rename, and continuation boundaries.
- [Command-name operands and declarations](declaration-operands.md): Command-name references in copies and declarations, indexing, and edit boundaries.
- [meaning command-name operands](meaning.md): Navigation while inspecting commands, cross-file rename, single-token boundaries, and dependency candidates.
- [TeX distributions and packages](distribution.md): Installed-file completion, source navigation, command metadata, and signature help.
- [Project settings and scope](project-settings.md): Directory inheritance, root-specific overrides, workspace-folder isolation, and configuration errors.
- [Indexing budgets and caches](indexing.md): Background workers, configurable limits, truncation status, and semantic token caches.
- [Source loading with the import family](imports.md): Nested directory contexts, dependencies, path completion, and navigation.
- [Projects and files](projects.md): Workspace indexing, root files, dependencies, and path resolution.
- [Graphics resolution and moves](graphics.md): Search directories, extension order, local and global settings, and graphics rename.
- [Graphics filenames](graphics-filenames.md): Quoted filenames and search directories, engine differences, whitespace, path completion, and missing-graphics fixes.
- [Static aliases for graphics commands](graphics-calls.md): Aliases for graphics calls and search settings, named calls, and support boundaries.
- [File operations](file-operations.md): File move requests, reference updates, file URLs, creation, and deletion.
- [Source filenames and moves](source-filenames.md): Spaces in filenames, extension preservation, and representable references.

## Builds and diagnostics

- [Static diagnostics and ChkTeX](static-diagnostics.md): Undefined references, duplicate bibliography fields, optional math delimiter checks, spelling fixes, and save-time checks.
- [Static brace diagnostics](brace-diagnostics.md): Edit-time matching, single-token copies, macro bodies, and quick fix boundaries.
- [Builds](build.md): Build entry points, commands, notifications, and rebuild actions.
- [Cleaning build artifacts](build-cleaning.md): Auxiliary and full cleanup, previews, file protection, and status updates.
- [Build configuration](build-configuration.md): Build tools, engines, argument templates, and artifact names.
- [Build result analysis](build-analysis.md): Page counts, citations, rerun requests, and external processing tools.
- [Last-page changes and reruns](last-page.md): Temporary extra pages, last-page hook positions, actual page counts, and clearing rerun requests.
- [Index and glossary processing](index-processing.md): Required makeindex tools, explicit pipelines, atomic artifact updates, and convergence checks.
- [Build inputs and stale results](build-inputs.md): Save checks, cancellation, dependency tracking, and artifact validity.
- [Build diagnostics](diagnostics.md): Error locations, warnings, and spelling fix rules.
- [Length diagnostics and fixes](length-diagnostics.md): Undefined etoolbox lengths, candidate types, and spelling fixes.
- [Toggle diagnostics and fixes](toggle-diagnostics.md): Undefined toggles, independent namespaces, and project-scoped spelling fixes.
- [Boolean diagnostics and fixes](boolean-diagnostics.md): Base-name locations, project candidates, and read-only built-in conditions.
- [Boolean expression diagnostics and fixes](boolean-expressions.md): Boolean and toggle operands in static logical expressions, and loop boundaries.
- [Unicode input errors](unicode-errors.md): Unsupported characters, code-point summaries, and location boundaries.
- [Special character diagnostics and fixes](special-characters.md): Exact locations and escape fixes for literal special characters and extra alignment tabs.
- [Extra closing braces](extra-braces.md): Character locations and deletion fixes in text, math, and explicit groups.
- [Duplicate math scripts](math-scripts.md): Exact script-symbol locations and explicit empty-base fixes.
- [Extra right delimiters](right-delimiters.md): Command locations, delimiter preservation, and fix validation for `Extra \right.`.
- [Missing right delimiters](missing-right-delimiters.md): Math end-marker locations, invisible delimiter insertion, and fixes across successive builds.
- [Missing delimiter operands](delimiter-operands.md): Source checks for direct primitives, invisible operand insertion, and source-file validation.
- [Macro sources for special character fixes](literal-macro-scopes.md): Cross-file argument exclusion, load order, and revalidation of old fixes.
- [Font declarations, diagnostics, and analysis](fonts.md): NFSS completion and signature help, font and size substitutions, missing-glyph summaries, and location limits.
- [Typesetting warning analysis](typesetting.md): Overfull/Underfull counts by category and maximum measurements.
- [Float height warnings](floats.md): Source lines, warning counts, and maximum excess height for oversized floats.
- [Explicit bibliography processing](bibliography-processing.md): BibTeX/Biber with direct TeX engines, staged artifacts, subsequent passes, and cancellation.
- [Bibliography build diagnostics](bibliography-diagnostics.md): BibTeX/Biber source locations, missing resources, and filename fixes.
- [SyncTeX queries](synctex.md): Source-to-PDF and PDF-to-source query protocols.

## Development and testing

- [Maintaining this website](documentation.md): Local preview, navigation, link checks, and GitHub Pages deployment.
- [Development and testing](development.md): Quality checks, module ownership, and analysis architecture.
- [Continuous editing benchmarks](benchmarks.md): Real stdio sessions, feature latency, raw samples, and before/after comparisons.
- [Feature reviews and implementation queue](roadmap.md): Prioritized feature gaps, implementation status, and subsequent reviews.
- [Tests with real tools](testing.md): External program requirements, PDF builds, and editor test commands.
