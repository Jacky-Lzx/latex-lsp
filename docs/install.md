# Installation and startup

[Back to README](../README.md) · [Editor configuration](editors.md)

Install the current stable Rust toolchain. Editing features do not require a TeX distribution. Builds require the relevant tools on the server's `PATH`, defaulting to latexmk and pdfLaTeX; see the [build guide](build.md).

Build and launch from source:

```sh
cargo build --release --locked
./target/release/texquill --stdio
```

The server implements stdio JSON-RPC directly with Tokio and uses `lsp-types` for protocol data types. Timed-out or cancelled requests drop their response receivers; late or duplicate client replies are safely ignored.

On `shutdown`, `exit`, or closed standard input, the server cancels and reaps build and SyncTeX process groups so external tools do not remain after the editor disconnects. As required by LSP, `exit` without a completed `shutdown` still returns exit code `1`.

The process reads LSP messages from standard input and writes JSON-RPC messages with `Content-Length` headers to standard output. Editors normally launch it; waiting for input when run directly is expected. To install it in Cargo's executable directory:

```sh
cargo install --path . --locked
texquill --version
```

## Migrating from latex-lsp

TeXquill was previously named `latex-lsp`. Rebuild or reinstall the server, then update editor launch commands to `texquill --stdio`. The Rust crate is now `texquill`; the Neovim configuration name and VS Code extension settings use `texquill` as well.

Rename project configuration files from `.latex-lsp.json` to `.texquill.json`. The default output directory is now `build/texquill/`; update any custom PDF or log paths, or set `build.outputDirectory` explicitly. Repackage the VS Code extension as `texquill.vsix` and migrate its settings to `texquill.*`.

Server extension commands and notifications retain their `latex.*` and `latex/…` protocol names, and the optional `latex` settings wrapper retains its name. Historical benchmark JSON files retain the original executable paths and metadata from each measurement.
