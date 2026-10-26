# TeXquill for VS Code

LaTeX / TeX / BibTeX / BST completion, navigation, rename, diagnostics, formatting, quick fixes, and build commands powered by this repository's Rust language server. The extension reuses VS Code's built-in LaTeX and BibTeX syntax highlighting and adds the `.bst` file type.

## Installation

The extension is currently packaged from source. Install the language server separately; the VSIX includes neither the server nor a TeX distribution. First, run this in the repository root:

```sh
cargo install --path . --locked
```

Then build the extension with Node.js 22.13 or later:

```sh
cd editors/vscode
npm ci
npm run package
code --install-extension texquill.vsix
```

Open a LaTeX project folder in VS Code, then open a `.tex` or `.bib` file. If VS Code cannot find the server on its PATH, set an absolute path in your user `settings.json`:

```json
{
  "texquill.serverPath": "/absolute/path/to/texquill",
  "texquill.settings": {
    "build": {
      "engine": "xelatex",
      "onSave": true
    }
  }
}
```

For local development, `serverPath` can point to `target/release/texquill` in this repository. The path does not expand `~`, `${workspaceFolder}`, or shell expressions. The default command is `texquill --stdio`; do not append arguments to `serverPath`.

Builds require the corresponding TeX tools, defaulting to latexmk and pdfLaTeX. LaTeX formatting requires tex-fmt by default, or optionally latexindent; BibTeX uses the built-in formatter. If a GUI-launched VS Code cannot find these tools, set `texquill.serverEnvironment` in user settings, for example `{"PATH": "/Library/TeX/texbin:/opt/homebrew/bin:/usr/bin:/bin"}`.

The extension runs in trusted local file workspaces. With Remote SSH, WSL, or Dev Containers, the extension and server run remotely; `serverPath` and the TeX tools must also be available on that remote host.

## Configuration and usage

- `texquill.serverPath`: server executable; changing it automatically restarts the server.
- `texquill.serverEnvironment`: overrides inherited server environment variables; changing it automatically restarts the server.
- `texquill.settings`: complete server settings, passed at initialization and updated through `workspace/didChangeConfiguration` without a restart. An empty object preserves server defaults, including build on save being disabled.
- `texquill.trace.server`: `off` / `messages` / `verbose`, for tracing protocol communication.

