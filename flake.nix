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

        # Fast Free Testing (FFT) check contract. These replace the legacy
        # local-ci fmt/clippy/test stages. Check names match propel.toml so the
        # FFT NixOS runner can dispatch `nix build .#checks.<system>.<name>`.
        checks = {
          # Type safety: compiles, is formatted, and lints clean.
          type-safety = pkgs.runCommand "check-type-safety" {
            buildInputs = [ rustToolchain pkgs.pkg-config ];
            src = self;
          } ''
            cd $src
            cargo fmt --all -- --check
            cargo check --workspace --all-targets
            cargo clippy --workspace --all-targets -- -D warnings
            touch $out
          '';

          # Unit tests: the full workspace test suite.
          unit-tests = pkgs.runCommand "check-unit-tests" {
            buildInputs = [ rustToolchain pkgs.pkg-config ];
            src = self;
          } ''
            cd $src
            cargo test --workspace
            touch $out
          '';

          # Secrets: gitleaks scan of the working tree.
          secrets = pkgs.runCommand "check-secrets" {
            buildInputs = [ pkgs.gitleaks ];
            src = self;
          } ''
            cd $src
            gitleaks detect --source . --no-git --exit-code 1 --redact
            touch $out
          '';

          # Config lint: no hardcoded localhost URLs or inline passwords.
          config-lint = pkgs.runCommand "check-config-lint" {
            src = self;
          } ''
            cd $src
            failed=0
            if grep -rn "http://localhost" --include="*.rs" --include="*.toml" . ; then
              echo "✗ Hardcoded localhost URLs found"; failed=1
            fi
            if grep -rnE 'password[[:space:]]*[=:][[:space:]]*["'"'"'][^"'"'"']+["'"'"']' \
                 --include="*.rs" --include="*.toml" --include="*.json" . ; then
              echo "✗ Hardcoded passwords found"; failed=1
            fi
            if [ "$failed" -ne 0 ]; then exit 1; fi
            echo "✓ Config lint passed"
            touch $out
          '';
        };
      });
}
