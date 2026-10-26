# Tests with real tools

[Back to documentation](README.md) · [Development and testing](development.md)

## Environment and entrance

Default `cargo test --locked` skips regressions that require external procedures. After installing latexmk, pdfLaTeX, XeLaTeX, LuaLaTeX, BibTeX, Biber, biblatex, logreq, SyncTeX and Neovim 0.10+, all tests can be performed:

```sh
cargo test --locked -- --include-ignored
```

When only some tools are installed, press the following command to select the corresponding test.

## Editor language identification

Default regression overrides priority of editor language type, non-standard file names, unnamed documents, and project input roles. True Neovim regression requires Neovim 0.11+. After verifying that the unsaved `.data` file is set to the `bib` file type, entry symbols, incremental editing and completion are all parsed using BibTeX:

```sh
cargo test --locked --test language_ids -- --include-ignored
cargo test --locked --test protocol language
```

biblatex's `.bbx`, `.cbx`, `.lbx`, `.dbx` additionally checks root directory scanning, dynamic listener registration, disk creation/update/deletion, unsaved buffer priority and shutdown recovery. The font definition `.fd` shares these scan and listen regressions with the Babel language file `.ldf`. Explicit input regression checks project scope, path completion, file movement, and the native priority order of files with the same name suffix such as `.bbx.tex`; `.fd` / `.ldf` checks the exact file priority. The log regression includes nested contexts for four biblatex files with two font/language files, missing source barriers, UTF-16 and number positioning, and the recorder regression confirms the input role.

```sh
cargo test --locked --test language_ids --test path_completion
cargo test --locked --test protocol latex_and_biblatex_sources_
cargo test --locked --test generated_log real_corrupt_sources_ -- --include-ignored
```

The last item requires pdfLaTeX, triggering errors in `.aux`, `.bbl`, `.toc` and six supported sources, verifying the actual source of legacy and file line number logs respectively.

## Document field diagnostics

Duplicate field diagnostic regression covers native value end of line, trailing commas and closing characters, multiple duplicates on the same line and full record order, truncation and ambiguity fallback, case and quote keys, UTF-16, CRLF and source roles. Native BibTeX confirms first value retention; stdio regression check push/pull diagnostic clears after move and fix after saving incremental edit:

```sh
cargo test --locked --test bibliography_field_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_fields_
```

## Semantic highlighting

Semantic highlighting regression covers BibTeX types, fields, string and numeric operands, operators, relational keys, comments, and incomplete input. Protocol regression checking extends legend, UTF-16 range queries, continuous incremental editing of fields/values/comments, delta restore, unchanged results and reopening documents:

```sh
cargo test --locked --test semantic
cargo test --locked --test protocol semantic_token
```

Custom field name regression checks for legal punctuation and Unicode, full field completion range, subsequent metadata and relationship references, abbreviation navigation and renaming; native testing uses custom `.bst` to read these fields:

```sh
cargo test --locked --test bibtex_field_names -- --include-ignored
cargo test --locked --test protocol bibtex_custom_field_names
```

## Completion

BibTeX custom field regression covers cross-document resource candidates, item isolation, local spelling priority, case deduplication and sorting, and complete field replacement including `$`, backslashes, Chinese and emoji. The literal name after expansion of the real Neovim validation fragment, and the native BibTeX validation completion retains existing assignments and subsequent fields; both tools need to be installed at the same time. Protocol tests check for unsaved incremental editing of resources, disk notifications during file opening, disk recovery candidates after closing, and client snippet support:

```sh
cargo test --locked --test bibliography_completion -- --include-ignored
cargo test --locked --test protocol custom_bibliography_field_completions
```

Command-name operands regression covers built-in and cross-file user commands, declaration target, copy source, `meaning`, empty command prefix, unclosed name grouping, comment continuation line, mid-word substitution and UTF-16 range; check that incomplete input will not produce empty name declarations, comments, verbatim and textual content will not provide command completion, while confirming ordinary calls, `futurelet` Continued commands and calls after explicit redefinition in this file still provide argument snippets:

```sh
cargo test --locked --test operand_completion
```

Parameter count regression checks comments, CRLFs, comment-split numbers, optional defaults, and illegal and dynamic numbers, checking signatures and completions across file macros, environments, and pairwise delimiters. External regression compares the actual parameter values of macros, environments, and delimiters across the three engines before and after file renames, preserving quantitative annotations:

```sh
cargo test --locked --test signature commented_argument_counts -- --include-ignored
```

Defaults preview regression covers legacy macros/environments with xparse defaults, checking for comment elimination, escaping percent signs, explicit control of whitespace, control of word boundaries, Unicode and long comments. The three engines use `\ifx` to compare the token identity of the original default value and the preview text, and also check that the signature help and completion details are consistent:

```sh
cargo test --locked --test signature optional_default_previews -- --include-ignored
```

## Rename

Regression of tags, commands, environments, counters and PDF targets covering cross-file editing, comment preservation, name conflicts and special character boundaries. True compilation also checks for consecutively renamed PDF links, bookmarks, and reference values; the combined target overrides empty names and paired or unpaired square brackets, and preserves ordinary tags with the same name.

The following tests require a current LaTeX distribution, pdfLaTeX, latexmk, and hyperref, mathtools, refcount, and aliascnt:

```sh
cargo test --locked --test rename --test bracket_labels --test counter_brackets --test counter_rename --test commented_labels -- --include-ignored
```

