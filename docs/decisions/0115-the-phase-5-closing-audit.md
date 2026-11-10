# D-115 The Phase 5 closing audit

Status: Accepted, 2026-08-24.

**Context.** Slice 11 re-attacks the interfaces the phase froze (D-77's
pattern): can a hostile ring-3 program reach UB through any `wost` export, a
safe caller manufacture a handle, or a ring entry be double-fetched? The audit
was held to D-105's two rules: a check must not be satisfiable by the algebra of
its own inputs, and it must fail for the reason it names.

**Decision.** Fix what it found, and record two findings *about* the audit.

### Five that were reachable

**A boot check read numbers ring 3 owns.** `ring_shape_of` read the ring's
*header page*, mapped `USER|WRITABLE` into a program that is live when the check
runs. One store of `0x8000_0000` to `[RING + 4]` makes `2 * submissions`
overflow and panic the debug kernel. The conjunct also asserted a property of
userspace-writable memory. `Queue::shape` now reports the kernel's own record;
`published_shape` reads the page and asserts only *publication*.

**`xstate::init` enabled SSE and then refused.** `cpu::enable_extended_state`
clears `CR0.EM` and sets `CR4.OSFXSR`, so SSE becomes legal at CPL 3, and it was
called before three `return None` paths. On any of them `FORM` stays zero, the
switch saves nothing, and two ring-3 processes share the x87/SSE register file.
Before `init` that would have been a `#UD`, so the failure path left the machine
*worse than not calling it*. `CPUID.0DH` needs no control bit, so every refusal
now happens first and nothing below can fail.

**`translate_in_root` took a bare physical address.** It was a safe public
function walking caller-chosen physical memory through the direct map, and its
own doc left reachability to the caller. That is a fault on an unhandled address
or an arbitrary-read oracle. This was D-77's named debt in read form, in the
phase meant to discharge it. It now takes a `Root`, mintable only by
`task::address_space_of`.

**`map_shared` and `map_user` could alias one run.** Share it, map it owned,
unmap the owned mapping, and the frames are freed while a live `USER` mapping
still reaches them. `USegment::share` is one-way to a `Shared` that `map_user`
will not take, so the aliasing is no longer expressible.

**Three `lock_leaf` obligations were breached**, all added by this phase.
`Channel::send` reserved a `VecDeque` under a leaf guard, which reaches the
allocator, so `heap`'s `assert!(!in_trap())` would turn a `Channel::send` from a
handler into a panic, not an `OutOfMemory`. Reserving `DEPTH` once at
construction, so `push_back` cannot grow, fixed it before the audit reported it.
`Queue::install` held the run's guard across `map_shared`, which allocates
page-table frames. The new `SpinLock::get_mut` fixes it: `&mut self` proves
nobody holds the guard, where copying the address out would hand on a number in
place of the type's evidence. And `post` took `shared` then `state`, the reverse
of `take`: no deadlock on one CPU, but an inversion pre-seeded for SMP.

### Two structural repairs

`map_stack` verified its guard *after* mapping the stack. A `Conflict` there
left the stack mapped, its frames owned by the space with no handle, while
`map_owned`'s own doc says "on failure nothing was mapped". It now rolls back,
the shape D-105 gave `load_image`. And `USegment::reclaim` **leaks rather than
frees** when the clear fails. Those frames still hold a program's bytes, and
freeing them was the one path where the type's purpose lapsed silently.

`serve`'s silent `break` on a missing reservation would wedge a ring for a
process's life: `outstanding()` never drains and `admit` returns zero for ever.
D-112's model shows it unreachable, so it is now a `debug_assert`.
`spawn_prepared` retires `wost`'s slot as well as the kernel's when binding
fails. Releasing only one leaves the tables disagreeing, and every later spawn
of that slot is refused for ever.

### Nine checks that could not fail, and three that failed wrongly

The audit has the reductions; each check is rewritten or deleted. Three show one
mistake in three forms:

- `a_rewritten_submission_cannot_change_what_was_served` read the completion
  slot 128 bytes away from the submission the "hostile rewrite" touched, so it
  reduced to `posted == posted`: **no line in `queue.rs` could falsify it.** It
  now reads the rewritten submission back and asserts that serving again yields
  nothing.
- `xstate::extended_control()` began `if !is_enabled() { return 0 }`, so the
  check's `extended_control() == 0` conjunct read a Rust early return, not
  `XGETBV`. This is D-105's own `VERW`-arming finding again, three ADRs later.
  It guards on `CR4.OSXSAVE` now.
- **Nothing in 124 checks reached `posted > SUBMISSIONS`**, the bound that stops
  the kernel walking submission slots nothing was written into. The runaway-tail
  case ran against a *drained* ring, where `take` refuses anyway. The new case
  admits three and publishes a tail of four billion, and deleting the bound
  turns it red.

`attenuate` returning its own argument made three "narrowing" assertions
`X == X`. The content was always in the refusals, and the vacuous conjuncts are
gone.

### Two findings about the audit, not about the code

**One finding did not reproduce as described.** The audit rated `reap_up_to`'s
unvalidated head as defeating D-79's headline claim. It does not: `Ring::reap`
returns `None` on an empty completion queue, so a lying head reaps what is there
and no more. I added the explicit bound anyway, then broke each in turn.
**Neither can be shown necessary, because the other covers every input**. Both
stay; the explicit one states the intent and would matter if `reap` became
infallible. It is recorded so the next audit does not flag a redundant guard.

**A doc comment claimed an exception did not exist.** `Segment::read_at` said it
was "deliberately on `Segment` alone" because a `USegment` holds ring 3's bytes,
while `USegment::read_at` sat forty lines below forwarding to it. It was false
within the hour, and now names the exception and why `Shared` has one.

**Consequences.** 124 checks, and the rewrites let several of them fail for the
first time. `SpinLock::get_mut`, `task::retire`, `mm::Shared`, `task::Root` and
`Queue::published_shape` are new. What held: no double fetch in `queue.rs`, no
userspace index reaching memory, `serve` bounded whatever ring 3 publishes,
handles unforgeable and unwidenable, and `bind`'s comparison of the whole higher
half doing what D-106 claimed.

Left owed and named: the cross-process arena-occupancy oracle.
`Mixer::for_table` is a public pure function of a small counter, so slot
occupancy leaks across tables. No authority crosses, and the crate doc says the
mixer removes only the accidental case. Three unreachable paths in `queue.rs`
consume a submission without posting its completion, now loud in debug. The two
`lock_leaf` closure holes D-105 recorded are ~~still open~~, **closed by
D-121**.
