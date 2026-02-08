{
  description = "realm-cli: A command-line interface for Realm, a database for mobile applications.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        realm = pkgs.rustPlatform.buildRustPackage {
          pname = "realm";
          version = "0.0.8";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = [ pkgs.git ];

          preCheck = ''
            export HOME=$TMPDIR
            git config --global user.name "Test"
            git config --global user.email "test@test.com"
          '';

          meta = with pkgs.lib; {
            description = "Homebrew-style wrapper for Nix using flake.nix";
            homepage = "https://github.com/yusukeshib/realm";
            license = licenses.mit;
            maintainers = [];
            mainProgram = "realm";
          };
        };
      in
      {
        packages = {
          default = realm;
          realm = realm;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = realm;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
          ];
        };
      }
    );
}
