{
  description = "Shire — One index to rule them all. Monorepo MCP-first indexer.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, crane, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;

        # Include Cargo source + tree-sitter query files (.scm)
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (builtins.match ".*\\.scm$" path != null) || (craneLib.filterCargoSources path type);
        };

        commonArgs = {
          inherit src;
          strictDeps = true;

          buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            pkgs.apple-sdk_15
          ];

          nativeBuildInputs = with pkgs; [
            git  # needed by integration tests (git init in temp dirs)
          ];
        };

        # Build deps separately for caching
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        shire = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;

          meta = {
            description = "One index to rule them all — monorepo MCP-first indexer";
            homepage = "https://github.com/justinjdev/shire";
            license = pkgs.lib.licenses.asl20;
            mainProgram = "shire";
          };
        });
      in
      {
        checks = {
          inherit shire;
        };

        packages = {
          default = shire;
          shire = shire;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = shire;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = with pkgs; [
            rust-analyzer
            cargo-watch
          ];
        };
      }
    );
}
