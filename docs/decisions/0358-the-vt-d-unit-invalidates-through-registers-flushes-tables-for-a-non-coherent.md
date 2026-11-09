# D-358 The VT-d unit invalidates through registers, flushes tables for a non-coherent walker, and lives with the storage candidate

Status: Accepted (2026-10-01), amends D-357; closes PW-567, PW-568, PW-569 and PW-570

**Context.** D-357 decided that the storage controller runs behind an
identity-scoped VT-d domain and that storage fails closed without one, and left
the programming to the implementation. Building it forced choices the design
section did not settle: which invalidation interface to drive, what to do with a
unit whose page walks bypass the processor caches, how to take the unit over
from firmware, where the unit lives relative to the storage candidate lifecycle,
how the domain proof binds to the function, and what a blocked access looks like
on the development machine so the fence check asks the right question. QEMU's
unit reports `ECAP.C = 0`, so the first draft, which refused such a unit as
unservable, would have left the development machine permanently unisolated. The
first boot also showed that a candidate which allocates on the heap cannot
restore its resource baseline (D-199), because a slab the heap grows into never
returns its frame.

**Decision.**

1. *Register-based invalidation.* The unit drives the Context Command and IOTLB
   registers and never enables queued invalidation. The takeover word clears
   `QIE` because the specification reserves both registers while it is set, and
   the quiesce poll waits for `TES` and `QIES` to clear together. The
   queued-invalidation encoder drafted beside the register encoder was removed
   under 005's no-speculative-API rule: no shipped caller, so no contract anyone
   could check.
2. *Non-coherent walks are served by flushing.* `IommuTable` records how its
   unit reads memory. For a unit without `ECAP.C`, every zeroed table and every
   written entry has its cache line written back with `clflush`, ordered by a
   fence, before the unit can be told about it. The instruction is one new
   two-line `unsafe` in `wost` (`cpu::flush_cache_line` and its one caller in
   `mm`), and the measured unsafe total moves to 379 lines.
3. *Takeover inherits only interrupt state.* The command shadow starts from
   `inherited_command(GSTS)`: interrupt remapping and compatibility-format
   interrupts keep whatever state firmware left, translation and queued
   invalidation are cleared, and the kernel waits for both to report off before
   it maps the regions firmware reserves for the function, sets its own root
   table and reads the pointer back, invalidates the context cache and the
   IOTLB, and enables translation. Any fault the unit recorded before the first
   segment was mapped is drained and reported.
4. *The unit lives in one kernel-owned slot.* `iommu::establish` runs after
   `nvme::prepare` and before bus-master admission, while the controller is
   verified disabled and its function cannot bus-master, and installs the unit
   in a `ThreadLock` slot beside the kept controller's own. Every DMA seam
   (admission, queue creation and every command runner) locks the slot and
   refuses when it is empty, so no signature carries the unit and no path mints
   a proof around it. A retired candidate releases the slot after its PCI lease
   is returned, so translation switches off only once the controller's stop is
   proven and firmware's command word restored; that word may leave bus
   mastering enabled on the stopped controller, so the interval after release
   is bounded by the proven stop alone (Left owed). The candidate baseline
   sees every table frame and the register window come back. A quarantined controller, or a unit whose
   translation will not switch off, leaves the unit installed with translation
   on while selection halts. The firmware table is read once at boot and the
   unit holds its second-level tables in one owned arena, so no heap allocation
   happens inside a candidate.
5. *The proof binds the function twice.* `Vtd::domain` mints a `DmaDomain` only
   when translation is on and the root and context entries name the kernel's
   own context table and second-level tree with the identifier and width it
   programmed. `pci-config` refuses a domain whose function differs from the
   mapped BAR's before any configuration access, and a forged proof fails to
   compile.
6. *Queues stay mapped; buffers come and go.* Queue frames enter the domain
   before the controller learns their addresses and leave it only with the
   domain, because queue frames free only after a proven stop. A transfer
   buffer is mapped before the doorbell and unmapped, with an invalidation, once
   the completion or a proven stop makes it host-owned; a buffer the stop could
   not prove quiescent stays mapped as it stays retained. A segment that reaches
   above the unit's address width is refused rather than aliased into a wrong
   table index.
7. *The fence witness asks what the hardware shows.* On QEMU a blocked device
   write completes the command into a bounce buffer and drops the bytes, so the
   check requires a clean completion or a command error, unchanged bytes and a
   recorded write fault from the storage function at the buffer's page. It
   first reads into the same buffer through the domain, so a skipped
   invalidation at that unmap would be visible through the unit's stale cache,
   and it runs before the translation toggle, which flushes the unit's caches.
8. *The unisolated boot joins the degraded gate.* `cargo xtask degraded-check`
   boots the shipped image a second time on a machine without the unit and
   requires `storage: no DMA isolation`, the first candidate's clean retirement
   and an empty selection, so the fail-closed refusal is exercised by the
   ordinary gate and witnessed by the same boot-marker mutation class.
