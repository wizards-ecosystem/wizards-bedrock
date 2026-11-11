# D-17 Toolchain pins and workarounds

Status: Accepted, 2026-07-18. Review trigger owned by PW-705 (2026-08-30).

**Decision.**

- **Rust 1.97.1**, pinned exactly via `rust-toolchain.toml`
  (`profile = "minimal"`, `rustfmt`/`clippy`, the `x86_64-unknown-none`
  target). `rust-mingw` is absent from the component list: rustup supplies
  it with a Windows-GNU host, and naming it breaks other hosts (D-24).
- **`limine` crate = 0.5.0 exactly**: 0.6.x requires nightly
  (`ptr_metadata`). The pinned crate requests protocol base revision 4
  through `with_revision` (D-42). Revisit the crate pin when 0.6's
  requirement reaches stable or we adopt nightly. Crate and protocol
  revisions are separate.
- **`fatfs` without its `chrono` feature**: chrono's Windows bindings need
  `dlltool.exe`, absent from the self-bundled GNU toolchain. Disk images get
  fixed timestamps, irrelevant for boot media.
- **Kernel link flags**: `code-model=kernel`, `relocation-model=static`
  (higher-half at `0xffffffff80000000`); `build.rs` passes the linker script
  by absolute path so builds work from any directory.

**Current disposition (2026-08-30).** Calendar review is retired. PW-705
fires only when stable support appears, a pin or workaround blocks a
supported host or required dependency, or a concrete configuration/ABI
caller needs a change.
