# D-46 KASLR waits, and this is what waiting costs

Status: Accepted, 2026-08-22.

**Context.** D-12 put KASLR in the security baseline "on from the start, not
retrofitted". It has been owed since Phase 0 and blocked throughout on the same
thing: `linker.ld` hardcodes `0xffffffff80000000`, so the image is not
relocatable and `kaslr: yes` in `boot/limine.conf` would have nothing to
randomise. `007-memory.md` requires this phase to either take the relocation
work or record the deferral.

**Options.** (a) Make the image relocatable now — position-independent code,
a relocation pass at boot, and every hardcoded address in the linker script and
the mapping code reworked to be base-relative; (b) defer, and name the cost.

**Decision.** (b), deferred to Phase 12 alongside the other hardware-security
items, with the cost stated rather than left implicit.

**Rationale.** The relocation work is not large, but it is the wrong shape for
this phase. Slice 2 has just made the image's layout load-bearing in a new way
— six linker symbols now bound the protection classes, and the mapping code
turns each into a physical address through the bootloader's reported base.
Making all of that base-relative in the same phase that first depends on it
would mean changing the foundation and the thing standing on it together, and
the failure mode is a triple fault with nothing on the wire.

**What the deferral costs, so the next session need not rediscover it.** Every
phase that adds a mapping adds an assumption about where the kernel lives. Slice
2 added three. Phase 4's per-CPU stacks will add more, and Phase 5's userspace
will add the first mappings whose *absence* of randomisation is externally
observable. The work is strictly cheaper now than after those land, and the
argument for deferring gets weaker with each one — it is a debt with interest,
not a debt at par.

Two Phase 12 items assume the kernel controls its own layout and would be
taken in the same breath: CET shadow stacks, which are a per-stack allocation
decision, and LASS, whose whole premise is a hard user/kernel address split
(`008-research-notes-2.md` §6).

**Consequences.** D-12's KASLR line remains outstanding and is the only
baseline item deferred as far as Phase 12. `001-boot.md`'s known limitation
stands. W^X, NX, SMEP, SMAP, UMIP, zero-on-allocate, and ordinary thread-stack
guard pages (D-52) are done. The boot and double-fault IST stacks remain a
separate Phase-5 guard-page debt, so KASLR is not literally the only unfinished
line of D-12.
