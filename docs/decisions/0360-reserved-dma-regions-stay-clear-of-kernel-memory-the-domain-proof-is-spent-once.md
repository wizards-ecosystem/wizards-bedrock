# D-360 Reserved DMA regions stay clear of kernel memory, the domain proof is spent once, and four source checks close their evasions

Status: Accepted, 2026-10-03. Amends D-345, D-351 and D-358; closes PW-573 and PW-574.

**Decision.**

1. *A reserved region may not reach kernel memory.*
   `MemoryMap::allows_reserved_region` in `libs/memory-map` takes a region by
   its first and last byte, rounds it out to the whole pages a remapping unit
   maps, and admits it only when `allows_device_mapping` admits those pages.
   That is the exclusion device-range admission already applies, so the kinds
   the kernel owns stay named in one predicate. An empty or inverted region is
   refused. `wost::mm::reserved_region_is_eligible` asks it of the boot map
   under the map's leaf lock and returns a bool, so no copy of the map enters
   the candidate's frames. `iommu::establish` asks it of every region the DMAR
   names for the function before the unit's registers are mapped. A region
   that fails is logged as
   `iommu: reserved region [base, limit] reaches memory the kernel owns`, and
   the candidate takes the existing `storage: no DMA isolation` refusal.
   QEMU's DMAR names no reserved region, so the machine check
   `a reserved dma region over kernel memory refuses isolation` drives the
   same screen through a self-test hook with two pages of the live boot map:
   one wholly inside a device hole, which it must admit, and one of usable
   RAM, which it must refuse.
2. *The domain proof is spent once.* `DmaDomain` derives only `Debug`. Every
   caller already moved the proof into `admit_bus_master` right after minting
   it under the slot lock, so no caller changed and the admission text
   `qemu.rs` pins is byte-identical. Beside the forgery probe, one compile
   probe spends a proof twice and requires E0382, and another requires E0277
   for a `Clone` bound, since a cloned proof would outlive its release as a
   copied one does.
3. *The port contract reads imports.* Besides qualified paths, the contract
   visits every `use` item in production kernel code, wherever it is written,
   and refuses an import of a raw door, plain or renamed, a rename of the
   `port` module, `self as` included, and a glob import from it. Drivers
   import the module by its own name and call typed doors, which is the shape
   the contract admits.
4. *A table header has one spelling.* The manifest reader refuses a header
   unless it is `[` or `[[`, dot-separated bare keys and the matching closing
   brackets with nothing after them; a target's specifier is the one quoted
   key it accepts. The refusal covers every table, because telling whether
   another spelling names a dependency table would take the TOML parser D-345
   declined.
5. *A path package is a workspace member.* Each source-less package in the
   lock must claim a workspace member by name, and no member is claimed twice,
   so a path crate outside the workspace, or one locked under a member's name,
   is refused before the runtime closure is computed.
6. *The notice check keeps its code and gains a witness.* The existing prose
   decoy, which a subset-only comparison accepts, is the retained witness for
   a missing entry.

**Rationale.** D-358 left two items of the VT-d domain owed, residuals R3 and
R4 of Daybreak findings 1 and 8 in the 2026-10-03 triage: reserved regions were
identity-mapped as declared, so a firmware table could hand the controller
kernel frames, and a `Clone` and `Copy` proof could outlive the release that
invalidated it. The same triage found four checks that let one input shape
through, each a defect of the check alone because the committed sources pass
every strengthened check:

- D-351's port-claim contract visited only expression paths, so
  `use wost::arch::current::port::claim;` followed by `claim::<u32>(0xF4)`, or
  `use wost::arch::current::port as io;`, passed it (finding 5).
- D-345's manifest reader marked a runtime section only for the exact
  spellings `[dependencies]` and `[target.<cfg>.dependencies]` and skipped
  every other header without an error, so `[dependencies] # c`,
  `[ dependencies ]`, `["dependencies"]`, `[dependencies . x]` and a target
  header followed by a comment each dropped their section from the runtime
  closure (finding 11).
- A source-less package in `Cargo.lock` was never checked to be a workspace
  member, so a path crate outside the workspace left the inventory and the
  SPDX document (finding 15).
- The notice check's set equality had no retained witness for a missing
  entry, because mutation 1,508 weakens only the extra-entry direction
  (finding 13).

**Evidence.** On 2026-10-03, on `2506a74` carrying this change, each retained
mutation was applied once, failed its exact witness and was restored byte for
byte. Mutation 1,589 (the screen checks only that a region is ordered) failed
`a reserved dma region over kernel memory refuses isolation`. In `memory-map`,
1,573 failed `tests::a_reserved_region_inside_a_device_hole_is_admitted`;
1,574 and 1,575 failed
`tests::a_reserved_region_one_byte_into_kernel_memory_is_refused`; 1,576
failed `tests::an_empty_or_inverted_reserved_region_is_refused`; and 1,577 and
1,578 failed `tests::a_reserved_region_sharing_a_page_with_kernel_memory_is_refused`.
Mutation 1,579 restored both derives and 1,580 derived `Clone` alone, and
`contracts::pci_configuration_writes_require_function_authority` failed each.
Mutations 1,581 through 1,584 claimed through an imported `claim_block`, a
`claim_pair as pair`, a `port as io` rename and a `port::*` glob, and
`contracts::production_kernel_code_claims_no_port_by_number` failed each.
Mutation 1,585 removed the header refusal and failed
`supply_chain::tests::every_dependency_declaration_shape_is_modeled_or_refused`;
1,586 removed the member check and 1,587 kept membership without distinctness,
and `supply_chain::tests::a_locked_path_package_must_be_a_distinct_workspace_member`
failed each; 1,588 made the notice comparison `noticed.is_subset(&runtime)` and
failed `supply_chain::tests::each_runtime_component_has_exactly_one_exact_notice_entry`.

**Alternatives.** Checking a reserved region byte by byte was rejected: the
unit maps whole pages, so a region that shares a page with kernel memory would
still hand the device the rest of that page. Copying the boot map into the
candidate path was rejected because the map is about 4 KiB by value and every
by-value temporary takes its own slot in the debug candidate frames (D-287); a
second copy held by the kernel would duplicate a boot fact `wost` already owns.
Mapping only the part of a region outside kernel memory was rejected: firmware
declared the region whole for the device, and refusing the candidate is the
fail-closed answer D-357 chose, which needs no guess about the part the device
uses. Resolving `port` aliases to the calls made through them was rejected as
machinery for a spelling no production driver needs. Making the raw doors
crate-private, the triage's stronger option for finding 5, was not taken, for
D-351's reason: the self-test harness probes the registry through them.
Refusing only header spellings that contain `dependencies` was rejected because
a quoted key can spell that word with escapes, which only a TOML parser
resolves. Matching path packages to members by name alone was rejected because
a path crate locked under a member's name would pass.

**Left owed.** Two shapes stay outside the checks; each was observed on
2026-10-03 through a throwaway host test, removed afterwards. The manifest
reader returns no names for a dependency table reached through a key line under
another table: `dependencies = { hidden = "1" }` and `dependencies.hidden = "1"`
under `[target.'cfg(unix)']`, and `dependencies.hidden = "1"` before the first
header. The port contract records nothing for a raw claim written inside a
macro's arguments, such as `info!("{:?}", port::claim::<u8>(0x80))` or a
`claim_block` inside `assert!`, because the parsed syntax keeps macro arguments
as unparsed tokens.