9. *Scopes resolve through the live topology.* A device scope names its target
   by a start bus and a path of (device, function) hops. `Dmar::unit_for` walks
   that path through a bridge lookup the kernel passes in, which reads a
   function's identity, class, header type and bus-number register through the
   configuration door and answers only for a function in segment 0 that both of
   enumeration's bridge tests accept. Each hop before the last must be such a
   bridge, and the next hop lies on its secondary bus. An endpoint scope governs
   the function its path reaches, a sub-hierarchy scope governs its bridge and
   every function from the bridge's secondary bus through its subordinate, and
   I/O APIC, HPET, namespace-device and unknown scopes govern no PCI function. A
   path names nothing when a hop before the last is not a bridge, a bridge's
   secondary bus is at or below its own bus or above its subordinate, or the
   path is longer than eight hops, and every lookup stays in the scope's own
   segment. The most specific scope wins: an endpoint scope outranks a bridge
   scope enclosing the function, an inner bridge outranks the bridges above it,
   an `INCLUDE_PCI_ALL` unit governs only what no scope in its segment names,
   and between equals the earlier unit wins. Reserved regions apply through the
   same resolution. The kernel keeps the validated table as a static slice and
   copies it out of its leaf lock before the walk, so the bridge reads run under
   no guard (D-121), and the boot logs which scope tied the storage function to
   its unit. The enumeration-id accessor was removed as speculative API, because
   only interrupt remapping (PW-710) would read an enumeration id, and the
   single-hop target shortcut went with the walk that replaced it. The
   subordinate-bus accessor an earlier sweep removed for want of a caller
   returns with this one, so its regrowth guard and mutation 1,005 are retired.

**Amended by D-371.** The eight-hop refusal and the piecemeal live bridge
reads above are retired. Every structurally valid path is walked to the length
bounded by its one-byte scope entry, and all range, presence and ancestry
answers come from one sealed topology. Mutation 1,563 is retired; 1,691 keeps
the valid-long-path wrong-unit regression.

**Evidence.** On 2026-10-03 the release and debug self-tests each passed 270/270
on QEMU's unit (`cap 0x80d2008c222f06c6`, `ecap 0xf02`: four-level walks,
caching mode, no page-walk coherency), the shipped boot settled, and the
degraded gate passed both its boots, the second refusing each candidate with
`storage: no DMA isolation`, retiring the first cleanly and ending with
`nvme: no usable controller`. Each retained mutation went red once. In release
boots that each failed one check at 269/270, mutation 1,545 failed
`the dma domain reaches exactly the live queue frames`, 1,546 failed
`a domain proof is refused while translation is off`, and 1,544, which skips the
unmap's invalidation, failed
`a device write into an unmapped frame faults and changes nothing` with no
translation failure in QEMU's log. Mutations 1,547 and 1,548 leave no
controller, so `translation is active for the storage function` failed under the
mutation runner's lenient boot, and 1,555, which programs the root pointer with
the context table's address, failed `the unit walks the kernel's root table`
the same way; 1,549 made the degraded check report the missing
refusal; and 1,540 through 1,543 and 1,550 through 1,554 each failed its named
host test. Two defects the gate found on the way shaped the design: a candidate
failed its resource baseline while the unit allocated on the heap, and the debug
boot double-faulted when the candidate path, moving the unit by value, overran
the guarded boot stack (D-287). The design above answers both: the DMAR is read
once at boot, the unit keeps its second-level tables in one owned arena, and it
lives in the kernel-owned slot of item 4, which every DMA seam locks, so no
signature in the candidate path carries it. Item 9 was verified the same day:
the release and debug self-tests again reported
`verdict v1 PASS passed=270 failed=0 expected=270`, each logging
`iommu: 00:05.0 is governed by the unit at 0xfed90000 through an endpoint scope`
before translation went on, and the degraded gate passed both its boots. Host
mutations 1,556 through 1,568 each failed its named `acpi-tables` test, 1,569
failed `pci-config`'s `tests::a_bridge_subordinate_bus_is_read_from_its_own_byte`,
and the debug self-test build's `iommu::establish` frame stayed at 3,304 bytes.

**Alternatives.** Implementing queued invalidation was rejected for a first
unit: the register interface is sufficient for one domain and both targets
support it, and the queue would have been boot-verified surface with no second
caller. Threading the unit by value through the candidate's stages was built
first and rejected by the debug boot: every by-value owner and every refusal
temporary took its own slot in the debug frames (D-287), and the candidate path
overran the guarded boot stack. A leaf spin lock for the slot was rejected
because the seams run commands under its guard, which D-121 forbids and which
would make it the fourth leaf-lock static PW-707 waits for; the thread lock
admits calls beneath it and is the discipline the kept controller already
uses. Holding the unit inside the controller was rejected because
`Controller::retire` is pinned and would drop the register window without
unmapping it, so a retired candidate could never restore its mapping count.
Unmapping queues at retirement was rejected because a stopped device cannot
reach them and the change would break four pinned DMA contracts. Refusing a
non-coherent unit was rejected once QEMU turned out to be one. Boxing the unit to
keep its moves small was rejected because the box is a heap allocation inside
the candidate window, and nothing on the storage path boxes.

**Left owed.** The domain fences the function only while translation is on. A
device that ignores its bus-master enable bit is unconstrained before
`iommu::establish`, on a machine whose unit cannot be taken over (storage is
refused but the device is not fenced), after `release_candidate` switches
translation off, and under any remapping unit the kernel never takes over; a
device that delivers interrupts would also need PW-710. The regions firmware
reserves for the function are mapped as the DMAR declares them, with no check
that they lie outside the kernel's usable memory, and `DmaDomain` is a `Copy`
proof, so a copy could outlive the release that invalidated it although no
caller keeps one. All of these are recorded as owed here and as limits in
`017`. D-360 closes the reserved-region and copied-proof items: a reserved
region that reaches RAM or firmware storage now refuses the candidate, and the
proof is spent by the one admission it permits.

**Amended by D-369.** Rejected storage leases no longer restore inherited BME.
They verify it clear, remove and invalidate every installed queue segment before
any queue frame drops, restore only the other captured command bits, and switch
translation off last. The earlier rejection of retirement-time queue unmapping
and reliance on controller-supplied stop state no longer describe the hostile
device boundary.
