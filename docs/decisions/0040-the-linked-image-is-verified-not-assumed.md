# D-40 The linked image is verified, not assumed

Status: Accepted, 2026-08-22.

**Context.** The same audit read the built ELF back and found `.got` placed
*after* `.bss`. It linked, it booted, and it was wrong. `linker.ld` named
neither `.got` nor `.got.plt`, so the linker placed them by orphan
heuristic, and a loadable section after a `NOBITS` one forces all of `.bss`
into the file. Every image carried 29048 bytes of zeros, and `.bss` was
loaded from disk instead of zeroed by the bootloader.

A linker script fails loudly when malformed and quietly when merely
incomplete. Nothing in the gate could tell the difference.

**Options.** (a) Name the orphans and move on; (b) name them and check the
result; (c) adopt a linker-script linter or an external `readelf` in the
gate.

**Decision.** (b). `linker.ld` now names `.got`, `.got.plt` and
`.data.rel.ro`, puts `.bss` last and `(NOLOAD)`, discards `.comment`, and
gives the Limine request block its own `KEEP()`-ed `.requests` section.
`xtask/src/elf.rs` reads the linked ELF after every build and requires: the
entry point in the higher half; every allocated section inside a `PT_LOAD`;
no segment both writable and executable; no `NOBITS` section costing file
space; and `.text` and `.requests` present and loaded. External tooling was
rejected on D-16's grounds: the build stays self-contained, and this is
~200 lines of header reading.

**Rationale.** Segment-level W^X was already true and already relied on
(Limine maps each `PT_LOAD` by its flags), but nothing asserted it, and
Phase 3 is about to build page tables that must reproduce it. An orphan
section is the failure that survives a link, boots, and surfaces months
later as something else.

**Consequences.** The check found a gap on its first run. The request block
had no section of its own and was folded into `.data`, so nothing could
confirm the block `wost::entry!` emits had linked at all. Naming it also let
it be `KEEP()`-ed, which is what D-21's note about linkers dropping `#[used]`
statics asks for. With a loadable section placed after `.bss` on purpose,
the build went red naming `.bss` and the 29048 bytes. Thirteen host tests
cover the policy and the reader, including a truncated-file sweep, because a
verifier that panics on malformed input stops the build for the wrong
reason.
