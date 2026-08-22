//! Points the linker at `linker.ld`.
//!
//! The script cannot be named in `.cargo/config.toml`, because the rustflags
//! there apply to every crate built for the kernel target and only the binary
//! wants a script. A `-C link-arg` from here reaches the final link alone.

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg=-T{manifest_dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
}
