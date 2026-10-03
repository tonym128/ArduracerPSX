//! Inject PSoXide's PSX linker script into the final link.
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest
        .parent()
        .expect("game crate must live in repo root");
    let ld = repo_root.join("psoxide/sdk/psoxide.ld");
    let ld = ld.canonicalize().unwrap_or(ld);

    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rustc-link-arg=--oformat=binary");
    println!("cargo:rerun-if-changed={}", ld.display());
}
