# D-386 Shipped debug information names stable virtual roots

Status: Accepted, 2026-10-11. Closes PW-709 when qualified.

**Decision.** Every `xtask` kernel build replaces the target configuration's
implicit rustflags with one exact encoded list: the kernel code model, static
relocation model and three `--remap-path-prefix` rules for the real source root,
compiler sysroot and Cargo home. They map to `/wizard/source`,
`/wizard/toolchain` and `/wizard/cargo`. The command also pins `RUSTC` and
`RUSTDOC` to the exact binaries beside that Cargo and puts their directory
first on `PATH`, so a dependency-local toolchain file cannot select another
compiler for one graph. The compiler audit uses the same flag
constructor with its authenticated copied source, compiler and materialized
registry roots, so its semantic graph is the graph that ships rather than a
parallel description. Compiler diagnostics that name the virtual source root
are mapped back only after an exact prefix match and a traversal-free relative
path check; both slash forms are admitted because Windows rustc renders its
native separator after the Unix-shaped virtual prefix. Host-only Cargo commands
keep empty encoded rustflags.

Release evidence copies the clean tracked tree into two exclusive temporary
checkout roots and builds the shipped kernel and image from each into its
existing private target directory. It rejects either artifact when the real
checkout, Cargo home or sysroot byte spelling remains, then retains the
existing byte-identity and exact-image boot requirements. Temporary source
copies are removed after their bounded build operations. The second build runs
through private toolchain and Cargo-home roots whose immutable compiler, library
and registry inputs are hard-linked from the already selected bytes, so the path
moves without duplicating roughly three gigabytes of payload. The
private Cargo home receives no credentials and the alternate Cargo process is
offline, locked, bounded and removed with those temporary roots. The debugger supplies
GDB's inverse `set substitute-path` from `/wizard/source` to the current
checkout before loading frames, preserving source-level debugging without
putting host paths back into the artifact.

Host tests bind the exact encoded codegen and three mapping arguments, show
that kernel commands install them while host commands do not, bind the evidence
builder to two different source roots and require all three original roots
absent from ELF and image bytes. Persistent mutations independently omit the
source remap, let evidence reuse one checkout and omit the GDB substitution.
Qualification builds byte-identical shipped ELFs and images from both copied
roots with different toolchain and Cargo-home paths, passes the compiler API
audit and passes the pinned `cargo xtask debug --check` workflow.

**Rationale.** Release kernels retain DWARF, which carried the absolute
checkout, Cargo home and Rust toolchain paths into the ELF and disk image;
publishing them would disclose a host layout and make an otherwise identical
build differ when a root moves. Evidence built two target directories from one
source directory, so it could not detect the dependency. Stripping DWARF would
discard deliberately retained diagnostics, mapping only the checkout still
leaks dependency and standard-library paths, and a post-link byte rewrite would
not understand DWARF lengths and checksums.
