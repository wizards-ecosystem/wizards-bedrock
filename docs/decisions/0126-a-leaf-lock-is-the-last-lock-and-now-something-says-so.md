# D-126 A leaf lock is the last lock, and now something says so

Status: Accepted, 2026-08-24.

**Context.** D-121 wrote the rule down: a `lock_leaf` static may not expose a
closure form, because a leaf guard's whole obligation is that **nothing is
called while it is live**. Its "Left owed" said the rule was not mechanically
enforced and that a lint would be worth writing when there was a fourth
offender.

There were four already. Nothing had looked.

The obligation is asymmetric, and the first version of the check got it
backwards. A leaf taken *under* a token-bearing lock is the design — it is what
makes D-47's `HEAP` → `FRAMES` order sound, and it is the only acquisition a
`Drop` can perform, since `drop` takes no arguments and a freed `Frame` must
reach the allocator from wherever it dies. It is the other direction that is
the violation. A check counting "deepest nesting" flagged `Segment::drop`
taking `FRAMES` under a caller's guard, which is exactly what `lock_leaf`
exists to permit.

**What it found, once it asked the right question.**

- **`pic::silence`** held `CONTROLLERS`' guard and claimed ports under it,
  which takes `CLAIMED`'s. An order `CONTROLLERS → CLAIMED` established by
  accident, in a file that never mentions the other lock.
- **`user::serve_ring`** held `BOOTSTRAPS`' guard across the *entire* service
  of a program's ring: reading its memory, performing each operation, posting
  completions, and waking whatever was waiting. Four more locks, all under a
  guard whose contract is that nothing is called at all.
- **`user::ring_shape_of`** and **`user::bootstrap_of`** did the same, smaller:
  `BOOTSTRAPS` → the ring's own lock, and `BOOTSTRAPS` → the handle arena.

**A second defect, found by a peer session auditing the same window.** The
user-copy recovery arm in `trap.rs` had the asymmetry D-124 had just fixed on
the syscall arm. It runs *before* the ring-3 test — it must, because the fault
it exists for is taken in ring 0 with the kernel dereferencing — and it never
looked at the saved `CS`. A ring-3 thread that set `RIP` inside
`[wost_copy_fault_start, wost_copy_fault_end)` had its instruction-fetch fault
"recovered": `resume_at` rewrote `RIP` to the fixup with `CS` still ring 3, it
faulted again there, and only then was retired. Not an escalation and not a
halt, but a program could drive `USER_COPY_RECOVERIES` at will — and the arm's
own comment asserted a privilege nothing enforced.

**Options for the enforcement.**

1. **A lint over the sources**, as D-121 imagined. It would have to decide
   whether a call reaches a lock, which is the whole call graph, statically,
   across two crates.
2. **A `debug_assert` in `acquire`.** Loud, immediate, and only in the debug
   boot — and the violation is a latent deadlock rather than a crash, so a
   panic overstates what happened and stops the boot before the other three
   are found.
3. **A counter and a boot-time check**, reporting where.

**Decision.** Option 3. `LEAVES_HELD` is incremented only by `lock_leaf` and
`lock_from_trap`, and any acquisition made while it is non-zero records
`Location::caller()` for **both ends** — the lock being taken, and the
outermost leaf that was held. A boot-time check reports the pair and fails.

The outermost, specifically. Recording every leaf acquisition names whichever
was taken most recently, held or not, which sent the first reading of this
check to a function three call levels from the real one.

The four violations are fixed by the same move D-121 made: what the caller
needs is copied out and the guard dropped. `Bootstrap`'s ring and handle table
are behind `Arc`s so a clone is what leaves the guard; `pic::silence` claims
its ports between the two acquisitions rather than under either.

The trap arm is gated on `frame.code_selector() & 0b11 == 0`, and a **real
ring-3 thread** drives the check: the emitted user program gained a fourth
behaviour — an argument in the kernel's half is jumped to — and the kernel
spawns one aimed at `usercopy::fault_window()`. Ring 3 cannot execute kernel
memory, so this is an instruction-fetch fault with `RIP` inside the window and
`CS` at ring 3, which is the exact state the arm accepted.

**Rationale.** D-121's rule was written and immediately not followed, four
times, including twice in code written after it. That is the argument for
enforcement over documentation, and it is the same argument D-119 made about
D-28: a rule that is applied once, at the moment the code is written, is a rule
that decays silently.

Why runtime rather than static: the property is *dynamic*. `serve_ring` takes
no lock itself — it calls `Queue::serve`, which calls `WaitQueue::wake_all`,
which takes one. No local reading of `serve_ring` shows a violation, and the
lint that would have to see it is a whole-program call-graph analysis. A
counter sees it on the first boot that exercises the path, which is what a
boot-time check is for.

What this does **not** catch: a path no boot exercises. `handle.rs`'s
`reserve_a_slot` drops the old `Vec` under `ARENA`'s guard — an `ARENA` → `HEAP`
order — and it is latent, because the first growth frees a zero-capacity
vector and a second growth needs sixteen live handles. The check will name it
the day a boot does that, and not before. Stated plainly so a green result is
not read as a proof.

*It named it the same day.* D-128 wrote a check that holds forty handles in one
table, walking the arena's capacity through 16 → 32 → 64, and the detector
printed `heap.rs` under `handle.rs:171` on the first boot that ran it. Two
things worth keeping from that. The scenario was built **before** the fix, so
the check is honest by D-28's standard — it failed once, for the reason it
exists. And the violation had **no call site**: `reserve_a_slot` is correct
about every acquisition it *makes*, and the lock was taken by a destructor at
scope exit. All four violations above are visible calls, findable by reading;
this one is not, and a static lock-order analysis would have to model
destructor insertion across every scope exit to see it. That is a better
argument for the runtime counter than anything in this entry.

**Consequences.**

- Every `SpinLock` acquisition costs one relaxed load and a branch. The counter
  beside it already cost that.
- `acquire` takes a `leaf: bool`, and `lock`/`try_lock` pass `false` while
  `lock_leaf`/`try_lock_leaf`/`lock_from_trap`/`try_lock_from_trap` pass `true`.
  `SpinLockGuard` carries the flag so its release knows which counter to undo.
- `Bootstrap::ring` and `Bootstrap::table` are `Arc`s. Both are read on paths
  that must not hold the table's guard, and both have only `&self` methods.
- Two machine checks: `no lock is taken while a leaf guard is held`, which
  names both ends when it fails, and `a program cannot aim at the copy window`.
  Both were broken and watched go red (D-28); a mutation covers the second.
- The branch-target assertion in `program.rs` was itself wrong and said so: it
  scanned for the byte `0x75` and counted a *displacement* that happened to be
  `0x75` as a branch. It consumes the pair now. A byte scan over emitted code
  is a decoder pretending not to be one, and this is the second time that has
  cost something (D-115's jump displacement was the first).

**Left owed.** ~~The check is one word for the whole machine, which is right on
one CPU and wrong the moment there are two — `LEAVES_HELD` must move into
`arch::percpu` beside `SPINLOCKS_HELD` before an AP starts, or two CPUs will
report each other's nesting.~~ **Closed by D-154**, which built
`arch::percpu` and moved both counters and the violation record behind it.
`arch::percpu` did not exist when this was written, which is why the sentence
says "move into" a module that had to be created first.

`lock_from_trap` counts as a leaf, which is conservative: a trap handler's
acquisition is take-mutate-return by the same argument, but nothing checks that
it is, and the panic path takes locks under it deliberately.
