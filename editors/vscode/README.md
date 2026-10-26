# TeXquill for VS Code

LaTeX / TeX / BibTeX / BST completion, navigation, rename, diagnostics, formatting, quick fixes, and build commands powered by this repository's Rust language server. The extension reuses VS Code's built-in LaTeX and BibTeX syntax highlighting and adds the `.bst` file type.

## Installation

The extension is currently packaged from source. Install the language server separately; the VSIX includes neither the server nor a TeX distribution. First, run this in the repository root:

```sh
cargo install --path . --locked
```

Then build the extension with Node.js 22 or later:

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

Use VS Code's standard completion, navigation, rename, formatting, and quick fix actions. New documents support completion after selecting the LaTeX or BibTeX language mode. Save the project and its dependencies before building. On-type formatting, linked editing, and inlay hints also follow VS Code's own settings.

The command palette provides:

| Command | Purpose |
| --- | --- |
| TeXquill: Build | Build the current document's project. |
| TeXquill: Rebuild | Force a rebuild. |
| TeXquill: Cancel Build | Cancel the current document's build, or all builds when no editor is active. |
| TeXquill: Show Build Status | Write the current project's status to the output panel. |
| TeXquill: Open PDF | Open a successfully built PDF using VS Code's registered PDF handler. |
| TeXquill: Restart Server | Stop the old server, reconnect, and resynchronize open documents. |
| TeXquill: Show Output | Show server logs and build results. |

Document code lenses also trigger builds. Build progress uses VS Code's native progress UI; start and completion details appear in the "TeXquill" output panel, and failed builds produce a notification. The extension does not save files automatically; the server rejects builds with unsaved dependencies.

The extension provides an Open PDF action, but no embedded PDF viewer or SyncTeX navigation yet. Running it alongside other LaTeX extensions can produce duplicate completion, diagnostics, or builds. Use one language service and automatic build workflow per workspace.

## Development and validation

Run `cargo build --release --locked` in the repository root, then open this directory in VS Code:

```sh
code editors/vscode
```

Run `npm ci`, press F5 to launch an Extension Development Host, set `serverPath` in the new window, and open a test project. `.vscode/launch.json` compiles TypeScript before launch.

```sh
npm run check
npm test
npm run package
```

By default, `npm test` downloads stable VS Code and runs real host tests with isolated temporary user data, extension directories, and LaTeX projects. It requires pdfLaTeX and a graphical environment. Tests cover cross-file navigation and rename, untitled completion, BibTeX, code lens builds, opening PDFs, rebuilds, live settings, build on save, cancellation and child process cleanup, diagnostics, unsaved content synchronization after restart, and reconnection after correcting an invalid executable path. On headless Linux, use `xvfb-run -a npm test`.

Use `VSCODE_EXECUTABLE_PATH` to select an installed VS Code executable and `TEXQUILL_TEST_SERVER` to select the server. On macOS:

```sh
VSCODE_EXECUTABLE_PATH='/Applications/Visual Studio Code.app/Contents/MacOS/Code' npm test
```

The extension and server can have separate release versions. When changing custom commands or the configuration protocol, update both sides in the same commit and run the host tests.

The packaging tool's indirect `@azure/msal-node` dependency uses an npm override to select a patched, CommonJS-compatible UUID 11.x release, avoiding known vulnerabilities in older UUID versions. This dependency is used only for development and packaging and is excluded from the VSIX.
