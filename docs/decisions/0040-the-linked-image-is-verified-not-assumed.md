# D-40 The linked image is verified, not assumed

Status: Accepted, 2026-08-22.

**Decision.** `linker.ld` names `.got`, `.got.plt` and `.data.rel.ro`, puts
`.bss` last and `(NOLOAD)`, discards `.comment`, and gives the Limine
request block its own `KEEP()`-ed `.requests` section. `xtask/src/elf.rs`
reads the linked ELF after every build and requires: the entry point in the
higher half; every allocated section inside a `PT_LOAD`; no segment both
writable and executable; no `NOBITS` section costing file space; and `.text`
and `.requests` present and loaded. External tooling was rejected on D-16's
grounds: the build stays self-contained.

**Rationale.** A linker script fails loudly when malformed and quietly when
merely incomplete: an orphan `.got` after `.bss` linked, booted, and forced
all of `.bss` into the file. Segment-level W^X was already relied on (Limine
maps each `PT_LOAD` by its flags) but nothing asserted it, and Phase 3's
page tables must reproduce it.

**Consequences.** Naming `.requests` lets it be `KEEP()`-ed, as D-21's note
about linkers dropping `#[used]` statics asks. The reader is host-tested on
truncated input, because a verifier that panics on malformed input stops the
build for the wrong reason.