`settings` supports `build`, `formatting`, `diagnostics`, `distribution`, `indexing`, and `inlayHints`. Fields match the server configuration; see [build configuration](https://github.com/Jacky-Lzx/texquill/blob/main/docs/build-configuration.md) and the [documentation index](https://github.com/Jacky-Lzx/texquill/blob/main/docs/README.md). The server rejects invalid settings and retains the previous valid configuration, with the reason recorded in the output panel.

Each VS Code window uses one server, supporting multiple workspace folders. `texquill.settings` supplies window-wide defaults; use `.texquill.json` for folder or project overrides. See [project settings](https://github.com/Jacky-Lzx/texquill/blob/main/docs/project-settings.md). Avoid treating a common parent directory of unrelated projects as one project root.

Use VS Code's standard completion, navigation, rename, formatting, and quick fix actions. New documents support completion after selecting the LaTeX or BibTeX language mode. Save the project and its dependencies before building. On-type formatting and inlay hints follow VS Code's own settings.

Environment name linked editing is enabled by default for LaTeX, TeX, and Plain TeX. Editing the name in a matching `\begin{name}` or `\end{name}` updates its partner, including nested environments. This uses the server's linked editing ranges and VS Code's native editing support. To disable it, set `"editor.linkedEditing": false` in the corresponding language-specific settings (for example, `[latex]`); other languages keep their existing defaults.

The command palette provides:

| Command | Purpose |
| --- | --- |
| TeXquill: Build | Build the current document's project. |
| TeXquill: Rebuild | Force a rebuild. |
| TeXquill: Cancel Build | Cancel the current document's build, or all builds when no editor is active. |
| TeXquill: Show Build Status | Write the current project's status to the output panel. |
| TeXquill: Open PDF | Open a successfully built PDF beside the source in the built-in viewer, keeping focus in the source editor. |
| TeXquill: Forward Search (TeX to PDF) | Locate the source cursor in the built-in PDF preview without moving keyboard focus. |
| TeXquill: Restart Server | Stop the old server, reconnect, and resynchronize open documents. |
| TeXquill: Show Output | Show server logs and build results. |

Document code lenses also trigger builds. Build progress uses VS Code's native progress UI; start and completion details appear in the "TeXquill" output panel, and failed builds produce a notification. The extension does not save files automatically; the server rejects builds with unsaved dependencies.

## PDF preview

After a successful build, run **TeXquill: Open PDF** from the source editor. The built-in, offline PDF.js viewer opens beside the source without taking keyboard focus. Repeating the command reuses that PDF's preview. No separate PDF viewer extension is required.

The toolbar supports page navigation, zoom, fit width, fit page, and manual reload. Text can be selected and copied. Successful TeXquill builds refresh open previews while retaining their page, zoom, and relative scroll position; enable `texquill.settings.build.onSave` for save-triggered builds and refreshes. Failed or stale builds leave the displayed PDF unchanged. Refreshing never activates the preview, including when you switch editors during a build. Clicking the preview still focuses it normally.

Preview tabs, their reading state, and their source association can be restored when VS Code reloads. The viewer does not yet provide document search, a table of contents, printing, or annotation editing. PDF scripting and external links are disabled. PDF.js and its worker, character maps, fonts, and decoder resources ship in the VSIX with their license; opening a preview does not require a CDN or a local HTTP server.

### SyncTeX navigation

Save and build the project first. The server uses the `synctex` executable from its PATH and the compiler's `.synctex` or `.synctex.gz` output. Default build arguments enable SyncTeX; custom compiler arguments must enable it explicitly (for example, `-synctex=1` with pdfLaTeX, XeLaTeX, or LuaLaTeX).

- **TeX to PDF:** press **Ctrl+Alt+J** (**Cmd+Alt+J** on macOS), run **TeXquill: Forward Search (TeX to PDF)**, or use the source editor's context menu. The preview opens beside the source when needed, scrolls to the first matching location, and briefly highlights its typesetting box. Keyboard focus stays in the source editor.
- **PDF to TeX:** **Ctrl+click** (**Cmd+click** on macOS) a location in the built-in preview. The extension opens the corresponding source file in the source editor group and moves the cursor to its line. Ordinary clicks and text selection keep their usual behavior.

Both directions support included TeX files and preserve the side PDF. Coordinates account for zoom and page rotation. The server owns project/artifact lookup and rejects unsaved sources, stale PDFs, and incomplete or failed builds; save and rebuild when asked. SyncTeX typically maps to source lines, rather than exact character columns. See the [query protocol](https://github.com/Jacky-Lzx/texquill/blob/main/docs/synctex.md) for details.

Running TeXquill alongside other LaTeX language services can produce duplicate completion, diagnostics, or builds. Use one language service and automatic build workflow per workspace.

## Development and validation

Run `cargo build --release --locked` in the repository root, then open this directory in VS Code:

```sh
code editors/vscode
```

Run `npm ci`, press F5 to launch an Extension Development Host, set `serverPath` in the new window, and open a test project. `.vscode/launch.json` compiles TypeScript and copies the PDF viewer assets before launch.

```sh
npm run check
npm test
npm run package
```

By default, `npm test` checks coordinate transforms with real PDF.js viewports, downloads stable VS Code, and runs real host tests with isolated temporary user data, extension directories, and LaTeX projects. It requires pdfLaTeX, `synctex`, and a graphical environment. Tests cover cross-file navigation and rename, untitled completion, BibTeX, code lens builds, actual built-in PDF rendering, SyncTeX round trips and unsaved-source rejection, preview focus, rebuild/save refreshes, rapid reloads, hidden tabs, invalid PDF recovery, live settings, build on save, cancellation and child process cleanup, diagnostics, unsaved content synchronization after restart, and reconnection after correcting an invalid executable path. On headless Linux, use `xvfb-run -a npm test`.

Use `VSCODE_EXECUTABLE_PATH` to select an installed VS Code executable and `TEXQUILL_TEST_SERVER` to select the server. On macOS:

```sh
VSCODE_EXECUTABLE_PATH='/Applications/Visual Studio Code.app/Contents/MacOS/Code' npm test
```

`npm run compile` also copies the viewer assets and required PDF.js distribution files into `dist/preview/assets`; the Rust build is independent.

The extension and server can have separate release versions. When changing custom commands or the configuration protocol, update both sides in the same commit and run the host tests.

The packaging tool's indirect `@azure/msal-node` dependency uses an npm override to select a patched, CommonJS-compatible UUID 11.x release, avoiding known vulnerabilities in older UUID versions. This dependency is used only for development and packaging and is excluded from the VSIX.
