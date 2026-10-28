# Release maintenance

[Back to README](../README.md) · [Installation](install.md)

The release version comes from `Cargo.toml`. Keep the root package entry in `Cargo.lock`, `editors/vscode/package.json`, and both root version entries in `editors/vscode/package-lock.json` in sync. Release tags use `vMAJOR.MINOR.PATCH`, starting with `v1.0.0`.

## Local preparation

Run these commands from the repository root:

```sh
python3 scripts/prepare-release.py --check --tag v1.0.0
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 scripts/prepare-release.py --tag v1.0.0
cargo publish --dry-run --locked
cargo build --release --locked
python3 scripts/smoke-lsp.py target/release/texquill --version 1.0.0
nix flake check --all-systems --no-build --no-update-lock-file
nix flake check --no-update-lock-file
```

On macOS, use `TMPDIR=/private/tmp` for Cargo tests. The scripts require Python 3.9 or later and only use its standard library. Add `--offline` to the preparation command when all Cargo dependencies are cached. For a preview before committing, add `--allow-dirty`; those artifacts must be regenerated from the final committed tag before publication.

`scripts/prepare-release.py` performs Cargo's actual package build verification, checks the compressed size and bundled metadata, and writes these files under `target/release-assets/`:

- `texquill-1.0.0.crate`: the verified crates.io package.
- `texquill-1.0.0.tar.gz`: identical source bytes with an extension recognized by Homebrew.
- `texquill.rb`: a Formula containing the real source SHA256 and versioned GitHub Release URL.
- `SHA256SUMS`: checksums for the package, source archive, and Formula.

To build a native precompiled archive locally, specify your host target explicitly. For example, on Apple Silicon macOS:

```sh
cargo build --release --locked --target aarch64-apple-darwin
python3 scripts/prepare-release.py --binary-target aarch64-apple-darwin --output-dir target/binary-assets
```

