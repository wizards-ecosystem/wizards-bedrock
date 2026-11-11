# D-139 The IDT is not the only door, and a guard nothing could observe

Status: Accepted, 2026-08-25.

**Decision.** `enable_protections` shuts both fast-syscall doors (`EFER.SCE`
and `IA32_SYSENTER_CS`), and `enter_user` **refuses** while either is open,
checked at the crossing, not only at boot, because the claim is about what a
particular program can reach. `syscall` and `sysenter` are *instructions*, not
vectors, so `0x80` being the only ring-3 gate in the table said nothing about
them, and `enable_protections` preserved whatever the firmware left in `EFER`.

`lookup`, `bind` and `reclaim` each refuse a reclaimed space, `bind` with
`ActivateError::Reclaimed`, and `reclaim` is idempotent. `may_reach` already
refused; the others walked root `0` as a top-level table, and a second
`reclaim` could hand **frame zero** to the allocator. A value that has been
emptied must refuse every door, not the doors someone remembered.

**The first two checks could not fail**, because frame zero reads as zeros and
the firmware leaves both doors shut, so a check on the *answer* was green with
the fix removed. D-28's *"break it once"* is a procedure and not a principle: a
green check is indistinguishable from a working one until you make it red on
purpose. The fixes:

- A `RECLAIMED_REFUSALS` tally lives **inside** each of the three guards, so
  removing one cannot move it. The check requires it to advance by exactly
  three.
- The syscall check is **driven**: open a door, confirm the reader sees it
  open, shut it through the kernel's own path, confirm it closed. The middle
  step is what proves the reader is not simply answering `true`.

The *shutting* is checked. Whether `enable_protections` **calls** it is not
falsifiable here, because the firmware already leaves both shut; that conjunct
is a statement about the machine, kept because it is what a *different*
firmware would break.

**Rationale.** Opening a door in a check is a deliberate risk, safe for a
reason that is checked rather than assumed: no program can be entered while it
is open, because `enter_user` refuses, so a failure stops programs from running
rather than exposing one.

**Consequences.** `cpu::open_fast_syscall_doors_for_test` is a `pub fn` in
`wost` that nothing but a check should call, following
`xstate::write_vector_probe`'s precedent: `wost` has no `selftest` feature, so
the name carries the intent. `paging::reclaimed_refusals` is a tally inside a
guard, like `CLEARED_ON_DROP` (D-123), because the property is invisible in the
answer.

**Left owed.** `entry_pointer`'s `// SAFETY:` comment claims *"`direct_pointer`
confirmed the direct map covers all `PAGE_SIZE` bytes"*, while `direct_pointer`'s
own doc says *"Arithmetic only. Whether the map actually reaches those bytes is
the caller's."* The claim is restated at eight sites. Correcting it is prose,
not code, and is owed.

`Root` is `Copy` while every sibling evidence type in this crate is
deliberately not, and `translate_in_root` never re-checks liveness. Same class
as the two fixed here, not yet closed.
