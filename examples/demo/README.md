# TeXquill LSP playground

Open this directory as your editor workspace, attach `texquill --stdio`, and
start with [main.tex](main.tex). The numbered comments identify places to try
each feature. The project is valid as shipped; diagnostic exercises are
commented out so they can be enabled individually and undone afterwards.

The project settings select `main.tex` as the build root, use pdfLaTeX, enable
reference/citation diagnostics and inlay hints, and disable automatic builds
on save. Editor support determines which LSP actions are exposed and how to
invoke them. In particular, linked editing, inlay hints, on-type formatting,
color pickers and build code lenses may need to be enabled in the client.

## Files

- [main.tex](main.tex): the entry point and numbered editing exercises.
- [definitions.tex](definitions.tex): documented macros, an environment, a
  theorem, a paired delimiter, a color, a counter, a length and a boolean.
- [sections/introduction.tex](sections/introduction.tex): cross-file labels,
  macro/environment calls, citations and a PDF inverse-search target.
- [references.bib](references.bib): three citations, a shared string
  abbreviation, and DOI/URL links.

## Walkthrough

| Feature | Where and what to try |
| --- | --- |
| Completion and item documentation | In exercise 1, remove a key inside `ref` or `cite` and request completion. Search citations by key, title or author. Try command prefixes such as `\demo`, an environment name after `\begin{`, or a path inside `\input{`. Restore the source afterwards. |
| Hover and signature help | In exercise 2, hover `\demoterm` for its cross-file documentation. Request signature help while editing its optional category or either required argument. Also try `demopanel`, `\norm*{...}` and built-in commands. |
| Definition and declaration | Jump from `sec:intro`, `knuth1984`, `\demoterm`, or `demopanel` to the corresponding declaration. Jump from an input/bibliography path to its file. |
| References and highlights | Find references to `sec:intro`, `\demoterm` or `knuth1984`; highlight occurrences in the current document. |
| Cross-file rename | Preview renames of a label, citation key, macro or custom environment. Exercise 4 adds counter, length, boolean and named color symbols. In the bibliography, rename the `aw` string abbreviation independently of citation keys. |
| Macro call hierarchy | In exercise 2, prepare the hierarchy on `\demowelcome` and inspect outgoing calls to `\demoterm`. On `\demoterm`, inspect incoming calls from the wrapper and file-level uses. |
| Linked environment editing | In exercise 3, edit a `demopanel` begin/end name with linked editing enabled. Both names in that pair should change together. Undo afterwards; symbol rename is the operation that also updates the declaration and other pairs. |
| Outline and workspace symbols | Open the document outline, then search workspace symbols for Introduction, `sec:intro`, `demoterm` or `shannon1948`. |
| Folding and selection expansion | Fold sections, nested lists, environments, comment blocks and BibTeX entries. Expand selection from a key or word to its argument, command and surrounding structure. |
| Semantic tokens | Enable semantic highlighting and inspect declarations, commands, labels, citations, colors, comments and verbatim text. Editing the source also exercises incremental token updates in clients that request them. |
| Colors | In exercise 5, open a color picker on `2563EB`, or on the `\definecolor` literal in `definitions.tex`. Rename or complete the named color `accent` in its uses. |
| Links and PDF destinations | In exercise 6, open the URL link; navigate, find references or rename `demo-target`. Inspect DOI/URL document links in `references.bib`. |
| Structural refactoring | In exercise 7, select `reusable phrase` and request wrapping or command extraction code actions. Extraction is available in the root document. Undo to restore the exercise. |
| Formatting | In exercise 8, disturb the list indentation and request document or range formatting. Try on-type indentation after Enter or `}`. Also format `references.bib`, which uses a built-in formatter. |
| Diagnostics and quick fixes | In exercise 9, uncomment one fault, inspect its diagnostic and request code actions. Try spelling fixes, a missing brace, a mismatched environment, and `source.fixAll` when the client supports that action kind. Enable ChkTeX with `diagnostics.chktex.enabled: true` and `arguments: ["-w46"]`, then uncomment and save the inline formula exercise to convert `$...$` to `\(...\)`. Undo each exercise afterwards. |
| Build errors | Uncomment the `\textbff` line, save and build. Inspect the compiler diagnostic and spelling action, then undo and rebuild to clear it. |
| Build code lens and commands | In exercise 10, build the root from a source buffer or its build code lens. Build/rebuild, status, cancellation and cleanup are described in the [build guide](../../docs/build.md). |
| Compiled inlay hints | After a successful build, enable the client's inlay hints and inspect `ref`, `eqref`, `pageref` and `cite` uses for numbers/pages/citation numbers read from real auxiliary files. |
| Source/PDF search | Forward-search from the final section to page 3 or later. Inverse-search the introduction's target sentence to return to `sections/introduction.tex`. Requires a viewer integration. |
| File move updates | Use an editor file rename operation that sends `workspace/willRenameFiles` to move `sections/introduction.tex`; inspect the updated `\input` path. Undo the move afterwards. |
| BibTeX editing | In exercise 11, open the database and try entry type, field and string completion; navigation, references and rename; formatting, outline, folding, semantic highlighting and links. |

Push/pull diagnostics, semantic token full/range/delta requests, synchronization
and workspace folder/file notifications are protocol/client behaviors. This
project supplies source content for them; some have no separate editor command.
It demonstrates most everyday language features rather than every supported
LaTeX package syntax. Comment and verbatim examples should remain untouched by
symbol renames and should not produce undefined-reference diagnostics.

## Build and PDF search

Build through your editor's TeXquill integration (`latex.build`) to use the
project settings. The default output is `build/texquill/main.pdf` alongside
auxiliary files and SyncTeX data. A successful build refreshes diagnostics and
compiled inlay hints in supporting clients. Save changes before building.

For an independent command-line build, run from this directory:

```sh
latexmk -pdf -interaction=nonstopmode -halt-on-error -synctex=1 \
  -outdir=build/texquill main.tex
```

The build requires `latexmk`, pdfLaTeX, BibTeX and the packages listed in
`main.tex` (available in a typical TeX Live or MiKTeX installation). LaTeX
document/range formatting additionally needs `tex-fmt` by default, or a
configured `latexindent`; BibTeX and on-type formatting are built in. Source/PDF
queries need `synctex` and a client/viewer integration, such as the
[VS Code extension](../../editors/vscode/README.md) or a configured Neovim PDF
plugin. The server itself does not open a PDF viewer.

See the [editing guide](../../docs/editing.md),
[recorded demos](../../docs/demos.md) and
[documentation index](../../docs/README.md) for additional details.
