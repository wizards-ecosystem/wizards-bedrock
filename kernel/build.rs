//! Points the final link alone at `linker.ld`.
//! `.cargo/config.toml` cannot name the script: its rustflags apply to every crate built for the
//! kernel target, and only the binary wants a script.

fn main() -> Result<(), std::env::VarError> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    println!("cargo:rustc-link-arg=-T{manifest_dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
    Ok(())
}
