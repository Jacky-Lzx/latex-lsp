# Static diagnostics and ChkTeX

Undefined static tag references and bibliographic references display warnings when editing without running TeX. Check source code and bibliographic databases using the current project, supporting reference lists, range references, structured biblatex references, and IDS aliases. Dynamic keys, comments, and verbatim codes are not checked as static keys. When you correct a name or add a statement, the warning is updated with the edit; you can use the spelling fix for the name within the project. Duplicate static warnings are omitted when there is already a compiler-acknowledged reference warning at the same location.

Reference locations reuse project-level command and parameter semantics. Redefined reference commands, parameter data for user macros, declaration bodies, literal conversions/tests, and single-token operands are not treated as actual references; references in normal text-style parameters continue to be checked. Using unsaved definitions of related files, redefinitions of independent projects do not affect each other. User macros are not expanded here, so references resulting from the replacement body when they are called are not predicted. When the project index is incomplete, or when analyzing more than 16 MiB of source code, 100,000 commands, or 4096 declarations, this type of static diagnostic is skipped; the compiler results remain visible.

Reference calls to static `csuse` and literal `csname` use the same check, for example `\csuse{ref}{missing}`, `\csname cite\endcsname[description]{paper}`. Key scopes only cover names in actual arguments, reference lists, ranges, optional descriptions, and structured biblatex references follow the rules of direct calls. When the named interface or target command is redefined by the project, the corresponding semantics will be skipped; dynamic command names and dynamic keys will not be expanded.

Duplicate tags, citation keys, and PDF targets in the same document continue to report `Duplicate key` and use the stable codes `duplicate-label`, `duplicate-bibliography-key`, and `duplicate-pdf-destination` respectively. Each duplicate position links to the first declaration of this namespace via `relatedInformation`, supporting unsaved documents, Unicode and CRLF. After the first declaration is deleted or modified, the associated position is updated with the buffer; different namespaces with the same name do not conflict with each other. Cross-file conflicts within the project are still reported by the existing `duplicate-definition` diagnostic.

```json
{
  "latex": {
    "diagnostics": {
      "undefinedReferences": true,
      "undefinedCitations": true,
      "duplicateBibliographyFields": true,
      "mathDelimiters": false,
      "chktex": {
        "enabled": false,
        "executable": "chktex",
        "arguments": ["-n1"],
        "timeoutSeconds": 10
      }
    }
  }
}
```

Duplicate fields in the document buffer display a warning when editing by default with code `repeated-bibliography-field` and source `texquill`. Field names within the same entry are compared ASCII case-insensitively; each repeated assignment tags the field name and links the first valid assignment with related information. Fields with the same name in different entries do not affect each other, and examples in field values or the body of special entries are not checked.

The check requires that the field name, assignment, and expression be complete; complete assignments can still be checked when the entry's closing delimiter has not been entered. Clear diagnostics immediately after correcting field names or removing duplicate assignments. When there is already a precisely located compiler duplicate field warning, the corresponding static warning is omitted; compiler messages that only target lines or whose sources have not been confirmed will not block editing checks. Setting `diagnostics.duplicateBibliographyFields: false` turns off static checking and compiler results continue to be displayed.

This check only reports duplicates and does not infer field values or retention rules for different document backends, nor does it automatically remove assignments. Supports editor-specified documentation language types, unnamed buffers, and project-confirmed documentation input roles. When a single row contains a large number of Unicode fields, a position scan is used to avoid repeatedly calculating UTF-16 coordinates field by field.

verify:

```sh
cargo test --locked --test static_bibliography_fields
cargo test --locked --test protocol bibliography_field
```

Math delimiter checking is enabled via `diagnostics.mathDelimiters: true` and is disabled by default. Checking the current LaTeX buffer for `$`, `$$`, `\(` / `\)` and `\[` / `\]` reports the following warning when editing (source: `texquill`):

- `unclosed-math`: A formula that is not closed before the end of the root file.
- `unexpected-math-end`: There is no explicit closing delimiter corresponding to the starting point.
- `mismatched-math`: The delimiter of the inline formula and the displayed formula are mismatched, and the associated starting position is associated.
- `nested-math`: Use the explicit start delimiter directly in the formula, and the association already has a starting point.

