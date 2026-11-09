# D-426 The first real VMware boots set the platform's ISA, capacity, descriptor and transcript facts

Status: Accepted (2026-10-20), implemented with qualification OPEN; amends D-392 and D-421; advances PW-003, PW-004, PW-596, PW-597, PW-598 and PW-599

**Context.** The first source-bound VMware self-test boot of candidate
`65f0f89` failed closed before memory initialization:
`00:07.0 identity 0x71108086 class 0x06010008 command read back as 0x0007,
expected 0x0003`. VMware's emulated Intel PIIX4 ISA bridge, like D-392's 440BX
host bridge, hard-wires command bit 2. Because the entry fence stops at the
first refusal, an uncommitted diagnostic kernel then recorded every function
on that machine without refusing. Of 43 present functions, only `00:00.0` and
`00:07.0` kept bit 2; the other 40 cleared it. Further diagnostic boots on
2026-10-20 then found three more VMware-only defects hidden behind the first:

- `FUNCTION_AUTHORITY_LIMIT` and `wost`'s `FUNCTION_CLAIMS` were both 12, but
  VMware's single remapping unit governs 42 live requesters, 32 of them
  virtual PCIe root ports. Storage therefore reported `the remapping unit has
  more than 12 live requesters`, no device was isolated, interrupts stayed off
  and about forty self-tests failed.
- With capacity raised, the first `iretq` into ring 3 stopped silently. A
  `vmss2core` dump of the suspended guest placed vCPU 0 at `wost_kpti_stop`
  with interrupts off; the saved frame was vector 14, error code 3 (a
  supervisor write to a present page) at `wost_kpti_exit_iret` on the
  stripped root. The live GDT held user data `0x00f2` and user code `0x00fa`
  with the accessed bit clear. Loading CS and SS sets that bit, which is a
  write to a table the stripped root maps read-only. QEMU's TCG does not
  enforce that write, so no earlier boot could observe it.
- The VMware transcript verifier rejected genuine boots: its shipped list
  required user isolation before `nvme: up`, and its self-test list required
  the local APIC before user isolation. The kernel emits both in the opposite
  order. Like D-421's drifted field, the order had been taken from duplicated
  fixtures rather than from any kernel transcript.

Unlike the 440BX, PIIX4 function 0 is a DMA requester: its two 8237
controllers master ISA DMA. Treating it as D-392's "present but not a
requester" would remove a real requester from VT-d inventory and coverage.

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

Both VMware marker lists follow the kernel's emitted order. The shipped list is
DMA, ISA quiesce, storage, FAT32, tmpfs, both devfs records, isolation, the
user slot and keyboard liveness. The self-test list places isolation before the
APIC. Their fixtures now reproduce the order the 2026-10-20 boots emitted. With
all four repairs, the uncommitted diagnostic kernel passed 290 of 290 self-test
checks on VMware, and both real transcripts passed the corrected verifier.

**Consequences.** The shipped image's liveness markers prove only that the
kernel reached its user boundary. Before the descriptor repair, the shipped
diagnostic printed the same markers and then halted inside the exit
trampoline. Ring-3 execution on VMware is proved by the self-test image's ring
three checks; a post-entry shipped marker remains open under PW-004.

Permanent mutations 2,272 through 2,297 cover the new checks. Mutations 324,
631, 632, 1,007, 1,701, 1,804 and 1,848 keep their identities while targeting
the edited source. On 2026-10-20 each new named witness passed on the pristine
source and failed under its mutation. Mutations 2,272 through 2,282, which
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
Mutations 2,293 through 2,295, which drop the ISA marker from a selector or let
the kernel record drift, red
`the_vmware_isa_dma_marker_is_required_by_both_selectors`, and mutations 2,296
and 2,297, which restore either fixture-only order, red
`the_vmware_dma_handoff_marker_is_required_by_both_selectors`. These are
implementation checks. Only fresh source-bound VMware self-test and shipped
qualifications and the complete current-registry sweep qualify them.

**Alternatives.** Extending D-392's non-requester exception to `00:07.0` was
rejected because it would remove a real DMA requester from VT-d inventory and
coverage. Refusing VMware entirely was rejected for the same reason as in
D-392. A mask readback was rejected as unobservable on the target. Relying on
the WZDH contract's quiescence claim alone was rejected because the platform
control exists and costs a few port writes. Raising capacity only for VMware
was rejected because the bound is a property of the qualified platform, not
the selector. Making the GDT writable in the stripped root was rejected because
it widens the user-visible enclave to protect a bit the kernel can set itself.
Loosening the verifier to accept either order was rejected; the kernel has one
order, and the verifier states it.
