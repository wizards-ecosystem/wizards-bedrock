# D-40 The linked image is verified, not assumed

Status: Accepted, 2026-08-22.

**Context.** The same audit read the built ELF back and found `.got` placed
*after* `.bss`. It linked, it booted, and it was wrong: `linker.ld` named
neither `.got` nor `.got.plt`, so the linker placed them by orphan heuristic,
and a loadable section following a `NOBITS` one forces the whole of `.bss`
into the file. 29048 bytes of zeros were being written into every image, and
`.bss` was no longer being zeroed by the bootloader but loaded from disk.

A linker script fails loudly when it is malformed and quietly when it is
merely incomplete. Nothing in the gate could tell the difference.

**Options.** (a) Name the orphans and move on; (b) name them and check the
result; (c) adopt a linker-script linter or an external `readelf` in the gate.

**Decision.** (b). `linker.ld` now names `.got`, `.got.plt` and
`.data.rel.ro`, puts `.bss` last and `(NOLOAD)`, discards `.comment`, and
gives the Limine request block its own `KEEP()`-ed `.requests` section.
`xtask/src/elf.rs` reads the linked ELF after every build and requires: the
entry point in the higher half; every allocated section inside a `PT_LOAD`;
no segment both writable and executable; no `NOBITS` section costing file
space; and `.text` and `.requests` present and loaded. External tooling was
rejected on D-16's grounds — the build stays self-contained, and this is
~200 lines of header reading.

**Rationale.** W^X at the segment level was already true and already load-
bearing (Limine maps each `PT_LOAD` by its flags), but nothing asserted it,
and Phase 3 is about to build page tables that must reproduce it. An orphan
section is the failure mode that survives a link, boots, and shows up as
something else entirely months later.

**Consequences.** The check found a real gap on its first run: the request
block had no section of its own — it was folded into `.data` — so nothing
could confirm the block `wost::entry!` emits had linked at all. Giving it a
name also let it be `KEEP()`-ed, which is what D-21's note about `#[used]`
statics being dropped by a linker actually asks for. Broken once by placing a
loadable section after `.bss`; the build went red naming `.bss` and the 29048
bytes. Thirteen host tests cover the policy and the reader, including a
truncated-file sweep, because a verifier that panics on a malformed input is
a verifier that stops the build for the wrong reason.
