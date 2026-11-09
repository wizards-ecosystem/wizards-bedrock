# D-430 Leaf callback review follows authority, not a stale static count

Status: Accepted, 2026-10-23. Clarifies D-121, D-126 and D-200; preserves PW-707.

**Context.** PW-707 still described three leaf-lock statics and a future fourth
as the trigger for enforcement. D-126 had already found four and replaced
that counting proposal with an observed lock-order witness. The current tree
also contains D-200's bounded atomic readiness predicate, so a claim that no
callback-shaped leaf API exists would misdescribe the implementation.

**Decision.** D-121's prohibition remains on exposing protected contents to
caller-supplied general work while a leaf guard is held. Copy owned results
out and drop the guard before that work. The existing crate-private
`WaitQueue::wait_from_trap_until` is the audited readiness exception: its only
production caller is `Queue::wait_from_trap` in `kernel/src/queue.rs`, whose
predicate compares `self.served_count()` with the sampled generation.
`served_count` performs one `AtomicUsize::load(Ordering::Acquire)`; it takes no
lock, exposes no guarded contents and performs no general callback work.
`kernel/src/wait.rs` checks this predicate while holding the waiter guard,
then drops that guard before `thread::wait_from_trap` can switch. This keeps
D-200's registration/recheck boundary intact.

The private closure inside `wost::pci::isa_dma_quiet` likewise supplies no
caller callback authority: it invokes only the fixed adapter's `reassert`
under its permanent controller guard. D-428 checks those remask and status
operations. A new static alone is not a callback defect. PW-707 stays stopped
until a proposed API exposes new caller-supplied work or protected contents
under a leaf guard, or expands the existing atomic-only readiness predicate.
Before admitting that change, remove the exposure or add the bounded
source/semantic contract for its exact authority and a deliberate failing
witness. This clarification does not authorize a generic callback lint or
new runtime infrastructure.

**Existing evidence and limits.** `SpinLock::lock_leaf` reaches `acquire` in
`wost/src/sync.rs`, which reports acquisitions to `PerCpu::took_lock` in
`wost/src/arch/x86_64/percpu.rs`. In self-test builds, an acquisition while
`leaves_held` is nonzero records the outer and inner sites. The existing
`a_leaf_guard_is_the_last_lock` machine witness reports those sites through
`no lock is taken while a leaf guard is held`. That checks exercised lock
nesting; it neither proves every unexecuted path nor statically enforces the
atomic-only predicate. The inspected call chain supplies the latter's present
review evidence. No new machine result or foundation closure is claimed.

**Alternatives.** Retaining the fourth-static threshold would reopen a
condition D-126 already resolved while overlooking the actual callback
authority. Banning the existing readiness recheck would undo D-200's wait
protocol without a demonstrated defect. Treating the observed nesting check
as a whole-program callback proof would overstate its coverage.
