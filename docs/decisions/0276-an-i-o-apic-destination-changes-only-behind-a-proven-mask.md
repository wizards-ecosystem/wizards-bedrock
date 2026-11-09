# D-276 An I/O APIC destination changes only behind a proven mask

Status: Accepted (2026-09-04), amends D-205 and D-251; closes PW-130

**Context.** Controller takeover wrote the low redirection word's mask and then
cleared its destination before reading either word back. An ignored mask write
therefore let the kernel alter a still-live firmware route, while the returned
probe failure exposed no evidence that its register state had been changed.

**Decision.** Masking is a two-phase transaction. It writes the exact inert low
word, reads the complete entry back, and refuses without touching the
destination unless that low word is exact. Only then may it clear the high word
and verify the complete inert entry. A takeover refusal returns the original
register authority and cannot mint masked-controller authority; routing keeps
the same ordered mask, destination and final-entry proofs.

**Alternatives and evidence.** Treating an accepted MMIO write as hardware
state was rejected because indexed register transport cannot supply readback.
Clearing the destination first was rejected because it changes a live route.
The indexed fake records any high-word write while the low word is live and
inspects the registers returned by an ignored takeover mask. Reintroducing the
old ordering made only `a refused keyboard route restores configuration,
masking and mappings` fail, 251/252; restoring the transaction passed release
QEMU 252/252. Mutation 1219 retains that exact defect and witness.
