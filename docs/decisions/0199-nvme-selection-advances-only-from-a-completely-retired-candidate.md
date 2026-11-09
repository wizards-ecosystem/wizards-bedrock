# D-199 NVMe selection advances only from a completely retired candidate

Status: Accepted, 2026-08-27. Amends D-88, D-138, D-168 and D-188; closes PW-208.

**Context.** Enumeration counted every NVMe function but remembered only the
first. A malformed, unsupported or non-responsive controller therefore hid a
later usable controller in deterministic BDF order. Simply putting the
addresses in a loop was not sound: a PCI claim and its dynamic I/O windows were
permanent, the enabled command word was not owned for later restoration, and a
device mapping could not be removed. An enable-time NVMe failure could also
return queue frames before a cleanup-owning `Controller` existed.

**Decision.** Permanent claims remain the ownership model for a selected
driver. Candidate discovery gets a narrower **lease** which occupies the same
function and I/O-window registries but may be consumed in one of two ways:
commit it after the controller passes its smoke read, or retire it after exact
rollback. The pure PCI claim registry and port-space bitmap gain all-or-none
release operations; `wost` exposes release only through the unique dynamic
port-span values. Successful command enablement returns the exact original
command, which a retiring lease restores and reads back before either registry
is released. A failed restore leaves the lease quarantined and stops selection.

A device mapping may now be claimed as a releasable register capability. That
capability is the only value which can yield the non-accessing mapping token
again; consuming it ends all safe MMIO access before `AddressSpace` removes the
exact physical/virtual extent, invalidates its pages and makes the last device
window reusable. Ordinary firmware and selected-driver mappings keep the old
permanent `MmioRegion` door.

NVMe constructs its cleanup owner before enabling a controller with host queue
addresses. Every later error explicitly stops the controller and verifies
`RDY = 0`; only a quiesced controller yields its mapping for removal. An
unverified stop retains every reachable DMA run, keeps the mapping and PCI
lease quarantined, and ends fallback. The boot walks candidates in the bus
enumerator's stable bus/device/function order, commits the first controller
which completes bring-up and a one-block smoke read, and tries the next only
after all five retirement boundaries report success.

**Rationale.** A retained DMA run is safe but is not complete teardown, and a
released function claim beside a still-live mapping is a stale capability
which makes reacquisition fail for a reason the registry denies. Conversely,
making every device claim releasable would reopen D-138's stale-handle window
for drivers which are meant to own their hardware for the boot. The lease is
therefore a pre-commit state, not a general unclaim API.

**Alternatives.** Keeping failed candidates permanently claimed as
"quarantined" was rejected because a bounded eight-slot registry would make
the ninth otherwise usable controller unreachable and because the row requires
proof of no leaked claim. Retrying after an unverified reset was rejected even
with retained DMA memory: bus mastering and a live register mapping would
remain outside the selected driver's ownership. Pre-probing controllers before
claim was rejected because reading or writing their BARs is precisely the
access the claim authorizes. A fake-only selector was rejected because it
would not exercise QEMU's real configuration, MMIO, reset or DMA teardown.

**Consequences.** The ordinary QEMU machine carries an earlier namespace-less
NVMe controller and the existing corpus-backed controller. The shipped and
self-test boots must show the first retiring and the second mounting; the
self-test additionally compares claim, device-mapping and free-frame baselines
across the failed attempt. Host tests exhaust release refusal, exact command
restoration and lease reuse. Manual breaks proved that the witnesses reject a
port bit left claimed, a function slot left claimed, a skipped command restore,
a retained mapping record, a missing device-window rollback, a missing dynamic
port return, selection stopped after the first candidate and enablement before
the cleanup owner exists. The releasable token, rather than every fixed
physical-ownership record, carries its virtual extent; the debug boot caught
the redundant alternative overflowing a guarded kernel stack. The complete
gate must pass 203 release and debug boot checks plus shipped and degraded
boots. The mutation register contains 130 entries, and the `wost` unsafe
measure remains within budget at 372 of 380.
