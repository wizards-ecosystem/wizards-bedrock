# D-46 KASLR waits, and this is what waiting costs

Status: Accepted, 2026-08-22.

**Context.** D-12 put KASLR in the security baseline "on from the start, not
retrofitted". It has been owed since Phase 0, blocked throughout on one
thing: `linker.ld` hardcodes `0xffffffff80000000`, so the image is not
relocatable and `kaslr: yes` in `boot/limine.conf` would have nothing to
randomise. `007-memory.md` requires this phase to take the relocation work
or record the deferral.

**Options.** (a) Make the image relocatable now: position-independent code,
a relocation pass at boot, and every hardcoded address in the linker script
and mapping code made base-relative; (b) defer, and name the cost.

**Decision.** (b), deferred to Phase 12 with the other hardware-security
items, with the cost stated.

**Rationale.** The relocation work is not large, but it is the wrong shape
for this phase. Slice 2 has just made the image's layout matter in a new
way: six linker symbols now bound the protection classes, and the mapping
code turns each into a physical address through the bootloader's reported
base. Making that base-relative in the phase that first depends on it would
change the foundation and what stands on it together. The failure mode is a
triple fault with nothing on the wire.

**What the deferral costs.** Every phase that adds a mapping adds an
assumption about where the kernel lives. Slice 2 added three. Phase 4's
per-CPU stacks will add more, and Phase 5's userspace will add the first
mappings whose *absence* of randomisation is externally observable. The work
is strictly cheaper now than after those land, and the case for deferring
weakens with each one: a debt with interest, not at par.

Two Phase 12 items assume the kernel controls its own layout and would be
taken together: CET shadow stacks, a per-stack allocation decision, and
LASS, whose premise is a hard user/kernel address split
(`008-research-notes-2.md` §6).

**Consequences.** D-12's KASLR line remains outstanding and is the only
baseline item deferred as far as Phase 12. `001-boot.md`'s known limitation
stands. W^X, NX, SMEP, SMAP, UMIP, zero-on-allocate, and ordinary
thread-stack guard pages (D-52) are done. The boot and double-fault IST
stacks remain a separate Phase-5 guard-page debt, so KASLR is not literally
the only unfinished line of D-12.
