// fn main() -> Result<(), Box<dyn std::error::Error>> {
//     tonic_prost_build::compile_protos("proto/greeter.proto")?;
//     tonic_prost_build::compile_protos("proto/loglevel.proto")?;
//     Ok(())
// }

use std::env;
use std::path::Path;

use eyre::{Result, WrapErr, eyre};

fn main() -> Result<()> {
    //    println!("cargo::rerun-if-changed=src/proto");
    println!("cargo::rerun-if-changed=src/proto");
    let hack = env::var_os("OUT_DIR").ok_or_else(|| eyre!("`OUT_DIR` env not set"))?;

    let out_dir = Path::new(&hack);

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("reflection.bin"))
        .compile_protos(
            &["src/proto/greeter.proto", "src/proto/loglevel.proto"],
            &["src/proto"],
        )
        .wrap_err("failed to compile protocol buffers in build.rs")?;

    Ok(())
}
