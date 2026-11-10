# D-21 The framekernel crate split lands now, not later

Status: Accepted, 2026-08-12.

**Context.** D-13 committed to Asterinas-style framekernel discipline: all
`unsafe` in a privileged framework crate, every other kernel crate safe. The
Asterinas authors' lesson is that safe MMIO/DMA abstractions must be
designed in from day one, not retrofitted. At the end of Phase 0 the kernel
was 242 lines, with `unsafe` spread across two peer driver modules, no
framework crate and no `// SAFETY:` comment.

**Options.** (a) Defer the split until there is more kernel to split.
(b) Impose the layering as module structure inside one crate, so the
boundary is convention. (c) Create `wost/` now, while the whole kernel is
242 lines.

**Decision.** (c). `wost/` owns boot handoff, port I/O, MMIO, CPU control,
locking and the panic token. `kernel/` carries `#![deny(unsafe_code)]` and
has no hand-written `unsafe`: the UART driver, console, logging and panic
reporting are safe Rust over `wost`'s abstractions. `BootInfo` and
`Framebuffer` remove `limine` from the kernel's dependency list.

**Rationale.** The migration will never be cheaper; later means
retrofitting the abstractions the research says cannot be retrofitted. The
split paid off at once: `MmioRegion`'s bounds check makes a non-32-bpp
framebuffer a typed error, not an out-of-bounds write, and `port::claim`'s
registry turns "no other driver owns these ports" from a comment into a
checked fact.

**Consequences.**

- **`kernel/` uses `deny(unsafe_code)`, not `forbid`, amending D-13.**
  `wost::entry!` expands `#[allow(unsafe_code)]` glue into the binary crate,
  and rustc rejects any `allow` under `forbid`, even in a macro expansion.
  Future service crates (scheduler, VFS, drivers) have no entry glue and
  keep `forbid`.
- **The entry glue expands in the binary crate, not in `wost`.** The linker
  can drop `#[used]` statics in an rlib wholesale: nothing references the
  archive member holding them, so `#[used]` never protects it
  (rust-lang/rust#47384). Every `-rt` crate in the ecosystem does the same.
  `linker.ld` and `build.rs` stay in `kernel/` too: a build script's
  `rustc-link-arg` does not reach dependents, so a `wost/build.rs` emitting
  `-T linker.ld` would silently do nothing.
- `ENTRY(kmain)` is unchanged; the macro still emits a symbol named `kmain`.
- Verified 2026-08-12 on QEMU: identical serial banner, framebuffer
  background byte-identical at `0x0B1021`, panic screen and double-panic
  guard exercised, scrolling verified over 60 lines.
