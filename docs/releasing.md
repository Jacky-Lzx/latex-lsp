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

The crate includes Rust sources, integration tests and their fixtures, bundled package metadata, README, license, and Cargo manifests. Website media, editor dependencies, and benchmarks are excluded. Cargo builds the extracted package during verification, so missing embedded assets are detected.

The Nix flake pins nixpkgs through `flake.lock` and fetches Rust dependencies through `Cargo.lock`. Its build runs library tests and checks the installed binary's version and LSP initialize/shutdown/exit lifecycle. Full integration tests run in the Rust and release workflows. Keep `flake.nix`, `flake.lock`, `nix/package.nix`, and `scripts/smoke-lsp.py` tracked before using the normal Git-backed `nix build` or `nix flake check` commands.

For an intentional nixpkgs refresh, run `nix flake update nixpkgs`, review the lockfile, and repeat the Nix checks. No separate registry upload is required for installation through the repository flake. Inclusion in nixpkgs is a separate contribution requiring a fetcher, source/dependency hashes, and a maintainer entry.

## GitHub Actions rehearsal

The [release workflow](../.github/workflows/release.yml) supports a manual rehearsal. After the workflow is on the default branch:

```sh
gh workflow run release.yml --ref main
```

Leave `publish` at its default `false`. The workflow checks versions, formatting, Clippy, and the full Rust suite on Linux and macOS; runs the reusable [Nix workflow](../.github/workflows/nix.yml); verifies Cargo packaging and a publish dry run; uploads release assets; and installs/tests the generated Homebrew Formula on macOS using the local verified archive. It does not upload a crate or create a GitHub Release during rehearsal.

The ordinary Rust CI also checks version consistency. The Nix workflow runs on pushes to `main` and pull requests, evaluating all three exposed systems and building the native Linux x86-64 and macOS ARM outputs. Intel macOS is excluded because the pinned nixpkgs has removed that platform; see the [platform release note](https://nixos.org/manual/nixpkgs/unstable/release-notes#x86_64-darwin-26.11).

## Publishing credentials

Before the first publication:

1. Sign in to crates.io, confirm ownership or availability of `texquill`, and verify your account email.
2. Create a GitHub Actions environment named `release`. For the first release, add a crates.io API token as its `CARGO_REGISTRY_TOKEN` secret with the permissions required to publish the new crate. Never commit the token.
3. Make sure Actions is allowed to write repository contents so the publish job can create the GitHub Release. The workflow requests this permission only for that job.

For subsequent releases, configure crates.io Trusted Publishing for repository `Jacky-Lzx/texquill`, workflow `release.yml`, and environment `release`, then remove the bootstrap token. When the token secret is absent, the workflow uses `rust-lang/crates-io-auth-action` with `id-token: write` to obtain a temporary credential. See [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing).

The VS Code extension version is synchronized, but Marketplace publication is independent and is not performed by this workflow. Its existing CI builds and packages the VSIX.

## Publish the committed version

After reviewing changes, committing them, pushing the release commit, and passing the rehearsal, create the release tag at that commit:

```sh
git tag -a v1.0.0 -m "TeXquill 1.0.0"
git push origin v1.0.0
```

Pushing the tag starts the release workflow. All checks and the Homebrew installation test must succeed before the publish job runs. The publish job verifies checksums, publishes the crate, and creates a GitHub Release containing the verified source archive, crate, Formula, and checksums. No precompiled server binaries or Homebrew bottles are generated in this initial pipeline.

A manual publication requires selecting the matching tag and setting `publish=true`:

```sh
gh workflow run release.yml --ref v1.0.0 -f publish=true
```

Do not move an already-published tag or reuse a published crates.io version. If crates.io succeeded but GitHub Release creation failed, recover only the GitHub step using that run's verified artifacts; rerunning the publish job will try to publish the existing crate again. Use `gh release create v1.0.0 ... --verify-tag` or `gh release upload v1.0.0 ...` as appropriate after checking release state.

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