The binary mode uses the existing build in `target/<target>/release/` (or Cargo's configured target directory). It creates a `.tar.gz` archive on Unix or `.zip` on Windows, containing the executable, README, and license under a versioned directory. It extracts that archive and runs both `--version` and the interactive LSP lifecycle test before writing an archive-specific `.sha256` file. Run it on the matching native system. Supported targets are `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, and `x86_64-pc-windows-msvc`; Intel Mac is excluded.

The crate includes Rust sources, integration tests and their fixtures, bundled package metadata, README, license, and Cargo manifests. Website media, editor dependencies, and benchmarks are excluded. Cargo builds the extracted package during verification, so missing embedded assets are detected.

The Nix flake pins nixpkgs through `flake.lock` and fetches Rust dependencies through `Cargo.lock`. Its build runs library tests and checks the installed binary's version and LSP initialize/shutdown/exit lifecycle. Full integration tests run in the Rust and release workflows. Keep `flake.nix`, `flake.lock`, `nix/package.nix`, and `scripts/smoke-lsp.py` tracked before using the normal Git-backed `nix build` or `nix flake check` commands.

For an intentional nixpkgs refresh, run `nix flake update nixpkgs`, review the lockfile, and repeat the Nix checks. No separate registry upload is required for installation through the repository flake. Inclusion in nixpkgs is a separate contribution requiring a fetcher, source/dependency hashes, and a maintainer entry.

## GitHub Actions rehearsal

The [release workflow](../.github/workflows/release.yml) supports a manual rehearsal. After the workflow is on the default branch:

```sh
gh workflow run release.yml --ref main
```

Leave both `publish` and `publish_crate` at their default `false`. The workflow checks versions, formatting, Clippy, and the full Rust suite on Linux and macOS; runs the reusable [Nix workflow](../.github/workflows/nix.yml); verifies Cargo packaging; uploads source and binary artifacts; and installs/tests the generated Homebrew Formula on macOS using the local verified archive. It does not upload a crate or create a GitHub Release during rehearsal. `cargo package` already builds and verifies the source package; the GitHub path does not run a registry-specific `cargo publish` command.

The binary matrix builds natively on Ubuntu 22.04 x64, Ubuntu 24.04 ARM64, macOS 15 ARM64, and Windows Server 2022 x64. Each extracted executable must pass the version and LSP tests before its archive is uploaded. The `assemble` job also runs during rehearsal, verifies all source and binary checksums, and uploads the complete `github-release-assets` artifact with a combined `SHA256SUMS`. Download that artifact to review the exact files intended for publication. The Linux packages use glibc from the corresponding build distribution; no musl packages are generated. Windows uses a static MSVC runtime. These archives are not signed/notarized installers, and Homebrew bottles are not generated.

The ordinary Rust CI also checks version consistency. The Nix workflow runs on pushes to `main` and pull requests, evaluating all three exposed systems and building the native Linux x86-64 and macOS ARM outputs. Intel macOS is excluded because the pinned nixpkgs has removed that platform; see the [platform release note](https://nixos.org/manual/nixpkgs/unstable/release-notes#x86_64-darwin-26.11).

## Publishing credentials

GitHub Releases only require the workflow's `GITHUB_TOKEN` with `contents: write`, which the `publish-github` job requests. Ensure repository Actions policy permits it. No crates.io credentials or `release` environment are needed to publish GitHub assets.

For the optional crates.io publication:

1. Sign in to crates.io, confirm ownership or availability of `texquill`, and verify your account email.
2. Create a GitHub Actions environment named `release`. For the first release, add a crates.io API token as its `CARGO_REGISTRY_TOKEN` secret with the permissions required to publish the new crate. Never commit the token.

For subsequent releases, configure crates.io Trusted Publishing for repository `Jacky-Lzx/texquill`, workflow `release.yml`, and environment `release`, then remove the bootstrap token. When the token secret is absent, the workflow uses `rust-lang/crates-io-auth-action` with `id-token: write` to obtain a temporary credential. See [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing).

The VS Code extension version is synchronized, but Marketplace publication is independent and is not performed by this workflow. Its existing CI builds and packages the VSIX.

## Publish the committed version

After reviewing changes, committing them, pushing the release commit, and passing the rehearsal, create the release tag at that commit:

```sh
git tag -a v1.0.0 -m "TeXquill 1.0.0"
git push origin v1.0.0
```

Pushing the tag starts the release workflow and publishes only to GitHub. All checks, native binary builds/tests, and the Homebrew installation test must succeed before the assembled artifact reaches `publish-github`. That job verifies the combined `SHA256SUMS` and creates a GitHub Release containing the source archive, crate, Formula, four precompiled archives, and checksums. It does not depend on the crates.io API or a crates.io publication; Cargo dependency downloads still require access to the package index and crate downloads.

A manual GitHub publication requires selecting the matching tag and setting `publish=true`:

```sh
gh workflow run release.yml --ref v1.0.0 -f publish=true
```

Publish to crates.io separately when its credentials and connectivity are ready:

```sh
gh workflow run release.yml --ref v1.0.0 -f publish_crate=true
```

This runs the validation jobs and `publish-crate`, with no GitHub Release creation. Setting both inputs to `true` requests both independent publication jobs. A crates.io publication failure cannot block the GitHub publication. The `release` environment applies only to `publish-crate`.

Do not move an already-published tag or reuse a published crates.io version. After a partial failure, check both publication states and retry only the missing destination. For GitHub, use the failed run's verified artifacts with `gh release create v1.0.0 ... --verify-tag` or `gh release upload v1.0.0 ...` as appropriate. If GitHub already published successfully, leave `publish=false` when retrying crates.io. If crates.io already succeeded, leave `publish_crate=false` when retrying GitHub. Neither publication job silently overwrites an existing release/version.

## Homebrew tap setup

The generated Formula is intended for `Jacky-Lzx/homebrew-tap`, with installation name `Jacky-Lzx/tap/texquill`. Create that repository once, for example using Homebrew's tap scaffold:

```sh
brew tap-new Jacky-Lzx/tap
gh repo create Jacky-Lzx/homebrew-tap --public --source "$(brew --repository Jacky-Lzx/tap)" --push
```

After the GitHub Release is public, download `texquill.rb` from that release, copy it into the tap's `Formula/` directory, and validate before committing and pushing it:

```sh
gh release download v1.0.0 --repo Jacky-Lzx/texquill --pattern texquill.rb --dir /tmp/texquill-v1.0.0
cp /tmp/texquill-v1.0.0/texquill.rb "$(brew --repository Jacky-Lzx/tap)/Formula/texquill.rb"
brew install --build-from-source Jacky-Lzx/tap/texquill
brew test Jacky-Lzx/tap/texquill
brew audit --strict Jacky-Lzx/tap/texquill
```

The Formula builds from source and tests both `--version` and an LSP session. Keep TeX distributions and feature-specific external tools separately installed. The tap scaffold can later build bottles. Updating the external tap is deliberately a separate maintenance step; the project's release workflow needs no cross-repository token. Homebrew/core inclusion requires a separate review under [Homebrew's acceptance rules](https://docs.brew.sh/Acceptable-Formulae).
