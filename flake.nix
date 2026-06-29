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
        #
        # The cargo-based checks are derived from the `local-ci` package via
        # overrideAttrs so they inherit its vendored dependencies (cargoLock).
        # This keeps them hermetic: cargo runs fully offline, so the checks
        # build on a sandboxed runner instead of failing on a network fetch.
        checks = {
          # Type safety: formatted and lints clean (clippy compiles the
          # workspace, so it subsumes `cargo check`).
          type-safety = local-ci.overrideAttrs (old: {
            pname = "check-type-safety";
            nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ rustToolchain ];
            buildPhase = ''
              runHook preBuild
              cargo fmt --all -- --check
              cargo clippy --workspace --all-targets -- -D warnings
              runHook postBuild
            '';
            doCheck = false;
            installPhase = "touch $out";
          });

          # Unit tests: the full workspace test suite, against vendored deps.
          unit-tests = local-ci.overrideAttrs (old: {
            pname = "check-unit-tests";
            nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ rustToolchain ];
            buildPhase = ''
              runHook preBuild
              cargo test --workspace
              runHook postBuild
            '';
            doCheck = false;
            installPhase = "touch $out";
          });

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
