//! Points the linker at `linker.ld`.
//!
//! `.cargo/config.toml` rustflags apply to every kernel-target crate, so only this script passes
//! `-C link-arg`.

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("Cargo sets CARGO_MANIFEST_DIR for build scripts");
    println!("cargo:rustc-link-arg=-T{manifest_dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
}
