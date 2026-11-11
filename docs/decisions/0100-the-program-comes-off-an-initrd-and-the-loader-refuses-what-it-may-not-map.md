# D-100 The program comes off an initrd, and the loader refuses what it may not map

Status: Accepted, 2026-08-24.

**Decision.** The image carries one program as a Limine module, and the kernel
loads it by walking its program headers.

**The program is assembled by `xtask`, not compiled.** A compiled userland needs
a second crate, a second link address and a fourth pass in the gate's
configuration matrix (D-22), and none of that tests the loader, which needs a
genuine ELF on a genuine initrd. A compiled userland arrives with something
worth running.

**The builder is checked against the consumer.** `xtask` writes the header field
by field from the specification, and its tests parse it back with `libs/elf64`,
the kernel's crate. A builder verified by its own reading of the format agrees
with itself whatever it does; verified against the reader, it proves the two
agree. A test checks that the entry point lands inside a loadable segment.

**The refusals are the loader.** An image is bytes a program supplied, so each
of these asks the kernel to map something it should not:

- **Writable and executable at once:** refused. *W^X is not relaxed for an image
  that asks nicely* (D-12). A loader that dropped the write bit would load
  something that faults later and blames the kernel.
- **Contents that run past the file**, or a segment claiming more file than
  memory: refused, with the overflow-safe helpers `libs/elf64` provides.
- **A segment in the kernel's half:** refused against `USER_END` (D-97), never
  by a shift (D-83).
- **Anything that is not an ELF:** refused by the parser, which never panics
  because the bytes are not the kernel's.

**Pages are filled before they are mapped.** `Segment::write_at` fills the
frames while only the loader holds them, then the mapping is read-only and
executable. Mapping writable, copying, then demoting has an instant where the
page is both, and W^X is a property that holds at every instant or not at all.

**Rationale.** The check doctors a copy of the *shipped* initrd one field at a
time, so what is refused differs from what loads by exactly the thing under
test.

**Consequences.**

- `wost::boot` takes one `Requests` struct, the shape the handoff needs as it
  grows with every protocol feature. `Requests` carries no `Debug`, against
  C-DEBUG, because a hand-written impl would log static request addresses
  that D-46's KASLR ledger would rather not see.
- **Nothing shipped runs a program.** `mod user` is still
  `#[cfg(feature = "selftest")]`: there is an initrd in the image and only the
  checks read it. A shipped caller needs `spawn`, which needs a second address
  space.
- The two programs still share the kernel's address space and load at addresses
  shifted apart. That is not isolation, and it remains the largest thing slice 7
  owes.
