# D-390 DMA safety is a verified boot-handoff invariant

Status: Accepted (2026-10-11), amends D-42, D-45, D-357 through D-358 and D-389

**Context.** D-389 closes every unrestricted-DMA interval after the kernel
owns its VT-d unit, but that owner was still constructed after the allocator,
kernel page tables, guarded stacks and heap. A requester left bus-mastering by
firmware could therefore corrupt privileged memory before the first kernel
translation boundary. Moving the existing takeover earlier is not sufficient:
its tables allocate from the very memory that needs protection, VT-d registers
cannot be accessed through Limine's write-back HHDM as ordinary RAM, and no
software action at `kmain` can retroactively establish that the already-loaded
kernel image and handoff were not modified. An exhaustive BDF scan alone also
cannot prove that a requester which answered absent will stay absent, or that a
request already posted before BME clear has drained.

**Decision.** DMA safety becomes a required firmware-to-kernel contract. One
unique checksummed revision-1 `WZDH` ACPI table attests three inseparable
properties for PCI segment zero: legacy CF8/CFC reaches every DMA-capable
requester, requester presence and identity stay static for the boot, and the
platform has kept every requester quiescent with no in-flight DMA from before
the executable and handoff were loaded until the kernel completes its entry
fence. The fixed table accepts exactly those three defined flags; a missing,
duplicate, malformed, partial or unknown contract has no useful prefix.
`WZDH` is platform authority, not a claim synthesized from a PCI snapshot or a
DMAR, and is independent of D-389's `WZDM` whole-segment remapping-unit
coverage. RAM cannot authenticate RAM against a requester which can already
write it, so the table has no authority on its own. The release kernel first
requires the exact architectural hypervisor base leaf of the fixed QEMU TCG or
VMware environment. That CPU-resident value is outside PCI DMA reach and binds
the contract to the two qualified virtual platforms; bare metal and every
other hypervisor refuse before interpreting the table. On those platforms the
hypervisor, its device models and firmware are one pinned trusted boot
boundary. A future physical platform needs a separate requester-inaccessible
hardware root rather than merely copying these ACPI bytes.

The first kernel operation validates the CPU-resident platform identity, then
validates the structured authority through an allocation-free ACPI walk over
the copied boot memory map and HHDM. An unreadable root-table target refuses
the entire lookup because its signature and therefore uniqueness are unknown.
It then reserves CF8/CFC and
probes every bus, device and function number without trusting multifunction
hints. For every present function it reads the complete identity, clears BME
while preserving every other command bit, reads the command back exactly, and
re-reads the identity. Any inaccessible cycle, refused bit, identity change or
unavailable configuration mechanism stops the boot before allocator or paging
initialization. A private linear handoff token is the only argument accepted by
the kernel memory-initialization door, so later source motion cannot silently
put mutable privileged memory above the fence. A compiler fence after the last
configuration-space readback also prevents LTO from moving memory mutation
above port instructions whose inline assembly intentionally carries no memory
operand. The early clear is permanent;
ordinary PCI ownership later begins from BME-off state, and D-389's
presence-independent empty VT-d root remains the boundary before any kernel
driver may re-enable one selected requester. On a no-IOMMU boot, the static
handoff contract and permanent BME fence keep PCI DMA closed for the lifetime
of that boot.

The QEMU and VMware generators each create the same source-defined `WZDH`
bytes as a private machine input and bind them into their qualification
artifacts. QEMU passes it with `-acpitable`; VMware names its copied table with
`acpi.addtable.filename`. The QEMU interactive runner accepts only logging and
display diagnostics, so device, firmware, migration, storage and monitor
options cannot reuse that attestation for another machine. Neither environment
may replace or omit the table through interactive or runtime configuration.
This release does not accept a production bare-metal firmware assertion:
supporting one requires a new hardware-rooted handoff decision.

**Proof required.** Pure ACPI tests must reject each missing or unknown flag,
revision, length, checksum, reserved byte and signature. Pure PCI tests must
enumerate the entire requester-ID space, clear inherited BME exactly, preserve
unrelated command bits, detect refused writes and identity replacement, and
fail on any unavailable cycle. Source contracts must bind the allocation-free
unique-table lookup and exhaustive fence before `mem::init`, require the private
token at that door, and retain D-389's later default-deny takeover. Machine
checks must observe the entry fence before the first memory record and continue
through the ordinary self-test, shipped and no-IOMMU paths. Every generated
QEMU and VMware machine and every corresponding qualification record must bind
the exact table bytes. Each new check is broken once and retained in the
mutation register before the complete gate passes.

**Alternatives.** Treating a checksummed ACPI table as self-authenticating was
rejected after isolated post-patch review: the requester the table discusses
could otherwise rewrite the table and the code consuming it. Clearing BME after memory initialization leaves the reported
window intact. Clearing it at entry without a trusted quiescence and static-
topology contract cannot exclude an already posted request or a requester that
appears after its BDF was read. Requiring firmware to leave an active IOMMU
domain would also be sound, but neither qualified firmware currently exports a
verifiable root-ownership and live-replacement protocol; accepting an opaque
active root would make firmware page tables an unaudited lifetime dependency.
A pre-kernel EFI DMA guard would have to authorize every bootloader I/O buffer
and transfer ownership continuously, which is a separate boot-chain product,
not a smaller form of this repair.
