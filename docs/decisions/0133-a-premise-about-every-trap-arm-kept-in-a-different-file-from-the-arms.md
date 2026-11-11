# D-133 A premise about every trap arm, kept in a different file from the arms

Status: Accepted, 2026-08-25.

**Decision.** `unblock` arms the timer whenever it woke anything, trap context
included. This keeps D-57's rule in `thread.rs`: *every path that changes who is
runnable ends by arming the timer for the next moment a decision is due, or
stopping it when there is none.* D-93's `in_trap` exemption rested on the
premise that every arm of `on_trap` reschedules (D-54), and the syscall arm
does not: `Call::RingEnter` can reach `unblock` through `Queue::serve` and
`WaitQueue::wake_all`, then return to ring 3 with nothing armed.

`queue.rs`'s `take` carries `debug_assert!` on its two unreachable paths, as
D-115 did to `serve`: a silent failure there would hold a reservation for ever
and wedge the ring, which is worse than a panic because nothing can diagnose it.

**Rationale.** A premise about a set of call sites was written next to the code
that depends on it rather than next to the sites, so the two drifted with
nothing to notice. Adding a reschedule to the syscall arm would restore the
agreement and leave the same trap for the next arm, and it changes when a user
thread can be preempted, a larger behaviour change than arming a timer that may
fire slightly early. Arming unconditionally removes the reason the two files
had to agree at all, for one APIC write.

**The check is about the arming, not the running.** A woken thread runs
eventually whatever this does, so a check that waited for it to run passes with
the bug present: *the failure is only observable on a machine that would
otherwise be idle, and a self-test is not idle.* `a wake from a handler arms
the timer for what it woke` reads `HANDLER_ARMS`, a counter of
wakes-from-a-handler that armed here, and requires the blocked reader to have
woken *and* the counter to have moved.

**Left owed at adoption; closed by D-223.** The check exercises the *keyboard*
path, because that is the only wake from a handler this kernel performs. **The
syscall path, the one that was actually broken, is not exercised**, and it is a
check that is *constructible and not built* rather than one that cannot be
built: the missing part is the orchestration to get a kernel thread blocked on
a program's ring *before* the program rings it, a race the check would have to
win rather than assume.

D-223 pays it by stopping a real child before ring-3 entry, registering a
kernel thread on that child's production ring wait queue, and only then
letting the child execute `RingEnter`. The old exemption now reddens the exact
syscall check rather than a keyboard proxy.

The redundant arming on the timer and keyboard paths is real and unmeasured. It
is one write to the APIC's initial-count register, on a path already committed
to a `reschedule` that writes it again; if that ever shows up in a latency
measurement, the fix is a check that the deadline actually changed, not a
return to the exemption.
