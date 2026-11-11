# D-115 The Phase 5 closing audit

Status: Accepted, 2026-08-24.

**Decision.** Slice 11 re-attacks the interfaces the phase froze (D-77's
pattern), held to D-105's two rules: a check must not be satisfiable by the
algebra of its own inputs, and it must fail for the reason it names. Its fixes:

- **No boot check reads numbers ring 3 owns.** `Queue::shape` reports the
  kernel's own record; `published_shape` reads the user-writable header page and
  asserts only *publication*.
- **`xstate::init` refuses before it enables.** `CPUID.0DH` needs no control
  bit, so every refusal happens first and nothing below can fail; enabling SSE
  and then refusing left two ring-3 processes sharing the register file.
- **`translate_in_root` takes a `Root`**, mintable only by
  `task::address_space_of`, not a bare physical address: D-77's named debt in
  read form.
- **`USegment::share` is one-way to a `Shared` that `map_user` will not take**,
  so a run cannot be both shared and owned.
- **`lock_leaf` obligations hold.** `Channel::send` reserves `DEPTH` at
  construction, so `push_back` cannot grow under a leaf guard.
  `Queue::install` uses `SpinLock::get_mut`: `&mut self` proves nobody holds
  the guard, where copying the address out would hand on a number in place of
  the type's evidence. `post` takes its locks in `take`'s order: it took
  `shared` then `state`, the reverse of `take`, which is no deadlock on one
  CPU but an inversion pre-seeded for SMP.
- **`map_stack` rolls back** a failed guard check, the shape D-105 gave
  `load_image`.
- **`USegment::reclaim` leaks rather than frees when the clear fails**, because
  those frames still hold a program's bytes.
- `serve`'s missing-reservation `break` is a `debug_assert`: silent, it would
  wedge a ring for a process's life, and D-112's model shows it unreachable. `spawn_prepared` retires `wost`'s slot as well as the
  kernel's when binding fails, so the tables never disagree.
- Checks that could not fail are rewritten or deleted. A rewritten submission
  is read back so the rewrite is shown to have landed. `extended_control()`
  guards on `CR4.OSXSAVE`, not a Rust early return. A runaway tail is tested
  against an admitted ring, so deleting the `posted > SUBMISSIONS` bound turns
  it red.

**Rationale.** What held: no double fetch in `queue.rs`, no userspace index
reaching memory, `serve` bounded whatever ring 3 publishes, handles
unforgeable and unwidenable, and `bind`'s comparison of the whole higher half
doing what D-106 claimed.

**Two findings about the audit, not about the code.** `reap_up_to`'s
unvalidated head does not defeat D-79's claim: `Ring::reap` returns `None` on
an empty completion queue. The explicit bound stays beside it, though
**neither can be shown necessary, because the other covers every input**; it
states the intent and would matter if `reap` became infallible, and is
recorded so the next audit does not flag a redundant guard. And
`Segment::read_at`'s doc named the `USegment` exception it had denied.

**Consequences.** 124 checks at the close.

Left owed and named: the cross-process arena-occupancy oracle.
`Mixer::for_table` is a public pure function of a small counter, so slot
occupancy leaks across tables. No authority crosses, and the crate doc says the
mixer removes only the accidental case. Three unreachable paths in `queue.rs`
consume a submission without posting its completion, now loud in debug. The two
`lock_leaf` closure holes D-105 recorded are ~~still open~~, **closed by
D-121**.
