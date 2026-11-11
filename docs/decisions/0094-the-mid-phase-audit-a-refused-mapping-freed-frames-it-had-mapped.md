# D-94 The mid-phase audit: a refused mapping freed frames it had mapped

Status: Accepted, 2026-08-24.

**Decision.** D-77's attack on green applies *mid*-phase too, through a pass
told to break slices 1 to 4. D-87 gave `map_owned` this contract:

> On failure nothing was mapped — the whole range is preflighted before any
> entry is written (D-77) — so `run` is dropped and its frames go back to the
> allocator.

**There was no preflight.** `map_range` wrote each entry as it walked, so a
range failing part-way left the earlier pages mapped while `map_owned` returned
their frames to the allocator: memory a live mapping could write, and a later
`unmap` would free twice. The fixes:

1. **`map_range` rolls back.** A failed mapping clears every entry it wrote, so
   "nothing was mapped" is true rather than claimed. Tables allocated on the way
   stay as zeroed frames pointing nowhere; reclaiming one means proving no
   sibling survives.
2. **`unmap` refuses non-canonical addresses.** A walk masks bits 48 and above,
   so the low-half alias of a higher-half address reached the same tables while
   the `invlpg` named a form that invalidates nothing, leaving a stale writable
   TLB entry.
3. **`set_double_fault_stack` takes `&MappedStack`**, not a bare `u64`, so no
   safe caller can point IST1 at anything without evidence.
4. **The store is the eight-byte entry, not the whole segment**, since a
   `TaskState` memcpy can be observed torn. `TaskState::interrupt_stack_offset`
   names the offset, host-tested against SDM figure 8-11.
5. **`guard_the_fault_stack` goes through `map_stack`**, which verifies the
   guard.
6. **`map_owned` maps four-kilobyte pages only.** A large page carrying `OWNED`
   is one `unmap` must refuse for ever, since it does not split (D-45): an
   unreachable leak.

`cpuid(7, 0)` is read behind a max-leaf guard, since without one it is a
`#GP` on pre-2012 hardware. `continue_on_stack` asserts that
no lock guard is live on the stack it abandons, the rule `prepare` enforces for
the analogous case.

**The checks.** *the double-fault stack is guarded* ended on a tautology, D-77's
own corrected failure mode. *unmapping refuses what it does not own* now checks
six distinct refusals, with the direct-map address *probed* rather than
assumed, because the first two megabytes are mapped in small pages.
`libs/slab-layout`'s test compares against counts and offsets transcribed by
hand.

**The prose.** Taking two spin locks is not always `E0499`:
`ThreadContext::claim` mints a token wherever the interrupt flag says thread
context, so a second one can be minted inside a live guard. The compile-time
half is a strong default, not a proof: one visible, greppable line defeats it,
audited like an `unsafe` block. `011` referenced `libs/lock-rank`, which D-85
explains was never written. `mm::with_memory_map` holds a lock across a call
inside `wost`.

**Rationale.** Reading the code against its own documentation reached every
finding, and the gate found none, because the checks had drifted toward
restating the implementation. That is why the adversarial pass is a *pass*, told
to break the code.

**Consequences.**

- *a refused mapping leaves no trace* is the check the first defect earned. It
  went red when the fix was reverted, the only way to know it catches its bug.
- **Three findings are recorded and not fixed**, because each is a design
  decision rather than a repair. The closure-under-guard hole
  (`with_memory_map`, `with_io_apic`, `with_active_space`) needs a guard type
  that forbids calls. `mem::forget` on a `SpinLockGuard` strands the interrupt
  flag, which `Unmapped`'s own doc says no soundness argument may rest on.
  `MappedStack` can be invalidated by `unmap` while the evidence is still live.
  They are in `011`'s open list.
- **A property restated in a second place is not held in both.** D-87's contract
  was true where D-77 wrote it and false where D-87 repeated it. Only reading
  the two together showed that.
