use std::env;
use std::fs;
use std::path::Path;

use eyre::{Result, WrapErr, eyre};
use git_version::git_version;
use rustc_version::version;

fn main() -> Result<()> {
    println!("cargo::rerun-if-changed=src/proto");
    println!("cargo::rerun-if-changed=build.rs");

    let rustc_version = version().expect("Failed to get rustc version");
    println!("cargo:rustc-env=RUSTC_VERSION={}", rustc_version);

    let cargo_profile = std::env::var("PROFILE").expect("couldn't determine cargo profile");
    println!("cargo:rustc-env=BUILD_CARGO_PROFILE={}", cargo_profile);

    // Why is it so hard to just include .git as a fileset in a nix flake in the
    // source derivation, some build systems are ass. All I wanted is .git in
    // the source derivation.
    let nix_hack_env = "STUPIDNIXFLAKEHACK";
    let nix_rev_missing = "nixunknown";
    let unknown = "unknown";

    let gitver = git_version!(fallback = unknown);
    let nixisbusted = std::env::var("STUPIDNIXFLAKEHACK").unwrap_or(String::from(nix_rev_missing));

    if gitver == unknown && nixisbusted == nix_rev_missing {
        panic!();
    } else if gitver == unknown || nixisbusted != nix_rev_missing {
        println!(
            "cargo:rustc-env={}={}",
            nix_hack_env,
            std::env::var(nix_hack_env).unwrap_or(String::from(nix_rev_missing))
        );
    } else {
        println!("cargo:rustc-env={}={}", nix_hack_env, gitver,);
    }

    let hack = env::var_os("OUT_DIR").ok_or_else(|| eyre!("`OUT_DIR` env not set"))?;

    let out_dir = Path::new(&hack);

    let proto_base = "src/proto";
    let proto_dir = Path::new(proto_base);
    let proto_files: Vec<String> = fs::read_dir(proto_dir)
        .wrap_err("failed to read src/proto directory")?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("proto") {
                path.to_str().map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();

    if proto_files.is_empty() {
        return Err(eyre!("couldn't find .proto files in {proto_base}"));
    }

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("reflection.bin"))
        .compile_protos(&proto_files, &[proto_base.to_string()])
        .wrap_err("failed to compile protocol buffers in build.rs")?;

    Ok(())
}
