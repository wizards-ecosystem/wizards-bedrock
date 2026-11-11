# D-46 KASLR waits, and this is what waiting costs

Status: Accepted, 2026-08-22.

**Decision.** KASLR is deferred to Phase 12 with the other hardware-security
items, with the cost stated.

**Rationale.** `linker.ld` hardcodes `0xffffffff80000000`, and making the
image relocatable means position-independent code, a boot relocation pass
and base-relative mapping code. Doing that in the phase whose mapping code
first depends on the image layout would change the foundation and what
stands on it together, with a triple fault as the failure mode.

**What the deferral costs.** Every phase that adds a mapping adds an
assumption about where the kernel lives, and Phase 5's userspace adds the
first mappings whose *absence* of randomisation is externally observable.
The work is strictly cheaper now than later: a debt with interest, not at
par. Two Phase 12 items assume the kernel controls its own layout and would
be taken together: CET shadow stacks, a per-stack allocation decision, and
LASS, whose premise is a hard user/kernel address split
(`008-research-notes-2.md` §6).

**Consequences.** D-12's KASLR line remains outstanding and is the only
baseline item deferred as far as Phase 12. `001-boot.md`'s known limitation
stands. W^X, NX, SMEP, SMAP, UMIP, zero-on-allocate, and ordinary
thread-stack guard pages (D-52) are done. The boot and double-fault IST
stacks remain a separate Phase-5 guard-page debt, so KASLR is not literally
the only unfinished line of D-12.
