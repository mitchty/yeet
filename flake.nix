{
  description = "WIP DO NOT USE THIS IS A TOTAL HOLD MY BEER EXPERIMENT";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # TODO for now I'm only building static linux binaries thats where I needs
  # this stuff. I'll be sure it works on macos but I'll save that for later.
  #
  # What I want to test is linux specific anyway. When I get a new macbook air I
  # can put in the effort to port things.
  outputs = { nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-darwin" ] (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        inherit (pkgs) lib;

        craneLib = if system == "x86_64-linux" then (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.default.override {
          targets = [ "x86_64-unknown-linux-musl" ];
        }) else (crane.mkLib pkgs);

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = [ ];
          nativeBuildInputs = [ ] ++ lib.optionals pkgs.stdenv.isLinux [
            pkgs.mold-wrapped
            pkgs.lld
          ];
        };

        staticEnv = {
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
          OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include/";
        } // lib.attrsets.optionalAttrs pkgs.stdenv.isLinux {
          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
          CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
          RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
        };

        yeet = craneLib.buildPackage (commonArgs // staticEnv // {
          CARGO_PROFILE = "dev";
        });

        yeet-release = craneLib.buildPackage (commonArgs // staticEnv // {
          CARGO_PROFILE = "release";
          RUSTFLAGS = "-D warnings";
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

        devShells.default = craneLib.devShell {
          packages = [
            (pkgs.writeScriptBin "fmtall" ''
              taplo fmt
              cargo fmt
            '')
            pkgs.cargo-outdated
            pkgs.cargo-bloat
            pkgs.cargo-edit
            pkgs.cargo-unused-features
            pkgs.gnumake
            pkgs.taplo
          ];
        };
      });
}
