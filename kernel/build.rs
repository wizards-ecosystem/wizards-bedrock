//! Points the final link alone at `linker.ld`.
//! `.cargo/config.toml` cannot name the script: its rustflags apply to every crate built for the
//! kernel target, and only the binary wants a script.

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("Cargo sets CARGO_MANIFEST_DIR for build scripts");
    println!("cargo:rustc-link-arg=-T{manifest_dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
}
