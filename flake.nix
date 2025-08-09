{
  description = "WIP DO NOT USE THIS IS A TOTAL HOLD MY BEER EXPERIMENT";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # TODO for now I'm only building static linux binaries thats where I needs
  # this stuff. I'll be sure it works on macos but I'll save that for later.
  #
  # What I want to test is linux specific anyway. When I get a new macbook air I
  # can put in the effort to port things.
  outputs =
    { self, ... }@inputs:
    inputs.flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-darwin" ] (
      system:
      let
        pkgs = import inputs.nixpkgs {
          inherit system;
          overlays = [
            inputs.fenix.overlays.default
          ];
        };

        treefmtEval = inputs.treefmt-nix.lib.evalModule pkgs {
          projectRootFile = "flake.nix";
          programs = {
            nixpkgs-fmt.enable = true;
            rustfmt.enable = true;
            taplo.enable = true;
          };
        };
        inherit (pkgs) lib;

        # TODO: iff I set this up to run on nixos arm don't be so explicit
        staticToolchain = with pkgs; [
          (
            fenix.targets.x86_64-unknown-linux-musl.stable.withComponents [
              "cargo"
              "clippy"
              "rust-src"
              "rustc"
              "rustfmt"
            ]
            ++ pkgs.fenix.targets.x86_64-unknown-linux-musl.stable.rust-std
          )
        ];

        commonToolchain = with pkgs; [
          (fenix.complete.withComponents [
            "cargo"
            "clippy"
            "rust-src"
            "rustc"
            "rustfmt"
          ])
          rust-analyzer-nightly
        ];

        toolchain = pkgs.fenix.combine commonToolchain; # (if pkgs.stdenv.isLinux then muslToolchain else commonToolchain);

        craneLib = (inputs.crane.mkLib pkgs).overrideToolchain toolchain;

        srcRoot = ./.;

        src = lib.fileset.toSource {
          root = srcRoot;
          fileset = lib.fileset.unions [
            (craneLib.fileset.commonCargoSources srcRoot)
            (lib.fileset.maybeMissing ./proto)
            (lib.fileset.maybeMissing ./src)
          ];
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = with pkgs; [
            protobuf
            grpcurl
          ];
          nativeBuildInputs =
            [ ]
            ++ lib.optionals pkgs.stdenv.isLinux [
              pkgs.mold-wrapped
              pkgs.lld
            ];
        };

        staticEnv =
          {
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgs.protobuf}/include";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include/";
            RUSTFLAGS = "-Aclippy::uninlined_format_args ";
            # RUSTFLAGS =
            #   lib.optionalString pkgs.stdenv.isLinux "-C link-arg=-fuse-ld=mold "
            #   + "-Aclippy::uninlined_format_args ";
            # }
            # // lib.attrsets.optionalAttrs pkgs.stdenv.isLinux {
            #   CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
            #   CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
          };

        yeet = craneLib.buildPackage (
          commonArgs
          // staticEnv
          // {
            CARGO_PROFILE = "dev";
          }
        );

        yeet-release = craneLib.buildPackage (
          commonArgs
          // staticEnv
          // {
            CARGO_PROFILE = "release";
            RUSTFLAGS = "-D warnings";
          }
        );
      in
      {
        formatter = treefmtEval.config.build.wrapper;

        checks = {
          inherit yeet;
          formatter = treefmtEval.config.build.check self;
        };

        packages = {
          default = yeet;
          release = yeet-release;
        };

        devShells.default = craneLib.devShell {
          packages = (
            with pkgs;
            [
              cargo-outdated
              cargo-bloat
              cargo-edit
              cargo-unused-features
              gnumake
              taplo
              treefmt
              nixfmt-rfc-style
              protobuf
              grpcurl
            ]
            ++ commonArgs.buildInputs
          );
        };
      }
    );
}
