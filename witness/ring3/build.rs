// Only the final link takes the script: config rustflags would apply it to every crate.
fn main() -> Result<(), std::env::VarError> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    println!("cargo:rustc-link-arg-bins=-T{manifest_dir}/link.ld");
    println!("cargo:rustc-link-arg-bins=--orphan-handling=error");
    println!("cargo:rerun-if-changed=link.ld");
    Ok(())
}
