# D-87 A mapping carries its evidence: owned frames, or a range the handoff described

Status: Accepted, 2026-08-24. Discharges D-76's first residue.

**Context.** D-76 audited `wost` for what a *safe* caller could reach and found
`AddressSpace::map` taking a bare physical address. D-77 re-checked it and,
unable to close it, recorded it: *"Two authority debts intentionally remain for
the phase that introduces the needed owner types: raw `AddressSpace::map` can
still name a physical address... Phase 5 must replace the first with owned
user-memory mappings."* This is that phase. D-86 has just made mapping reachable
after activation, so the debt is live: safe kernel code could now map any
physical address into the running tables.

**Decision.** No public `map` takes a bare physical address. Three entries
remain, each carrying the evidence its case can produce:

- **`map_owned(virtual_base, run: Segment, flags)`**: the caller owns the frames
  and ownership moves into the space. This is D-44's type discipline applied to
  mapping. On failure the run is dropped and its frames go back. That is sound:
  D-77 made mapping preflight the whole range first, so a failed `map` mapped
  nothing.
- **`map_boot(virtual_base, region: BootRegion, flags)`**: for memory the
  bootloader described. Its only constructor, `BootRegion::describe`, returns
  `None` unless the memory map covers the whole range, the base is page-aligned,
  the length is non-zero, and the end does not overflow. It is not `Copy`: it is
  a statement about one range, made once.
- **`map_device`**: unchanged, and already the narrowest of the three (D-49,
  D-64, D-77). It refuses RAM, boot, kernel, ACPI and framebuffer storage,
  reserves the physical interval, and verifies every claimed translation.

**Rationale.** The two legitimate cases differ, and neither can be expressed as
the other. The direct map and the kernel image are *not* owned frames: nobody
allocated them, the firmware and the bootloader placed them. Demanding ownership
there would invent a fiction. What can be demanded is that the handoff described
the range. D-64 gave physical reads that bound: *a bound stated in prose is not
a bound*, so the arithmetic that checks it becomes the type that carries it.

Everything else is somebody's frame, and a frame already has an owner type with
a `Drop`. Moving it into the space states authority and lifetime in one move,
and gives unmap something to hand back when it arrives.

**Alternatives.** Keep `map` and add a `# Safety` section (rejected: it is not
an `unsafe fn`, and D-76's standard is that a safe caller must not reach UB
however it is called). Require ownership everywhere and fabricate `Frame`s for
firmware memory (rejected: a `Frame` whose `Drop` frees bootloader memory into
the allocator is worse than the hole it closes). Check the memory map inside
`map` instead of in a type (rejected: the direct map would re-derive per call
what one description settles, and the type would not show the evidence).

**Consequences.**

- `MapError::Undescribed` names the refusal, with the address it would not map.
  D-77 gave the firmware-read path the same courtesy, for the same reason: the
  narrower authority makes a legitimate-looking call the likely first contact.
- `map_stacks` stops leaking its run by hand and calls `map_owned`, now the one
  place the `forget` lives.
- Two checks were rewritten. *a differing remap is refused* used bare physical
  `4096` and now presents described regions. The new *an undescribed range
  cannot be mapped* covers the four ways evidence can be absent: undescribed,
  unaligned, empty, and overflowing. The break makes `describe` assume instead
  of ask, and it goes red.
- **The second half of D-77's debt is untouched and stays open**: port claims
  still prove exclusivity but not device entitlement. That one needs PCI
  discovery and is Phase 6's, as recorded.
- Eighty-two named machine checks.
