#!/usr/bin/env python3
"""Validate versions and generate verified Cargo/Homebrew release artifacts locally."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile


ROOT = Path(__file__).resolve().parent.parent


def versions(tag):
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked", "--offline"], cwd=ROOT,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == "texquill")
    version = package["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise SystemExit("Release versions must be stable MAJOR.MINOR.PATCH versions")
    if tag is not None and tag != f"v{version}":
        raise SystemExit(f"Tag {tag!r} does not match Cargo version v{version}")
    extension = json.loads((ROOT / "editors/vscode/package.json").read_text())
    lock = json.loads((ROOT / "editors/vscode/package-lock.json").read_text())
    if {extension["version"], lock["version"], lock["packages"][""]["version"]} != {version}:
        raise SystemExit("Cargo and VS Code package/lockfile versions must match")
    return version, Path(metadata["target_directory"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", help="Expected release tag, e.g. v1.0.0")
    parser.add_argument("--check", action="store_true", help="Only check version consistency")
    parser.add_argument("--allow-dirty", action="store_true", help="Allow local preview of uncommitted changes")
    parser.add_argument("--offline", action="store_true", help="Use Cargo's cached dependencies")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/release-assets")
    args = parser.parse_args()
    version, target = versions(args.tag)
    if args.check:
        print(f"Release versions match: v{version}")
        return
    command = ["cargo", "package", "--locked"]
    if args.allow_dirty:
        command.append("--allow-dirty")
    if args.offline:
        command.append("--offline")
    subprocess.run(command, cwd=ROOT, check=True)
    crate = target / f"package/texquill-{version}.crate"
    if crate.stat().st_size > 10_000_000:
        raise SystemExit("The compressed crate exceeds the default crates.io 10 MB limit")
    with tarfile.open(crate) as archive:
        files = {Path(member.name).relative_to(f"texquill-{version}").as_posix()
                 for member in archive.getmembers() if member.isfile()}
    if not {"Cargo.toml", "Cargo.lock", "src/main.rs", "assets/packages.json", "LICENSE"} <= files:
        raise SystemExit("The crate is missing required source or package metadata")
    if any(path.startswith(("docs/", "editors/", "node_modules/", "benchmarks/")) for path in files):
        raise SystemExit("The crate includes website, editor, or benchmark assets")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    # The Homebrew archive is exactly the verified crate, with a tar.gz extension.
    source = output / f"texquill-{version}.tar.gz"
    shutil.copyfile(crate, source)
    shutil.copyfile(crate, output / crate.name)
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    template = (ROOT / "packaging/homebrew/texquill.rb.in").read_text()
    formula = template.replace("@VERSION@", version).replace("@SHA256@", digest)
    (output / "texquill.rb").write_text(formula)
    artifacts = [output / crate.name, source, output / "texquill.rb"]
    (output / "SHA256SUMS").write_text("".join(
        f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in artifacts
    ))
    print(f"Prepared v{version}: {len(files)} files, {crate.stat().st_size} compressed bytes")
    print(f"Artifacts: {output}")
    if args.allow_dirty:
        print("Local preview only: regenerate from the committed release tag before publishing")


if __name__ == "__main__":
    main()
