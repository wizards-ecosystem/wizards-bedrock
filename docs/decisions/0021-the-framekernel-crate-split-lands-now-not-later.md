# D-21 The framekernel crate split lands now, not later

Status: Accepted, 2026-08-12.

**Decision.** Create `wost/` now, while the kernel is small, rather than
defer the split or impose it as module convention inside one crate. `wost/`
owns boot handoff, port I/O, MMIO, CPU control, locking and the panic token.
`kernel/` carries `#![deny(unsafe_code)]` and has no hand-written `unsafe`:
the UART driver, console, logging and panic reporting are safe Rust over
`wost`'s abstractions. `BootInfo` and `Framebuffer` remove `limine` from the
kernel's dependency list.

**Rationale.** The migration will never be cheaper, and D-13's lesson is
that safe MMIO/DMA abstractions cannot be retrofitted.

**Consequences.**

- **`kernel/` uses `deny(unsafe_code)`, not `forbid`, amending D-13.**
  `wost::entry!` expands `#[allow(unsafe_code)]` glue into the binary crate,
  and rustc rejects any `allow` under `forbid`, even in a macro expansion.
  Future service crates (scheduler, VFS, drivers) have no entry glue and
  keep `forbid`.
- **The entry glue expands in the binary crate, not in `wost`.** The linker
  can drop `#[used]` statics in an rlib wholesale: nothing references the
  archive member holding them, so `#[used]` never protects it
  (rust-lang/rust#47384). `linker.ld` and `build.rs` stay in `kernel/` too:
  a build script's `rustc-link-arg` does not reach dependents, so a
  `wost/build.rs` emitting `-T linker.ld` would silently do nothing.
- `ENTRY(kmain)` is unchanged; the macro still emits a symbol named `kmain`.
