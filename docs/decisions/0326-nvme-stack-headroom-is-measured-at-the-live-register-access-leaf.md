# D-326 NVMe stack headroom is measured at the live register-access leaf

Status: Accepted (2026-09-27), closes PW-541

**Context.** D-287 split controller preparation after two nested debug frames
overflowed the guarded 32 KiB boot stack. Its post-split 22,980-byte linked-path
report was a local inspection rather than a retained check. Per-function frame
ceilings cannot prove the sum along a call chain, while merely surviving a boot
does not prove useful distance from the guard.

**Decision.** A self-test-only meter surrounds each complete `nvme::prepare`
call. The existing audited platform primitive reads the live stack pointer at
every NVMe register-access leaf; pure `stack-layout` arithmetic rejects a
pointer outside the boot slot and derives used bytes and remaining headroom
from its checked bounds. The driver retains the largest valid observation
across both QEMU candidates. Every passing self-test emits one versioned,
strictly parsed record and requires the 32 KiB capacity to preserve at least
two pages, 8 KiB, of headroom. The meter and record do not exist in a shipped
image.

**Evidence.** The restored debug image passed 262/262 with 22,480 bytes used
and 10,288 bytes free; adding a live 3 KiB leaf frame measured 24,960/7,808 and
made only `nvme preparation preserves two pages of boot-stack headroom` red at
261/262. The restored release image passed 262/262 with 9,496/23,272; its
calibrated live-frame defect measured 25,768/7,000 and made the same sole check
red. Restoration passed both complete integrated boot modes. Mutation 1,389
retains the mode-calibrated defect, and the host parser independently rejects
missing, duplicate, malformed, non-conserving, wrong-capacity and under-margin
records.

**Alternatives.** Enlarging the stack was rejected because it hides rather
than bounds the regression. Summing individual debug-frame entries was
rejected because it cannot recover linked nesting reliably through generated
and indirect calls. Painting the stack and scanning a watermark was rejected
because reserved but unwritten frame space is invisible. A debugger-only
measurement was rejected as non-repeatable gate evidence; it remains useful
calibration for the retained machine check.

**Amended by D-341.** The mutation cited above as 1,389 was array position 1
when this record was written. Its permanent id is 17.

**Amended by D-361.** On 2026-10-03 the meter came to paint the dead boot
stack at the start of each storage candidate and to scan it when the
candidate settles. It refuses a scan shallower than the deepest leaf, observes
the leaf across the whole candidate, and reports the painted mark in a v2
record. The check is now named
`nvme candidates preserve two pages of boot-stack headroom`.
