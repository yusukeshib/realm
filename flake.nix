{
  description = "box-cli: Sandboxed Docker environments for git repos";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        box = pkgs.rustPlatform.buildRustPackage {
          pname = "box";
          version = "0.0.2";

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
            description = "Sandboxed Docker environments for git repos";
            homepage = "https://github.com/yusukeshib/box";
            license = licenses.mit;
            maintainers = [];
            mainProgram = "box";
          };
        };
      in
      {
        packages = {
          default = box;
          box = box;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = box;
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
