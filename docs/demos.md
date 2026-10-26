# See TeXquill in Neovim

Short recordings of a real Neovim session: find a paper, follow a reference, rename a label, and fix a mistake without leaving your source.

Recorded with the maintainer's Neovim configuration, Catppuccin theme, Blink completion, and TeXquill. The menus, previews, key mappings, and environment editing adapter are supplied by the editor configuration; the language results come from TeXquill. See [editor setup](editors.md) to connect your own client.

On GitHub, browse the [MP4 and GIF files](https://github.com/Jacky-Lzx/texquill/tree/main/docs/public/demos). The documentation website includes video players below.

## Find a citation by title

Type `attention` inside a citation, preview the matching BibTeX entry, and press **Tab** to insert `vaswani2017`. You can also [search by author, year, or citation key](citation-search.md).

<DemoVideo name="completion" title="Search citations by title" />

## Navigate and rename across files

Use **gd** on a reference to jump to its label in another file. Then use **Space r n** to rename `sec:method` to `sec:approach`; the declaration and reference update together, including the other buffer.

<DemoVideo name="navigation" title="Navigate and rename across files" />

Read about [navigation](editing.md) and [supported symbols and rename](symbols.md).

## Preview a fix before applying it

An undefined-reference diagnostic catches `sec:metohd`. Open the diagnostic with **Space d**, then use **Space a** to preview the suggested replacement. Apply it and the warning clears, without compiling the document.

<DemoVideo name="diagnostics" title="Preview and apply a quick fix" />

See [static diagnostics and quick fixes](static-diagnostics.md).

## Edit both environment names at once

Change `quote` to `quotation` with **ciw**. The matching `end` name follows as you type, and a single **u** restores both names. This recording uses the maintainer's Neovim adapter for the server's `linkedEditingRange` results; clients need linked editing support.

<DemoVideo name="linked-editing" title="Linked environment editing" />

## Keep your macro documentation close

Use **K** to read documentation written above a macro declaration in `macros.tex`. Inside a different macro call, **gK** shows its signature, the current argument, and the declaration's comments.

<DemoVideo name="signatures" title="Macro documentation and signature help" />

See [macro documentation](command-documentation.md) and [signature help](signatures.md).

## Format and build from the editor

Format the source through TeXquill's LSP formatting request, then press **Space l b** to build a PDF. The editor reports the real build result. This example uses separately installed **tex-fmt** and **latexmk**.

<DemoVideo name="build" title="Format and build from Neovim" />

Read the [formatting guide](formatting.md) and [build guide](build.md).

## Forward search: source to PDF

Start on a line in `sections/results.tex` while Skim shows the introduction. Forward search takes the PDF to page 3 and highlights the corresponding text, keeping the editor on the source line.

<DemoVideo name="forward-search" title="Forward search from Neovim to Skim" />

The recording invokes the same tex-tools adapter used by **Space l v** in the maintainer's configuration. TeXquill returns the PDF page and coordinates through `latex.forwardSearch`; tex-tools positions Skim and requests its native SyncTeX indicator. The server does not launch the viewer itself.

## Reverse search: PDF to source

While Neovim shows the results, **Shift-Command-click** the method paragraph on PDF page 2. The editor opens `sections/method.tex` at line 4, matching the clicked paragraph.

<DemoVideo name="reverse-search" title="Reverse search from Skim to Neovim" />

This uses Skim's existing SyncTeX editor callback and tex-tools' session registry. Skim resolves the PDF click to a source location, then the callback navigates the matching Neovim session. It demonstrates the native viewer integration; the separate `latex.inverseSearch` coordinate-query API is described in the [SyncTeX guide](synctex.md).

Both clips capture the two native application windows simultaneously and place their footage side by side. They use a saved, successfully compiled three-page project with SyncTeX enabled.

## Reproduce the recordings

The [recording scripts and sample project](https://github.com/Jacky-Lzx/texquill/tree/main/scripts/demos) run the installed Neovim configuration against the local server binary. They check actual buffer changes, popup content, diagnostics, and PDF output before exporting each clip. Videos have no audio and wait for playback; GIF alternatives are available for embedding in Markdown.
