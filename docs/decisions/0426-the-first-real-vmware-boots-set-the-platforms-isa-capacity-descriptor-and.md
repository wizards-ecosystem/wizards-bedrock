# D-426 The first real VMware boots set the platform's ISA, capacity, descriptor and transcript facts

Status: Accepted, 2026-10-20. Implemented with qualification OPEN; amends D-392 and D-421; advances PW-003, PW-004, PW-596, PW-597, PW-598 and PW-599.

**Decision.** The VMware policy adds exactly one quiesced requester. When the
VMware CPU identity, BDF `00:07.0`, identity `0x7110_8086`, ISA-bridge class
`06:01:00` with revision ignored, command before exactly `0x0007` and readback
exactly `0x0007` all agree, the pure fence asks the platform to master-clear
both 8237 controllers, mask every channel, and return both status registers.
Any request bit (`0xf0f0`) refuses the boundary; terminal-count bits clear on
read and prove nothing. The mask registers are write-only: on VMware they read
`0xff` before and after every write, so the proof is the defined master-clear
state plus the explicit all-mask write and a quiet status, not a mask readback.
`wost` claims both controller port blocks (`0x00..0x10` and `0xC0..0xE0`) as
one permanent claim before it takes the configuration guard, so no later port
owner can unmask a channel. The fence mints a private non-copyable
`QuiescedIsaRequester`. The later topology snapshot consumes it, compares the
exact BDF, identity, class and command, and keeps the function in both the
present and the requester inventory. A unit-wide VT-d fence then holds that one
function by its exclusive claim, an unchanged identity and command, and a fresh
remask with no pending request, instead of by a BME clear it cannot perform.
The kernel records `iommu: entry DMA fence quiesced VMware PIIX4 00:07.0 at its
masked 8237 controllers; it remains a requester`, and both VMware selectors
require that record exactly once. QEMU's strict policy never quiesces.

Function authority capacity becomes one shared constant,
`pci_config::FUNCTION_AUTHORITY_LIMIT = 64`, used by both the kernel and
`wost`. A host test replays the captured 43-function VMware segment through
the real fence and requires its 42 requesters to fit.

Every code and data descriptor is written with the accessed bit already set,
as Linux does, so the processor never needs to write the descriptor table.

Both VMware record lists follow the kernel's emitted order. The shipped list is
DMA, ISA quiesce, storage, FAT32, tmpfs, both devfs records, isolation, the
user slot and keyboard liveness. The self-test list places isolation before the
APIC. Their fixtures reproduce the order the 2026-10-20 boots emitted, and both
real transcripts passed the corrected verifier.

**Consequences.** The shipped image's liveness records prove only that the
kernel reached its user boundary: before the descriptor repair, the shipped
diagnostic printed the same records and then halted inside the exit
trampoline. Ring-3 execution on VMware is proved by the self-test image's ring
three checks; a post-entry shipped record remains open under PW-004.

Permanent mutations 2,272 through 2,297 cover the new checks. Mutations 324,
631, 632, 1,007, 1,701, 1,804 and 1,848 keep their identities while targeting
the edited source. On 2026-10-20 each new named witness passed on the pristine
source and failed under its mutation: mutations 2,272 through 2,282, which
widen the PIIX4 BDF, identity, class or either command word, skip the 8237
quiesce, ignore a pending request, accept a changed later function, drop the
function from requester inventory, or answer for another function or an
unsealed snapshot, red
`only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester`.
Mutation 2,283, which restores the old capacity, reds
`the_captured_vmware_segment_fences_and_fits_the_function_authority_limit`.
Mutations 2,284 through 2,288, which drop the sibling-fence branch, its
identity, command or 8237 recheck, or report a quiet pair without masking it,
red `memory_initialization_requires_the_exhaustive_entry_dma_boundary`.
Mutations 2,289 through 2,292, which leave one descriptor's accessed bit for
the processor to write, red `every_code_and_data_descriptor_is_already_accessed`.
Mutations 2,293 through 2,295, which drop the ISA record from a selector or let
the kernel record drift, red
`the_vmware_isa_dma_marker_is_required_by_both_selectors`, and mutations 2,296
and 2,297, which restore either fixture-only order, red
`the_vmware_dma_handoff_marker_is_required_by_both_selectors`. These are
implementation checks. Only fresh source-bound VMware self-test and shipped
qualifications and the complete current-registry sweep qualify them.

**Rationale.** The first source-bound VMware self-test boot of `65f0f89`
failed closed with `00:07.0 identity 0x71108086 class 0x06010008 command read
back as 0x0007, expected 0x0003`: VMware's PIIX4 ISA bridge, like D-392's
440BX, hard-wires command bit 2, and of 43 present functions only `00:00.0` and
`00:07.0` kept it. Unlike the 440BX, PIIX4 function 0 is a DMA requester whose
two 8237 controllers master ISA DMA, so extending D-392's non-requester
exception would remove a real requester from VT-d inventory and coverage, and
refusing VMware fails for the same reason as in D-392. The WZDH quiescence
claim alone is not relied on because the platform control exists and costs a
few port writes. VMware's single remapping unit governs 42 live requesters, 32
of them virtual PCIe root ports, against a limit of 12, and the bound is a
property of the qualified platform, not the selector. The first `iretq` into
ring 3 then faulted (vector 14, error code 3, at `wost_kpti_exit_iret` on the
stripped root) because loading CS and SS sets the accessed bit in a GDT the
stripped root maps read-only, which QEMU's TCG does not enforce; making the GDT
writable there would widen the user-visible enclave to protect a bit the
kernel can set itself. The verifier's orders had been taken from duplicated
fixtures, like D-421's drifted field, and the kernel has one order, which the
verifier states.
