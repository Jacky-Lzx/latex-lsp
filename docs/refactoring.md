# Structural refactoring and fix all

`textDocument/codeAction` provides the following operations:

- `refactor.rewrite`: Wrap the selection into `\emph{...}`, `\textbf{...}` or `quote` environments.
- `refactor.extract`: Extract the text selection in the main document as no-parameter `\newcommand`, insert a statement before `\begin{document}`, and replace the original text. Unused command names within the project are automatically selected and can later be changed to the desired name using symbolic renaming.
- `source.fixAll`: Merge static syntax fixes that have no ambiguity in the current file and no conflicting edit scopes. Only returned when the client explicitly requests this type of operation, can be used for save-time fixes; no spelling suggestions or candidate fixes for compiler errors are automatically selected.

The refactoring only handles the top-level normal text and the complete set of common text style commands. Some control sequences, command parameter internals, formulas, special environments, comments, verbatim codes, raw macro parameters, and unbalanced grouping do not provide reconstruction to avoid changing parameter boundaries. Command extraction requires the current file to have the `document` environment; TeX macro expansion and arbitrary catcode settings are not emulated.

Parameter boundaries reuse the current project's static signature, including single-token parameters without curly braces, optional parameters, TeX/xparse-delimited parameters, flags, and static `csuse` / `csname` calls. Pure forward aliases, unsaved cross-file definitions, ambiguities, and unknown calls use the same conservative checks. For example, when `\take Hello` reads the first `H`, `Hello` will not be packaged into a new command or group; the text after selecting the consumed `H` can still be reconstructed. Single-token operands such as `noexpand` also retain the original boundaries, and the complete selection of top-level style calls continues to be available.

When extracting commands, the direct control words, user declarations, environment and other derived control words of the current project, the static name of the named interface and the literal `csname` are collected at once. The environment call also retains its start/end command name. Cross-file and unsaved statements participate in the check, and the names of independent projects do not affect each other. Inherit a budget of up to 26 candidate names; when no names are available, only the body wrapping operation is retained. This check does not perform dynamic name expansion.

Context checks differentiate between single-dollar and double-dollar formulas, inline/display math controls and their static forward-only aliases (including alias chains, named calls (including `csuse{(}` / `csname (\endcsname`) and cross-file unsaved definitions), explicit `begingroup` / `endgroup` and `bgroup` / `egroup` grouping, and identifies grouping and environment boundaries for static named calls and pure forwarding aliases. The body of formulas, groups, and environments can still be reconstructed after they are completely closed; parameter data, declaration bodies, and token operands do not create execution boundaries. Structural reconstruction is skipped when pairing cannot be confirmed or mathematical analysis exceeds the job budget.

Redefined style commands in the project cannot be extracted by built-in single-argument calls. The wrapping operation checks the original meaning of `emph`, `textbf`, `quote` / `endquote` respectively, and the extraction operation requires that `newcommand` has not been rewritten; only operations that rely on the rewritten command are hidden. The text entry is confirmed through static environment calls, and is conservatively skipped when `begin` / `end` or mathematical control characters are rewritten.

Restructuring is not provided when the project index is incomplete, or when the analysis scope exceeds 16 MiB of source code, 100,000 commands, or 4096 definitions. This restriction does not change existing quick fix entries with explicit `source.fixAll`.

Editing uses client-negotiated document versions. When an old client applies an operation via `latex.applyCodeAction`, the server recalculates and checks the document version and operation type; expired or forged operations will not be performed.

The server supports standard `codeAction/resolve`. When the client declares `codeActionLiteralSupport`, `dataSupport: true` at the same time, and `resolveSupport.properties` contains `edit`, refactoring, quick fix, and `source.fixAll` return operations with `data`, and the edit is returned in the parse request. Title, type, diagnostics, preferred tags, and other legacy fields remain unchanged; remaining clients continue to receive inline editing or existing application commands. The recompilation operation retains the original command and warning verification process.

The parsing will check the content of the project file against the version, scope, settings and operating identity, and then recalculate the edits according to the original scope and diagnostic context. `-32801` is returned when the document is updated, related project definition changes, diagnostic repair disappears, or the operation is rewritten, and the editor should re-request the operation; `data` with an illegal structure returns `InvalidParams`. Buffer updates from other independent projects will not invalidate the current project's operations. Operations without parsed data are returned unchanged.

verify:

```sh
cargo test --locked --lib code_action_resolve
cargo test --locked --test protocol code_action_resolution
cargo test --locked --test refactor --test rebuild_actions
cargo test --locked --test refactor real_refactors_after_consumed -- --include-ignored
cargo test --locked --test refactor real_extractions_preserve -- --include-ignored
cargo test --locked --test refactor real_refactors_after_balanced -- --include-ignored
cargo test --locked --test protocol structural_refactors_track_math_groups
cargo test --locked --test protocol refactor_math_alias_contexts
cargo test --locked --test refactor real_formula_aliases -- --include-ignored
```
