use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("app.manifest");
        println!("cargo:rustc-link-arg-bin=warp-engine-generator=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=warp-engine-generator=/MANIFESTINPUT:{}", manifest.display());
    }
}
