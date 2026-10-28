# Installation and startup

[Back to README](../README.md) · [Editor configuration](editors.md)

Editing features do not require a TeX distribution. Builds require the relevant tools on the server's `PATH`, defaulting to latexmk and pdfLaTeX; see the [build guide](build.md). Formatting, ChkTeX, and SyncTeX also use separately installed tools. The packages below install the language server; they do not install a complete TeX distribution.

## Release installation

These commands require version 1.0.0 to have been published to the relevant channel. Until then, use the source installation below. Homebrew uses the project's own tap; the package is not assumed to be available in Homebrew/core or nixpkgs.

### Precompiled binaries

Download the archive matching your system from [GitHub Releases](https://github.com/Jacky-Lzx/texquill/releases). These packages do not require Rust or access to crates.io:

- Linux x64: `texquill-1.0.0-x86_64-unknown-linux-gnu.tar.gz` (built on Ubuntu 22.04).
- Linux ARM64: `texquill-1.0.0-aarch64-unknown-linux-gnu.tar.gz` (built on Ubuntu 24.04).
- Apple Silicon macOS: `texquill-1.0.0-aarch64-apple-darwin.tar.gz`.
- Windows x64: `texquill-1.0.0-x86_64-pc-windows-msvc.zip`.

Extract the archive, add its directory to your `PATH`, and run `texquill --version` (`texquill.exe --version` on Windows). Each archive contains the executable, README, and MIT license. Release `SHA256SUMS` covers the archives; on Linux use `sha256sum --check --ignore-missing SHA256SUMS`, or on macOS use `shasum -a 256 --check --ignore-missing SHA256SUMS`, from the directory containing your downloaded archive and checksum file. On Windows, compare `Get-FileHash <archive> -Algorithm SHA256` with the corresponding entry.

Linux packages use glibc and target systems compatible with the build distribution or newer; they are not Alpine/musl builds. Windows packages include the MSVC runtime statically. There is no Intel Mac precompiled package. The archives are not macOS notarized installers or Windows signed installers.

### Cargo

With the current stable Rust toolchain installed:

```sh
cargo install texquill --version 1.0.0 --locked
texquill --version
```

### Homebrew

Once the generated Formula has been added to `Jacky-Lzx/homebrew-tap`:

```sh
brew install Jacky-Lzx/tap/texquill
texquill --version
```

The Formula builds from a verified release source archive. Without a bottle, this requires a local Rust build; Rust is a build dependency only.

### Nix

With `nix-command` and `flakes` enabled:

```sh
nix run github:Jacky-Lzx/texquill/v1.0.0 -- --version
nix profile install github:Jacky-Lzx/texquill/v1.0.0
```

The flake exposes `packages.<system>.texquill`, a default package and app, and `overlays.default` for `aarch64-darwin`, `aarch64-linux`, and `x86_64-linux`. CI builds the native Linux x86-64 and macOS ARM packages. Linux ARM requires its own build validation. The pinned nixpkgs no longer supports Intel macOS (`x86_64-darwin`); use Cargo or a separately validated package set there. See the [nixpkgs release notes](https://nixos.org/manual/nixpkgs/unstable/release-notes#x86_64-darwin-26.11). Nix builds from source unless the exact package is already available from a configured binary cache.

For Home Manager, add an input and use its package in `home.packages`:

```nix
# In your flake's inputs:
texquill.url = "github:Jacky-Lzx/texquill/v1.0.0";

# In a Home Manager module receiving inputs through extraSpecialArgs:
home.packages = [ inputs.texquill.packages.${pkgs.stdenv.hostPlatform.system}.texquill ];
```

## Source installation

Clone this repository and install the current stable Rust toolchain. From the checkout:

Build and launch from source:

```sh
cargo build --release --locked
./target/release/texquill --stdio
```

You can also build or run the local checkout with Nix:

```sh
nix build
./result/bin/texquill --version
nix run . -- --stdio
```

## Server lifecycle

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
