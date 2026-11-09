# D-133 A premise about every trap arm, kept in a different file from the arms

Status: Accepted (2026-08-25)

**Context.** `thread.rs` opens with the rule the whole tickless design rests
on: *every path that changes who is runnable ends by arming the timer for the
next moment a decision is due, or stopping it when there is none.* D-57
established it and D-93 fixed `unblock` for breaking it.

D-93's fix was conditional:

```rust
if woken > 0 && !interrupt::in_trap() {
    reschedule_timer(Some(time::now_ns()));
}
```

with the reason beside it: *"From a handler that is fine — the trap exit
reschedules (D-54)."* That is a claim about **every arm of `on_trap`**, and
`on_trap` is in a different file. Nothing compared the two, and they stopped
agreeing the moment ring 3 got a gate.

The timer arm ends in `thread::reschedule`. The keyboard arm ends in
`thread::reschedule` when a keystroke woke somebody. **The syscall arm does
not.** `Call::RingEnter` runs `serve_ring`, which runs `Queue::serve`, which
runs `WaitQueue::wake_all`, which runs `unblock` — and then returns an answer
in `rax` and goes back to ring 3. A `ring_enter` that wakes a waiter leaves it
Ready with nothing armed for it, on a kernel where there may be no other timer
running at all (D-57).

Latent, and only just: `wake_all` returns zero when nothing is registered, so
`unblock` is never reached, and nothing blocks on a `Queue`'s wait queue yet.
"Yet" is doing real work in that sentence — a blocking receive on a ring is the
first thing anything built on this ABI will want.

**Options.**

1. **Add `thread::reschedule(frame)` to the syscall arm.** Restores the
   agreement, and leaves the coupling in place for the next arm somebody
   writes. It also changes when user threads yield, which is a scheduling
   change made to fix a bookkeeping bug.
2. **Arm unconditionally in `unblock`.** Deletes the coupling instead of
   repairing it. Costs one APIC write on the two paths that were going to
   reschedule anyway.
3. **Leave it and record the debt.** Nothing is broken today.

**Decision.** Option 2. `unblock` arms whenever it woke anything, trap context
included.

**Rationale.** The bug is not that the syscall arm forgot something. It is that
a premise about a set of call sites was written next to the code that depends
on it rather than next to the sites, so the two could drift with nothing to
notice — and they did. Option 1 restores the agreement and leaves the same trap
set for the next person to add an arm; option 2 removes the reason the two files
had to agree at all.

The cost is one register write on a path that has just taken an interrupt, and
`unblock`'s own comment already stated the rule unconditionally. The `in_trap`
exemption was the thing stopping it obeying, added on a premise that has since
become false.

Option 1 is also the larger behaviour change of the two, which is the opposite
of how it reads: arming a timer that may fire slightly early is smaller than
changing the point at which a user thread can be preempted.

**The check is about the arming, not the running, and that is the whole
difficulty.** A woken thread runs eventually whatever this does — the checking
thread's own yields and sleeps arm timers, and any of them picks it up — so a
check that waited for it to run passes with the bug present. That is the shape
D-124 kept finding, and it is exactly why this survived: *the failure is only
observable on a machine that would otherwise be idle, and a self-test is not
idle.*

So the check reads a counter of wakes-from-a-handler that armed here. The
keyboard is the only handler that wakes anything today, which is what makes a
zero a falsifiable claim rather than an absence of evidence — the check
requires the blocked reader to have woken *and* the counter to have moved.
Broken and watched go red (D-28). The two lines together are the argument, and
they are worth reading in the order the boot printed them:

```
[info ] selftest: the blocked reader woke with the injected key, without polling
[error] selftest: a wake from a handler arms the timer for what it woke FAILED
[error] selftest: verdict v1 FAIL passed=136 failed=1 expected=137
```

**The reader still wakes.** The keyboard arm reschedules a moment later, so the
consequence arrives whether or not `unblock` armed anything — and every check
that asserts the consequence is green on that boot. Only the counter is red.
Any version of this check written against "did the thread run" would have
passed against the defect it was written for.

**Consequences.**

- One machine check, `a wake from a handler arms the timer for what it woke`,
  and one counter, `HANDLER_ARMS`, which was zero for the whole of the kernel's
  life before this.
- `queue.rs`'s `take` no longer fails silently on either of its two unreachable
  paths. `submit` reserves a completion slot and `take` consumes the
  submission; a `None` from `Ring::consume` after a successful `produce`, or a
  `read_at` refusing a range that is inside the shared pages by construction,
  would hold the reservation for ever. The ring then admits nothing more and
  the program waits on a completion that will not come — a permanent wedge,
  which is worse than a panic because nothing can diagnose it. `debug_assert!`
  in both, matching what D-115 already did to `serve`'s twin two functions
  down; the inconsistency was in one file, from one ADR.

**Left owed at adoption; closed by D-223.** The check exercises the *keyboard* path, because that is the
only wake from a handler this kernel performs. **The syscall path — the one
that was actually broken — is not exercised**, and it is worth being precise
that this is a check that is *constructible and not built* rather than one that
cannot be built: `Bootstrap::ring` is an `Arc` since D-126, so a kernel thread
can hold the same ring a program does; `queue.rs` already carries
`reserve_for_test`, `post_submission_for_test` and `publish_tail_for_test`, so
a `wait_for_test` is in the file's idiom; and the program that rings its own
doorbell exists. What is missing is the orchestration to get a kernel thread
blocked on that ring *before* the program rings it, which is a race the check
would have to win rather than assume. Those are different debts and the second
kind is the one nobody ever pays.

D-223 pays it by stopping a real child before ring-3 entry, registering a
kernel thread on that child's production ring wait queue, and only then
letting the child execute `RingEnter`. The old exemption now reddens the exact
syscall check rather than a keyboard proxy.

The redundant arming on the timer and keyboard paths is real and unmeasured. It
is one write to the APIC's initial-count register, on a path already committed
to a `reschedule` that writes it again; if that ever shows up in a latency
measurement, the fix is a check that the deadline actually changed, not a
return to the exemption.
