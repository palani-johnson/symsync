{
  description = "A stateful symlink farm manager";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = ["aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
  in {
    packages = forAllSystems (pkgs: rec {
      symsync = pkgs.rustPlatform.buildRustPackage {
        pname = cargoToml.package.name;
        version = cargoToml.package.version;
        src = self;
        cargoLock.lockFile = ./Cargo.lock;
        nativeCheckInputs = [pkgs.git];
        meta.mainProgram = "symsync";
      };

      schema = pkgs.runCommand "symsync-config-schema" {} ''
        ${symsync}/bin/symsync gen-config-schema > $out
      '';

      default = symsync;
    });
  };
}
