# D-115 The Phase 5 closing audit

Status: Accepted (2026-08-24)

**Context.** Slice 11's instruction (D-77's pattern) is to re-attack the
interfaces the phase froze: can a hostile ring-3 program reach UB through any
`wost` export, can a safe caller manufacture a handle, can a ring entry be
double-fetched. The audit was held to D-105's two rules — a check must not be
satisfiable by the algebra of its own inputs, and a check must fail for the
reason it names.

**Decision.** Fix what it found, and record two things it found *about* the
audit process itself.

### Five that were reachable

**A boot check read numbers ring 3 owns.** `ring_shape_of` read the ring's
*header page*, which is mapped `USER|WRITABLE` into a program that is live by
the time the check runs. One store of `0x8000_0000` to `[RING + 4]` makes
`2 * submissions` overflow and panic the debug kernel — and independently, the
whole conjunct asserted a property of userspace-writable memory. `Queue::shape`
now reports the kernel's own record; `published_shape` reads the page and is
documented as an assertion about *publication* and never about shape.

**`xstate::init` enabled SSE and then refused.** `cpu::enable_extended_state`
clears `CR0.EM` and sets `CR4.OSFXSR`, so SSE becomes legal at CPL 3 — and it
was called before three `return None` paths. On any of them `FORM` stays zero,
so the switch saves nothing, and two ring-3 processes share the x87/SSE
register file. Before `init` ran that would have been a `#UD`, so the failure
path left the machine *worse than not calling it at all*. `CPUID.0DH` needs no
control bit, so every refusal now happens first and nothing below can fail.

**`translate_in_root` took a bare physical address.** A safe public function
that walks caller-chosen physical memory through the direct map — whose own
doc says reaching is the caller's to establish — is either a fault on an
unhandled address or an arbitrary-read oracle. This is D-77's named debt in
read form, in the phase meant to discharge it. It takes a `Root` now, mintable
only by `task::address_space_of`.

**`map_shared` and `map_user` could alias one run.** Share it, map it owned,
unmap the owned mapping: the frames go back to the allocator while a live
`USER` mapping still reaches them. `USegment::share` is one-way to a `Shared`
that `map_user` will not take, so the aliasing is no longer expressible.

**Three `lock_leaf` obligations were breached**, all added by this phase.
`Channel::send` reserved a `VecDeque` under a leaf guard — which reaches the
allocator, and `heap`'s own `assert!(!in_trap())` would have turned a
`Channel::send` from a handler into a panic rather than an `OutOfMemory`. Fixed
before the audit reported it, by reserving `DEPTH` once at construction so
`push_back` cannot grow. `Queue::install` held the run's guard across
`map_shared`, which allocates page-table frames: `SpinLock::get_mut` is new for
this, because `&mut self` proves nobody holds the guard and copying the address
out instead would have handed on a number where the type was the evidence. And
`post` took `shared` then `state` where `take` takes them in the other order —
no deadlock on one CPU, and an inversion pre-seeded for SMP.

### Two structural repairs

`map_stack` verified its guard *after* mapping the stack, so a `Conflict` there
returned an error with the stack mapped and its frames owned by the space with
no handle — while `map_owned`'s own doc says "on failure nothing was mapped".
It rolls back now, the same shape D-105 gave `load_image`. And
`USegment::reclaim` **leaks rather than frees** when the clear fails: those
frames still hold a program's bytes, and returning them to the allocator was
the one path where the type's whole purpose lapsed silently.

`serve`'s silent `break` on a missing reservation would wedge a ring for a
process's life — `outstanding()` never drains, `admit` returns zero for ever.
Unreachable by D-112's model, and a `debug_assert` now rather than a bug report
nobody can diagnose. `spawn_prepared` retires `wost`'s slot as well as the
kernel's when binding fails, because releasing only one leaves the two tables
disagreeing and every later spawn that picks that slot refused for ever.

### Nine checks that could not fail, and three that failed wrongly

The reductions are in the audit and each is now either rewritten or deleted.
Three are worth stating because they are the same mistake in three costumes:

- `a_rewritten_submission_cannot_change_what_was_served` read the completion
  slot 128 bytes away from the submission the "hostile rewrite" touched, so it
  reduced to `posted == posted` — **no line in `queue.rs` could falsify it.**
  It now reads the rewritten submission back, so the rewrite is shown to have
  landed, and asserts that serving again yields nothing.
- `xstate::extended_control()` began `if !is_enabled() { return 0 }`, so the
  check's `extended_control() == 0` conjunct read a Rust early return rather
  than `XGETBV`. This is D-105's own `VERW`-arming finding, in a fresh
  instance, three ADRs later. It guards on `CR4.OSXSAVE` now.
- **Nothing in 124 checks reached `posted > SUBMISSIONS`** — the bound that
  stops the kernel walking submission slots nothing was written into. The
  runaway-tail case was tested against a *drained* ring, where `take` refuses
  anyway. The new case admits three and publishes a tail of four billion, and
  deleting the bound turns it red.

`attenuate` returning its own argument meant three "narrowing" assertions were
`X == X`; the genuine content was always in the refusals, and the vacuous
conjuncts are gone.

### Two findings about the audit, not about the code

**One finding did not reproduce as described, and saying so is the point.** The
audit rated `reap_up_to`'s unvalidated head as defeating D-79's headline claim.
It does not: `Ring::reap` returns `None` on an empty completion queue, so a
lying head reaps what is there and no more. I added the explicit bound anyway
and then established, by breaking each in turn, that **neither bound can be
shown load-bearing, because the other covers every input.** Both are kept — the
explicit one states the intent and would matter if `reap` ever became
infallible — and this paragraph exists because a redundant guard with a
confident comment is exactly what the next audit would flag.

**A doc comment claimed an exception did not exist.** `Segment::read_at` said
it was "deliberately on `Segment` alone" because a `USegment` holds ring 3's
bytes — while `USegment::read_at` sat forty lines below forwarding to it. The
sentence was true when written and false within the hour. It now names the
exception and why `Shared` has one.

**Consequences.** 124 checks, and the rewrites mean several of them can fail
for the first time. `SpinLock::get_mut`, `task::retire`, `mm::Shared`,
`task::Root` and `Queue::published_shape` are new. What held is worth recording
too: no double fetch anywhere in `queue.rs`, no userspace index reaching
memory, `serve` bounded whatever ring 3 publishes, handles unforgeable and
unwidenable, and `bind`'s comparison of the whole higher half doing exactly
what D-106 claimed.

Left owed and named: the cross-process arena-occupancy oracle (`Mixer::for_table`
is a public pure function of a small counter, so slot occupancy leaks across
tables — no authority crosses, and the crate doc is honest that the mixer only
removes the accidental case); three unreachable paths in `queue.rs` that
consume a submission without posting its completion, now loud in debug; and the
two `lock_leaf` closure holes D-105 recorded — ~~still open~~, **closed by
D-121**.
