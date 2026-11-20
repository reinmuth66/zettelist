{
  description = "zettelist: Zettelkasten on Omnigraph の書き込みと整理の道具";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    omnigraph.url = "github:ModernRelay/omnigraph/v0.15.0";
  };

  outputs = { self, nixpkgs, omnigraph }:
    let
      systems = [ "aarch64-darwin" "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (system: pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "zettelist";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          # nix のビルドの中には omnigraph-server がないので、統合テストは流さない
          cargoTestFlags = [ "--lib" ];
          # backup のテストが git を呼ぶ
          nativeCheckInputs = [ pkgs.git ];
        };
      });

      devShells = forAllSystems (system: pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.rust-analyzer
            pkgs.ollama
            omnigraph.packages.${system}.default
          ];
        };
      });
    };
}
