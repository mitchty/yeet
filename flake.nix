{
  description = "Building static binaries with musl";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" ] (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        inherit (pkgs) lib;

        craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.default.override {
          targets = [ "x86_64-unknown-linux-musl" ];
        });

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = [ ];
          # buildInputs = [ ] ++ lib.optionals pkgs.stdenv.isLinux [
          #   pkgs.mold-wrapped
          #   pkgs.lld
          # ];
          nativeBuildInputs = [ ] ++ lib.optionals pkgs.stdenv.isLinux [
            pkgs.mold-wrapped
            pkgs.lld
          ];
        };

        staticEnv = {
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
          OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include/";

          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
          RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
        };

        yeet = craneLib.buildPackage (commonArgs // staticEnv // {
          strictDeps = true;

          CARGO_PROFILE = "dev";
        });

        yeet-release = craneLib.buildPackage (commonArgs // staticEnv // {
          strictDeps = true;

          CARGO_PROFILE = "release";
        });
      in
      {
        checks = {
          inherit yeet;
        };

        packages = {
          default = yeet;
          release = yeet-release;
        };
      });
}
