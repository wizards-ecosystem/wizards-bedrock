# D-123 Four ways safe code reached undefined behaviour, and the road back that was not the only one

Status: Accepted, 2026-08-24.

**Decision.** The high findings of two audits held to D-105's two rules, beside
D-122's critical one:

- **No safe call delivers an interrupt on an error-code vector.** The entry
  stubs push a synthetic zero error code exactly where the CPU does not, and an
  APIC delivery pushes none whatever the vector *means*, so such a delivery
  shifts the frame eight bytes for the rest of its life.
  `apic::send_self_interrupt`, `set_timer_one_shot` and `interrupt::init`'s
  `user_gate` (the last ring-3-callable) refuse
  `exception::pushes_error_code`.
- **`reclaim` takes a `Released` from the task table.** Whether anything still
  names a space is a property of the **task table**, which `AddressSpace` cannot
  see, and `prepare` loads a slot's root unconditionally; a `CR3` comparison
  was the shape of the bug. `task::release_address_space` looks, clears the
  binding from the dead slots naming it, and mints a `Released`. `reclaim`
  **borrows** rather than consumes, because a refusal that consumed the thing
  it declined to act on leaks the tables it was protecting. On success the
  space is *emptied*: `is_reclaimed` is true and `may_reach` refuses.
  `address_space_of` reports only a **live** slot's tables.
- **`cpu::enable_extended_state` is `pub(super)`**: D-115's ordering argument is
  about one call site, and a public door made it an argument about nothing.
- **`unmap` is not a second road out of untyped memory.** It checked `OWNED`
  and never `USER`, so a dead program's pages went back to the allocator
  **holding its bytes**. `Unmapped` carries `was_user`: any page ring 3 could
  reach makes the whole run untyped, because a run is handed back as one thing.
  `flush` refuses such a range, `take_user` gives a `USegment`, and `Drop`
  clears before the frames go back. The clear is *counted*, so the line is
  testable.

**Rationale.** Nothing disclosed anything through `unmap`, because every
consumer happens to zero on the way in. That is the honest limit and also the
point: the mechanism named as the guarantee was not the one doing the work, and
defence in depth was carrying a property its owner claimed. `cargo xtask
mutate` reported the uncounted clear as a **survivor**, which is what the tool
is for.

**Consequences.** `a_user_page_comes_back_cleared` follows one handle the whole
way (map, unmap, `take_user`, `reclaim`, read) rather than asking the allocator
for the frame back, because a conjunct conditional on reuse is one skipped on
most boots.

Left owed from these audits: the `libs/` mutation survivors (`x86-cpuid`'s four
SDM constants, `handle-space`'s three identity asserts, `acpi-tables`'
`Sdt::LEN`, and five more), `xtask/src/elf.rs`'s missing `e_machine` and
`e_phentsize` refusals, the gate's two enumeration holes, the ABI cross-check's
19 unchecked constants, thirteen checks that cannot fail, and a list of doc
comments. Each is named in the audits and none is a soundness hole.
