{
  lib,
  rustPlatform,
  python3,
}:

let
  manifest = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "texquill";
  version = manifest.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../assets/packages.json
      ../README.md
      ../LICENSE
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  # Full integration tests run in CI; Nix also checks the installed LSP below.
  cargoTestFlags = [ "--lib" ];
  doInstallCheck = true;
  nativeInstallCheckInputs = [ python3 ];
  installCheckPhase = ''
    runHook preInstallCheck
    "$out/bin/texquill" --version
    python3 ${../scripts/smoke-lsp.py} "$out/bin/texquill" --version ${manifest.package.version}
    runHook postInstallCheck
  '';

  meta = {
    description = manifest.package.description;
    homepage = manifest.package.homepage;
    license = lib.licenses.mit;
    mainProgram = "texquill";
    platforms = [
      "aarch64-darwin"
      "aarch64-linux"
      "x86_64-linux"
    ];
  };
}