Checks known parameter syntax in reused projects: dollar, math control characters, and static named calls read as parameters by user macros are not used as formula boundaries, and formulas in normal text styles continue to be checked. The mathematical control character, alias chain, `csuse` and literal `csname` call without parameters are paired according to the final built-in meaning; the diagnostic mark is actually called, and the actual opening position is associated with the mismatch. Related cross-file unsaved definitions participate in the analysis, and independent projects do not affect each other.

Check pairing by inline/display mode. Basic LaTeX accepts `$x\)`, `\(x$`, `$$x\]`, and `\[x$$`; different packages can change the meaning of these commands, and the relevant additional constraints are checked by the actual compiler. Comments, verbatim codes, literal conversions/tests, token operands, command/environment defined parameter headers and replacement bodies, supported expl3 definitions, `csname` names, and file/reference parameters are not scanned as formula bodies. The existing math environment, `ensuremath`, and text box content in formulas independently select math mode and process them as opaque areas.

Cross-file input and its static pure forwarding aliases form an indeterminate boundary; the shared file is the union of the boundaries of all consumer items. The check does not infer mathematical patterns at the beginning and end of subfiles, so isolated starts and stops that may span files are not reported; explicit mismatches written out clearly within boundaries are still reported. Skip checking if character class modification or delimiter redefinition exists in the project. Skip checking for a shared source when its math calls or parameter interpretations are inconsistent between consumers. Conditional branches invalidate the current pattern. A maximum of 128 items are reported per document, and related indexes are only built when checking is enabled. Skip if the project index is incomplete, or if a single project exceeds 16 MiB of source code, 100,000 commands, and 4,096 declarations; the delimiter parameter query for a single source is limited to an estimate of 4,000,000 command accesses, and compiler diagnostics are retained when exceeded.

This check does not expand arbitrary macros, does not check package internal implementations, and does not automatically repair delimiters. Projects that turn math mode on/off via runtime macros can remain turned off. Once you fix the buffer or turn off the settings, push and pull diagnostics clear.

```sh
cargo test --locked --test static_math
cargo test --locked --test protocol math_delimiters
cargo test --locked --test protocol math_semantics_diagnostics
cargo test --locked --test static_math -- --include-ignored
```

The last one uses pdfLaTeX and amsmath to compare legal mixtures of notations, defined parameter data, and actual rejected formulas. Shared files, directory configuration, UTF-16/CRLF positioning and on-the-fly cleanup are covered by default tests.

ChkTeX is turned off by default. When turned on, it checks the current buffer when saving and runs in the directory where the document is located, so the local `.chktexrc` can be read. `arguments` is used to append warning switches or rc file parameters. The server then appends the output formats, `-I0` and `TabSize=1`, and does not follow `input` to read other files. The output is converted to UTF-16 range and the diagnostic source is `chktex`.

ChkTeX warning 46 (`Use \( ... \) instead of $ ... $.`) offers a quick fix
that replaces the selected inline formula's two dollar delimiters with `\(`
and `\)`. Formula content, comments, whitespace and line endings are preserved,
including multiline formulas. Request code actions at the warning after saving;
save again after applying the fix to refresh ChkTeX results. Enable the checker
and warning in `.texquill.json` if needed:

```json
{
  "diagnostics": {
    "chktex": { "enabled": true, "arguments": ["-w46"] }
  }
}
```

The fix requires a current server-side warning and a confirmed local pair.
Display math (`$$...$$`), escaped dollars, comments, verbatim text, macro
definition bodies and parameter tokens are excluded. Uncertain pairing,
redefined math delimiters, changed catcodes and incomplete project analysis
also suppress the fix. It supports versioned edits, `codeAction/resolve` and
legacy `latex.applyQuickFix`; it is not included in `source.fixAll`.

```sh
TMPDIR=/private/tmp cargo test --locked --test chktex_actions
TMPDIR=/private/tmp cargo test --locked --test protocol chktex_46
TMPDIR=/private/tmp cargo test --locked --test protocol chktex_46_real_tool -- --include-ignored
```

A timeout, closing the connection, or shutting down the server terminates the checking process. If you continue to edit, close the document, or modify the configuration during the check, the old results will be discarded; the previous ChkTeX results will be cleared immediately when editing and rechecked next time. Errors are reported via logs when the tool is not installed, other diagnostics continue to be available.

Static diagnostics do not perform macro expansion or conditional branching. Keys dynamically generated by runtime macros or external projects may require corresponding static checking to be turned off; the distribution filename database itself does not introduce tag or document definitions.
