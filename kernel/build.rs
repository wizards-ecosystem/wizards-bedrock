// Only the final link takes the script: config rustflags would apply it to every kernel crate.
fn main() -> Result<(), std::env::VarError> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    println!("cargo:rustc-link-arg=-T{manifest_dir}/linker.ld");
    println!("cargo:rustc-link-arg=--orphan-handling=error");
    println!("cargo:rerun-if-changed=linker.ld");
    Ok(())
}
