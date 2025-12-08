{
  description = "WIP DO NOT USE THIS IS A TOTAL HOLD MY BEER EXPERIMENT yeet files and dirs across systems";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    fenix.url = "github:nix-community/fenix";
    treefmt-nix.url = "github:numtide/treefmt-nix";

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs =
    { self, ... }@inputs:
    inputs.flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import inputs.nixpkgs {
          inherit system;

          overlays = [
            inputs.fenix.overlays.default
            (self: super: {
              apple-sdk-test = super.apple-sdk_15;
            })
          ];
        };

        inherit (pkgs) lib;

        craneLib = inputs.crane.mkLib pkgs;

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            (lib.fileset.fileFilter (file: file.hasExt "rs") ./.)
            (lib.fileset.fileFilter (file: file.hasExt "toml") ./.)
            (lib.fileset.fileFilter (file: file.hasExt "proto") ./crates/yeet/src/proto)
            ./Cargo.lock
          ];
        };

        treefmtEval = inputs.treefmt-nix.lib.evalModule pkgs {
          projectRootFile = "flake.nix";
          programs = {
            nixpkgs-fmt.enable = true;
            rustfmt.enable = true;
            taplo.enable = true;
            protolint.enable = true;
          };
        };

        # Common arguments can be set here to avoid repeating them later
        commonArgs = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = [ pkgs.git ];

          buildInputs =
            with pkgs;
            [
              protobuf
              grpcurl
            ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              pkgs.mold-wrapped
              pkgs.lld
            ]
            ++ lib.optionals pkgs.stdenv.isDarwin [
              # Additional darwin specific inputs can be set here
              #              pkgs.libiconv
            ];

          # Additional environment variables can be set directly
          # MY_CUSTOM_VAR = "some value";
        };

        # Build *just* the cargo dependencies (of the entire workspace),
        # so we can reuse all of that work (e.g. via cachix) when running in CI
        # It is *highly* recommended to use something like cargo-hakari to avoid
        # cache misses when building individual top-level-crates
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        version = self.rev or self.dirtyShortRev or "nix-flake-cant-get-git-commit-sha";

        individualCrateArgs = commonArgs // {
          inherit cargoArtifacts;
          #          inherit (craneLib.crateNameFromCargoToml { inherit src; }) version;
          # NB: we disable tests since we'll run them all via cargo-nextest
          doCheck = false;
        };

        fileSetForCrate =
          crate:
          lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              (craneLib.fileset.commonCargoSources crate)
              (lib.fileset.fileFilter (file: file.hasExt "rs") ./crates/yeet/src)
              (lib.fileset.fileFilter (file: file.hasExt "proto") ./crates/yeet/src/proto)
              (lib.fileset.maybeMissing ./crates/${crate}/Cargo.toml)
              (lib.fileset.maybeMissing ./crates/${crate}/build.rs)
            ];
          };

        nixEnvArgs = {
          STUPIDNIXFLAKEHACK = version;
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          PROTOC_INCLUDE = "${pkgs.protobuf}/include";
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
          OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include/";
          RUSTFLAGS = "-Aclippy::uninlined_format_args ";
        };

        devArgs = {
          CARGO_PROFILE = "dev";
        };

        releaseArgs = {
          CARGO_PROFILE = "release";
          RUSTFLAGS = "-D warnings";
        };

        # Build the top-level crates of the workspace as individual derivations.
        # This allows consumers to only depend on (and build) only what they need.
        # Though it is possible to build the entire workspace as a single derivation,
        # so this is left up to you on how to organize things
        #
        # Note that the cargo workspace must define `workspace.members` using wildcards,
        # otherwise, omitting a crate (like we do below) will result in errors since
        # cargo won't be able to find the sources for all members.
        yeet = craneLib.buildPackage (
          individualCrateArgs
          // nixEnvArgs
          // releaseArgs
          // {
            pname = "yeet";
            cargoExtraArgs = "-p yeet";
            src = fileSetForCrate ./crates/yeet;
          }
        );
        yeet-dev = craneLib.buildPackage (
          individualCrateArgs
          // nixEnvArgs
          // devArgs
          // {
            pname = "yeet";
            cargoExtraArgs = "-p yeet";
            src = fileSetForCrate ./crates/yeet;
          }
        );
      in
      {
        checks = {
          formatter = treefmtEval.config.build.check self;
          # Build the crates as part of `nix flake check` for convenience
          inherit yeet;

          # Run clippy (and deny all warnings) on the workspace source,
          # again, reusing the dependency artifacts from above.
          #
          # Note that this is done as a separate derivation so that
          # we can block the CI if there are issues here, but not
          # prevent downstream consumers from building our crate by itself.
          yeet-clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          yeet-doc = craneLib.cargoDoc (
            commonArgs
            // {
              inherit cargoArtifacts;
              # This can be commented out or tweaked as necessary, e.g. set to
              # `--deny rustdoc::broken-intra-doc-links` to only enforce that lint
              env.RUSTDOCFLAGS = "--deny warnings";
            }
          );

          # Check formatting
          yeet-fmt = craneLib.cargoFmt {
            inherit src;
          };

          yeet-toml-fmt = craneLib.taploFmt {
            src = pkgs.lib.sources.sourceFilesBySuffices src [ ".toml" ];
            # taplo arguments can be further customized below as needed
            # taploExtraArgs = "--config ./taplo.toml";
          };

          # Audit dependencies
          yeet-audit = craneLib.cargoAudit {
            inherit src;
            inherit (inputs) advisory-db;
          };

          # Run tests with cargo-nextest
          # Consider setting `doCheck = false` on other crate derivations
          # if you do not want the tests to run twice
          yeet-nextest = craneLib.cargoNextest (
            commonArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass";
            }
          );
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          yeet-int-basic = pkgs.callPackage ./nix/basic.nix { inherit yeet; };
        };

        packages = {
          inherit yeet yeet-dev;
          default = yeet-dev;
        };

        apps = {
          yeet = inputs.flake-utils.lib.mkApp {
            drv = yeet;
          };
          yeet-dev = inputs.flake-utils.lib.mkApp {
            drv = yeet-dev;
          };
          # Makes updating everything at once a bit easier.
          # nix run .#update
          update = {
            type = "app";
            program = "${
              pkgs.writeShellApplication {
                name = "update";
                # runtimeInputs = [
                #   pkgs.nix
                #   pkgs.jq
                # ];
                text = ''
                  set -e
                  nix flake update
                  cargo update --verbose
                  cargo upgrade --verbose
                '';
              }
            }/bin/update";
          };
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = (
            with pkgs;
            [
              cargo-bloat
              cargo-edit
              cargo-outdated
              cargo-unused-features
              gitFull
              grpcui
              grpcurl
              nil
              nixfmt-rfc-style
              protobuf
              taplo
              treefmt
              protolint
            ]
            ++ commonArgs.buildInputs
            ++ commonArgs.nativeBuildInputs
            ++ lib.optionals pkgs.stdenv.isDarwin [
              pkgs.apple-sdk-test
            ]
          );
        };
      }
    );
}
