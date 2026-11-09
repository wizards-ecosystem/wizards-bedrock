# D-139 The IDT is not the only door, and a guard nothing could observe

Status: Accepted (2026-08-25)

**Context.** Three latent soundness findings from the Phase 0–5 audit, and one
lesson learned by getting the checks for them wrong first.

**`vector::SYSCALL`'s doc says `0x80` is "the only gate in the table ring 3 may
reach".** True of the table. `syscall` and `sysenter` are *instructions*, not
vectors. Neither `EFER.SCE` nor `IA32_SYSENTER_CS` was ever read, cleared or
asserted anywhere in the tree — and `enable_protections` writes `EFER` by
**OR**-ing a bit into what it read, so whatever the firmware left was
preserved. With either open, a ring-3 `syscall` transfers to ring 0 at `LSTAR`
with no stack switch, at an address this kernel never chose.

Nothing in the tree executes either instruction. So the doors were open, unused
and unguarded — the "a claim nothing enforces" shape of D-124 and D-126.

**Two read-only doors walked a reclaimed space.** `reclaim` empties a space by
setting `self.root = 0`. `may_reach` has carried an `is_reclaimed()` guard
since D-123 — *"Mapping into one would walk frames the allocator has given to
something else"* — and `lookup` and `bind` did not. Both walk `self.root`, so
both read **physical frame zero as a top-level table**; any word there with bit
0 set sends the next level to an arbitrary fifty-two-bit address. `bind` then
installs it.

**And `reclaim` was not idempotent.** A second call on a never-bound space fell
past the `Released` gate and reached `drop(Frame::from_raw(0))` — safe code
violating that function's stated contract, handing **frame zero** to the
allocator. Nothing excludes frame zero from the allocatable set and the
first-fit hint starts there, so it is plausibly the kernel's own PML4.

**Decision.** `enable_protections` shuts both fast-syscall doors, and
`enter_user` **refuses** while either is open — checked at the crossing, not
only at boot, because the claim is about what a particular program can reach.
`lookup`, `bind` and `reclaim` each refuse a reclaimed space, `bind` with a new
`ActivateError::Reclaimed`.

**The part worth recording is that the first two checks could not fail.**

Both were written, both passed, and both stayed green when the fix was removed:

- Physical frame zero reads as zeros on both test machines. An unguarded
  `lookup` walks it, finds no present entry, and returns the same `None` the
  guard returns. A check on the *answer* is green with the guard deleted.
- Limine leaves `SCE` clear and `SYSENTER_CS` is zero at reset, so
  `fast_syscall_doors_are_shut()` reads true whether or not the kernel ever
  wrote anything.

Both are D-124's shape — a check measuring a description — written into the
same session that removed four of them, by the author of the entry that names
the pattern. That is the argument for D-28's *"break it once"* being a
procedure and not a principle: it is not that the rule is hard to believe, it
is that a green check is indistinguishable from a working one until you make it
red on purpose.

The fixes:

- A `RECLAIMED_REFUSALS` tally lives **inside** each of the three guards, so
  removing one cannot move it. The check requires it to advance by exactly
  three.
- The syscall check is **driven**: open a door, confirm the reader sees it
  open, shut it through the kernel's own path, confirm it closed. The middle
  step is what proves the reader is not simply answering `true`.

**And what the second check still does not establish, stated rather than
implied.** The *shutting* is checked — break `shut_the_fast_syscall_doors` and
it goes red. Whether `enable_protections` **calls** it is not falsifiable here,
because the firmware already leaves both shut. That conjunct is a statement
about the machine, not the kernel, and it is kept because it is what a
*different* firmware would break.

**Rationale.** The three guards are one idea: a value that has been emptied
must refuse every door, not the doors someone remembered. `is_reclaimed`'s own
doc already said "a reclaimed space answers nothing and maps nothing" — the
mapping half was enforced and the answering half was prose.

Opening a door in a check is a deliberate risk and worth naming. It is safe
here for a reason that is checked rather than assumed: no program can be
entered while it is open, because `enter_user` refuses — so a failure of this
check stops programs from running rather than exposing one, which the break
test showed by turning two ring-3 checks red alongside it.

**Consequences.**

- 141 checks. Two of them exist only because the obvious versions could not
  fail.
- `cpu::open_fast_syscall_doors_for_test` is a `pub fn` in `wost` that nothing
  but a check should call, following `xstate::write_vector_probe`'s precedent —
  `wost` has no `selftest` feature, so the name carries the intent.
- `ActivateError` gains `Reclaimed`. `reclaim` is idempotent.
- `paging::reclaimed_refusals` is the second tally-inside-a-guard in this
  crate, after `CLEARED_ON_DROP` (D-123). Both exist because the property is
  invisible in the answer.

**Left owed.** `entry_pointer`'s `// SAFETY:` comment claims *"`direct_pointer`
confirmed the direct map covers all `PAGE_SIZE` bytes"*. `direct_pointer`'s own
doc says the opposite — *"Arithmetic only. Whether the map actually reaches
those bytes is the caller's."* The claim is restated at eight sites and is what
made these two findings look guarded. Correcting it is prose, not code, and is
owed.

`Root` is `Copy` while every sibling evidence type in this crate is
deliberately not, and `translate_in_root` never re-checks liveness. Same class
as the two fixed here, not yet closed.
