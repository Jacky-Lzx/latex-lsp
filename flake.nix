{
  description = "TeXquill LaTeX language server";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          texquill = pkgs.callPackage ./nix/package.nix { };
        in
        {
          inherit texquill;
          default = texquill;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = nixpkgs.lib.getExe self.packages.${system}.texquill;
          meta.description = "TeXquill LaTeX language server";
        };
      });

      checks = forAllSystems (system: {
        default = self.packages.${system}.texquill;
      });

      overlays.default = final: _prev: {
        texquill = final.callPackage ./nix/package.nix { };
      };
    };
}
