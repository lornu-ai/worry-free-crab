{
  description = "local-ci - lightweight local CI runner (Rust rewrite)";

  nixConfig = {
    extra-substituters = [ "https://nix-cache.stevedores.org" ];
    extra-trusted-public-keys = [ "stevedores-cache-1:bXLxkipycRWproIJnk8pPWNFdgVfeV+I2mJXCoW4/ag=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "clippy" "rustfmt" ];
        };
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };
        local-ci = rustPlatform.buildRustPackage {
          pname = "local-ci";
          version = "0.3.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
        };
      in {
        packages.local-ci = local-ci;
        packages.default = local-ci;

        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.git
            local-ci
          ];
        };

        checks = {
          fmt = pkgs.runCommand "check-fmt" {
            buildInputs = [ rustToolchain pkgs.pkg-config ];
            src = self;
          } ''
            cd $src
            cargo fmt --all -- --check
            touch $out
          '';

          clippy = pkgs.runCommand "check-clippy" {
            buildInputs = [ rustToolchain pkgs.pkg-config ];
            src = self;
          } ''
            cd $src
            cargo clippy --workspace --all-targets -- -D warnings
            touch $out
          '';

          test = pkgs.runCommand "check-test" {
            buildInputs = [ rustToolchain pkgs.pkg-config ];
            src = self;
          } ''
            cd $src
            cargo test --workspace
            touch $out
          '';
        };
      });
}