For the three-engine return of source code file movement, see [Typesetting log and file movement](#typesetting-log-and-file-movement). For the support range, see [Tag and cross-reference](labels.md), [PDF target and bookmark](pdf-destinations.md) and [Symbol and renaming](symbols.md).

## Counters and Theorems

The default tests check cross-file navigation and renaming, completion, signature help, semantic highlighting, name conflicts, and exclusion of illegal asterisks and dynamic names:

```sh
cargo test --locked --test etoolbox_counters --test counters --test counter_rename --test counter_brackets --test counter_fixes --test undefined_counters --test shortened_counter_context --test theorem
```

True regression of etoolbox requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox. Compare the local assignments, global assignments and existence checks of ordinary counters and theorems before and after the theorem name change in the three engines, covering the new name containing `]`; at the same time, use traditional and file line number logs to verify the precise location of undefined counters, apply spelling fixes and recompile. Check that the branch body, expressions and independent symbols with the same name have not been changed by mistake:

```sh
cargo test --locked --test etoolbox_counters -- --include-ignored
```

For support range and examples, see [Counters and Theorems](counters.md).

## Register and length

Default regression covers name navigation and renaming of etoolbox register type checks, derived counter names, completion, argument hints, operand and branch boundaries, and shared file ambiguities. Real regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox, rename the length, size, counting register, LaTeX counter, theorem and ordinary macros one by one, and check that the type judgment results, local/global assignments and the elongation and contraction components of glue are consistent:

```sh
cargo test --locked --test etoolbox_registers
cargo test --locked --test etoolbox_registers -- --include-ignored
```

For operation examples and static analysis boundaries, see [Registers and Length](registers.md).

## Undefined length diagnostics

[Length Diagnostics and Fixes](length-diagnostics.md) Regression checks for the exact range of `deflength`, naked control sequences and cross-line expressions, duplicate and shortened source code echoes, candidate register types, project isolation and versioning, and old client fixes. External regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox, uses 60/79 column logs from all three engines, fixes custom lengths, dimensions and built-in lengths in legacy vs. file line number formats, and checks actual values after recompilation:

```sh
cargo test --locked --test undefined_lengths
cargo test --locked --test undefined_lengths -- --include-ignored
cargo test --locked --test protocol undefined_length_diagnostics_offer_versioned_fixes_and_reject_changed_candidate_types
```

## boolean diagnostic

Default regression checks etoolbox base name positioning for five direct operations, crossing lines and UTF-16 ranges, duplicate and truncated source code echo, declaration and derived command conflicts, project isolation, read-only built-in conditions, versioning and old client fixes. External regression requires pdfLaTeX, XeLaTeX, LuaLaTeX, ifthen and etoolbox; recompile after fixing in 60 / 79 columns of the three engines and two log formats, check the independence of local and global assignments, branches and toggle with the same name:

```sh
cargo test --locked --test undefined_booleans
cargo test --locked --test undefined_booleans -- --include-ignored
cargo test --locked --test protocol undefined_boolean_diagnostics_offer_versioned_fixes_and_reject_changed_candidate_namespaces
```

For the support range, see [boolean diagnostics and repair](boolean-diagnostics.md).

## toggle diagnostic

Default regression checks for actual parameter read boundaries for five direct operations, UTF-16 and cross-line positioning, duplicate and shortened echoes, candidate namespaces, project isolation, build command conflicts, and obsolescence fixes. Real regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox, using 60 / 79 columns and two log formats respectively. After applying the fix, recompile and check the local and global assignments and branch results:

```sh
cargo test --locked --test undefined_toggles
cargo test --locked --test undefined_toggles -- --include-ignored
cargo test --locked --test protocol undefined_toggle_diagnostics_offer_versioned_fixes_and_reject_changed_candidate_namespaces
```

For the support scope, see [toggle Diagnostics and Repair](toggle-diagnostics.md).

## Boolean expression diagnostics

For the commands and verification scope of default, stdio protocol and three-engine regression, see [Boolean expression verification](boolean-expressions.md#verify). True regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox; see [Boolean expression diagnostics and repair](boolean-expressions.md) for functional boundaries.

## String test

For the data boundaries, editor protocol and three-engine verification commands of string testing, see [String Test Verification](string-tests.md#verify); for the regression of import redefinition, shared files, incremental editing, close recovery and disk changes, see [Cross-file context verification](string-test-projects.md#verify). For regressions on direct calling of copies, changes in the number of parameters, and renaming of macro names and booleans, see [String copy verification](string-test-copy-calls.md#verify). For the branch selection, execution sequence and non-source file notification regression of conditional file hooks, see [Conditional File Hook Verification](conditional-string-tests.md#verify). For undo operands, redefinition, shared context and three-engine regression, see [Undo Definition Verification](string-test-undefinitions.md#verify). External regression requires pdfLaTeX, XeLaTeX, LuaLaTeX and etoolbox.

## Typesetting log and file movement

Page count regression checks for spurious page counts in output paths, PDF/DVI/XDV formats, quotes, log wraps, incomplete summaries, and read budgets. The external regression uses the 60 / 79 column logs of three LaTeX engines, and checks the compilation results to return the actual number of PDF pages:

```sh
cargo test --locked --test page_counts -- --include-ignored
```

Empty document regression checking criteria with LuaTeX's pageless reporting, old PDF retention on successful exit, log vs. pipeline priorities, and log elision boundaries. External regression continuously generates PDF, compiles empty documents, and restores the text in the 60 / 79 column logs of the three engines. The verification results will not reuse the old product, and the compilation can be successful after re-adding the text:

```sh
cargo test --locked --test no_pages -- --include-ignored
```

Information log regression checks for five types of standard `Info` messages, filename brackets, byte wrapping, whitespace and package continuations, longer messages, true file switches, wrapping error paths, and log omission boundaries. Examples of errors and warnings, source code echoes, resources, and PDF output text that are explicitly line continuations will not be published as diagnostics. External regression checks the exact range of diagnostic counts, warning attributions, error line numbers, and undefined commands in the 60 / 79 column output of three LaTeX engines and in the traditional / file line number format:

```sh
cargo test --locked --test information_logs -- --include-ignored
```

Standard recompilation warning regressions cover column-by-byte wrapping, LF/CRLF, end-of-line spaces, continuation lines after the first sentence, extra comments, and adjacent file errors for LaTeX, natbib, longtable, and hyperref at columns 60–82. The three-engine regression uses 60 / 63 / 79 column logs to rename cross-file labels and citation keys respectively, or modify the column width of subsequent partitions of the table to check the reappearance of requests, the actual number of PDF pages, and clearing after convergence. Protocol regression checks for complete warnings, responses to both client operations, final log priority, and invalidation of old operations:

```sh
cargo test --locked --test rerun_warnings
cargo test --locked --test rerun_warnings -- --include-ignored
cargo test --locked --test protocol wrapped_standard_rerun
```

[Last page change regression](last-page.md) Continuously shorten and lengthen the cross-file body in the 60/70/79 column logs of the three engines, check the temporary extra pages, the old page and the correct page of the last page hook, the actual PDF page number, and the request and operation clearing after compilation again:

```sh
cargo test --locked --test rerun_warnings real_last_page -- --include-ignored
```

rerunfilecheck regression checks the standard `Rerun.` request, hyperref complete bookmark and reverse literature reference prompt, line breaks, extra blank lines after full lines, sample text and makeindex request boundaries, and checks that the result analysis is consistent with the recompilation operation. The default regression also checks for parentheses in file names, seemingly complete warning text, and real file switches after incomplete warnings. External regression verifies special file names, subsequent diagnostic ownership of main files and sub-files, requests after the first file change, and clearing after the next pass of content remains unchanged in the 60 / 79 column logs of the three LaTeX engines:

```sh
cargo test --locked --test rerunfilecheck -- --include-ignored
```

Back reference regression first generates stable literature citations, then moves the citation to the next page, checks the number of pages in the three engines, reappeared requests, and clearing after re-convergence; real log regression of indexes and glossaries ensures that makeindex instructions will not generate operations that simply rerun TeX. stdio regression covering standard CodeAction / legacy Command clients, completion notifications consistent with status, final log priority, diagnostic clearing, and legacy menu actions failing:

```sh
cargo test --locked --test protocol backreference_rerun
```

Statement operand regression Use three LaTeX engines to verify that ordinary calls after copying `newcommand`, `newif`, `newenvironment`, and the output before and after macros, switches, and environment renames are consistent; also verify that the literal curly braces are copied directly with the static `csname` target, and that the macro body and parameter delimited text are not executed. The default test also checks various symbol indexes, signature help and name completion of command name operands, static `csname`, verbatim boundaries and label editing ranges, and checks that curly brace copying does not produce error fixes, and true mismatches still retain precise positions:

```sh
cargo test --locked --test operand_declarations -- --include-ignored
```

`meaning`'s default regression checks declarations with verbatim scan boundaries, literal curly braces, signature help, and tag editing exclusions, cross-file navigation and renaming directly with static names, and verifies that dependency candidates are not mixed in with plain text after copying the source. The above three-engine regression simultaneously checks the execution results of `newcommand`, ordinary macros and `verb`, macro text, parameter-delimited text and the actual call after this file is rewritten to `meaning`; compare the output before and after renaming. There are additional regressions for the root file and dependency rules of the project context, covering declaration targets, copy sources, and view operations of uncertain meaning:

The default unit test also checks the literal boundaries of the command token, and when `meaning` is rewritten across files or is uncertain, changes in packages, document classes, and document candidate files will still make the compilation results expire.

```sh
cargo test --locked --test meaning_operands
cargo test --locked --test quoted_roots --test source_dependencies --test file_macro_operands
```

`futurelet` rename regression requires pdfLaTeX, XeLaTeX and LuaLaTeX, compares the look-ahead judgment and continuation results before and after the name change, covering direct targets, static `csname` targets and new Chinese names. Default tests check for cross-file edits, illegal targets, command to continue loading files, and signature boundaries; stdio regression checks for UTF-16 name ranges and document versions. The command snippet is also verified by real Neovim expansion:

```sh
cargo test --locked --test futurelet -- --include-ignored
cargo test --locked --test protocol futurelet_targets_return_versioned_cross_file_renames_over_stdio
cargo test --locked --test snippets -- --include-ignored
```

Unicode input error regression requires pdfLaTeX, XeLaTeX, and LuaLaTeX. Checks subfile characters, duplicate positions, emojis and zero-width connectors in 60 / 79 column logs and legacy / file line number formats, verifies clearing after pdfLaTeX explicit mapping, and the Unicode engine's missing word classification:

```sh
cargo test --locked --test unsupported_unicode -- --include-ignored
```

Special character fix regression requires pdfLaTeX, XeLaTeX and LuaLaTeX. Checked 60 / 79 column logs for bare `&` / `#` / `_` / `^` in subfiles, recompiled after applying quick fixes, checked that legal tables, mathematical superscripts and subscripts, escape characters, comments and verbatim content remained intact. Regression of excess alignment separators also covers `tabular`, math `array`, raw `halign`, legacy and file line number logs, expanded internal template frames, and macro parameter exclusions:

```sh
cargo test --locked --test special_character_fixes -- --include-ignored
```

The regression of extra right curly braces covers normal text, math mode, `begingroup` and `left` groups, single character deletion and recompilation are verified in three engines, 60 / 79 column logs, traditional and file line number formats, default and expansion contexts, while retaining legal groupings, escape characters and verbatim content, and excluding macro expansion sources. Run alone:

```sh
cargo test --locked --test special_character_fixes extra_closing -- --include-ignored
```

Duplicate superscript regression checks inline, display, grouping and comment continuation formulas across three engines, 60 / 79 column logs, traditional and file line number formats, and default and expanded contexts. Recompile after applying the empty basis operation, and also check that the bare parameters and delimited parameters will not be modified when the hidden macro expands the frame:

```sh
cargo test --locked --test special_character_fixes double_script -- --include-ignored
```

Sources, protocols, and three-engine regressions for math delimiter fixes see [Redundant right delimiter](right-delimiters.md#verify), [Missing right delimiter](missing-right-delimiters.md#verify), and [Missing delimiter parameter](delimiter-operands.md#verify).

Cross-file macro origin regression requires these three engines plus etoolbox, verifying that naked parameters, delimited parameters, duplicate declarations, and alias forwarding are not accidentally escaped edits in 60 / 79 column and traditional / file line number logs. Legacy and xparse environments declared by the parent file, optional parameters and the `\begin` alias also participate in the validation; the actual delimiter after zero parameters or completed parameters is recompiled after the fix. The replication regression also covers `let`, the kernel replication command, and the etoolbox replication interface, verifying that subsequent redefinitions of the source, environment copies, captured `begin`, and source operands are not mistaken for calls. The default test also covers loading timing, pausing parent files, dynamic sources, source code changes during compilation, and old repair rejections after macro file changes; the protocol test also covers duplicate declarations, aliases, and copy changes of Code Action and old clients:

```sh
cargo test --locked --test literal_macro_scopes -- --include-ignored
cargo test --locked --test protocol literal_build_fixes_recheck_cross_file_macro_changes_without_a_main_version_change
```

Source code file name movement regression requires pdfLaTeX, XeLaTeX and LuaLaTeX. Verify the first space, the last space of the trunk, the bare path and the quotation mark path, as well as the real input and navigation after the move:

```sh
cargo test --locked --test source_filename_renames -- --include-ignored
```

Default regression for NFSS font declaration and selection checks all parameters, annotations and nested settings for category six declarations and category seven parameterized selection commands, starred and unstarred forms of `\DeclareMathSizes`, fragment negotiation, preservation of existing parameters, and user signature overrides. Also check command boundaries, file bug fixes and local redefinition after parameterless `\selectfont` and its aliases. The stdio regression verifies the actual editor request using the `.fd` document; the external regression fills in the actual completion fragment, compiled in pdfLaTeX, XeLaTeX, and LuaLaTeX, checking the selected Computer Modern font, encoding, glyph, math font size, and baseline distance after the line spacing factor is applied:

```sh
cargo test --locked --test nfss
cargo test --locked --test protocol nfss_declarations_
cargo test --locked --test nfss real_completed_nfss_ -- --include-ignored
```

Font substitution regression requiring pdfLaTeX, XeLaTeX, LuaLaTeX and Computer Modern fonts, verification of long font names, font and size substitution in 60 / 79 column logs, and summary clearing after fix. Missing word regression requires XeLaTeX:

```sh
cargo test --locked --test font_substitutions -- --include-ignored
cargo test --locked --test missing_characters -- --include-ignored
```

The real box warning regression requires pdfLaTeX, XeLaTeX and LuaLaTeX, and uses a narrow log output width to verify the cross-line recovery and complete paragraph range of the Overfull warning; the result analysis regression also checks four types of indicators and the clearing behavior after typesetting repair:

```sh
cargo test --locked --test box_warnings real_ -- --ignored
cargo test --locked --test box_analysis -- --include-ignored
```

High float regression requires pdfLaTeX, XeLaTeX and LuaLaTeX, check warning lines, counts and maximum overruns of subfile charts in 60 / 79 column log, legacy and file line number formats, recompile after minifying content and confirm summary clearing. Default tests also cover decimal point wrapping, adjacent diagnostic boundaries, exact decimal comparisons, and numerical budgeting:

```sh
cargo test --locked --test oversized_floats -- --include-ignored
```

Real file URL renaming regression requires pdfLaTeX and hyperref. Verify that the PDF generated before and after the name change still retains the query parameters, page target and four-parameter link target:

```sh
cargo test --locked --test file_operations real_file_url_renames -- --ignored
```

True image extension declaration regression requires pdfLaTeX, graphicx and etoolbox, validating PNG first, explicit PDF, local vs. global scope, and compiled input after moving images and main document:

```sh
cargo test --locked --test graphics_extensions real_ -- --ignored
```

The global settings return covers `\global`, positive and negative `\globaldefs`, nested grouping, static aliases and naming calls of declarations and prefixes, and literal integers, and the actual read images before and after navigation and name changes are checked item by item. Directory scope regression also verifies multiple overwrites, partial restoration, global assignments, static aliases, and real compilation input before and after the image directory and main document are moved at the same time:

```sh
cargo test --locked --test graphics_paths real_ -- --ignored
```

Image call alias regression also requires pdfLaTeX, graphicx and etoolbox to verify pure aliases, static naming calls, recorder input after the image and main document are moved at the same time, as well as missing image location, spelling repair and recompilation. Also verify that the dynamically loaded source code after macro expansion restores image navigation, dependencies and renames after being discovered by the recorder, and continues to compile successfully after moving the image:

```sh
cargo test --locked --test indirect_graphics real_ -- --ignored
```

Quoted image path regression requires pdfLaTeX, XeLaTeX, LuaLaTeX, graphicx and etoolbox, verification of complete and segmented quotes, whitespace merging, recorder input after image movement, as well as missing image location, spelling fixes and recompilation. Search directory regression also checks the engine's differences in directory quotation marks, the actual images after the directory and main document have been moved at the same time, and the recompile after the missing images are repaired:

```sh
cargo test --locked --test quoted_graphics real_ -- --ignored
```

Default regression also covers alias completion for complete and uncompleted paths, aliases across input files and local packages, macro definitions and parameter boundaries in recorder sources, file exclusions for code display only, shared root file conflicts, exclusion of unexecuted and indeterminate calls, as well as scope for directory completion and missing image suggestions, shared source code ambiguity, expiration checks for candidate images, and versioned stdio file movement.

## listings code

`listings_environments` covers header options, name grouping, comment continuation lines and pseudo-close tags at the end of the first line, checks symbols, dependencies, versioned renaming and file movement at the end of the body and closing lines; also covers active line wrapping, CRLF, Unicode, precise closing strings, folding columns, linked editing, highlighting and unclosed repairs. Header signature regression covers complete and incomplete options, comment and parameter boundaries, project redefinition, internal reader overrides and unsaved settings refresh. Protocol regression incrementally modifies the header and real closing mark, checks diagnostics, symbols, signature help and refreshes the editing range:

`listings_inline` covers `*` and Unicode delimiters, the first literal closing bracket of curly braces, header whitespace and comment continuation lines, header command index, end-of-line recovery, redefinition, code isolation, completion fragments, existing code retention, hover, signature help and highlighting. It also covers known macro parameters, stable command copying and copy chains, pure static forwarding, nested curly braces in the macro definition body, escaping, comments and cross-line code, as well as normal scope, zero-argument commands, control words read as parameters and counterexamples of unknown macros. Protocol regression incrementally switches between asterisks, curly braces, options, missing terminators, macro parameter packaging, and copy macro parameter syntax, and checks versioned rename ranges and verbatim isolation; native compilation checks the boundary differences between pretokenization and ordinary text. Two sets of in-line tests also cover the static aliases of the reader itself, copy chains, alias renaming and the isolation of commands with the same name in the code, as well as loops, unknown sources, parameterized substitutions and declaration counterexamples as parameter consumption. The macro and environment definitions, required parameters, xparse delimited parameters and TeX delimited parameters have not been completed. Also check the diagnostics of missing outer brackets; both sets of inline protocol regressions incrementally delete and restore the outer right brackets to check diagnostics and code isolation.

```sh
cargo test --locked --test listings_inline --test listings_environments
cargo test --locked --test protocol listings_environment_
cargo test --locked --test protocol listings_inline_
```

Add `-- --include-ignored` to the first command after installing pdfLaTeX, Inline code regression also checks the actual execution of the header command, and the recovery of subsequent tags when native end-of-line errors are preserved.

## FancyVerb inline text

`fancyvrb_inline` overrides `Verb` in text commands in known macro parameters, copy wrappers and macro definitions, comment continuation lines, nested grouping and group delimiters, check tags and macro renames, file dependencies and file movement in normal format commands. It also covers ordinary scope, zero-parameter packaging, reading only headers and consumed format control words, ensuring that these scenarios continue to isolate verbatim text; the ordinary parameter index is restored after the project is redefined. stdio regression incrementally adds and removes macro wrappers, fvextra/minted package loading with `extra=false`, check diagnostics, versioned renames, file linking and symbol refreshing. Default and native regression also cover fvextra's balanced braces, multi-line code, save text, call-level switches, global and inline setting priorities, and group recovery. Static alias regression covers copy chains, traditional and TeX forwarding, copying after forwarding, alias renaming, isolation of commands with the same name in code, read rule replacement for reserved words, and undefined declaration counterexamples; native compilation comparison of labels and actual input before and after renaming under FancyVerb, fvextra default and `extra=false`. stdio incrementally adds, replaces and removes copy and forward declarations, checks body isolation and versioned alias editing. Header regression also covers fvextra's `%`/backslash delimiter, the effect of options on readahead, and comment continuation lines within macro arguments. `fancyvrb_completion` Verifies fragment negotiation, code and bare name preservation, save name hints, first unprotected right square bracket, comment and verbatim text rejection and project redefinition; native compilation fills in the actual completion fragment and checks save and read behavior and pseudo command isolation. Protocol regression synchronizes validation of incremental completion, hover, and signature help. `fvextra_scopes` Checks the setting priority of nested environments, single token names, group mixins, pseudo-closed flags and error recovery in verbatim environments, and compares symbols, file dependencies and rename scopes; native compilation verifies aux tags and actual input before and after renaming. Scope stdio returns incremental addition, removal and nested environments, checks unclosed diagnostics and refreshes each edit index. `escaped_inline` covers `EscVerb`'s escaping, comments, balanced and unclosed text, executable options, static aliases and redefinition, checks navigation, completion, prompts, semantic highlighting and versioned renaming; native compilation checks tags, file input and compiler errors under valid and invalid escapes to ensure that error text will not be executed. EscVerb stdio returns incremental switching aliases, redefinition, header reading mode and unfinished body.

```sh
cargo test --locked --test fancyvrb_inline --test fancyvrb_completion --test fvextra_scopes --test escaped_inline
cargo test --locked --test protocol fancyvrb_inline_
cargo test --locked --test protocol fvextra_environment_
cargo test --locked --test protocol escaped_inline_
```

After installing pdfLaTeX and XeLaTeX, add `-- --include-ignored` to the first command; native regression checks the actual execution of the text command, aux tags, compilation input, and output before and after renaming. The native regression of `minted` also compares the direct source code behavior of listings, minted and FancyVerb when only the header is used as a macro parameter, which requires listings and minted v3.

## FancyVerb environment signature help

`fvextra_environments` also covers `VerbatimWrite` and `VerbatimBuffer`'s header, text isolation, cross-platform line breaks, closing tags, folding and linked editing, semantic highlighting, dependencies and versioned renaming. Native regression checks the actual written text, buffer length and content, aux tags and unread pseudo input; scope regression ensures that these two environments will not close the outer environment in advance, and stdio header returns incremental completion options and text.

`verb_environment` covers `VerbEnv`'s single lines of code, header comments and grouped continuation lines, strict line closings, index and setting restoration after invalid closings, eponymous tags in lines of code, and unfinished input at every character boundary in Unicode and CRLF source. Compare the native regression of pdfLaTeX and XeLaTeX before and after renaming the aux tags and inputs when they are valid and when the error is closed, check the source code that continues to execute after the error and the subsequent repair environment scope; the stdio header regression synchronizes the check for incremental option prompts.

Default regression covers options, save names, and output file prompts for ten FancyVerb environments, including unfinished parameters, comment continuation lines, CRLF, Unicode, first unprotected closing bracket, active newlines when no options, project redefinition, and unsaved setting refresh. Protocol regression enters the header one by one, completes the parameters, adds the body and closes the end of the line, and checks the prompts and dependency isolation:

```sh
cargo test --locked --test fancyvrb_environments --test fvextra_environments --test verb_environment
cargo test --locked --test protocol fancyvrb_environment_header_
```

Add `-- --include-ignored` to the first command after installing pdfLaTeX,

## minted code

minted's default regression covers inline verbatim code, known macro parameters and pre-word boundaries in copied macros, normal scope and zero-argument wrapping counterexamples, cross-line and Unicode delimiters, complete and incomplete signature help in environment headers, comments and optional parameter boundaries, project redefinition and unsaved settings refresh, as well as file completion and movement, data/source code roles, global settings protection and missing file repair. Protocol regression checks incremental input of header parameters, isolation of code from hints at closing line ends, and the impact of code file content changes and in-build path editing on PDF/SyncTeX history:

```sh
cargo test --locked --test minted --test minted_environments --test minted_inputs --test missing_minted_inputs
cargo test --locked --test protocol minted_inline_
cargo test --locked --test protocol minted_environment_
cargo test --locked --test protocol static_code_
cargo test --locked --test protocol in_flight_code_path_
```

After installing pdfLaTeX with minted v3, add `-- --include-ignored` for the four test targets. Inline code uses verbatim mode with shell escape turned off, and XeLaTeX is used for pretokenization regression. Pseudo-commands and file inclusions in the code are not executed after checking comments, escapes, nested grouping, and tag renaming. Path, environment, and dual-mode missing file repair regressions also require `latexminted` on PATH. Highlight regression uses restricted shell escape and immediate highlighting modes to check for individual content markers in generated files, confirming that Python actually reads whitespace and Unicode filenames. Ambient regression checks for pseudo closing tags in header and body, verbatim and highlight modes, native errors at the end of line, versioned tag renaming, and actual aux output. See [minted code](minted.md) for the supported range.

## Documentation and Build diagnostics

Dynamic collection regression checks declarations and independent renaming of members, new keys with quotes, comments and incomplete list completion, hints for two parameters, command snippets, semantic declarations and selection ranges, item isolation, same name and new key conflicts, error forms and buffer updates. Real Biber processes the collection in the index through the normal mode control file, comparing the complete `.bbl` after the collection key and member key are renamed in sequence; this test does not run the biblatex macro. stdio overwrites old clients with versioned edits, scope moves, dynamic name removal declarations, restoring disk definitions after closing the document, and member edits:

```sh
cargo test --locked --test biblatex_sets -- --include-ignored
cargo test --locked --test protocol bibliography_dynamic_sets_
```

Extended name component regression covers full templates, field and entry boundaries, Unicode model fields, NFD keys, literals and macro expressions, field duplication and model collisions, mapped fields, source order and invalidation, overrun caching and guess-free fixes. True Biber validation AUTHOR and TRANſLATOR, Unicode unknown components, range moves and fix cleanup, string macros, model collisions and source maps; just Biber. stdio checks for pushes and pulls, incremental moves, macro expression scopes, key fallback for mapped fields, and fix cleanup:

```sh
cargo test --locked --test biber_namepart_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_nameparts_
```

Biber collection regression covers three native templates, Unicode model matching for field names, dynamic values, duplicate and incomplete fields, source history ordering and invalidation, normalized key ambiguity, out-of-limit sources and guess-free fixes. The real Biber test constructs a matching version of `.bcf` from the installed `--tool-config` and its `Constants.pm`, removes the tool's default source map and runs in normal mode, verifying native `.blg` and `.bbl`, range moves, repair clearing, model name collisions and fields generated by the source map; just Biber, no dependencies `biblatex.sty`. stdio checks for pushes and pulls, incremental editing, field and key fallbacks, successful builds with warnings, and fix cleanups:

```sh
cargo test --locked --test biber_set_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_sets_
```

Biber loop inheritance regression checks sources before fatal errors, cross-database read order, out-of-limit sources, field warning fallback after errors, history cannot be restored on subsequent inputs, normalized key ambiguity, field fallbacks, self-references, relative position caps, and full templates. Real Biber verifies normal and XDATA double-entry loops, self-loops, field-level XDATA, macros, source maps and alias references, and compares the results after source code movement and loop interruption. stdio simultaneously validates push and pull, incremental moves, dynamic fields and field-level XDATA, subsequent warning source failure, cleanup and success status after failed builds and repairs:

```sh
cargo test --locked --test biber_cycle_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_inheritance_cycles_
```

Biber alias conflict regression covers the full UTF-16 range of three complete templates, static and dynamic fields, cross-database read order, NFD and case, duplicate keys and candidate upper limits, historical invalidations, and 1500 mixed Unicode alias conflicts in a single line. Real Biber verifies scope moves and repair cleanups for three types of conflicts under two entry orders, as well as main document fallback for macro-expanded aliases and original scope after source map removal of IDS; stdio simultaneously checks push and pull, incremental moves, dynamic alias fallback to main document, relative locations, and guess-free fixes:

```sh
cargo test --locked --test biber_alias_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_alias_conflicts_
```

biblatex `ids` Alias regression covers static lists, ASCII field name matching, primary key and alias independent renaming, ambiguity rejection, project isolation, UTF-16/CRLF, semantic declarations and incremental updates. Real Biber Use `--tool --output-resolve-crossrefs` to verify that subentries still inherit the same title after renaming aliases and primary keys, and to verify that the double-quoted list is changed to the new alias with quotes; only Biber. stdio also checks for old clients and versioned edits, range moves, indexing for dynamic field removal, and restoring disk content after closing the document:

```sh
cargo test --locked --test bibliography_aliases -- --include-ignored
cargo test --locked --test protocol bibliography_ids_aliases_
```

Document log record boundary regression covers the input path, source code echo, and level/line number style text in Biber messages. In addition, the actual `INFO` record can be verified to establish the database role. Entry type diagnostics regression validates type-to-key mapping, line headers, ambiguities, source roles, and UTF-16; real BibTeX tests are recompiled after correcting types, and protocol use cases cover push and pull diagnostics:

```sh
cargo test --locked --test bibliography_log_records --test bibliography_type_diagnostics -- --include-ignored
cargo test --locked --test biber_data_sources biber_database_roles_
cargo test --locked --test protocol bibliography_type_warnings_
```

Default regression for BibTeX string abbreviations covering value expressions, completion, case, UTF-16, project isolation, shared references, and safe renaming. Real tool regression requires just BibTeX, comparing the full `.bbl` content before and after renaming across databases, preambles and self-references:

```sh
cargo test --locked --test bibliography_strings -- --include-ignored
cargo test --locked --test protocol bibliography_abbreviations_
```

Undefined string abbreviation regression covers two-line logs, exact ranges, input order, invalid declarations, ambiguous positions, and old fixes breaking down. Real BibTeX test recompiles after fix and checks warning disappears and `.bbl` content; stdio test also verifies push/pull diagnostics, versioned editing, and `workspace/applyEdit` for the old client:

```sh
cargo test --locked --test bibliography_string_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_string_warnings_
```

The duplicate document entry test checks case equivalence keys, line branch headers, same line ambiguity, database order, source roles, and relevant position upper limits. The real BibTeX regression verifies the actual error line number and the original location of the cross-repository, and the stdio test covers the exact range and `relatedInformation` in push and pull diagnostics:

```sh
cargo test --locked --test bibliography_duplicate_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_entries_
```

Biber field diagnostic regression covers invalid fields, entry type restrictions, dates and date ranges, data types, boundary and schema constraints, ISBN/ISSN/ISMN, full special file names, database roles, source code echo, levels and record integrity. Also checks the full scope of literals, nulls, string concatenation, field attribution, inheritance and ambiguity fallback, Unicode normalization collisions, CRLF, cache updates, and 6000 diagnostics per line. There are also 1500 single-line entries for input and model field matching, and 7500 mixed diagnostics for item-by-item checking, covering complete Unicode folding, original spelling positioning separately, and key fallback after collisions. Default year, month, ISBN and malformed range warnings without filenames Additional checks for full previous history, cross-repository normalization key uniqueness, unreadable candidates, upper limit on number of reads and sources, symlink deduplication, and log literal-like boundaries within values. Real Biber uses a minimal custom model to verify combined/decomposed characters, ß/ẞ, two sigmas, dotted/undotted I's, and ligatures, checking value moves, fix clearing, model field collisions, and unknown field independent positioning; default mode additionally checks `IſBN` aliases and their collision with `ISBN`, month fixes, and `PAGES` / `Pageſ` Independent primitive positioning of malformed range. stdio simultaneously verifies input and model namespace push and pull, incremental save, warning mode switching, default field five-parallel warning clearing and successful build:

```sh
cargo test --locked --test biber_field_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_field_warnings_
cargo test --locked --test protocol biber_unqualified_field_warnings_
```

Biber XDATA regression covers three complete templates, section boundaries, native levels, static list exact items, macros and splicing, inheritance, repeated fields and canonical equivalence targets, unclosed entries, previous history, and cross-library key uniqueness. 4500 XDATA references mixed with 1500 year warnings in a single row of 1500 entries validating the full UTF-16 range. Real Biber checks for two messages for missing targets, wrong target types, list item movement, cleanup after completion, as well as actual macro expansion, source maps, key fallbacks for duplicate fields and targets. stdio validates pushes, pulls, moves after saves, dynamic field fallbacks, and fix clears:

```sh
cargo test --locked --test biber_xdata_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_xdata_warnings_
```

Field-level XDATA testing covers four native messages, field and key boundaries, native source location text, single literals and complete expressions, input and model namespace distinctions, canonical equivalent field collisions, inherited/mapped/incomplete field fallbacks, and history invalidations. Single row of 1500 entries, 6000 mixed field warnings validating UTF-16 ranges individually, including `Straße` and `STRASSE` at separate locations. Real Biber verifies position repair, source field completion, type repair, value movement and clearing of name lists and ordinary lists, and confirms the positioning of macro original expressions, source map generated fields and custom models. stdio covers four types of warning juxtaposition, push and pull, incremental movement, mapped field rollback, clear after save and successful build:

```sh
cargo test --locked --test biber_xdata_field_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_granular_xdata_warnings_
```

Biber relational reference regression covers four native relational templates, section and level boundaries, single value and list targets, independent ranges across fields for the same target, dynamic/inherited/duplicate field fallback, previous input order, historical invalidation after cache creation, and normalized key collisions. 6000 normal relational warnings and 1500 XDATA warnings on a single line of 1500 entries to itemize UTF-16 locations. Real Biber verifies the target movement and completion clearing of four relationships, as well as the key fallback of string macros and actual source maps; stdio simultaneously checks the push, pull, incremental save, dynamic field fallback and successful build status of four types of diagnostics:

```sh
cargo test --locked --test biber_relation_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_relation_warnings_
```

Biber name regression covers three native templates, native levels, normalized key uniqueness, special key quotes, filenames and similar suffix text in names, previous input order, unreadable or overrun sources, no-guess name fixes, and the full UTF-16 key range with 3000 mixed errors and warnings on a single line. Real Biber validates that year warnings after two name errors are still pinpointed, and checks for LaTeX decoded names, contiguous `and`, Unicode custom model name fields, range moves, and fix clears. Unknown or incomplete errors continue to invalidate subsequent history. stdio checks both push and pull, field position after both errors, key movement, success status on warning only, and clear all:

```sh
cargo test --locked --test biber_name_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_name_errors_
```

Biber date override regression checks the uniqueness of the original value being overridden and the associated date, complete templates, previous input history, normalized key collisions, inheritance boundaries, no automatic deletions or overwrites, and primary and associated ranges of 1500 entries per row and 3000 warnings. The real Biber verifies the movement and component consistency of the two field orders and date ranges respectively, and the actual source map verifies the original range after the date, year, or both are renamed and falls back. stdio simultaneously verifies push and pull, incremental editing, associated date disappearance, key fallback when original year is missing, and fix cleanup:

```sh
cargo test --locked --test biber_overwritten_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_overwritten_fields_
```

Biber entry type regression checks for native record integrity, special filenames, exact entry attribution, normalized key collisions, unclosed bodies, unconfirmed and unreadable origins, key fallback on type inconsistencies, and no automatic `misc` replacement. Unicode folding covers combined/decomposed forms, ß/ẞ expansions, Greek two sigma, dotted vs. non-dotted I, ligatures, 3000 entries per line and 6000 type and field warnings to verify native UTF-16 ranges on an item-by-item basis. Real Biber verifies the range move item by item, type repair and clearing, and uses the actual source map to change the valid `book` to invalid `boook`, confirming that only the key is marked; stdio verifies the type move, rollback and clearing when the original type is inconsistent with the mapped type, all covering push and pull:

```sh
cargo test --locked --test biber_type_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_type_warnings_
```

Biber required fields regression distinguishes single fields from groups of fields with at least one item filled in, checks original requirements, full special filenames, database roles, single field and group template boundaries, Unicode normalization collisions, source failures, not borrowing from other libraries, and post-save range refresh and guessless insertion fixes. A single row of 1500 character combination keys corresponds to 3000 single field and alternative group warnings, and the original UTF-16 range is checked item by item. Real Biber verifies empty entries, moves entry keys, and clears warnings after completing required requirements; stdio simultaneously verifies push, pull, incremental save, and successful build status:

```sh
cargo test --locked --test biber_required_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_required_fields_
```

Biber duplicate key regression covers truncated keys after no line number ambiguity, all candidates for the same file, cross-library order, exact case, missing and overrun sources, special file names, symbolic links, record levels, and syntax errors. Unicode regression checks combining/decomposed forms, canonical equivalence collisions of Hangul and combining markers, compatible character distinctions, and candidate upper bounds shared by multiple canonical equivalent spellings. It also verifies the full range of 3000 duplicate keys and 3000 pairs of uppercase and lowercase keys in the same Unicode long line, the CRLF position after the per-key upper limit, and stops cross-database correlation when an error occurs after the cache is established. Real Biber tests compare the default mode to the actual output of `--noskipduplicates`, and verify position moves and fix cleanups of canonical equivalence declarations; stdio also verifies ASCII and canonicalized Unicode cross-library relative positions, delta saves, successful builds, and pushes and pulls:

```sh
cargo test --locked --test biber_duplicate_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_duplicate_warnings_
```

Biber database key case warning regression covers case-sensitive canonical equivalence keys, cross-database read order, primary key normalization collisions, multiple canonical equivalence candidates for associated keys and associated position caps, error and truncation history, origin roles, full special filenames, symbolic links, UTF-16/CRLF, and bounded reads. The real Biber test confirms that both spellings are retained in the output, the source code combination characters accurately correspond to the log decomposition characters, diagnostics is moved after saving, and cleared after repair; stdio verifies the push and pull of ASCII and Unicode, cross-library related locations and successful build status:

```sh
cargo test --locked --test bibliography_case_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_case_warnings_
```

`crossref` Error and nested warning tests check the entries to which the fields belong, literal value ranges, duplicate keys and fields, dynamic values, source roles, log truncation, scanning upper limits and related parent entry locations. Real BibTeX regression verifies missing parent entries, preceding unreferenced, trailing, explicit references, wildcard references, null values, peripheral whitespace, as well as ordinary nesting, self-references, cycles, dynamic parent references, and missing ancestors. Nested warnings and missing ancestor errors must be located independently and retain their respective levels; unmatched or missing continuation lines cannot swallow the next record. stdio covers push, pull, build success with warnings, coexistence of warnings and errors, and recompile to clear diagnostics after repair:

```sh
cargo test --locked --test bibliography_crossref_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_crossref_errors_
cargo test --locked --test protocol bibliography_nested_crossrefs_
```

Document outline regression checks complete entries against string ranges, field hierarchies, assignment ranges, unfinished constructs, comments and nested literals, duplicate keys and fields, UTF-16/CRLF, language types, and field affiliations for large files. Static IDS aliases check field sub-symbols, associated primary keys, duplicate declarations and dynamic value exclusions, as well as a full selection of 2000 Unicode aliases in a single row. Real BibTeX regression validates fieldless brace entries, linebreak keys, and quote keys; stdio covers hierarchical and tiled clients, unnamed and other extension documents, incremental editing, and old field purge, plus checks for alias searches, static to dynamic purge, and document recovery on disk.

```sh
cargo test --locked --test bibliography_symbols -- --include-ignored
cargo test --locked --test protocol bibliography_outlines_
cargo test --locked --test protocol bibliography_alias_symbols_
```

Quoted citation key regression covers various citation parameters, handwritten documents, item relationship lists, double-quoted fields to curly braces, UTF-16/CRLF, comment and description retention, dynamic value exclusion, item isolation and shared ambiguity. stdio validates versioning versus traditional workspace editing, post-completion incremental updates and cross-file renaming; real pdfLaTeX/BibTeX regression for consecutively renaming parent entries, compares complete `.bbl` and checks for crossref inheritance and missing reference warnings.

```sh
cargo test --locked --test quoted_citations -- --include-ignored
cargo test --locked --test protocol quoted_citation_keys_
```

Selected range regression checks for full citation keys, tag lists and range endpoints, descriptions/page numbers/keys/field roles for structured citations, BibTeX relational values and entry hierarchy, cross-line commands and trailing text, CRLF comments, UTF-16 and incomplete parameters. stdio regression validates key name and field delimiter incremental modification after range refresh, close and reopen, position order and invalid proxy position rejection.

It also covers BibTeX ordinary fields, string assignment and preamble literal content/delimiters/operands/splicing expressions/complete construction, two types of entry delimiters, protected quotation marks and percent signs, cross-value punctuation isolation, trailing whitespace, empty expressions and unfinished input. Real stdio editing converts between quotes, nested braces, splices and unclosed values, checks range refreshes and reviews abbreviation navigation; real BibTeX string regression verifies that existing output is consistent.

```sh
cargo test --locked --test selection_ranges
cargo test --locked --test protocol selection_ranges
```

`entrydata` Default regression covers warning attribution for normal and asterisk calls, single key and code roles, comments and cross-line completion, Unicode/quote keys, semantic tokens, selection ranges, dynamic key and list exclusions, aliases, item isolation, shared meaning ambiguity, and nested normal references. stdio returns to cover navigation, hover, completion, signature help, references, highlighting, two workspace editing formats, and clears the original references after the increment is changed to a key list:

```sh
cargo test --locked --test biblatex_entrydata
cargo test --locked --test protocol biblatex_entrydata_
```

Entry condition query regression covers single-key editing of two commands, two branches and wait parameters, fragments, user signatures, comments/UTF-16, semantics and selection ranges, dynamic keys and unsupported calls, alias item isolation, shared ambiguity and nested reference warning attribution. stdio checks two types of workspace editing, three parameter roles, reference clearing and closing and reopening after increments are converted to lists:

```sh
cargo test --locked --test biblatex_entry_queries
cargo test --locked --test protocol biblatex_entry_queries_
```

Data selector regression covers field/literal list/name list candidate isolation, case and in-word substitutions, annotations and unfinished parameters, UTF-16/CRLF, optionally formatted bracket boundaries, precision hover, and independence from citation keys for six commands. Also check the exclusion of dynamic parameters, verbatim content, command name operands, user-defined and alias rewrites, and verify candidate flushes after buffer rewrites. stdio applies actual incremental completion in normal text and fragment clients, checking hover, original key navigation, three parameter types, user redefinition clear and close-reopen recovery:

Field reading and three undef checks additionally cover the independence of the first parameter from the two branches, signature help, counter/alias counter/theorem and newif derived command conflicts, as well as declaration isolation of other items; the same stdio regression is extended to all eleven commands.

Field comparison regression checks for independent replacement of two field parameters, string/command operands and branch boundaries, the second field after the dynamic first parameter, comment continuation lines, and unfinished parameters. Five compare commands simultaneously check list type isolation, four-parameter signatures, nested calls, user-defined and `newif` overrides, and buffer recovery.

Macro comparison targets also cover plain text completion of command names, empty prefixes, uncompleted groups, and the compared documents, statements, and verbatim commands are not executed. Reuse the `operand_completion` regression to check the execution branch and the calling fragment after explicit redefinition; stdio also checks the comparison target's mid-word substitution and outer four-signature help.

`ifentrytype` Additional checks on the distinction between default output types and special entries/input aliases, shared type descriptions, independence from citation keys and fields with the same name, cross-line comment editing, branch signature help, user definitions and recovery after `newif` overwrite.

The derived command regression also checks that command name completion uses pure names, removes built-in descriptions and signatures that no longer apply, and restores documentation argument snippets after removing overrides.

```sh
cargo test --locked --test biblatex_data_selectors
cargo test --locked --test biblatex_data_comparisons
cargo test --locked --test biblatex_entry_types
cargo test --locked --test protocol biblatex_data_selectors_
```

Continuous latexmk compilation regression coverage of existing products, error location preservation after cache failure, successful no-op compilation, corrected diagnostic clearing, and custom output directories and job names; the real tools part requires pdfLaTeX and XeLaTeX. The default test also checks that startup failures, timeouts, and other tool failures do not misuse old logs, and verifies duplicative diagnostic release and corrected clearing via stdio:

```sh
cargo test --locked --test repeated_build -- --include-ignored
cargo test --locked --test protocol repeated_latexmk_failures
```

Real biblatex reference and recompilation request regression also need to install biblatex and logreq, and use pdfLaTeX and BibTeX:

```sh
cargo test --locked --test biblatex_resources real_ -- --ignored
cargo test --locked --test biblatex_rerun real_biblatex -- --ignored
cargo test --locked --test biblatex_commands real_biblatex -- --ignored
cargo test --locked --test biblatex_data_citations real_ -- --ignored
cargo test --locked --test biblatex_categories real_ -- --ignored
cargo test --locked --test multicite real_biblatex -- --ignored
cargo test --locked --test volume_citations real_volume -- --ignored
```

Document resource regression verifies the pdfLaTeX / BibTeX compilation of global resources after the file and citation key are renamed, as well as the precise resource name in the Biber control file; the latter only generates `.bcf`, without running Biber.

Bibliography categories Regression verifies that after renaming a citation key, the classification name remains unchanged, assignments in the auxiliary file are updated simultaneously, and references filtered by classification still generate PDFs. The default tests also cover classification key editing, signature help, completion, diagnostic isolation, shared source code ambiguity, and versioned stdio renaming.

There are two sets of real tools for missing document resource repair:

```sh
cargo test --locked --test biblatex_resource_fixes real_biber -- --ignored
cargo test --locked --test biblatex_resource_fixes real_bibtex -- --ignored
```

The former requires Biber, and uses `--tool` to verify that the real missing file log and the database can be read after repair, but does not verify the complete PDF compilation; the latter requires pdfLaTeX, BibTeX, biblatex and logreq, and verifies that the PDF is regenerated after applying the file name repair. Default tests also cover special filenames, option retention, versioned editing, expiration fixes, and the stdio protocol.

Biber non-standard suffix database also has real log regression:

```sh
cargo test --locked --test biber_data_sources real_biber -- --ignored
```

This test uses Biber `--tool` to read different suffix and extension-free databases, verifying dependency discovery and localization of duplicate citation key warnings; no matching version of biblatex is required, and no PDF is generated. Default protocol regression covers database roles, navigation, versioned renames, file moves, and input changes during compilation.

General document warning regression covers duplicate keys in the same database and across databases, full case, unreadable or over-limit databases, symbolic link deduplication, special suffixes, UTF-16/CRLF, independent line numbers and multiple warning databases. The field warnings generated by the real BibTeX validation style are updated after the entry is moved and disappear after the fields are completed; stdio overrides push/pull, switches between unique and ambiguous states after saving edits, and is cleared after repair.

```sh
cargo test --locked --test bibliography_log_records -- --include-ignored
cargo test --locked --test protocol generic_bibliography_entry_warnings_
```

Duplicate field diagnostics additionally cover 3000 Unicode values in the same row, cross-row values and Unicode field names, interleaved database warning groups, and fallback in the event of a syntax error after the first warning. Real BibTeX also verifies that the 3000 ignored fields are located one by one, the first value is still used for output, and the diagnostics is cleared after removing duplicate fields; stdio checks that multiple warnings in the same group are moved together after saving the incremental edit, the relevant first assignment is retained and cleared after repair.

```sh
cargo test --locked --test bibliography_field_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_fields_
```


## Added new editing and external tools

```sh
cargo test --locked --test extra_symbols --test refactor --test expl3
cargo test --locked --test expl3 real_expl3_renames -- --ignored
cargo test --locked --test distribution -- --include-ignored
cargo test --locked --test protocol formatting_with_real_tex_fmt -- --ignored
cargo test --locked --test protocol chktex_with_real_tool -- --ignored
```

Real tests of expl3 use pdfLaTeX comparison functions and expansion results before and after variable renaming; release tests require kpsewhich. True formatting and static checking require tex-fmt and ChkTeX respectively. The remaining ad hoc tests do not rely on external tools.


Single-key testing of explicit taxonomic queries simultaneously checks taxonomic namespace isolation, four-argument hints, list/dynamic key exclusions, named calls/pure forward aliases, shared database owner, and diagnostic attribution. Real biblatex control requires pdfLaTeX, BibTeX and biblatex installed:

```sh
cargo test --locked --test biblatex_entry_categories
cargo test --locked --test protocol biblatex_entry_queries_keep_branch_roles
cargo test --locked --test biblatex_entry_categories real_entry_category_query -- --include-ignored
```


Project semantic testing of direct reference keys covers references after redefinition parameters, user parameter/token operands, static macro bodies, literal examples, shared consumers, index budgets, and standard formatting commands. A real pdfLaTeX check confirms that renaming the tag does not change the normal text output of the redefined reference:

```sh
cargo test --locked --test direct_reference_keys
cargo test --locked --test protocol direct_reference_keys_refresh
cargo test --locked --test direct_reference_keys real_label_rename -- --include-ignored
```


biblatex callback tests check official code/selector parameters, all branches, asterisks, unclosed calls, named calls and aliases, user redefinition and completion preservation. Native comparison requires pdfLaTeX, BibTeX and biblatex. Verify that the import/reference/entry key callbacks are actually executed after the name is changed:

```sh
cargo test --locked --test biblatex_hooks
cargo test --locked --test biblatex_hooks real_biblatex_callbacks -- --include-ignored
cargo test --locked --test bibliography_categories real_biblatex_category -- --include-ignored
```
