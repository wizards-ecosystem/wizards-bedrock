# D-87 A mapping carries its evidence: owned frames, or a range the handoff described

Status: Accepted, 2026-08-24. Discharges D-76's first residue.

**Context.** D-76 audited `wost` for what a *safe* caller could reach and found
`AddressSpace::map` taking a bare physical address. D-77 re-checked it, could
not close it, and recorded it deliberately: *"Two authority debts intentionally
remain for the phase that introduces the needed owner types: raw
`AddressSpace::map` can still name a physical address... Phase 5 must replace
the first with owned user-memory mappings."* This is that phase, and D-86 has
just made mapping reachable after activation, which turns the debt from latent
into live: safe kernel code could now map any physical address into the running
tables.

**Decision.** There is no public `map` taking a bare physical address. Three
entries remain, and each carries the evidence its case can actually produce:

- **`map_owned(virtual_base, run: Segment, flags)`** — the caller owns the
  frames and ownership moves into the space. D-44's type discipline applied to
  the mapping API. On failure the run is dropped and its frames go back, which
  is sound because D-77 made mapping preflight the whole range before writing
  any entry: a failed `map` mapped nothing.
- **`map_boot(virtual_base, region: BootRegion, flags)`** — for memory the
  bootloader described. `BootRegion::describe` is the only constructor and
  returns `None` unless the memory map covers the whole range, the base is
  page-aligned, the length is non-zero, and the end does not overflow. It is
  not `Copy`: it is a statement about one range, made once.
- **`map_device`** — unchanged, and already the narrowest of the three (D-49,
  D-64, D-77): it refuses RAM, boot, kernel, ACPI and framebuffer storage,
  reserves the physical interval, and verifies every claimed translation.

**Rationale.** The two legitimate cases are genuinely different and neither can
be expressed as the other. The direct map and the kernel image are *not* owned
frames — nobody allocated them, the firmware and the bootloader placed them —
so demanding ownership there would mean inventing a fiction. What can honestly
be demanded is that the handoff described the range, which is exactly the bound
D-64 gave physical reads: *a bound stated in prose is not a bound*, so the
arithmetic that checks it becomes the type that carries it.

Everything else is somebody's frame, and a frame already has an owner type with
a `Drop`. Moving it into the space is both the authority statement and the
lifetime statement in one move, and it is what gives unmap something to hand
back when it arrives.

**Alternatives.** Keep `map` and add a `# Safety` section (rejected: it is not
an `unsafe fn`, and D-76's whole standard is that a safe caller must not reach
UB however it is called); require ownership everywhere and fabricate `Frame`s
for firmware memory (rejected: a `Frame` whose `Drop` would free bootloader
memory into the allocator is worse than the hole it closes); check the memory
map inside `map` rather than in a type (rejected: the direct map's own
construction would then re-derive per call what one description settles, and
the evidence would not be visible in the signature).

**Consequences.**

- `MapError::Undescribed` names the refusal, with the address it would not map
  — the same courtesy D-77 gave the firmware-read path, and for the same
  reason: the narrower authority makes a legitimate-looking call the likely way
  it is first met.
- `map_stacks` stops leaking its run by hand; it calls `map_owned`, which is the
  one place the `forget` now lives.
- Two checks were rewritten rather than kept: *a differing remap is refused*
  used bare physical `4096` and now presents described regions, and the new
  *an undescribed range cannot be mapped* covers the four ways evidence can be
  absent — undescribed, unaligned, empty, and overflowing. The break is to make
  `describe` assume rather than ask, and it goes red.
- **The second half of D-77's debt is untouched and stays open**: port claims
  still prove exclusivity but not device entitlement. That one needs PCI
  discovery and is Phase 6's, exactly as recorded.
- Eighty-two named machine checks.
