{
  description = "WIP DO NOT USE THIS IS A TOTAL HOLD MY BEER EXPERIMENT yeet files and dirs across systems";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    fenix.url = "github:nix-community/fenix";
    treefmt-nix.url = "github:numtide/treefmt-nix";

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

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
        # DRY some of the meta definitions for apps/packages for this chungus amungus
        metaCommon = desc: {
          description = if desc == "" then "yeet" else "yeet " + desc;
          mainProgram = "yeet";
        };

        stableRust = (
          inputs.fenix.packages.${system}.stable.withComponents [
            "cargo"
            "clippy"
            "llvm-tools"
            "rustc"
            "rust-src"
            "rustfmt"
            "rust-analyzer"
          ]
        );

        pkgs = import inputs.nixpkgs {
          inherit system;

          overlays = [
            inputs.fenix.overlays.default
            (self: super: {
              apple-sdk-test = super.apple-sdk;
            })
          ];
        };

        pkgsMusl = import inputs.nixpkgs {
          inherit system;
          overlays = [ inputs.fenix.overlays.default ];
          crossSystem = {
            config = "${pkgs.stdenv.hostPlatform.parsed.cpu.name}-unknown-linux-musl";
          };
        };

        pkgsDarwin =
          if pkgs.stdenv.isDarwin then
            import inputs.nixpkgs {
              inherit system;
              overlays = [ inputs.fenix.overlays.default ];
              # Use the host platform to get system-only linking
              crossSystem = pkgs.stdenv.hostPlatform;
            }
          else
            null;

        pkgsWindows = import inputs.nixpkgs {
          inherit system;
          overlays = [ inputs.fenix.overlays.default ];
          crossSystem = {
            config = "x86_64-w64-mingw32";
            libc = "msvcrt";
          };
        };

        inherit (pkgs) lib;

        craneLib = inputs.crane.mkLib pkgs;

        craneLibMusl =
          let
            muslTarget = "${pkgs.stdenv.hostPlatform.parsed.cpu.name}-unknown-linux-musl";
          in
          (inputs.crane.mkLib pkgsMusl).overrideToolchain (
            p:
            p.fenix.combine [
              p.fenix.stable.rustc
              p.fenix.stable.cargo
              p.fenix.targets.${muslTarget}.stable.rust-std
            ]
          );

        # Crane lib for Darwin builds that only link system libraries
        craneLibDarwin =
          if pkgs.stdenv.isDarwin then
            (inputs.crane.mkLib pkgsDarwin).overrideToolchain (
              p:
              p.fenix.combine [
                p.fenix.stable.rustc
                p.fenix.stable.cargo
                p.fenix.stable.rust-std
              ]
            )
          else
            null;

        craneLibWindows = (inputs.crane.mkLib pkgsWindows).overrideToolchain (
          p:
          p.fenix.combine [
            p.fenix.stable.rustc
            p.fenix.stable.cargo
            p.fenix.targets.x86_64-pc-windows-gnu.stable.rust-std
          ]
        );

        # Constrained src fileset to ensure that cargo deps aren't rebuilt every
        # change to crates.
        #
        # Mostly just here to be sure that build.rs using tonic notices proto
        # files and anything that affects dependencies for cargo directly.
        srcDeps = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.lock
            ./Cargo.toml
            (lib.fileset.fileFilter (file: file.hasExt "toml") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "proto") ./crates)
            # build.rs is needed if I make changes to it that will affect deps+build time deps
            (lib.fileset.fileFilter (file: file.name == "build.rs") ./crates)
          ];
        };

        # All the junk in the trunk not used for cache dep validation
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            (lib.fileset.fileFilter (file: file.hasExt "rs") ./crates)
            (lib.fileset.fileFilter (file: file.hasExt "toml") ./.)
            (lib.fileset.fileFilter (file: file.hasExt "proto") ./crates/yeet/src/proto)
            ./Cargo.lock
          ];
        };

        treefmtEval = inputs.treefmt-nix.lib.evalModule pkgs {
          projectRootFile = "flake.nix";
          programs = {
            # Because I keep on forgetting, the rfc style formatter is the
            # default for at least year now.. ref:
            # https://github.com/numtide/treefmt-nix/blob/main/programs/nixfmt-rfc-style.nix
            nixfmt.enable = true;
            rustfmt = {
              enable = true;
              edition = "2024";
            };
            taplo.enable = true;
          };
          settings.formatter.protolint = {
            command = pkgs.protolint;
            options = [
              "lint"
              "-fix"
            ];
            includes = [ "*.proto" ];
          };
        };

        # TOO MANY DAM LAYERS OF SHENANIGANS
        #
        # So... because the hooks are their own derivation, need to be sure crap
        # like treefmt has all the formatters it needs in its derivation PATH
        # too.
        #
        # These tools are made available in the hook environment's PATH
        #
        # These things are common between the hook derivation setup and used for the devShell
        hookTools = with pkgs; {
          inherit
            # Formatters needed by treefmt
            taplo
            nixfmt-rfc-style
            rustfmt
            protolint
            # Build tools needed by nix flake check
            git
            protobuf
            grpcurl
            # Nix itself for running checks
            nix
            # treefmt itself
            treefmt
            ;
        };

        # Instead of running nix flake check on each commit (e.g. in
        # pre-commit), lets just be sure we're golden at push time.
        #
        # I can rewrite the commit history to fix it at that point if things
        # fail or not.
        git-hooks-check = inputs.git-hooks.lib.${system}.run {
          src = ./.;
          tools = hookTools;
          hooks = {
            nix-flake-check = {
              enable = true;
              name = "nix-flake-check";
              entry = "${pkgs.nix}/bin/nix flake check -L";
              language = "system";
              pass_filenames = false;
              stages = [ "pre-push" ];
            };
            # Make sure code is formatted in pre-commit
            # Note: We use the formatter check separately, so we disable this
            # in the git-hooks check to avoid sandbox timestamp issues
            treefmt.enable = false;
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

        commonArgsMusl = {
          inherit src;
          strictDeps = true;

          nativeBuildInputs = [ pkgsMusl.git ];

          buildInputs = with pkgsMusl; [
            protobuf
            openssl.dev
          ];

          # For now we'll only build for the host arch. I can deal with cross compilation later for x86_64->aarch64
          CARGO_BUILD_TARGET = "${pkgs.stdenv.hostPlatform.parsed.cpu.name}-unknown-linux-musl";
          CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static -C link-arg=-static";
        };

        # Common arguments for Darwin builds (system libraries only)
        commonArgsDarwin =
          if pkgs.stdenv.isDarwin then
            {
              inherit src;
              strictDeps = true;

              nativeBuildInputs = [ pkgsDarwin.git ];

              buildInputs = with pkgsDarwin; [
                protobuf
                apple-sdk
              ];
            }
          else
            { };

        # https://crane.dev/faq/cross-compiling-aws-lc-sys.html?highlight=aws-lc-sy#i-want-to-cross-compile-aws-lc-sys-to-windows-using-mingw
        commonArgsWindows =
          let
            buildPlatformSuffix = lib.strings.toLower pkgs.pkgsBuildHost.stdenv.hostPlatform.rust.cargoEnvVarTarget;
          in
          {
            inherit src;
            strictDeps = true;

            nativeBuildInputs = with pkgs; [
              git
              protobuf
              buildPackages.nasm
              buildPackages.cmake
            ];

            buildInputs = with pkgsWindows.windows; [ pthreads ];

            CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
            AWS_LC_SYS_PREBUILT_NASM = "0";
            CFLAGS = "-Wno-stringop-overflow -Wno-array-bounds -Wno-restrict";
            CFLAGS_x86_64-pc-windows-gnu = "-I${pkgsWindows.windows.pthreads}/include";
            "CC_${buildPlatformSuffix}" = "cc";
            "CXX_${buildPlatformSuffix}" = "c++";
          };

        # Build *just* the cargo dependencies (of the entire workspace),
        # so we can reuse all of that work (e.g. via cachix) when running in CI
        # It is *highly* recommended to use something like cargo-hakari to avoid
        # cache misses when building individual top-level-crates
        # Note: buildDepsOnly already uses --all-targets by default
        # Important: Must use same env vars (especially RUSTFLAGS) as actual builds
        # Using dev profile by default for better debug info on panics
        cargoArtifacts = craneLib.buildDepsOnly (
          commonArgs
          // nixEnvArgs
          // devArgs
          // {
            src = srcDeps;
          }
        );

        # Cargo artifacts for musl builds
        cargoArtifactsMusl = craneLibMusl.buildDepsOnly (
          commonArgsMusl
          // {
            src = srcDeps;
            PROTOC = "${pkgsMusl.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgsMusl.protobuf}/include";
          }
        );

        # Cargo artifacts for Darwin builds
        cargoArtifactsDarwin =
          if pkgs.stdenv.isDarwin then
            craneLibDarwin.buildDepsOnly (
              commonArgsDarwin
              // {
                src = srcDeps;
                PROTOC = "${pkgsDarwin.protobuf}/bin/protoc";
                PROTOC_INCLUDE = "${pkgsDarwin.protobuf}/include";
              }
            )
          else
            null;

        cargoArtifactsWindows = craneLibWindows.buildDepsOnly (
          commonArgsWindows
          // {
            src = srcDeps;
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgs.protobuf}/include";
          }
        );

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
          # Clippy lints can be set in source via attributes instead
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

        # Default build: dev profile with debug symbols to match cargo parlance
        yeet = craneLib.buildPackage (
          individualCrateArgs
          // nixEnvArgs
          // devArgs
          // {
            pname = "yeet";
            cargoExtraArgs = "-p yeet";
            src = fileSetForCrate ./crates/yeet;
          }
        );

        # Optimized LTO build with release profile
        yeet-lto = craneLib.buildPackage (
          individualCrateArgs
          // nixEnvArgs
          // releaseArgs
          // {
            pname = "yeet";
            cargoExtraArgs = "-p yeet";
            src = fileSetForCrate ./crates/yeet;
          }
        );

        yeet-release-linux = craneLibMusl.buildPackage (
          commonArgsMusl
          // {
            pname = "yeet-release";
            version = version;
            cargoArtifacts = cargoArtifactsMusl;
            cargoExtraArgs = "-p yeet";
            src = fileSetForCrate ./crates/yeet;

            STUPIDNIXFLAKEHACK = version;
            PROTOC = "${pkgsMusl.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgsMusl.protobuf}/include";

            OPENSSL_STATIC = "1";
            OPENSSL_LIB_DIR = "${pkgsMusl.pkgsStatic.openssl.out}/lib";
            OPENSSL_INCLUDE_DIR = "${pkgsMusl.pkgsStatic.openssl.dev}/include";

            # normal builds can run checks
            doCheck = false;

            meta = metaCommon "release static linux build" // {
              platforms = [
                "x86_64-linux"
                "aarch64-linux"
              ];
            };
          }
        );

        # Darwin release build (system libraries only, portable)
        yeet-release-darwin =
          if pkgs.stdenv.isDarwin then
            craneLibDarwin.buildPackage (
              commonArgsDarwin
              // {
                pname = "yeet-release";
                version = version;
                cargoArtifacts = cargoArtifactsDarwin;
                cargoExtraArgs = "-p yeet";
                src = fileSetForCrate ./crates/yeet;

                STUPIDNIXFLAKEHACK = version;
                PROTOC = "${pkgsDarwin.protobuf}/bin/protoc";
                PROTOC_INCLUDE = "${pkgsDarwin.protobuf}/include";

                # Don't check during cross-compilation
                doCheck = false;

                # abuse install_name_tool to rewrite the dynamic link to
                # /nix/store to /usr/lib for iconv. Can't find an easy way to
                # convince the rust toolchain to not do this in nix so whatever
                # its FINE I think...
                postInstall = ''
                  for binary in $out/bin/*; do
                    libiconv_path=$(otool -L "$binary" | grep libiconv | awk '{print $1}' | grep /nix/store || true)
                    if [ -n "$libiconv_path" ]; then
                      install_name_tool -change "$libiconv_path" /usr/lib/libiconv.2.dylib "$binary"
                    fi
                  done
                '';

                meta = metaCommon "release apple silicon build" // {
                  platforms = [
                    "x86_64-darwin"
                    "aarch64-darwin"
                  ];
                };
              }
            )
          else
            null;

        yeet-release-windows = craneLibWindows.buildPackage (
          commonArgsWindows
          // {
            pname = "yeet-release";
            version = version;
            cargoArtifacts = cargoArtifactsWindows;
            cargoExtraArgs = "-p yeet";
            src = fileSetForCrate ./crates/yeet;

            STUPIDNIXFLAKEHACK = version;
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgs.protobuf}/include";

            # Don't check during cross-compilation
            doCheck = false;

            meta = metaCommon "release windows x86_64 build";
          }
        );
      in
      {
        checks = {
          formatter = treefmtEval.config.build.check self;
          git-hooks = git-hooks-check;
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
            // nixEnvArgs
            // devArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );

          yeet-doc = craneLib.cargoDoc (
            commonArgs
            // nixEnvArgs
            // devArgs
            // {
              inherit cargoArtifacts;
              # This can be commented out or tweaked as necessary, e.g. set to
              # `--deny rustdoc::broken-intra-doc-links` to only enforce that lint
              env.RUSTDOCFLAGS = "--deny warnings";
            }
          );

          # Audit dependencies
          # 2025-12-16 commented out cause deps of deps are inactive and not sure how I want to handle that right now
          # yeet-audit = craneLib.cargoAudit {
          #   inherit src;
          #   inherit (inputs) advisory-db;
          # };

          # Run tests with cargo-nextest
          # Consider setting `doCheck = false` on other crate derivations
          # if you do not want the tests to run twice
          yeet-nextest = craneLib.cargoNextest (
            commonArgs
            // nixEnvArgs
            // devArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass";
            }
          );
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux (
          let
            integrationTestFiles = builtins.sort (a: b: a < b) (
              builtins.attrNames (
                lib.filterAttrs (
                  name: type: type == "regular" && lib.hasPrefix "integration-" name && lib.hasSuffix ".nix" name
                ) (builtins.readDir ./nix)
              )
            );

            integrationChecks = lib.listToAttrs (
              map (
                file:
                let
                  withoutPrefix = lib.removePrefix "integration-" file;
                  withoutSuffix = lib.removeSuffix ".nix" withoutPrefix;
                  matched = builtins.match "([0-9][0-9]-)(.+)" withoutSuffix;
                  namePart = if matched != null then builtins.elemAt matched 1 else withoutSuffix;
                in
                {
                  name = "yeet-int-${namePart}";
                  value = pkgs.callPackage (./nix + "/${file}") { inherit yeet; };
                }
              ) integrationTestFiles
            );

            # Should I name this like regression suite or something? Future me task.
            allIntegrationTests =
              pkgs.runCommand "yeet-integration-all"
                {
                  buildInputs = lib.attrValues integrationChecks;
                }
                ''
                  echo "All integration tests passed, build is probably worth using!"
                  touch $out
                '';
          in
          integrationChecks
          // {
            yeet-integration-all = allIntegrationTests;
          }
        );

        packages = {
          inherit yeet yeet-lto;
          default = yeet;
          # Expose checks as packages for individual running with shorter names
          clippy = self.checks.${system}.yeet-clippy;
          doc = self.checks.${system}.yeet-doc;
          nextest = self.checks.${system}.yeet-nextest;
        }
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          yeet-release = yeet-release-linux;
          inherit yeet-release-windows;
          # Make it slightly easier to run the integration nixos vm test suite
          # cause I'm a lazy goober.
          int = self.checks.${system}.yeet-integration-all;
        }
        // lib.optionalAttrs pkgs.stdenv.isDarwin {
          yeet-release = yeet-release-darwin;
        };

        apps = {
          yeet =
            (inputs.flake-utils.lib.mkApp {
              drv = yeet;
            })
            // {
              meta = metaCommon "Dev build";
            };
          yeet-lto =
            (inputs.flake-utils.lib.mkApp {
              drv = yeet-lto;
            })
            // {
              meta = metaCommon "LTO optimized build";
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
            meta = {
              description = "Update flake inputs and cargo dependencies";
              mainProgram = "update";
            };
          };
        }
        # Note its a bit jank but I'm using yeet-release for github action build
        # targets, -release in this parlance isn't cargo build --release its
        # "build release binaries for a commit/tag/version"
        // lib.optionalAttrs pkgs.stdenv.isLinux {
          yeet-release =
            (inputs.flake-utils.lib.mkApp {
              drv = yeet-release-linux;
            })
            // {
              meta = metaCommon "run release static musl based linux build" // {
                platforms = [
                  "x86_64-linux"
                  "aarch64-linux"
                ];
              };
            };
          yeet-release-windows =
            (inputs.flake-utils.lib.mkApp {
              drv = yeet-release-windows;
            })
            // {
              # TODO: is this yeet.exe as the main program? Maybe I can test
              # this out via wine?
              meta = metaCommon "run release cross compiled windows build";
            };
        }
        // lib.optionalAttrs pkgs.stdenv.isDarwin {
          yeet-release =
            (inputs.flake-utils.lib.mkApp {
              drv = yeet-release-darwin;
            })
            // {
              meta = metaCommon "run release portable macos build" // {
                platforms = [
                  "x86_64-darwin"
                  "aarch64-darwin"
                ];
              };
            };
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = (
            with pkgs;
            [
              # This craps all the random stuff I include in the devShell that
              # isn't really a part of the actual binary or anything. Aka
              # grpcurl is there so I can poke at yeet with grpc slightly
              # easier.
              adrs
              cargo-bloat
              cargo-edit
              cargo-outdated
              cargo-unused-features
              gitFull
              grpcui
              grpcurl
              nil
              pandoc
              protobuf
              protolint
              stableRust
            ]
            ++ (lib.attrValues hookTools)
            ++ commonArgs.buildInputs
            ++ commonArgs.nativeBuildInputs
            ++ lib.optionals pkgs.stdenv.isDarwin [
              # I should probably remove this it was a hacky way to debug.
              pkgs.apple-sdk-test
            ]
          );

          # Install git hooks when entering the dev shell
          shellHook = ''
            ${git-hooks-check.shellHook}
          '';

          # Make sure eglot+etc.. pick the right rust-src for eglot+lsp mode stuff using direnv
          RUST_SRC_PATH = "${stableRust}/lib/rustlib/src/rust/library";
        };
      }
    );
}
