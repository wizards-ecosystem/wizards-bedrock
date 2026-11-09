# D-100 The program comes off an initrd, and the loader refuses what it may not map

Status: Accepted, 2026-08-24.

**Context.** D-96 put four bytes of machine code in a page and jumped to them.
That proved the privilege boundary and nothing about loading. The roadmap's
Phase 5 wants ELF loading and an initrd, and `libs/elf64` has existed since
earlier in this slice with nothing on the machine using it.

**Decision.** The image carries one program as a Limine module, and the kernel
loads it by walking its program headers.

**The program is assembled by `xtask`, not compiled.** A real userland needs a
second crate, a second link address, and a fourth pass in the gate's
configuration matrix (D-22) — and none of that tests the loader. What the
loader needs is a genuine ELF on a real initrd, and 124 bytes is the smallest
thing that is one. A compiled userland arrives with something worth running.

**The builder is checked against the consumer, not against itself.** `xtask`
writes the header field by field from the specification, and its tests parse
the result back with `libs/elf64` — the crate the kernel uses. A builder
verified by its own understanding of the format agrees with itself whatever it
does; verified against the reader, it proves the two agree. Four host tests,
including that the entry point lands inside a loadable segment, which a
hand-built image gets wrong and a linker never does.

**The refusals are the loader.** An image is bytes a program supplied, so each
of these is a way of asking the kernel to map something it should not:

- **Writable and executable at once** — refused. *W^X is not relaxed for an
  image that asks nicely* (D-12). A loader that dropped the write bit to make a
  program load would be loading something that faults later and blames the
  kernel.
- **Contents that run past the file**, or a segment claiming more file than
  memory — refused, and both are checked with the overflow-safe helpers
  `libs/elf64` provides rather than by the caller remembering.
- **A segment in the kernel's half** — refused against `USER_END` (D-97), never
  by a shift (D-83).
- **Anything that is not an ELF** — refused by the parser, which never panics
  because the bytes are not the kernel's.

**Pages are filled before they are mapped.** `Segment::write_at` puts the image
in the frames while they are nobody's but the loader's, and the mapping that
follows is read-only and executable. The alternative — map writable, copy,
then demote — has an instant where the page is both, and W^X is a property
that holds at every instant or not at all.

**Rationale.** The check that matters doctors a copy of the *real* initrd, one
field at a time, so what is refused differs from what loads by exactly the
thing under test. Breaking the W^X refusal turns it red.

And the loaded program is what the user-copy check now reads back across the
boundary: bytes `xtask` wrote into the image, mapped from a program header, and
read through the *user* mapping by `copy_from_user`. Three subsystems agreeing
about four bytes.

**Consequences.**

- One hundred and one named machine checks.
- `wost::boot` grew a module request and `BootInfo` a `module` field. `enter`'s
  argument list grew past what clippy allows, so the requests became one
  `Requests` struct — which is the shape the handoff should have had anyway,
  since it grows with every protocol feature.
- `Requests` carries no `Debug`, against C-DEBUG, and says why: the bootloader's
  types have none, and a hand-written impl would print the addresses of static
  requests into a log — which is not information anyone wants and is exactly
  what D-46's KASLR ledger would rather not see.
- **Nothing shipped runs a program.** `mod user` is still
  `#[cfg(feature = "selftest")]`: there is an initrd in the image and only the
  checks read it. A shipped caller needs `spawn`, which needs a second address
  space — the rest of this slice.
- The two programs still share the kernel's address space and are loaded at
  addresses shifted apart. That is not isolation and remains the largest thing
  slice 7 owes.
