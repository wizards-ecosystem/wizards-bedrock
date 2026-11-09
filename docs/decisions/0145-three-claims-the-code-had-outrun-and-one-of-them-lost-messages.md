# D-145 Three claims the code had outrun, and one of them lost messages

Status: Accepted, 2026-08-25.

**Context.** The prose audit's Tier 4 — comments that assert away a real
behaviour, which makes them defects rather than untidiness.

**A closed channel lost what it was sent.** `receive_blocking`'s doc says
`None` means "the far end closed with nothing left to read". The loop took a
message, then tested orphanhood, in **two separate lock acquisitions with
interrupts on between them**. A peer that sent *and then* closed in that window
got `None` returned with its message sitting in the queue — a lost message,
reported as a clean close, to a caller told that cannot happen.

**`await_key` claimed two bounds, and neither reached its inner loop.** The doc
says "doubly bounded: a deadline on the clock, and an iteration cap". Both are
on the outer `for`. Inside it the drain was `while let Some(event) =
keyboard.next_event()`, and `next_event`'s sixteen-byte budget is *per call* —
refreshed every iteration. A partially wedged controller that keeps a byte
latched with its status bit set spins there for ever, holding `KEYBOARD`'s leaf
guard, under a doc claiming it cannot. Selftest-only, so it could wedge the gate
rather than a shipped kernel.

**The module doc promised every event was queued.** `Events::push` drops on a
full sixteen-deep ring and counts it. Fast typing at a busy reader loses
keystrokes, which is a reasonable design and the opposite of what was written.

**And two `# Errors` blocks listed what could not happen or omitted what could.**
`Channel::send` listed `OutOfMemory`, which it cannot return — the inbox is
reserved to `DEPTH` at construction precisely so `push_back` never reaches the
allocator under a leaf guard, and the body's own comment three lines down says
so. `elf64::Elf::parse` omitted `WrongMachine`, which it checks.

**Decision.** The orphan arm drains rather than refuses: `return self.receive()`
makes the documented sentence true in both orderings. The drain takes a cap of
its own, two batches per pass, so a controller with more to say gets the next
pass and one that never stops saying it does not get the loop. The two docs are
corrected to what the code does.

**What the check establishes, and what it does not.** It pins the **ordering** —
draining before refusing. Test orphanhood first and it goes red. It does **not**
reproduce the window, and cannot single-threaded: the fixed line is reachable
only when a send lands between two acquisitions, so removing that line alone
leaves the check green because the take at the top of the loop finds the
message. That is written into the check rather than left for an audit, which is
the second time in this sweep a fix has landed with a check that pins its
neighbourhood rather than its point (D-141 was the first).

**Rationale.** All three prose defects share a shape: the comment describes the
**intent** of the surrounding code and the code implements it *almost*. Nobody
wrote "doubly bounded" carelessly — two bounds are there, in the loop the
author was looking at. The inner loop was added later, by someone who read the
doc and believed it.

That is the argument for treating a false comment as a defect rather than
untidiness, and it is stronger than "the reader is misled": the reader here is
the next author, and the comment is what tells them the hazard is handled.

**Consequences.**

- 145 checks. `receive_blocking` cannot report a clean close over a queued
  message in any ordering.
- `await_key`'s drain is bounded. Its doc says three bounds and names what each
  one catches.
- Two `# Errors` blocks match their bodies. Nothing enforces that they stay
  matched, and nothing can — `# Errors` is prose about a `Result`, and the
  compiler has no opinion about which variants a function can produce.

**Left owed.** The two-thread window in `receive_blocking` is unreproduced, as
is D-141's arena race. Both need concurrency this kernel does not yet have, and
both would need a flake budget to test — a decision for whoever adds a second
CPU, alongside D-126's `LEAVES_HELD` and D-129's `pic::silence` race, all four
of which are waiting on the same thing.
